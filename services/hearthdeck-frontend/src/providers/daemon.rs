use futures::{StreamExt, future::join_all, stream};
use hearthdeck_protocol::{ApplicationSession, InputProfile};
use serde::Deserialize;
use std::{collections::BTreeMap, time::Duration};

use super::{
    art_cache, belongs_in_the_app_library, catalog_item_to_library_item,
    retro_game_to_library_item, retro_version_list,
};
use crate::icon_cache::{self, IconReference};
use crate::library_item::LibraryItem;
use crate::providers::filter::Facet;

/// Prefix a catalog item's id carries on its way into the library.
///
/// It is what ties a library entry back to the catalog row it came from, and it
/// is the id space a categorization report's `app_id`s have to be mapped into
/// before they can be used as a tab filter. It is never used to route a launch:
/// the daemon decides what an id names.
pub const CATALOG_ENTRY_PREFIX: &str = "hearthdeck:";

/// How long one daemon request may take before the client gives up.
///
/// Without a deadline a hung daemon leaves every poll chain waiting forever, and
/// the session poll that returns input to the pad never fires again.
const DAEMON_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Deserialize)]
pub struct RommSettings {
    pub base_url: String,
    pub configured: bool,
}

/// Configuration for connecting to the HearthDeck daemon.
#[derive(Clone, Debug)]
pub struct DaemonConfig {
    /// Base URL of the daemon API (e.g., "http://127.0.0.1:38400")
    pub base_url: String,
    /// Bearer token for authentication
    pub token: String,
}

/// Client for communicating with the HearthDeck daemon API.
#[derive(Clone)]
pub struct DaemonClient {
    transport: hearthdeck_client::Client,
}

pub use hearthdeck_protocol::{
    CatalogItem, CategorizationPhase, Collection, CollectionItem, HealthResponse, ModelStatus,
    RetroBiosSyncReport, RetroCoreInstallReport, RetroGame, RetroGameDetails, RetroRomVersion,
    RetroSystemReadiness, ScanReport, ServerEventKind as DaemonEvent,
};

/// The daemon's categorization state and shared scan report.
pub type CategorizationSnapshot = hearthdeck_protocol::CategorizationSnapshot<ScanReport>;

/// The curated collection a details screen toggles for favorites.
pub const FAVORITES_COLLECTION: &str = "favorites";

/// The curated collection a details screen toggles for play later.
pub const PLAY_LATER_COLLECTION: &str = "play-later";

/// The built-in collection whose items are the most recently played games.
pub const LAST_PLAYED_COLLECTION: &str = "last-played";

/// The Stremio account's rail.
///
/// Composed by the daemon from the records its Stremio provider published, so
/// unlike the model's rail this slug is the same string on both sides rather than an
/// owner plus an id.
pub const CONTINUE_WATCHING_COLLECTION: &str = "stremio:continue-watching";

/// What has just arrived on Stremio, resolved by the daemon from the records its
/// arrivals provider published.
///
/// The dashboard draws this one first, in front of the blank space at the top
/// of the page: its items carry the wide artwork the window's background draws
/// as the pad walks the rail, which no other collection's items have.
pub const NEW_ARRIVALS_COLLECTION: &str = "stremio:new-arrivals";

/// How many icons one rail, or one library page, resolves at a time.
///
/// A cached icon costs nothing and the ones that are not cached are the posters
/// the page is about to show, so this bounds a first paint rather than a shared
/// service: the daemon's asset route is loopback and RomM is the user's own.
const RAIL_ICON_CONCURRENCY: usize = 16;

/// A Stremio link the user approves on another device.
///
/// The daemon's answer also carries Stremio's `qr` image URL, and this client does
/// not model it: nothing draws that image yet (Phase 5 in
/// `docs/stremio-integration.md`), and a field nothing reads is a field that quietly
/// rots.
#[derive(Clone, Debug, Deserialize)]
pub struct StremioLinkOffer {
    /// The four characters the user enters in the Stremio app or site.
    pub code: String,
    /// The link to open, which already carries the code.
    pub link: String,
    /// How long the daemon will keep polling for this code.
    pub expires_in_seconds: u64,
}

/// The linked account, as the daemon reports it.
///
/// There is no field for the session: the daemon holds that and never sends it,
/// which is the whole reason the frontend can offer linking without ever seeing a
/// credential. `updated_at` is left out deliberately — the screen says when the
/// account was linked, and a re-link keeps that original time.
#[derive(Clone, Debug, Deserialize)]
pub struct StremioConnection {
    pub linked_at: String,
}

/// Where a pending link stands.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum StremioLink {
    /// No link is pending: never asked for, or already resolved.
    Idle,
    /// The window closed while the user was still deciding.
    Expired,
    /// Waiting on the user, who is approving `code` on another device.
    Waiting {
        code: String,
        link: String,
        seconds_left: u64,
    },
    /// Linked and stored by the daemon.
    Linked { connection: StremioConnection },
}

/// A RomM device code the user approves in RomM's web UI, usually by
/// scanning `link` with a phone on the same network.
#[derive(Clone, Debug, Deserialize)]
pub struct RommLinkOffer {
    pub code: String,
    pub link: String,
    pub expires_in_seconds: u64,
}

/// Where a pending RomM approval stands. Never carries the token.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RommLink {
    Idle,
    Expired,
    Denied,
    Waiting {
        code: String,
        link: String,
        seconds_left: u64,
    },
    Linked {
        settings: RommSettings,
    },
}

/// Response of the daemon's retro favorite write.
#[derive(Debug, Deserialize)]
struct FavoriteState {
    favorite: bool,
}

/// Response of the daemon's retro backlog write.
#[derive(Debug, Deserialize)]
struct BacklogState {
    backlogged: bool,
}

/// A game's detail payload with its artwork already cached on disk: the hero
/// cover plus every screenshot the details screen can show.
#[derive(Clone, Debug)]
pub struct RetroDetails {
    pub info: RetroGameDetails,
    /// Local path of the large cover, when one could be downloaded. `None`
    /// leaves the screen with no hero image.
    pub hero: Option<String>,
    /// Local paths of the game's screenshots, in RomM's order. A screenshot
    /// that fails to download is skipped rather than leaving a hole, so this
    /// is a filtered view of `info.screenshot_paths`.
    pub screenshots: Vec<String>,
}

/// Retro platform from the daemon's /v1/retro/consoles endpoint.
#[derive(Clone, Debug, Deserialize)]
pub struct RetroPlatform {
    pub id: i64,
    pub name: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub artwork_paths: Vec<String>,
}

