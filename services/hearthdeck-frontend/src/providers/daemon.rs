use async_trait::async_trait;
use futures::{StreamExt, future::join_all, stream};
use hearthdeck_protocol::{ApplicationSession, InputProfile};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env,
    hash::{DefaultHasher, Hasher},
    io::{Read, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::OnceCell;

use super::{GameProvider, GameRecord};
use crate::app_group::{romm_console_category, romm_genre_category, romm_region_category};
use crate::icon_cache::{self, IconReference};
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

/// How long one artwork download may take. Artwork comes from third-party hosts
/// and is fetched on a blocking thread, which a stalled server would hold for good.
const ICON_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15);

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
    config: DaemonConfig,
    http: reqwest::Client,
    token: Arc<OnceCell<String>>,
}

#[derive(Debug, Deserialize)]
struct PairingCodeResponse {
    code: String,
}

#[derive(Debug, Serialize)]
struct CompletePairingRequest<'a> {
    code: &'a str,
    client_name: &'a str,
}

#[derive(Debug, Deserialize)]
struct PairingCompleteResponse {
    token: String,
}

pub use hearthdeck_protocol::{
    CatalogItem, CategorizationPhase, Collection, CollectionItem, HealthResponse, ModelStatus,
    RetroGame, RetroGameDetails, RetroRomVersion, ScanReport, ServerEventKind as DaemonEvent,
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
            config,
            http,
            token: Arc::new(OnceCell::new()),
        }
    }

    /// Returns the base URL for API requests.
    fn api_url(&self, path: &str) -> String {
        format!("{}{}", self.config.base_url, path)
    }

    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    /// Opens the daemon's change feed (`GET /v1/events`).
    pub async fn connect_events(&self) -> Result<EventStream, DaemonError> {
        use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

        let mut url =
            url::Url::parse(&self.config.base_url).map_err(DaemonError::InvalidBaseUrl)?;
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
        let status = response.status();
        match response.bytes().await {
            Ok(body) => daemon_error_from_body(status, &body),
            Err(_) => DaemonError::UnexpectedStatus(status),
        }
    }

    /// Returns authorization headers for API requests.
    async fn auth_headers(&self) -> Result<reqwest::header::HeaderMap, DaemonError> {
        let token = self
            .token
            .get_or_try_init(|| async {
                if self.config.token.is_empty() {
                    self.pair_local().await
                } else {
                    Ok(self.config.token.clone())
                }
            })
            .await?;
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(DaemonError::InvalidToken)?,
        );
        Ok(headers)
    }

    async fn pair_local(&self) -> Result<String, DaemonError> {
        let mut api_url =
            url::Url::parse(&self.config.base_url).map_err(DaemonError::InvalidBaseUrl)?;
        if !matches!(api_url.host_str(), Some("127.0.0.1" | "localhost" | "::1")) {
            return Err(DaemonError::PairingRequiresLoopback);
        }
        let mut admin_url = api_url.clone();
        admin_url
            .set_port(Some(38401))
            .map_err(|_| DaemonError::PairingRequiresLoopback)?;
        admin_url.set_path("/v1/pairing");
        admin_url.set_query(None);

        let response = self
            .http
            .post(admin_url.clone())
            .send()
            .await
            .map_err(DaemonError::Connection)?;
        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }
        let pairing = response
            .json::<PairingCodeResponse>()
            .await
            .map_err(DaemonError::Deserialization)?;

        api_url.set_path("/v1/pairing/complete");
        api_url.set_query(None);
        let response = self
            .http
            .post(api_url)
            .json(&CompletePairingRequest {
                code: &pairing.code,
                client_name: "hearthdeck-cosmic-frontend",
            })
            .send()
            .await
            .map_err(DaemonError::Connection)?;
        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }
        response
            .json::<PairingCompleteResponse>()
            .await
            .map(|pairing| pairing.token)
            .map_err(DaemonError::Deserialization)
    }

    /// Checks if the daemon is reachable and returns its health status.
    pub async fn health(&self) -> Result<HealthResponse, DaemonError> {
        let response = self
            .http
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
        let response = self
            .http
            .get(self.api_url("/v1/library"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Reads the categorization scan's state and its last report.
    pub async fn categorization(&self) -> Result<CategorizationSnapshot, DaemonError> {
        let response = self
            .http
            .get(self.api_url("/v1/categorization"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

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
        let response = self
            .http
            .delete(self.api_url("/v1/categorization/collections"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;
        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }
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
        let response = self
            .http
            .post(self.api_url(path))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

        Ok(())
    }

    /// Launches whatever an item id names - the one route a provider-agnostic
    /// client posts to. The daemon resolves the id (catalog record, RomM rom)
    /// and picks the launch shape; this parses nothing.
    pub async fn launch_item(
        &self,
        item_id: &str,
        input_profile: InputProfile,
    ) -> Result<ApplicationSession, DaemonError> {
        let response = self
            .http
            .post(self.api_url(&format!("/v1/launch/{}", item_id)))
            .query(&[("input_profile", input_profile)])
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Gets the currently active application session.
    pub async fn active_session(&self) -> Result<Option<ApplicationSession>, DaemonError> {
        let response = self
            .http
            .get(self.api_url("/v1/sessions/active"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Lists available retro consoles from RomM, caching their artwork.
    pub async fn list_retro_consoles(&self) -> Result<Vec<RetroConsole>, DaemonError> {
        let response = self
            .http
            .get(self.api_url("/v1/retro/consoles"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

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

        let response = self
            .http
            .get(&url)
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

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

        let response = self
            .http
            .get(&url)
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Lists one page of retro records, converted and down to their covers.
    ///
    /// `search` is the source's own search, not a filter over the page that
    /// comes back: a grid that searched only what it had loaded would find
    /// nothing past the first page.
    pub async fn list_retro_records(
        &self,
        platform_id: Option<i64>,
        search: Option<&str>,
        filters: &BTreeMap<String, String>,
        limit: u32,
        offset: u32,
    ) -> Result<RetroRecordPage, DaemonError> {
        let page = self
            .list_retro_roms(platform_id, search, filters, limit, offset)
            .await?;
        let records = stream::iter(page.items.into_iter().map(|game| {
            let client = self.clone();
            async move {
                // The cover is a RomM path, which the same resolver every other
                // record's icon goes through turns into a cached file.
                let icon = client.resolve_icon(game.cover_path.as_deref()).await;
                retro_game_to_game_record(game, icon)
            }
        }))
        // A page is only returned once every cover in it is cached, so this
        // concurrency directly bounds first-paint latency; the daemon asset
        // endpoint is loopback/served by RomM, so 16 parallel covers is safe.
        .buffered(RAIL_ICON_CONCURRENCY)
        .collect()
        .await;
        Ok(RetroRecordPage {
            items: records,
            total: page.total,
            offset: page.offset,
        })
    }

    /// Downloads one RomM asset (cover, screenshot, console artwork) through
    /// the daemon's proxy and returns its local cached path, reusing an
    /// already-cached copy when there is one.
    async fn cache_retro_asset_path(&self, path: Option<&str>) -> Option<String> {
        if let Some(path) = path {
            let key = format!("romm:{path}");
            if let Some(cached) = cached_icon(&key) {
                return Some(cached);
            }
            let mut url = url::Url::parse(&self.api_url("/v1/retro/assets")).ok()?;
            url.query_pairs_mut().append_pair("path", path);
            let response = self
                .http
                .get(url)
                .headers(self.auth_headers().await.ok()?)
                .send()
                .await
                .ok()?;
            if response.status().is_success() {
                let extension = image_extension(
                    response
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|value| value.to_str().ok()),
                );
                let bytes = response.bytes().await.ok()?.to_vec();
                return tokio::task::spawn_blocking(move || {
                    cache_icon_bytes(&key, extension, &bytes)
                })
                .await
                .ok()
                .flatten();
            }
        }

        None
    }

    /// Fetches one game's full detail plus its artwork, both already cached
    /// locally, so the details screen paints without a second round trip.
    pub async fn retro_rom_details(&self, rom_id: i64) -> Result<RetroDetails, DaemonError> {
        let response = self
            .http
            .get(self.api_url(&format!("/v1/retro/roms/{rom_id}")))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

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
        // Bounded concurrency, as in `list_retro_records`: the daemon asset
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
        let response = self
            .http
            .get(self.api_url("/v1/collections"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

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
                // Blocking, and deliberately so: this reads and writes image
                // files, which is what a blocking HTTP client is for. The same
                // art is retried at Metahub's other sizes when the one the feed
                // sent will not come down - see `icon_cache::art_candidates`.
                let candidates = icon_cache::art_candidates(url);
                tokio::task::spawn_blocking(move || {
                    candidates
                        .iter()
                        .find_map(|candidate| cache_icon(candidate))
                })
                .await
                .ok()
                .flatten()
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
        let response = self
            .http
            .post(self.api_url(&format!("/v1/collections/{slug}/items")))
            .json(&serde_json::json!({
                "item_id": item.item_id,
                "name": item.name,
                "icon": item.icon,
            }))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Removes one item from a curated collection, answering with the collection
    /// as it now stands.
    pub async fn remove_collection_item(
        &self,
        slug: &str,
        item_id: &str,
    ) -> Result<Collection, DaemonError> {
        let response = self
            .http
            .delete(self.api_url(&format!("/v1/collections/{slug}/items")))
            .json(&serde_json::json!({ "item_id": item_id }))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Asks the daemon for a Stremio link code.
    ///
    /// The code is approved on another device, which is why nothing here sends a
    /// password: the daemon does not accept one, so there is none to send.
    pub async fn start_stremio_link(&self) -> Result<StremioLinkOffer, DaemonError> {
        let response = self
            .http
            .post(self.api_url("/v1/stremio/link"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// One poll of the pending link.
    ///
    /// The daemon is what talks to Stremio, so neither the code nor the session
    /// crosses this call: it only asks what the daemon found.
    pub async fn poll_stremio_link(&self) -> Result<StremioLink, DaemonError> {
        let response = self
            .http
            .get(self.api_url("/v1/stremio/link"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// The linked Stremio account, or `None` when none is linked.
    pub async fn stremio_connection(&self) -> Result<Option<StremioConnection>, DaemonError> {
        let response = self
            .http
            .get(self.api_url("/v1/stremio/settings"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    pub async fn romm_settings(&self) -> Result<Option<RommSettings>, DaemonError> {
        let response = self
            .http
            .get(self.api_url("/v1/retro/settings"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;
        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }
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
        let response = self
            .http
            .post(self.api_url("/v1/retro/login"))
            .headers(self.auth_headers().await?)
            .json(&serde_json::json!({
                "base_url": base_url,
                "username": username,
                "password": password,
            }))
            .send()
            .await
            .map_err(DaemonError::Connection)?;
        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }
        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Asks the daemon for a RomM device code the user approves on a phone.
    pub async fn start_romm_link(&self, base_url: &str) -> Result<RommLinkOffer, DaemonError> {
        let response = self
            .http
            .post(self.api_url("/v1/retro/link"))
            .headers(self.auth_headers().await?)
            .json(&serde_json::json!({ "base_url": base_url }))
            .send()
            .await
            .map_err(DaemonError::Connection)?;
        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }
        response.json().await.map_err(DaemonError::Deserialization)
    }

    pub async fn poll_romm_link(&self) -> Result<RommLink, DaemonError> {
        let response = self
            .http
            .get(self.api_url("/v1/retro/link"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;
        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }
        response.json().await.map_err(DaemonError::Deserialization)
    }

    pub async fn clear_romm_settings(&self) -> Result<(), DaemonError> {
        let response = self
            .http
            .delete(self.api_url("/v1/retro/settings"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;
        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }
        Ok(())
    }

    /// Unlinks the account: the session and the rows it published go together.
    pub async fn unlink_stremio(&self) -> Result<(), DaemonError> {
        let response = self
            .http
            .delete(self.api_url("/v1/stremio/settings"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

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
        let response = self
            .http
            .post(self.api_url(&format!("/v1/retro/roms/{rom_id}/favorite")))
            .json(&serde_json::json!({ "favorite": favorite }))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

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
        let response = self
            .http
            .post(self.api_url(&format!("/v1/retro/roms/{rom_id}/backlog")))
            .json(&serde_json::json!({ "backlogged": backlogged }))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(Self::http_error(response).await);
        }

        let state: BacklogState = response
            .json()
            .await
            .map_err(DaemonError::Deserialization)?;
        Ok(state.backlogged)
    }
}

/// Converts a daemon CatalogItem into a GameRecord for display in the UI.
pub fn catalog_item_to_game_record(item: CatalogItem) -> GameRecord {
    let metadata = &item.metadata;

    let mut categories: Vec<String> = metadata
        .get("categories")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    categories.retain(|category| !category.eq_ignore_ascii_case("game"));
    if item.kind.eq_ignore_ascii_case("game") {
        categories.push("Game".to_string());
    }
    if let Some(store) = metadata.get("store").and_then(|v| v.as_str())
        && !categories
            .iter()
            .any(|category| category.eq_ignore_ascii_case(store))
    {
        categories.push(store.to_string());
    }

    // Extract summary/description for the comment field
    let comment = metadata
        .get("summary")
        .or_else(|| metadata.get("description"))
        .and_then(|v| v.as_str())
        .map(String::from);

    // Extract source-specific information
    let source = match item.source_id.as_str() {
        "heroic" => {
            let runner = metadata.get("runner").and_then(|v| v.as_str());
            let store = metadata.get("store").and_then(|v| v.as_str());
            format!("Heroic ({})", store.unwrap_or(runner.unwrap_or("Unknown")))
        }
        "lutris" => "Lutris".to_string(),
        "flatpak" => "Flatpak".to_string(),
        "desktop-apps" | "desktop" => "System".to_string(),
        _ => item.source_id.clone(),
    };

    // Build metadata JSON value
    let mut game_metadata = serde_json::Map::new();
    if let Some(comment) = comment {
        game_metadata.insert("comment".to_string(), serde_json::Value::String(comment));
    }
    if let Some(developer) = metadata.get("developer").and_then(|v| v.as_str()) {
        game_metadata.insert(
            "developer".to_string(),
            serde_json::Value::String(developer.to_string()),
        );
    }
    if let Some(license) = metadata.get("project_license").and_then(|v| v.as_str()) {
        game_metadata.insert(
            "license".to_string(),
            serde_json::Value::String(license.to_string()),
        );
    }
    if let Some(version) = metadata.get("version").and_then(|v| v.as_str()) {
        game_metadata.insert(
            "version".to_string(),
            serde_json::Value::String(version.to_string()),
        );
    }
    if let Some(store) = metadata.get("store").and_then(|v| v.as_str()) {
        game_metadata.insert(
            "store".to_string(),
            serde_json::Value::String(store.to_string()),
        );
    }
    // Where the application was installed. It travels beside the store and the
    // license because the entry it becomes drops the metadata that carried it.
    if let Some(source) = metadata.get("source").and_then(|v| v.as_str()) {
        game_metadata.insert(
            "source".to_string(),
            serde_json::Value::String(source.to_string()),
        );
    }
    // What the categorization model decided, for the form-factor tag.
    if let Some(form) = metadata.get("form_factor").and_then(|v| v.as_str()) {
        game_metadata.insert(
            "form_factor".to_string(),
            serde_json::Value::String(form.to_string()),
        );
    }

    // Determine if this prefers dGPU based on metadata
    let prefers_dgpu = metadata
        .get("prefers_dgpu")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    GameRecord {
        id: format!("{CATALOG_ENTRY_PREFIX}{}", item.id),
        name: item.title,
        exec: item.launch_id,
        icon: item.icon,
        path: None,
        categories,
        terminal: false,
        prefers_dgpu,
        source,
        metadata: serde_json::Value::Object(game_metadata),
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

pub fn retro_game_to_game_record(game: RetroGame, icon: Option<String>) -> GameRecord {
    // The filterable metadata a record carries has to travel as categories:
    // `DesktopEntryData` has no metadata field, and the grid filters on the
    // converted entry. Built before the metadata map below, which moves the
    // fields it copies.
    let mut categories = vec!["Game".to_string(), romm_console_category(game.platform_id)];
    categories.extend(game.genres.iter().map(|genre| romm_genre_category(genre)));
    categories.extend(
        game.regions
            .iter()
            .map(|region| romm_region_category(region)),
    );

    let mut metadata = serde_json::Map::new();
    if let Some(summary) = game.summary {
        metadata.insert("comment".to_string(), serde_json::Value::String(summary));
    }
    metadata.insert("genres".to_string(), serde_json::json!(game.genres));
    metadata.insert(
        "screenshot_paths".to_string(),
        serde_json::json!(game.screenshot_paths),
    );
    if let Some(release_year) = game.release_year {
        metadata.insert("release_year".to_string(), release_year.into());
    }
    // Carried on the record so the context menu can offer each version without
    // another daemon round-trip. Absent for single-file games.
    if !game.sibling_roms.is_empty() {
        metadata.insert(
            "sibling_roms".to_string(),
            serde_json::json!(game.sibling_roms),
        );
        if let Some(label) = game.version_label {
            metadata.insert(
                "version_label".to_string(),
                serde_json::Value::String(label),
            );
        }
    }

    GameRecord {
        id: format!("romm:{}", game.id),
        name: game.title,
        exec: Some(game.id.to_string()),
        icon,
        path: None,
        categories,
        terminal: false,
        prefers_dgpu: false,
        source: "RomM".to_string(),
        metadata: serde_json::Value::Object(metadata),
    }
}

/// Reads the sibling versions stored in a RomM [`GameRecord`]'s metadata by
/// [`retro_game_to_game_record`]. Empty for records with no variants.
pub fn retro_rom_versions(metadata: &serde_json::Value) -> Vec<RetroRomVersion> {
    metadata
        .get("sibling_roms")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

/// The current version's file-derived label, stored alongside `sibling_roms`
/// by [`retro_game_to_game_record`]. `None` for single-file games.
pub fn retro_version_label(metadata: &serde_json::Value) -> Option<String> {
    metadata
        .get("version_label")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

/// The files a console record collapsed: the entry's own first — Play's
/// target, and what the details screen opens on — then every sibling RomM
/// folded into it, as the disc picker lists them.
///
/// Built here, where the record still carries its metadata: the grid keeps
/// only `DesktopEntryData`, which has no metadata field. Two files can share
/// a name, so a repeat gets its id appended to stay distinguishable.
pub fn retro_version_list(record: &GameRecord) -> Vec<RetroRomVersion> {
    let siblings = retro_rom_versions(&record.metadata);
    if siblings.is_empty() {
        return Vec::new();
    }
    let mut versions = Vec::with_capacity(siblings.len() + 1);
    if let Some(current_id) = record
        .id
        .strip_prefix("romm:")
        .and_then(|id| id.parse::<i64>().ok())
    {
        versions.push(RetroRomVersion {
            id: current_id,
            title: retro_version_label(&record.metadata).unwrap_or_else(|| record.name.clone()),
            is_main_sibling: true,
        });
    }
    versions.extend(siblings);
    let mut seen = std::collections::HashSet::new();
    for version in &mut versions {
        if !seen.insert(version.title.clone()) {
            version.title = format!("{} (#{})", version.title, version.id);
        }
    }
    versions
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

/// Maps a daemon error-response body to a [`DaemonError`], surfacing the
/// `{"error": ...}` message the daemon attaches to launch failures (e.g. a
/// bridge rejection reason) instead of dropping it for the bare status code.
fn daemon_error_from_body(status: reqwest::StatusCode, body: &[u8]) -> DaemonError {
    let message = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|body| {
            body.get("error")
                .and_then(serde_json::Value::as_str)
                .map(|message| message.trim().to_owned())
                .filter(|message| !message.is_empty())
        });
    match message {
        Some(message) => DaemonError::Api { status, message },
        None => DaemonError::UnexpectedStatus(status),
    }
}

/// A provider that fetches games/apps from the HearthDeck daemon.
pub struct DaemonProvider {
    client: DaemonClient,
}

impl DaemonProvider {
    /// Creates a new daemon provider with an existing client.
    pub fn with_client(client: DaemonClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl GameProvider for DaemonProvider {
    async fn discover(&self) -> anyhow::Result<Vec<GameRecord>> {
        let items = self
            .client
            .fetch_library()
            .await
            .map_err(|e| anyhow::anyhow!("failed to fetch library: {}", e))?;

        log::info!("fetched {} catalog items from daemon", items.len());

        let items = join_all(items.into_iter().map(|mut item| {
            let client = self.client.clone();
            async move {
                // Every icon is resolved here, once, before a record is built:
                // a poster becomes the file on disk the renderer can read, and
                // what cannot be fetched stops being a reference no view could
                // have drawn anyway.
                item.icon = client.resolve_icon(item.icon.as_deref()).await;
                item
            }
        }))
        .await;

        // Films and series are not applications. The rail draws them from the daemon's
        // own collection, so leaving them here only put them in the All Apps grid,
        // sorted in among the programs.
        Ok(items
            .into_iter()
            .filter(belongs_in_the_app_library)
            .map(catalog_item_to_game_record)
            .collect())
    }
}

/// Whether a catalog record belongs in the application library.
///
/// A streaming provider publishes films and series so the dashboard can draw a rail, and
/// the daemon serves them from the same endpoint as desktop entries, so without this a
/// Continue Watching film appeared in the All Apps list beside the programs, with the
/// same open-it action as one.
///
/// Keyed on the record's own kind rather than on which provider published it: the rule is
/// *a television title is not an application*, not *Stremio is special*, so the next
/// media provider inherits it. A record with no kind at all is kept, because dropping
/// something unrecognised from the library is worse than showing it.
fn belongs_in_the_app_library(item: &CatalogItem) -> bool {
    let kind = item.kind.trim();
    if kind.is_empty() {
        return true;
    }
    matches!(kind.to_ascii_lowercase().as_str(), "game" | "application")
}

fn cache_icon(url: &str) -> Option<String> {
    // A poster URL frequently carries no extension at all — Metahub serves
    // `/poster/medium/tt0149460/img` — so the format cannot be read off the URL.
    // Deriving the extension there made every such file `<hash>.png`, and the
    // half of the covers the server sends as webp were then saved with an
    // extension the image decoder refuses: the placeholder. The extension is now
    // taken from the response after the fetch, and the early return probes every
    // extension the cache can hold rather than one guessed from the URL.
    if let Some(cached) = cached_icon(url) {
        return Some(cached);
    }

    // `ureq` is blocking, and that is deliberate: this function runs inside
    // `spawn_blocking` (it reads and writes image files), so a blocking HTTP client
    // is the honest choice. `reqwest` is async and would want a runtime handle this
    // call does not have.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(ICON_DOWNLOAD_TIMEOUT))
        .build()
        .into();
    let response = agent.get(url).call().ok()?;
    // Resolved to a `&'static str` here, before `response` is consumed below, so
    // the borrow of its headers does not outlive the move into the body reader.
    let extension = image_extension(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
    );
    let mut body = Vec::new();
    response
        .into_body()
        .into_reader()
        .read_to_end(&mut body)
        .ok()?;
    cache_icon_bytes(url, extension, &body)
}

pub(crate) fn cached_icon(key: &str) -> Option<String> {
    // `svg` and `ico` matter for RomM's bundled console art, which is served
    // in those formats; omitting them made platform icons re-download on every
    // refresh because the cache lookup never matched.
    ["jpg", "jpeg", "png", "webp", "svg", "ico"]
        .into_iter()
        .map(|extension| icon_cache_dir().join(cache_file_name(key, extension)))
        .find(|path| path.metadata().is_ok_and(|metadata| metadata.len() >= 16))
        .map(|path| path.to_string_lossy().into_owned())
}

/// The file name a cached download is stored under: a stable hash of its key,
/// then the given extension.
///
/// Pure and free of any network or disk access, which is the point: the whole of
/// the extension-mismatch bug lived in this naming decision, so a test can pin it
/// here. Every writer names its file through it, and `cached_icon`'s probe
/// rebuilds the same names, so what one writes the others find.
fn cache_file_name(key: &str, extension: &str) -> String {
    let mut hasher = DefaultHasher::new();
    hasher.write(key.as_bytes());
    format!("{:016x}.{extension}", hasher.finish())
}

fn cache_icon_bytes(key: &str, extension: &str, bytes: &[u8]) -> Option<String> {
    if bytes.len() < 16 {
        return None;
    }
    let cache_dir = icon_cache_dir();
    std::fs::create_dir_all(&cache_dir).ok()?;
    let cached = cache_dir.join(cache_file_name(key, extension));
    std::fs::File::create(&cached).ok()?.write_all(bytes).ok()?;
    Some(cached.to_string_lossy().into_owned())
}

fn image_extension(content_type: Option<&str>) -> &'static str {
    let Some(content_type) = content_type else {
        return "jpg";
    };
    let content_type = content_type.to_ascii_lowercase();
    if content_type.starts_with("image/png") {
        "png"
    } else if content_type.starts_with("image/webp") {
        "webp"
    } else if content_type.starts_with("image/svg") {
        "svg"
    } else if content_type.contains("icon") {
        // Covers both `image/x-icon` and `image/vnd.microsoft.icon`; RomM's
        // bundled console art ships `.ico` for platforms with no `.svg`.
        "ico"
    } else {
        "jpg"
    }
}

fn icon_cache_dir() -> PathBuf {
    env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("hearthdeck/icons")
}

/// Response from the /v1/retro/roms endpoint.
#[derive(Clone, Debug, Deserialize)]
pub struct RetroGamesResponse {
    pub items: Vec<RetroGame>,
    pub total: u64,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct RetroRecordPage {
    pub items: Vec<GameRecord>,
    pub total: u64,
    pub offset: u32,
}

#[cfg(test)]
mod tests {
    use super::{
        CatalogItem, Collection, CollectionItem, DaemonClient, DaemonConfig, DaemonError,
        DaemonEvent, GameRecord, RetroGame, RetroRomVersion, belongs_in_the_app_library,
        cache_file_name, catalog_item_to_game_record, daemon_error_from_body, image_extension,
        retro_game_to_game_record, retro_rom_query_params, retro_rom_versions, retro_version_label,
    };

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

    /// The extension a cached cover is stored with must come from the response,
    /// not the URL. Metahub poster URLs carry no extension, so the old URL-derived
    /// name was always `<hash>.png` — and a webp poster saved as `.png` is a file
    /// the image decoder refuses, which draws the placeholder. That is exactly why
    /// only some Continue Watching tiles were blank, and always the same ones.
    #[test]
    fn cached_icon_name_follows_the_response_content_type() {
        let poster = "https://images.metahub.space/poster/medium/tt0149460/img";

        assert_eq!(image_extension(Some("image/webp")), "webp");
        assert_eq!(image_extension(Some("image/jpeg")), "jpg");
        assert_eq!(image_extension(Some("image/png")), "png");

        // An extension-free URL no longer forces "png": the name carries the
        // extension the response asked for.
        assert!(cache_file_name(poster, image_extension(Some("image/webp"))).ends_with(".webp"));
        assert!(cache_file_name(poster, image_extension(Some("image/jpeg"))).ends_with(".jpg"));

        // One URL, any extension: one stem. That is what lets `cached_icon`'s
        // multi-extension probe find whatever `cache_icon` wrote.
        let stem = |name: String| name.rsplit_once('.').unwrap().0.to_string();
        assert_eq!(
            stem(cache_file_name(poster, "png")),
            stem(cache_file_name(poster, "webp"))
        );
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

    /// Where an application was installed reaches the converted record. The
    /// entry the record becomes drops the catalog's metadata, so the origin has
    /// to be copied onto it the way the store and the license are - otherwise
    /// the library has nothing to filter applications by.
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
        };

        let record = catalog_item_to_game_record(item);

        assert_eq!(record.metadata["source"], "flatpak");
    }

    #[test]
    fn daemon_error_bodies_surface_the_bridge_launch_reason() {
        let status = reqwest::StatusCode::BAD_GATEWAY;
        let body = br#"{"error":"input compatibility service failed to start"}"#;

        assert_eq!(
            daemon_error_from_body(status, body).to_string(),
            "502 Bad Gateway: input compatibility service failed to start"
        );
    }

    #[test]
    fn non_daemon_error_bodies_fall_back_to_the_status_code() {
        let status = reqwest::StatusCode::BAD_GATEWAY;

        assert!(matches!(
            daemon_error_from_body(status, b"<html>proxy error</html>"),
            DaemonError::UnexpectedStatus(reqwest::StatusCode::BAD_GATEWAY)
        ));
        assert!(matches!(
            daemon_error_from_body(status, b""),
            DaemonError::UnexpectedStatus(reqwest::StatusCode::BAD_GATEWAY)
        ));
    }

    #[test]
    fn catalog_item_maps_to_game_record_with_prefix() {
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
        };

        let record = catalog_item_to_game_record(item);

        assert!(record.id.starts_with("hearthdeck:"));
        assert_eq!(record.id, "hearthdeck:test:app");
        assert_eq!(record.name, "Test App");
        assert_eq!(record.exec, Some("test.app".to_string()));
        assert_eq!(record.categories, vec!["Utility"]);
        assert_eq!(record.source, "System");
        assert_eq!(
            record.metadata["comment"],
            serde_json::Value::String("A test application".to_string())
        );
    }

    #[test]
    fn heroic_items_show_runner_and_store() {
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
        };

        let record = catalog_item_to_game_record(item);

        assert_eq!(record.source, "Heroic (Epic)");
        assert_eq!(record.categories, vec!["Game", "Epic"]);
        assert_eq!(record.metadata["store"], "Epic");
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
        };

        let record = catalog_item_to_game_record(item);

        assert!(record.exec.is_none());
        assert_eq!(record.source, "Flatpak");
    }

    #[test]
    fn romm_game_maps_to_a_console_record_with_dedicated_id() {
        let record = retro_game_to_game_record(
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
        assert_eq!(record.icon.as_deref(), Some("/tmp/example.jpg"));
        assert_eq!(record.metadata["genres"], serde_json::json!(["RPG"]));
        assert_eq!(record.metadata["release_year"], serde_json::json!(1994));
        // The console facets read a record's RomM metadata off its categories,
        // which is the only field `DesktopEntryData` keeps.
        assert_eq!(
            record.categories,
            vec![
                "Game",
                "hearthdeck-console:7",
                "hearthdeck-genre:RPG",
                "hearthdeck-region:Japan",
            ]
        );
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
    fn romm_sibling_versions_round_trip_through_record_metadata() {
        let record = retro_game_to_game_record(
            RetroGame {
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
            },
            None,
        );

        let versions = retro_rom_versions(&record.metadata);
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].id, 43);
        assert!(versions[0].is_main_sibling);
        assert_eq!(versions[1].title, "Shenmue (Disc 3)");
        assert_eq!(
            retro_version_label(&record.metadata).as_deref(),
            Some("Shenmue (Disc 1)")
        );
    }

    fn record_with_siblings(metadata: serde_json::Value) -> GameRecord {
        GameRecord {
            id: "romm:42".to_owned(),
            name: "Metal Gear Solid".to_owned(),
            exec: None,
            icon: None,
            path: None,
            categories: Vec::new(),
            terminal: false,
            prefers_dgpu: false,
            source: "RomM".to_owned(),
            metadata,
        }
    }

    #[test]
    fn the_version_list_puts_the_records_own_file_first() {
        let record = record_with_siblings(serde_json::json!({
            "version_label": "Disc 1",
            "sibling_roms": [
                {"id": 43, "title": "Disc 2", "is_main_sibling": false}
            ],
        }));
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
        let record = record_with_siblings(serde_json::json!({}));
        assert!(
            super::retro_version_list(&record).is_empty(),
            "nothing to pick between means no picker either",
        );
    }

    #[test]
    fn two_files_sharing_a_name_stay_distinguishable() {
        let record = record_with_siblings(serde_json::json!({
            "version_label": "Game",
            "sibling_roms": [
                {"id": 43, "title": "Game", "is_main_sibling": false},
                {"id": 44, "title": "Game", "is_main_sibling": false}
            ],
        }));
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