impl RetroPlatform {
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }
}

/// A RomM console ready for the frontend: its label and, when one could be
/// cached, the local path to its bundled console artwork.
#[derive(Clone, Debug)]
pub struct RetroConsole {
    pub id: i64,
    pub label: String,
    pub artwork: Option<String>,
}

impl DaemonClient {
    /// Creates a new daemon client with the given configuration.
    pub fn new(config: DaemonConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(DAEMON_REQUEST_TIMEOUT)
            .build()
            .unwrap_or_else(|error| {
                log::error!("daemon client built without a timeout: {error}");
                reqwest::Client::new()
            });
        Self {
            transport: hearthdeck_client::Client::new(
                http,
                config.base_url,
                config.token,
                "hearthdeck-cosmic-frontend",
            ),
        }
    }

    /// Returns the base URL for API requests.
    fn api_url(&self, path: &str) -> String {
        self.transport.api_url(path)
    }

    pub fn base_url(&self) -> &str {
        self.transport.base_url()
    }

    /// Opens the daemon's change feed (`GET /v1/events`).
    pub async fn connect_events(&self) -> Result<EventStream, DaemonError> {
        use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

        let mut url = url::Url::parse(self.base_url()).map_err(DaemonError::InvalidBaseUrl)?;
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme).map_err(|()| {
            DaemonError::Events(tungstenite::Error::Url(
                tungstenite::error::UrlError::UnsupportedUrlScheme,
            ))
        })?;
        url.set_path("/v1/events");
        url.set_query(None);

        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(DaemonError::Events)?;
        request.headers_mut().extend(self.auth_headers().await?);
        let (stream, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(DaemonError::Events)?;
        Ok(stream)
    }

    /// Converts a non-success daemon response into a [`DaemonError`],
    /// preferring the daemon's JSON `{"error": ...}` body - which carries the
    /// bridge's launch-rejection reason (e.g. a RetroArch core/ROM validation
    /// failure) - over the bare status code, so the launch overlay can tell
    /// the user *why* a launch failed instead of only showing "502 Bad
    /// Gateway". Falls back to the status code when the body is not the
    /// daemon's error JSON (proxy errors, non-daemon endpoints).
    async fn http_error(response: reqwest::Response) -> DaemonError {
        hearthdeck_client::response_error(response).await.into()
    }

    /// Returns authorization headers for API requests.
    async fn auth_headers(&self) -> Result<reqwest::header::HeaderMap, DaemonError> {
        self.transport.auth_headers().await.map_err(Into::into)
    }

    /// Checks if the daemon is reachable and returns its health status.
    pub async fn health(&self) -> Result<HealthResponse, DaemonError> {
        let response = self
            .transport
            .http()
            .get(self.api_url("/v1/health"))
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Fetches the full catalog from the daemon.
    pub async fn fetch_library(&self) -> Result<Vec<CatalogItem>, DaemonError> {
        let request = self.transport.http().get(self.api_url("/v1/library"));
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Reads the categorization scan's state and its last report.
    pub async fn categorization(&self) -> Result<CategorizationSnapshot, DaemonError> {
        let request = self
            .transport
            .http()
            .get(self.api_url("/v1/categorization"));
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Asks the daemon to categorize the library and publish its dashboard rails.
    ///
    /// This returns as soon as the scan is accepted, because the scan itself
    /// runs for minutes in the daemon's own process: follow it with
    /// [`DaemonClient::categorization`]. Requesting a scan while one runs is not
    /// an error; the daemon coalesces the two.
    pub async fn start_categorization_scan(&self) -> Result<(), DaemonError> {
        self.post_empty("/v1/categorization/scan").await
    }

    /// Asks the model which applications are couch-friendly. Returns once the
    /// job is accepted; the verdicts arrive as a library change.
    pub async fn detect_form_factors(&self) -> Result<(), DaemonError> {
        self.post_empty("/v1/categorization/form-factors").await
    }

    /// Removes the dashboard rails the model published.
    pub async fn remove_categorization_collections(&self) -> Result<(), DaemonError> {
        let request = self
            .transport
            .http()
            .delete(self.api_url("/v1/categorization/collections"));
        self.send(request).await?;
        Ok(())
    }

    /// Turns smart categorization on, which only fetches the checkpoint when
    /// none is cached.
    pub async fn enable_categorization(&self) -> Result<(), DaemonError> {
        self.post_empty("/v1/categorization/enable").await
    }

    /// Turns it off. A scan already running is left to finish.
    pub async fn disable_categorization(&self) -> Result<(), DaemonError> {
        self.post_empty("/v1/categorization/disable").await
    }

    /// A POST that carries nothing and answers nothing.
    async fn post_empty(&self, path: &str) -> Result<(), DaemonError> {
        let request = self.transport.http().post(self.api_url(path));
        self.send(request).await.map(drop)
    }

    /// Sends an authenticated request; a non-success status becomes the
    /// daemon's own error.
    async fn send(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, DaemonError> {
        let response = request
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;
        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }
        Ok(response)
    }

    /// Launches whatever an item id names - the one route a provider-agnostic
    /// client posts to. The daemon resolves the id (catalog record, RomM rom)
    /// and picks the launch shape; this parses nothing.
    pub async fn launch_item(
        &self,
        item_id: &str,
        input_profile: InputProfile,
    ) -> Result<ApplicationSession, DaemonError> {
        let request = self
            .transport
            .http()
            .post(self.api_url(&format!("/v1/launch/{}", item_id)))
            .query(&[("input_profile", input_profile)]);
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Gets the currently active application session.
    pub async fn active_session(&self) -> Result<Option<ApplicationSession>, DaemonError> {
        let request = self
            .transport
            .http()
            .get(self.api_url("/v1/sessions/active"));
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Lists available retro consoles from RomM, caching their artwork.
    pub async fn list_retro_consoles(&self) -> Result<Vec<RetroConsole>, DaemonError> {
        let request = self
            .transport
            .http()
            .get(self.api_url("/v1/retro/consoles"));
        let response = self.send(request).await?;
        let platforms: Vec<RetroPlatform> = response
            .json()
            .await
            .map_err(DaemonError::Deserialization)?;

        let mut consoles = Vec::new();
        for platform in platforms {
            let artwork = if !platform.artwork_paths.is_empty() {
                // Try each artwork path until one loads
                let mut result = None;
                for path in &platform.artwork_paths {
                    if let Some(cached) = self.cache_retro_asset_path(Some(path)).await {
                        result = Some(cached);
                        break;
                    }
                }
                result
            } else {
                None
            };
            consoles.push(RetroConsole {
                id: platform.id,
                label: platform.label().to_owned(),
                artwork,
            });
        }
        Ok(consoles)
    }

    /// Whether each platform with roms can play: its core installed and its
    /// BIOS files in place. Read whenever the settings screen opens.
    pub async fn retro_readiness(&self) -> Result<Vec<RetroSystemReadiness>, DaemonError> {
        let request = self
            .transport
            .http()
            .get(self.api_url("/v1/retro/readiness"));
        self.send(request)
            .await?
            .json()
            .await
            .map_err(DaemonError::Deserialization)
    }

    /// Places the BIOS files the active platforms are missing, fetching only
    /// those from the user's own BIOS source.
    pub async fn sync_retro_bios(&self) -> Result<RetroBiosSyncReport, DaemonError> {
        let request = self
            .transport
            .http()
            .post(self.api_url("/v1/retro/readiness/sync"));
        self.send(request)
            .await?
            .json()
            .await
            .map_err(DaemonError::Deserialization)
    }

    /// Installs the missing libretro core packages through the daemon's
    /// polkit-scoped helper. The daemon names the packages itself.
    pub async fn install_retro_cores(&self) -> Result<RetroCoreInstallReport, DaemonError> {
        let request = self
            .transport
            .http()
            .post(self.api_url("/v1/retro/cores/install"));
        self.send(request)
            .await?
            .json()
            .await
            .map_err(DaemonError::Deserialization)
    }

    /// Lists retro ROMs, optionally narrowed by platform, a search term, and
    /// facet selections (facet id -> value, e.g. `genre` -> `Action`).
    pub async fn list_retro_roms(
        &self,
        platform_id: Option<i64>,
        search: Option<&str>,
        filters: &BTreeMap<String, String>,
        limit: u32,
        offset: u32,
    ) -> Result<RetroGamesResponse, DaemonError> {
        let params = retro_rom_query_params(platform_id, search, filters, limit, offset);
        let url = format!("{}?{}", self.api_url("/v1/retro/roms"), params.join("&"));

        let request = self.transport.http().get(&url);
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// The filter facets the current console scope offers: every genre and
    /// region the source reports, not just the ones a loaded page happens to
    /// carry, so the grid can offer complete options before it pages anything.
    pub async fn retro_facets(&self, platform_id: Option<i64>) -> Result<Vec<Facet>, DaemonError> {
        let mut url = self.api_url("/v1/retro/facets");
        if let Some(platform_id) = platform_id {
            url = format!("{url}?platform_id={platform_id}");
        }

        let request = self.transport.http().get(&url);
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Lists one page of retro records, converted and down to their covers.
    ///
    /// `search` is the source's own search, not a filter over the page that
    /// comes back: a grid that searched only what it had loaded would find
    /// nothing past the first page.
    pub async fn list_retro_items(
        &self,
        platform_id: Option<i64>,
        search: Option<&str>,
        filters: &BTreeMap<String, String>,
        limit: u32,
        offset: u32,
    ) -> Result<RetroLibraryPage, DaemonError> {
        let page = self
            .list_retro_roms(platform_id, search, filters, limit, offset)
            .await?;
        Ok(self.resolve_retro_page(page).await)
    }

    async fn resolve_retro_page(&self, page: RetroGamesResponse) -> RetroLibraryPage {
        let mut versions = BTreeMap::new();
        let items = stream::iter(page.items.into_iter().map(|game| {
            versions.insert(format!("romm:{}", game.id), retro_version_list(&game));
            let client = self.clone();
            async move {
                // The cover is a RomM path, which the same resolver every other
                // record's icon goes through turns into a cached file.
                let icon = client.resolve_icon(game.cover_path.as_deref()).await;
                retro_game_to_library_item(game, icon)
            }
        }))
        // A page is only returned once every cover in it is cached, so this
        // concurrency directly bounds first-paint latency; the daemon asset
        // endpoint is loopback/served by RomM, so 16 parallel covers is safe.
        .buffered(RAIL_ICON_CONCURRENCY)
        .collect()
        .await;
        RetroLibraryPage {
            items,
            versions,
            total: page.total,
            offset: page.offset,
        }
    }

    /// Downloads one RomM asset (cover, screenshot, console artwork) through
    /// the daemon's proxy and returns its local cached path, reusing an
    /// already-cached copy when there is one.
    async fn cache_retro_asset_path(&self, path: Option<&str>) -> Option<String> {
        let path = path?;
        let key = format!("romm:{path}");
        if let Some(cached) = art_cache::cached(&key) {
            return Some(cached);
        }
        let mut url = url::Url::parse(&self.api_url("/v1/retro/assets")).ok()?;
        url.query_pairs_mut().append_pair("path", path);
        let request = self
            .transport
            .http()
            .get(url)
            .headers(self.auth_headers().await.ok()?);
        art_cache::download(request, key).await
    }

    /// Fetches one game's full detail plus its artwork, both already cached
    /// locally, so the details screen paints without a second round trip.
    pub async fn retro_rom_details(&self, rom_id: i64) -> Result<RetroDetails, DaemonError> {
        let request = self
            .transport
            .http()
            .get(self.api_url(&format!("/v1/retro/roms/{rom_id}")));
        let response = self.send(request).await?;
        let info: RetroGameDetails = response
            .json()
            .await
            .map_err(DaemonError::Deserialization)?;

        // The screen's hero falls back to the tile cover, which may be the only
        // artwork the game has.
        let hero = self
            .cache_retro_asset_path(
                info.cover_large_path
                    .as_deref()
                    .or(info.cover_path.as_deref()),
            )
            .await;
        let screenshot_paths = info.screenshot_paths.clone();
        // Bounded concurrency, as in `list_retro_items`: the daemon asset
        // endpoint is loopback (or the user's own RomM), so a handful of
        // parallel image fetches is cheaper than a serial wait.
        let screenshots: Vec<String> = stream::iter(screenshot_paths)
            .map(|path| {
                let client = self.clone();
                async move { client.cache_retro_asset_path(Some(&path)).await }
            })
            .buffered(8)
            .filter_map(|cached| async move { cached })
            .collect()
            .await;

        Ok(RetroDetails {
            info,
            hero,
            screenshots,
        })
    }

    /// The dashboard's collections, in composition order.
    ///
    /// Every item's icon is resolved on the way out, so a rail arrives drawable
    /// rather than as a set of references each surface would have to interpret
    /// for itself - which is how the dashboard ended up looking its own items up
    /// and drawing a generic mark for anything its library did not also hold.
    pub async fn list_collections(&self) -> Result<Vec<Collection>, DaemonError> {
        let request = self.transport.http().get(self.api_url("/v1/collections"));
        let response = self.send(request).await?;
        let collections: Vec<Collection> = response
            .json()
            .await
            .map_err(DaemonError::Deserialization)?;
        Ok(self.resolve_rail_icons(collections).await)
    }

    /// The same collections, with every item's art resolved.
    ///
    /// Bounded rather than one at a time: a rail of two dozen posters is a page
    /// the user is looking at, and an already-cached one costs nothing. The
    /// window's wide art and wordmark go through the same single place as a
    /// card's poster, so the background draws a file and never the URL it was
    /// sent.
    async fn resolve_rail_icons(&self, collections: Vec<Collection>) -> Vec<Collection> {
        stream::iter(collections)
            .map(|mut collection| {
                let client = self.clone();
                async move {
                    collection.items = stream::iter(collection.items)
                        .map(|mut item| {
                            let client = client.clone();
                            async move {
                                item.icon = client.resolve_icon(item.icon.as_deref()).await;
                                // The window's wide art is the coldest art
                                // there is - it belongs to titles that have
                                // just arrived - so it is the one a fetch is
                                // most likely to lose. Losing it is worth a
                                // line in the log rather than a background
                                // that quietly became the wallpaper.
                                if item.background.is_some() {
                                    let background =
                                        client.resolve_icon(item.background.as_deref()).await;
                                    if background.is_none() {
                                        log::warn!(
                                            "the wide art for {} could not be fetched; the \
                                             background falls back to the wallpaper",
                                            item.name
                                        );
                                    }
                                    item.background = background;
                                }
                                item.logo = client.resolve_icon(item.logo.as_deref()).await;
                                item
                            }
                        })
                        .buffered(RAIL_ICON_CONCURRENCY)
                        .collect()
                        .await;
                    collection
                }
            })
            .buffered(RAIL_ICON_CONCURRENCY)
            .collect()
            .await
    }

    /// One icon reference, resolved into something a view can draw.
    ///
    /// The single place a reference becomes artwork, so every record and rail
    /// item a surface receives has been through it: a poster is downloaded into
    /// the icon cache, RomM's own artwork is fetched through the daemon's asset
    /// route, and a file or a theme name is already drawable. `None` - there was
    /// no reference, or fetching it failed - is the generic mark, because a card
    /// with no artwork is drawn as the mark rather than as a path that does not
    /// exist.
    pub async fn resolve_icon(&self, reference: Option<&str>) -> Option<String> {
        match icon_cache::classify(reference?)? {
            IconReference::Remote(url) => {
                // The same art is retried at Metahub's other sizes when the one
                // the feed sent will not come down - see
                // `icon_cache::art_candidates`.
                for candidate in icon_cache::art_candidates(url) {
                    if let Some(cached) = art_cache::cached(&candidate) {
                        return Some(cached);
                    }
                    // Third-party hosts: no auth headers, and a shorter leash
                    // than the daemon gets.
                    let request = self
                        .transport
                        .http()
                        .get(&candidate)
                        .timeout(art_cache::DOWNLOAD_TIMEOUT);
                    if let Some(cached) = art_cache::download(request, candidate).await {
                        return Some(cached);
                    }
                }
                None
            }
            IconReference::RommAsset(path) => self.cache_retro_asset_path(Some(path)).await,
            IconReference::File(path) => Some(path.to_owned()),
            IconReference::Name(name) => Some(name.to_owned()),
        }
    }

    /// Adds one item to a curated collection, answering with the collection as it
    /// now stands.
    pub async fn add_collection_item(
        &self,
        slug: &str,
        item: &CollectionItem,
    ) -> Result<Collection, DaemonError> {
        let request = self
            .transport
            .http()
            .post(self.api_url(&format!("/v1/collections/{slug}/items")))
            .json(&serde_json::json!({
                "item_id": item.item_id,
                "name": item.name,
                "icon": item.icon,
            }));
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Removes one item from a curated collection, answering with the collection
    /// as it now stands.
    pub async fn remove_collection_item(
        &self,
        slug: &str,
        item_id: &str,
    ) -> Result<Collection, DaemonError> {
        let request = self
            .transport
            .http()
            .delete(self.api_url(&format!("/v1/collections/{slug}/items")))
            .json(&serde_json::json!({ "item_id": item_id }));
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Asks the daemon for a Stremio link code.
    ///
    /// The code is approved on another device, which is why nothing here sends a
    /// password: the daemon does not accept one, so there is none to send.
    pub async fn start_stremio_link(&self) -> Result<StremioLinkOffer, DaemonError> {
        let request = self.transport.http().post(self.api_url("/v1/stremio/link"));
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// One poll of the pending link.
    ///
    /// The daemon is what talks to Stremio, so neither the code nor the session
    /// crosses this call: it only asks what the daemon found.
    pub async fn poll_stremio_link(&self) -> Result<StremioLink, DaemonError> {
        let request = self.transport.http().get(self.api_url("/v1/stremio/link"));
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// The linked Stremio account, or `None` when none is linked.
    pub async fn stremio_connection(&self) -> Result<Option<StremioConnection>, DaemonError> {
        let request = self
            .transport
            .http()
            .get(self.api_url("/v1/stremio/settings"));
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    pub async fn romm_settings(&self) -> Result<Option<RommSettings>, DaemonError> {
        let request = self
            .transport
            .http()
            .get(self.api_url("/v1/retro/settings"));
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Signs in to RomM through the daemon, which mints and keeps a client
    /// token. The password crosses this call once and is stored nowhere.
    pub async fn romm_login(
        &self,
        base_url: &str,
        username: &str,
        password: &str,
    ) -> Result<RommSettings, DaemonError> {
        let request = self
            .transport
            .http()
            .post(self.api_url("/v1/retro/login"))
            .json(&serde_json::json!({
                "base_url": base_url,
                "username": username,
                "password": password,
            }));
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Asks the daemon for a RomM device code the user approves on a phone.
    pub async fn start_romm_link(&self, base_url: &str) -> Result<RommLinkOffer, DaemonError> {
        let request = self
            .transport
            .http()
            .post(self.api_url("/v1/retro/link"))
            .json(&serde_json::json!({ "base_url": base_url }));
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    pub async fn poll_romm_link(&self) -> Result<RommLink, DaemonError> {
        let request = self.transport.http().get(self.api_url("/v1/retro/link"));
        let response = self.send(request).await?;
        response.json().await.map_err(DaemonError::Deserialization)
    }

    pub async fn clear_romm_settings(&self) -> Result<(), DaemonError> {
        let request = self
            .transport
            .http()
            .delete(self.api_url("/v1/retro/settings"));
        self.send(request).await?;
        Ok(())
    }

    /// Unlinks the account: the session and the rows it published go together.
    pub async fn unlink_stremio(&self) -> Result<(), DaemonError> {
        let request = self
            .transport
            .http()
            .delete(self.api_url("/v1/stremio/settings"));
        self.send(request).await?;
        Ok(())
    }

    /// Asks the daemon to refresh one discovery source now.
    ///
    /// Used after a Stremio link resolves: the provider would get there on its own
    /// interval, but a person who just approved a code expects the dashboard to
    /// fill in front of them.
    pub async fn refresh_source(&self, source_id: &str) -> Result<(), DaemonError> {
        self.post_empty(&format!("/v1/discovery/{source_id}/refresh"))
            .await
    }

    /// Adds or removes one game from the user's RomM favorites, returning the
    /// state that then holds. RomM owns the favorite list, so this only proxies
    /// the change.
    pub async fn set_retro_favorite(
        &self,
        rom_id: i64,
        favorite: bool,
    ) -> Result<bool, DaemonError> {
        let request = self
            .transport
            .http()
            .post(self.api_url(&format!("/v1/retro/roms/{rom_id}/favorite")))
            .json(&serde_json::json!({ "favorite": favorite }));
        let response = self.send(request).await?;
        let state: FavoriteState = response
            .json()
            .await
            .map_err(DaemonError::Deserialization)?;
        Ok(state.favorite)
    }

    /// Sets RomM's per-user "play later" flag for one game, returning the state
    /// that then holds.
    pub async fn set_retro_backlogged(
        &self,
        rom_id: i64,
        backlogged: bool,
    ) -> Result<bool, DaemonError> {
        let request = self
            .transport
            .http()
            .post(self.api_url(&format!("/v1/retro/roms/{rom_id}/backlog")))
            .json(&serde_json::json!({ "backlogged": backlogged }));
        let response = self.send(request).await?;
        let state: BacklogState = response
            .json()
            .await
            .map_err(DaemonError::Deserialization)?;
        Ok(state.backlogged)
    }
}

/// The query parameters for `/v1/retro/roms`, in the order the daemon receives
/// them. A facet selection's key is already the daemon route's own parameter
/// name (`genre`, `region`), so it passes through unchanged; an unknown id is
/// harmless, and the route ignores it.
fn retro_rom_query_params(
    platform_id: Option<i64>,
    search: Option<&str>,
    filters: &BTreeMap<String, String>,
    limit: u32,
    offset: u32,
) -> Vec<String> {
    let mut params = Vec::new();
    if let Some(platform_id) = platform_id {
        params.push(format!("platform_id={platform_id}"));
    }
    if let Some(search) = search.filter(|search| !search.is_empty()) {
        params.push(format!("q={}", urlencoding::encode(search)));
    }
    for (facet, value) in filters {
        params.push(format!("{facet}={}", urlencoding::encode(value)));
    }
    params.push(format!("limit={limit}"));
    params.push(format!("offset={offset}"));
    params
}

pub type EventStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("connection failed: {0}")]
    Connection(reqwest::Error),

    #[error("unexpected status: {0}")]
    UnexpectedStatus(reqwest::StatusCode),

    #[error("{status}: {message}")]
    Api {
        status: reqwest::StatusCode,
        message: String,
    },

    #[error("deserialization failed: {0}")]
    Deserialization(reqwest::Error),

    #[error("invalid daemon base URL: {0}")]
    InvalidBaseUrl(url::ParseError),

    #[error("invalid pairing token: {0}")]
    InvalidToken(reqwest::header::InvalidHeaderValue),

    #[error("automatic pairing requires a loopback daemon")]
    PairingRequiresLoopback,

    #[error("event feed failed: {0}")]
    Events(tokio_tungstenite::tungstenite::Error),
}

impl From<hearthdeck_client::Error> for DaemonError {
    fn from(error: hearthdeck_client::Error) -> Self {
        match error {
            hearthdeck_client::Error::Connection(error) => Self::Connection(error),
            hearthdeck_client::Error::UnexpectedStatus(status) => Self::UnexpectedStatus(status),
            hearthdeck_client::Error::Api { status, message } => Self::Api { status, message },
            hearthdeck_client::Error::Deserialization(error) => Self::Deserialization(error),
            hearthdeck_client::Error::InvalidBaseUrl(error) => Self::InvalidBaseUrl(error),
            hearthdeck_client::Error::InvalidToken(error) => Self::InvalidToken(error),
            hearthdeck_client::Error::PairingRequiresLoopback => Self::PairingRequiresLoopback,
        }
    }
}

impl DaemonClient {
    /// Fetch library entries with their icons resolved before display.
    ///
    /// # Errors
    /// Returns the original [`DaemonError`] if fetching the catalog fails.
    pub async fn fetch_library_items(&self) -> Result<Vec<LibraryItem>, DaemonError> {
        let items = self.fetch_library().await?;

        log::info!("fetched {} catalog items from daemon", items.len());

        let items = join_all(items.into_iter().map(|mut item| async move {
            item.icon = self.resolve_icon(item.icon.as_deref()).await;
            item
        }))
        .await;

        // Films and series are not applications. The rail draws them from the daemon's
        // own collection, so leaving them here only put them in the All Apps grid,
        // sorted in among the programs.
        Ok(items
            .into_iter()
            .filter(belongs_in_the_app_library)
            .map(catalog_item_to_library_item)
            .collect())
    }
}

/// Response from the /v1/retro/roms endpoint.
#[derive(Clone, Debug, Deserialize)]
pub struct RetroGamesResponse {
    pub items: Vec<RetroGame>,
    pub total: u64,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct RetroLibraryPage {
    pub items: Vec<LibraryItem>,
    pub versions: BTreeMap<String, Vec<RetroRomVersion>>,
    pub total: u64,
    pub offset: u32,
}

#[cfg(test)]
mod tests {
    use crate::form_factor::{FormFactor, of};

    use super::{
        CatalogItem, Collection, CollectionItem, DaemonClient, DaemonConfig, DaemonError,
        DaemonEvent, RetroGame, RetroRomVersion, belongs_in_the_app_library,
        catalog_item_to_library_item, retro_game_to_library_item, retro_rom_query_params,
    };

    #[tokio::test]
    async fn library_loading_preserves_invalid_token_errors() {
        let client = DaemonClient::new(DaemonConfig {
            base_url: "http://127.0.0.1:0".to_owned(),
            token: "invalid\nheader".to_owned(),
        });

        assert!(matches!(
            client.fetch_library_items().await,
            Err(DaemonError::InvalidToken(_))
        ));
    }

    #[test]
    fn daemon_events_are_read_by_kind_and_unknown_kinds_are_tolerated() {
        let parse = |json: &str| serde_json::from_str::<DaemonEvent>(json).unwrap();

        assert_eq!(
            parse(r#"{"type":"library_changed","source_id":"romm","record_count":3}"#),
            DaemonEvent::LibraryChanged
        );
        assert_eq!(
            parse(r#"{"type":"application_session_changed","session":null}"#),
            DaemonEvent::ApplicationSessionChanged
        );
        assert_eq!(parse(r#"{"type":"from_the_future"}"#), DaemonEvent::Unknown);
    }

    /// A reference that is already drawable is resolved to itself, so every
    /// record and rail item a surface receives holds something it can draw
    /// rather than a reference it would have to interpret for itself. None of
    /// these branches touches the network, which is the point: a file and a
    /// theme name are not fetched, they are read.
    #[tokio::test]
    async fn an_already_drawable_reference_resolves_to_itself() {
        let client = DaemonClient::new(DaemonConfig {
            // Nothing in this test is dialled; the port only has to be a real one.
            base_url: "http://127.0.0.1:38400".to_owned(),
            token: String::new(),
        });

        // No reference at all, and one that names nothing: both draw the mark.
        assert_eq!(client.resolve_icon(None).await, None);
        assert_eq!(client.resolve_icon(Some("")).await, None);

        let cached = "/home/user/.cache/hearthdeck/icons/1a2b3c4d.png";
        assert_eq!(
            client.resolve_icon(Some(cached)).await.as_deref(),
            Some(cached)
        );

        // Artwork a launcher wrote as a URL is the same file with the scheme off.
        assert_eq!(
            client
                .resolve_icon(Some("file:///home/user/art/cover.png"))
                .await
                .as_deref(),
            Some("/home/user/art/cover.png")
        );

        assert_eq!(
            client
                .resolve_icon(Some("org.example.App"))
                .await
                .as_deref(),
            Some("org.example.App")
        );
    }

    /// The window's art travels on the item itself, and an item from a
    /// collection that carries none of it is still the item it always was: a
    /// plain rail card's payload must not have grown a row of nulls.
    #[test]
    fn an_item_carries_its_art_and_a_plain_item_still_parses() {
        let banner: CollectionItem = serde_json::from_str(
            r#"{"item_id":"hearthdeck:stremio-arrivals:tt1","name":"A series",
                "background":"https://images.metahub.space/background/medium/tt1/img",
                "logo":"https://images.metahub.space/logo/medium/tt1/img",
                "description":"About A series"}"#,
        )
        .unwrap();
        assert_eq!(
            banner.background.as_deref(),
            Some("https://images.metahub.space/background/medium/tt1/img")
        );
        assert_eq!(
            banner.logo.as_deref(),
            Some("https://images.metahub.space/logo/medium/tt1/img")
        );

        let rail: CollectionItem =
            serde_json::from_str(r#"{"item_id":"hearthdeck:romm:1","name":"A game"}"#).unwrap();
        assert!(rail.background.is_none());
        assert!(rail.logo.is_none());
    }

    /// The window's art is resolved the way a rail card's poster is — through
    /// the one place a reference becomes artwork — so the background draws the
    /// file it resolved to rather than the reference it was sent.
    #[tokio::test]
    async fn an_items_art_is_resolved_before_the_surface_sees_it() {
        let client = DaemonClient::new(DaemonConfig {
            // Nothing in this test is dialled; the port only has to be a real one.
            base_url: "http://127.0.0.1:38400".to_owned(),
            token: String::new(),
        });

        let resolved = client
            .resolve_rail_icons(vec![Collection {
                slug: "stremio:new-arrivals".to_owned(),
                kind: "membership".to_owned(),
                rule: None,
                role: String::new(),
                position: 0,
                owner: String::new(),
                name: None,
                items: vec![CollectionItem {
                    item_id: "hearthdeck:stremio-arrivals:tt1".to_owned(),
                    name: "A series".to_owned(),
                    icon: Some("file:///home/user/art/poster.png".to_owned()),
                    background: Some("file:///home/user/art/background.png".to_owned()),
                    logo: Some("file:///home/user/art/logo.png".to_owned()),
                    description: None,
                    played: None,
                }],
            }])
            .await;

        let item = &resolved[0].items[0];
        assert_eq!(item.icon.as_deref(), Some("/home/user/art/poster.png"));
        assert_eq!(
            item.background.as_deref(),
            Some("/home/user/art/background.png")
        );
        assert_eq!(item.logo.as_deref(), Some("/home/user/art/logo.png"));
    }

    /// A film or a series is not an application. The daemon serves both from one
    /// endpoint, and without this rule a Continue Watching title was listed in All Apps
    /// beside the programs. It is keyed on the record's own kind, so the next media
    /// provider inherits it rather than needing to be named here.
    #[test]
    fn only_applications_and_games_reach_the_app_library() {
        let item = |kind: &str| -> CatalogItem {
            serde_json::from_value(serde_json::json!({
                "id": "stremio:tt1",
                "source_id": "stremio",
                "title": "A title",
                "kind": kind,
                "launch_id": null,
                "icon": null,
                "metadata": {},
                "classification": {
                    "section": "applications", "streaming": false, "form_factor": "desktop"
                },
            }))
            .unwrap()
        };

        assert!(belongs_in_the_app_library(&item("application")));
        assert!(belongs_in_the_app_library(&item("game")));
        assert!(belongs_in_the_app_library(&item("GAME")));

        // What this rule exists for.
        assert!(!belongs_in_the_app_library(&item("movie")));
        assert!(!belongs_in_the_app_library(&item("series")));
        // And anything else a provider invents, without that provider being named.
        assert!(!belongs_in_the_app_library(&item("channel")));

        // A record with no kind is kept: losing something from the library would be
        // worse than showing it.
        assert!(belongs_in_the_app_library(&item("")));
    }

    #[test]
    fn an_applications_source_reaches_the_record() {
        let item = CatalogItem {
            id: "hearthdeck:org.example.App".to_owned(),
            source_id: "desktop-apps".to_owned(),
            title: "Example".to_owned(),
            kind: "application".to_owned(),
            launch_id: Some("org.example.App.desktop".to_owned()),
            icon: None,
            metadata: serde_json::json!({ "categories": ["Utility"], "source": "flatpak" }),
            classification: hearthdeck_protocol::CatalogClassification::default(),
        };

        let record = catalog_item_to_library_item(item);

        assert_eq!(record.sources, ["flatpak"]);
    }

    #[test]
    fn catalog_form_verdicts_come_from_the_daemon_not_local_name_rules() {
        for (id, expected) in [
            ("org.gimp.GIMP", FormFactor::Couch),
            ("net.stremio.Stremio", FormFactor::Desktop),
        ] {
            let record = catalog_item_to_library_item(CatalogItem {
                id: id.to_owned(),
                source_id: hearthdeck_protocol::sources::DESKTOP_APPS.to_owned(),
                title: id.to_owned(),
                kind: "application".to_owned(),
                launch_id: None,
                icon: None,
                metadata: serde_json::json!({"source": "flatpak", "form_factor": "couch"}),
                classification: hearthdeck_protocol::CatalogClassification {
                    form_factor: Some(expected),
                    ..hearthdeck_protocol::CatalogClassification::default()
                },
            });

            assert_eq!(of(&record), expected, "id: {id}");
            assert!(record.sources.contains(&"flatpak".to_owned()));
        }
    }

    #[test]
    fn optional_catalog_fields_keep_their_tolerant_defaults() {
        for metadata in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({"store": [], "source": false, "form_factor": 42}),
        ] {
            let record = catalog_item_to_library_item(CatalogItem {
                id: "example".to_owned(),
                source_id: "custom".to_owned(),
                title: "Example".to_owned(),
                kind: "application".to_owned(),
                launch_id: None,
                icon: None,
                metadata: metadata.clone(),
                classification: hearthdeck_protocol::CatalogClassification::default(),
            });
            assert_eq!(record.store, None, "metadata: {metadata}");
            assert!(record.sources.is_empty(), "metadata: {metadata}");
            assert_eq!(
                record.form_factor,
                Some(FormFactor::Desktop),
                "metadata: {metadata}"
            );
        }
    }

    #[test]
    fn catalog_item_maps_directly_to_library_item_with_prefix() {
        let item = CatalogItem {
            id: "test:app".to_string(),
            source_id: "desktop-apps".to_string(),
            title: "Test App".to_string(),
            kind: "application".to_string(),
            launch_id: Some("test.app".to_string()),
            icon: Some("/usr/share/icons/test.png".to_string()),
            metadata: serde_json::json!({
                "categories": ["Utility", "Game"],
                "summary": "A test application",
            }),
            classification: hearthdeck_protocol::CatalogClassification::default(),
        };

        let record = catalog_item_to_library_item(item);

        assert!(record.id.starts_with("hearthdeck:"));
        assert_eq!(record.id, "hearthdeck:test:app");
        assert_eq!(record.name, "Test App");
        assert_eq!(record.exec, Some("test.app".to_string()));
        assert_eq!(record.categories, vec!["Utility"]);
    }

    #[test]
    fn heroic_items_keep_the_store_for_filtering() {
        let item = CatalogItem {
            id: "heroic:epic:Fortnite".to_string(),
            source_id: "heroic".to_string(),
            title: "Fortnite".to_string(),
            kind: "game".to_string(),
            launch_id: Some("legendary:Fortnite".to_string()),
            icon: None,
            metadata: serde_json::json!({
                "store": "Epic",
                "runner": "legendary",
            }),
            classification: hearthdeck_protocol::CatalogClassification {
                section: hearthdeck_protocol::Section::PcGames,
                streaming: false,
                form_factor: None,
            },
        };

        let record = catalog_item_to_library_item(item);

        assert_eq!(record.categories, vec!["Game"]);
        assert_eq!(record.store.as_deref(), Some("Epic"));
    }

    #[test]
    fn missing_launch_id_yields_none_exec() {
        let item = CatalogItem {
            id: "flatpak:org.example.App".to_string(),
            source_id: "flatpak".to_string(),
            title: "Example".to_string(),
            kind: "application".to_string(),
            launch_id: None,
            icon: None,
            metadata: serde_json::json!({}),
            classification: hearthdeck_protocol::CatalogClassification::default(),
        };

        let record = catalog_item_to_library_item(item);

        assert!(record.exec.is_none());
    }

    #[test]
    fn romm_game_maps_to_a_console_record_with_dedicated_id() {
        let record = retro_game_to_library_item(
            RetroGame {
                id: 42,
                platform_id: 7,
                title: "Example ROM".to_string(),
                summary: Some("A console game".to_string()),
                cover_path: Some("/assets/example.jpg".to_string()),
                cover_url: None,
                screenshot_paths: Vec::new(),
                has_manual: false,
                genres: vec!["RPG".to_string()],
                player_count: None,
                release_year: Some(1994),
                regions: vec!["Japan".to_string()],
                sibling_roms: Vec::new(),
                version_label: None,
            },
            Some("/tmp/example.jpg".to_string()),
        );

        assert_eq!(record.id, "romm:42");
        assert_eq!(record.exec.as_deref(), Some("42"));
        assert!(matches!(
            &record.icon,
            cosmic::desktop::fde::IconSource::Path(path) if path == std::path::Path::new("/tmp/example.jpg")
        ));
        assert_eq!(record.categories, ["Game"]);
        let item = record;
        let console = item.console.as_ref().expect("console metadata");
        assert_eq!(console.platform_id, 7);
        assert_eq!(console.genres, ["RPG"]);
        assert_eq!(console.regions, ["Japan"]);
        assert!(item.matches_category("hearthdeck-console:7"));
        assert!(item.matches_category("hearthdeck-genre:rpg"));
        assert!(item.matches_facet("hearthdeck-region:Japan"));
    }

    /// A facet selection travels as the daemon route's own parameter names, so
    /// the server, not the loaded page, does the filtering.
    #[test]
    fn a_facet_selection_becomes_the_daemons_filter_parameters() {
        let filters = std::collections::BTreeMap::from([
            ("genre".to_owned(), "Action".to_owned()),
            ("region".to_owned(), "USA".to_owned()),
        ]);

        let params = retro_rom_query_params(Some(7), None, &filters, 60, 0);

        assert!(params.contains(&"platform_id=7".to_owned()));
        assert!(params.contains(&"genre=Action".to_owned()));
        assert!(params.contains(&"region=USA".to_owned()));
        assert!(params.contains(&"limit=60".to_owned()));
        assert!(params.contains(&"offset=0".to_owned()));
    }

    /// The console grid's search is the source's own search, so it travels as
    /// `q` and the daemon narrows the whole library with it. An empty term is
    /// no term: the parameter is left off rather than sent blank.
    #[test]
    fn a_search_term_becomes_the_daemons_own_query_parameter() {
        let params =
            retro_rom_query_params(None, Some("street fighter"), &Default::default(), 60, 0);
        assert!(params.contains(&"q=street%20fighter".to_owned()));

        let unsearched = retro_rom_query_params(None, Some(""), &Default::default(), 60, 0);
        assert!(unsearched.iter().all(|param| !param.starts_with("q=")));
    }

    #[test]
    fn romm_sibling_versions_reach_the_record_without_json_conversion() {
        let game = RetroGame {
            id: 42,
            platform_id: 7,
            title: "Shenmue".to_string(),
            summary: None,
            cover_path: None,
            cover_url: None,
            screenshot_paths: Vec::new(),
            has_manual: false,
            genres: Vec::new(),
            player_count: None,
            release_year: None,
            regions: Vec::new(),
            sibling_roms: vec![
                RetroRomVersion {
                    id: 43,
                    title: "Shenmue (Disc 2)".to_string(),
                    is_main_sibling: true,
                },
                RetroRomVersion {
                    id: 44,
                    title: "Shenmue (Disc 3)".to_string(),
                    is_main_sibling: false,
                },
            ],
            version_label: Some("Shenmue (Disc 1)".to_string()),
        };
        let versions = super::retro_version_list(&game);
        assert_eq!(versions.len(), 3);
        assert_eq!(versions[0].id, 42);
        assert_eq!(versions[0].title, "Shenmue (Disc 1)");
        assert_eq!(versions[1].id, 43);
        assert!(versions[1].is_main_sibling);
        assert_eq!(versions[2].title, "Shenmue (Disc 3)");
    }

    fn record_with_siblings(label: Option<&str>, sibling_roms: Vec<RetroRomVersion>) -> RetroGame {
        RetroGame {
            id: 42,
            platform_id: 7,
            title: "Metal Gear Solid".to_owned(),
            summary: None,
            cover_path: None,
            cover_url: None,
            screenshot_paths: Vec::new(),
            has_manual: false,
            genres: Vec::new(),
            player_count: None,
            release_year: None,
            regions: Vec::new(),
            sibling_roms,
            version_label: label.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn retro_pages_return_display_items_and_matching_version_lists() {
        let client = DaemonClient::new(DaemonConfig {
            base_url: "http://127.0.0.1:38400".to_owned(),
            token: String::new(),
        });
        let mut game = record_with_siblings(
            Some("Disc 1"),
            vec![RetroRomVersion {
                id: 43,
                title: "Disc 2".to_owned(),
                is_main_sibling: false,
            }],
        );
        game.cover_path = Some("file:///tmp/hearthdeck-test-cover.png".to_owned());
        game.genres = vec!["Action".to_owned()];
        let mut single = record_with_siblings(None, Vec::new());
        single.id = 99;
        let page = client
            .resolve_retro_page(super::RetroGamesResponse {
                items: vec![game, single],
                total: 120,
                offset: 60,
            })
            .await;

        assert_eq!(page.total, 120);
        assert_eq!(page.offset, 60);
        assert_eq!(
            page.items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["romm:42", "romm:99"]
        );
        assert!(matches!(
            &page.items[0].icon,
            cosmic::desktop::fde::IconSource::Path(path) if path == std::path::Path::new("/tmp/hearthdeck-test-cover.png")
        ));
        assert!(page.items[0].matches_category("hearthdeck-genre:Action"));
        let versions = &page.versions[&page.items[0].id];
        assert_eq!(
            versions
                .iter()
                .map(|version| (version.id, version.title.as_str()))
                .collect::<Vec<_>>(),
            [(42, "Disc 1"), (43, "Disc 2")]
        );
        assert!(page.versions[&page.items[1].id].is_empty());
        assert_eq!(page.versions.len(), page.items.len());
    }

    #[test]
    fn the_version_list_puts_the_records_own_file_first() {
        let record = record_with_siblings(
            Some("Disc 1"),
            vec![RetroRomVersion {
                id: 43,
                title: "Disc 2".to_owned(),
                is_main_sibling: false,
            }],
        );
        let versions = super::retro_version_list(&record);
        let listed: Vec<(i64, &str, bool)> = versions
            .iter()
            .map(|version| (version.id, version.title.as_str(), version.is_main_sibling))
            .collect();
        assert_eq!(
            listed,
            vec![(42, "Disc 1", true), (43, "Disc 2", false)],
            "Play's target leads, its label naming the file",
        );
    }

    #[test]
    fn a_single_file_game_has_no_version_list() {
        let record = record_with_siblings(None, Vec::new());
        assert!(
            super::retro_version_list(&record).is_empty(),
            "nothing to pick between means no picker either",
        );
    }

    #[test]
    fn two_files_sharing_a_name_stay_distinguishable() {
        let record = record_with_siblings(
            Some("Game"),
            vec![
                RetroRomVersion {
                    id: 43,
                    title: "Game".to_owned(),
                    is_main_sibling: false,
                },
                RetroRomVersion {
                    id: 44,
                    title: "Game".to_owned(),
                    is_main_sibling: false,
                },
            ],
        );
        let titles: Vec<String> = super::retro_version_list(&record)
            .into_iter()
            .map(|version| version.title)
            .collect();
        assert_eq!(
            titles,
            vec![
                "Game".to_owned(),
                "Game (#43)".to_owned(),
                "Game (#44)".to_owned()
            ],
            "a repeated name gets the id appended so no two rows read the same",
        );
    }
}
