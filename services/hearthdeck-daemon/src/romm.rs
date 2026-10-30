//! The RomM HTTP client: platforms, roms, filters, favorites and artwork, as
//! RomM's API reports them. RomM is an external server the daemon reads on the
//! user's behalf; diagnostics only probes it, and the device link lives in
//! `romm_link`.

use std::{path::Path, sync::OnceLock};

use chrono::{DateTime, Datelike};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::config::RommPaths;
use crate::settings::{RommCredentials, SettingsRepository};

/// Anything above this is a `first_release_date` in milliseconds, not seconds.
/// RomM's merged `metadatum` reports timestamps in milliseconds while its raw
/// IGDB payload uses seconds, and any plausible release date in seconds is far
/// below this bound (year 5138).
const RELEASE_MILLIS_THRESHOLD: i64 = 100_000_000_000;

#[derive(Debug)]
pub enum RommQueryError {
    NotConfigured,
    Failed(anyhow::Error),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RommPlatform {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub rom_count: u64,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub fs_slug: Option<String>,
    /// Absolute URL of the platform logo RomM resolved from its metadata
    /// providers (IGDB/SteamGridDB/etc.); absent when the platform is
    /// unidentified. Kept for completeness but no longer used for tiles:
    /// these are wide wordmarks that read poorly at tile size.
    #[serde(default)]
    pub url_logo: Option<String>,
    /// Ordered candidate paths (relative to the RomM server) for this
    /// platform's bundled console artwork. Mirrors RomM's own
    /// `RPlatformIcon` fallback chain so the client can try each through the
    /// asset proxy until one exists. Populated from [`platforms`], so it
    /// is empty on the raw `/api/platforms` deserialization (RomM does not
    /// send this field; the server cannot know its own host's static paths).
    #[serde(default, skip_deserializing)]
    pub artwork_paths: Vec<String>,
}

/// Candidate RomM artwork paths for a platform, mirroring RomM's bundled
/// console-art naming. The filenames match RomM's canonical platform slug
/// (e.g. Dreamcast is `dc`, so `/assets/platforms/dc.svg`), so the slug is
/// tried first and the filesystem folder name is only a secondary fallback
/// for installs whose slug does not line up. `default.ico` is last.
///
/// The daemon only names the candidates; it deliberately does not probe RomM
/// to learn which file exists. The client walks the list through the
/// `/v1/retro/assets` proxy and caches the first one that loads, so an
/// unidentified platform costs a couple of local requests instead of making
/// every `/v1/retro/consoles` call probe the network.
fn platform_artwork_paths(platform: &RommPlatform) -> Vec<String> {
    fn candidate(slug: &str, extension: &str) -> String {
        format!("/assets/platforms/{slug}.{extension}")
    }

    let normalized = |value: Option<&String>| {
        value
            .map(|slug| slug.trim().to_ascii_lowercase())
            .filter(|slug| !slug.is_empty())
    };
    let slug = normalized(platform.slug.as_ref());
    let fs_slug = normalized(platform.fs_slug.as_ref());

    let mut paths = Vec::new();
    if let Some(slug) = slug.as_deref() {
        paths.push(candidate(slug, "svg"));
        paths.push(candidate(slug, "ico"));
    }
    if let Some(fs_slug) = fs_slug.as_deref()
        && Some(fs_slug) != slug.as_deref()
    {
        paths.push(candidate(fs_slug, "svg"));
        paths.push(candidate(fs_slug, "ico"));
    }
    paths.push(candidate("default", "ico"));
    paths
}

#[derive(Clone, Debug, Deserialize)]
pub struct RommGamePage {
    pub items: Vec<RommGame>,
    pub total: u64,
    pub limit: u32,
    pub offset: u32,
}

/// The metadata a RomM rom query can be narrowed by, as Hearthdeck's own retro
/// route receives it.
///
/// Only the multi-value fields the console grid filters on are modelled. RomM
/// takes each as repeated query parameters plus a logic operator, and Hearthdeck
/// always sends `any` (a rom carrying any of the listed values), which is what a
/// single-select chip means.
#[derive(Clone, Debug, Default)]
pub struct RommGameFilters {
    pub genres: Vec<String>,
    pub regions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RommGame {
    pub id: i64,
    pub platform_id: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub fs_name_no_tags: String,
    /// The leaf name on RomM's disk, including tags/extension: a file name for a
    /// single-file rom, a folder name for a multi-file one. Distinct from
    /// `fs_name_no_tags`, which is only for display.
    #[serde(default)]
    pub fs_name: Option<String>,
    /// Directory of the rom inside RomM's library, relative to RomM's own
    /// `LIBRARY_BASE_PATH`. Joined with `fs_name` it is the file to launch; the
    /// daemon maps it onto the host's library root (see
    /// [`crate::config::RommPaths`]).
    #[serde(default)]
    pub fs_path: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub path_cover_small: Option<String>,
    #[serde(default)]
    pub path_cover_large: Option<String>,
    #[serde(default)]
    pub url_cover: Option<String>,
    #[serde(default)]
    pub merged_screenshots: Vec<String>,
    #[serde(default)]
    pub path_manual: Option<String>,
    #[serde(default)]
    pub has_manual: bool,
    #[serde(default)]
    pub metadatum: RommGameMetadata,
    #[serde(default)]
    pub regions: Vec<String>,
    /// Other files of the same game, as RomM reports them: another region or
    /// revision, or an individual disc of a multi-disc title. Only populated
    /// per page; [`games`] asks RomM to collapse each sibling group into
    /// one entry so the grid shows a single tile per game. The returned entry
    /// is the group's representative (RomM prefers the user's main sibling),
    /// and this list holds the *other* variants.
    #[serde(default)]
    pub sibling_roms: Vec<RommSiblingRom>,
    /// Name RomM resolved for the game's platform ("Super Nintendo
    /// Entertainment System"), shown on the details screen. Distinct from the
    /// local console tab label, which comes from the platform list.
    #[serde(default)]
    pub platform_display_name: Option<String>,
    /// Languages RomM matched from the game's metadata and file name.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Region tags RomM matched, such as `NA` or `EU`. Often populated when
    /// `regions` is not: `regions` holds RomM's own region metadata while
    /// `tags` carries what the file-name parser recognised.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Size of the ROM file as stored on RomM's disk.
    #[serde(default)]
    pub fs_size_bytes: Option<u64>,
    /// Whether the game is in the user's RomM favorites. Some RomM versions
    /// inline this on the rom; when they do not, the favorite is found through
    /// the favorite collection instead (see [`favorite`]).
    #[serde(default)]
    pub is_favorite: Option<bool>,
    /// This user's own state for the game. RomM sends an all-default stub when
    /// the user has never touched it.
    #[serde(default)]
    pub rom_user: RommRomUser,
}

/// The per-user slice of a RomM ROM that the frontend shows. RomM sends more
/// (rating, difficulty, completion, notes); serde ignores the rest until a
/// screen needs them.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RommRomUser {
    /// RomM's "play later" flag.
    #[serde(default)]
    pub backlogged: bool,
}

/// One related ROM of the same game in RomM's `sibling_roms` list. RomM sends
/// only the fields its own UI needs to label a version, so this is a narrow
/// mirror of `RommGame` rather than a nested full ROM.
#[derive(Clone, Debug, Deserialize)]
pub struct RommSiblingRom {
    pub id: i64,
    #[serde(default)]
    pub name: Option<String>,
    /// File name with the extension stripped but tags kept, e.g.
    /// `Shenmue (Disc 2)`. This is the field that tells versions apart: RomM
    /// strips trailing `(...)`/`[...]` groups from `fs_name_no_tags`, so all
    /// discs of a set collapse to the same string there.
    #[serde(default)]
    pub fs_name_no_ext: String,
    #[serde(default)]
    pub fs_name_no_tags: String,
    /// Whether this is the user's chosen main file for the group. False when
    /// the user never picked one.
    #[serde(default)]
    pub is_main_sibling: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct RommGameMetadata {
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub player_count: String,
    #[serde(default)]
    pub first_release_date: Option<i64>,
    /// Studios and publishers merged from every metadata source. RomM does not
    /// separate the two roles, so callers label them generically.
    #[serde(default)]
    pub companies: Vec<String>,
    /// Play modes ("Single player", "Co-op"), from the merged metadata.
    #[serde(default)]
    pub game_modes: Vec<String>,
    /// Age rating codes ("E", "12") from whichever board rated the game.
    #[serde(default)]
    pub age_ratings: Vec<String>,
    /// Community score out of 100, merged across sources.
    #[serde(default)]
    pub average_rating: Option<f64>,
}

impl RommGameMetadata {
    /// The release date in Unix **seconds**, normalised from whichever unit
    /// RomM sent. Reading the merged millisecond value as seconds put every
    /// release year roughly 25,000 years into the future.
    fn release_seconds(&self) -> Option<i64> {
        self.first_release_date.map(|timestamp| {
            if timestamp.abs() > RELEASE_MILLIS_THRESHOLD {
                timestamp / 1000
            } else {
                timestamp
            }
        })
    }

    /// Calendar year of the first release, for the grid's tile metadata.
    pub fn release_year(&self) -> Option<i32> {
        DateTime::from_timestamp(self.release_seconds()?, 0).map(|date| date.year())
    }

    /// Full `YYYY-MM-DD` release date, for the details screen.
    pub fn release_date(&self) -> Option<String> {
        DateTime::from_timestamp(self.release_seconds()?, 0)
            .map(|date| date.format("%Y-%m-%d").to_string())
    }
}

pub struct RommAsset {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// One of the user's RomM collections. RomM keeps favorites as a collection
/// flagged `is_favorite` rather than a per-rom column, so this is also how
/// favorite state is read and written.
#[derive(Clone, Debug, Deserialize)]
pub struct RommCollection {
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub is_favorite: bool,
    /// Roms in the collection, already filtered to the ones this token's user
    /// may see.
    #[serde(default)]
    pub rom_ids: Vec<i64>,
}
/// The one HTTP client the RomM calls share.
///
/// Each call used to build its own `reqwest::Client`, which means a fresh
/// connection pool, DNS cache and TLS session for every request — and the
/// frontend polls these endpoints several times a second. One client keeps the
/// pool warm. The per-request timeout still lives on each request builder, so the
/// differing deadlines below are unchanged.
fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

pub async fn platforms(
    settings: &SettingsRepository,
) -> std::result::Result<Vec<RommPlatform>, RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let mut platforms = query_romm(&credentials).await?;
    for platform in &mut platforms {
        platform.artwork_paths = platform_artwork_paths(platform);
    }
    Ok(platforms)
}

pub async fn games(
    settings: &SettingsRepository,
    platform_id: Option<i64>,
    search_term: Option<&str>,
    filters: &RommGameFilters,
    limit: u32,
    offset: u32,
) -> std::result::Result<RommGamePage, RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let mut params: Vec<(&str, String)> = vec![
        ("limit", limit.to_string()),
        ("offset", offset.to_string()),
        ("with_char_index", "false".to_owned()),
        ("with_filter_values", "false".to_owned()),
        ("with_rom_id_index", "false".to_owned()),
        // Collapse a game's variants (regions, revisions, discs) into one
        // entry, with the rest carried in `sibling_roms`. Without this RomM
        // pages each file separately, so a multi-disc title would occupy
        // several grid tiles. RomM keeps `total` in terms of these collapsed
        // groups, so pagination stays correct.
        ("group_by_meta_id", "true".to_owned()),
        ("order_by", "name".to_owned()),
        ("order_dir", "asc".to_owned()),
    ];
    if let Some(platform_id) = platform_id {
        params.push(("platform_ids", platform_id.to_string()));
    }
    if let Some(search_term) = search_term {
        params.push(("search_term", search_term.to_owned()));
    }
    // Each multi-value filter is repeated per value, with a `_logic` operator
    // that says how to combine them: `any` matches a rom carrying any listed
    // value, which is what selecting one chip means.
    for genre in &filters.genres {
        params.push(("genres", genre.clone()));
    }
    if !filters.genres.is_empty() {
        params.push(("genres_logic", "any".to_owned()));
    }
    for region in &filters.regions {
        params.push(("regions", region.clone()));
    }
    if !filters.regions.is_empty() {
        params.push(("regions_logic", "any".to_owned()));
    }
    let response = http()
        .get(format!("{}/api/roms", credentials.base_url))
        .query(&params)
        .header(reqwest::header::ACCEPT, "application/json")
        .bearer_auth(&credentials.token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(RommQueryError::Failed(anyhow::anyhow!(
            "RomM returned {status}"
        )));
    }
    response
        .json::<RommGamePage>()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))
}

/// The complete set of filter values RomM reports for a library scope.
///
/// RomM keeps these on a dedicated endpoint rather than on the paged rom list,
/// so the console grid can offer every genre and region without loading (or
/// scanning) the whole library first. Only the fields Hearthdeck filters on are
/// read; serde ignores the rest of RomM's filter dictionary.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RommFilterValues {
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub regions: Vec<String>,
}

pub async fn filter_values(
    settings: &SettingsRepository,
    platform_id: Option<i64>,
) -> std::result::Result<RommFilterValues, RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let mut request = http().get(format!("{}/api/roms/filters", credentials.base_url));
    if let Some(platform_id) = platform_id {
        request = request.query(&[("platform_ids", platform_id.to_string())]);
    }
    let response = request
        .header(reqwest::header::ACCEPT, "application/json")
        .bearer_auth(&credentials.token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(RommQueryError::Failed(anyhow::anyhow!(
            "RomM returned {status}"
        )));
    }
    response
        .json::<RommFilterValues>()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))
}

/// Fetches a single ROM by ID, for the RetroArch launch path: [`games`]
/// only returns the fields needed for browsing, and does not carry the
/// on-disk filename a launch needs to build a download URL.
pub async fn rom(
    settings: &SettingsRepository,
    rom_id: i64,
) -> std::result::Result<RommGame, RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let response = http()
        .get(format!("{}/api/roms/{rom_id}", credentials.base_url))
        .header(reqwest::header::ACCEPT, "application/json")
        .bearer_auth(&credentials.token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(RommQueryError::Failed(anyhow::anyhow!("rom not found")));
    }
    if !status.is_success() {
        return Err(RommQueryError::Failed(anyhow::anyhow!(
            "RomM returned {status}"
        )));
    }
    response
        .json::<RommGame>()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))
}

/// Name RomM's own UI gives the collection it flags as favorites. Only used
/// when creating one: an existing favorite collection is found by its flag, so
/// a differently named one is still reused.
const FAVORITE_COLLECTION_NAME: &str = "Favorites";

/// RomM credentials, or the error explaining why they are missing.
async fn romm_credentials(
    settings: &SettingsRepository,
) -> std::result::Result<RommCredentials, RommQueryError> {
    settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)
}

/// A RomM API request, carrying the base URL, JSON accept header, bearer token
/// and timeout every call shares.
fn romm_request(
    credentials: &RommCredentials,
    method: reqwest::Method,
    path: &str,
) -> reqwest::RequestBuilder {
    http()
        .request(method, format!("{}{path}", credentials.base_url))
        .header(reqwest::header::ACCEPT, "application/json")
        .bearer_auth(&credentials.token)
        .timeout(std::time::Duration::from_secs(10))
}

/// Names the failing status, so a token missing a write scope reads as a 403
/// instead of a generic failure.
fn romm_status_error(status: reqwest::StatusCode) -> RommQueryError {
    RommQueryError::Failed(anyhow::anyhow!("RomM returned {status}"))
}

async fn fetch_collections(
    credentials: &RommCredentials,
) -> std::result::Result<Vec<RommCollection>, RommQueryError> {
    let response = romm_request(credentials, reqwest::Method::GET, "/api/collections")
        .send()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    if !response.status().is_success() {
        return Err(romm_status_error(response.status()));
    }
    response
        .json::<Vec<RommCollection>>()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))
}

/// The user's favorite collection, if they have ever favorited anything.
async fn romm_favorite_collection(
    credentials: &RommCredentials,
) -> std::result::Result<Option<RommCollection>, RommQueryError> {
    Ok(fetch_collections(credentials)
        .await?
        .into_iter()
        .find(|collection| collection.is_favorite))
}

/// Whether `game` is in the user's favorites.
///
/// Uses the flag RomM puts on the rom when it sends one, and otherwise asks the
/// favorite collection, which is where favorites actually live.
pub async fn favorite(
    settings: &SettingsRepository,
    game: &RommGame,
) -> std::result::Result<bool, RommQueryError> {
    if let Some(favorite) = game.is_favorite {
        return Ok(favorite);
    }
    let credentials = romm_credentials(settings).await?;
    let Some(collection) = romm_favorite_collection(&credentials).await? else {
        return Ok(false);
    };
    Ok(collection.rom_ids.contains(&game.id))
}

/// Adds or removes `rom_id` from the user's favorites, returning the new state.
///
/// Creating the favorite collection on first use is what RomM's own UI does:
/// there is no per-rom favorite column to write, only a collection flagged as
/// the favorite list.
pub async fn set_favorite(
    settings: &SettingsRepository,
    rom_id: i64,
    favorite: bool,
) -> std::result::Result<bool, RommQueryError> {
    let credentials = romm_credentials(settings).await?;
    let collection = match romm_favorite_collection(&credentials).await? {
        Some(collection) => collection,
        None if favorite => romm_create_favorite_collection(&credentials).await?,
        // Removing from a favorite list that was never created leaves the user
        // in the state they asked for.
        None => return Ok(false),
    };
    // Which collection RomM treats as the favorite list is not something the
    // user can see anywhere, so name it in the log.
    debug!(id = collection.id, name = %collection.name, "resolved the RomM favorite collection");
    let method = if favorite {
        reqwest::Method::POST
    } else {
        reqwest::Method::DELETE
    };
    let response = romm_request(
        &credentials,
        method,
        &format!("/api/collections/{}/roms", collection.id),
    )
    .json(&serde_json::json!({ "rom_ids": [rom_id] }))
    .send()
    .await
    .map_err(|error| RommQueryError::Failed(error.into()))?;
    if !response.status().is_success() {
        return Err(romm_status_error(response.status()));
    }
    Ok(favorite)
}

/// Creates the collection RomM treats as this user's favorites.
async fn romm_create_favorite_collection(
    credentials: &RommCredentials,
) -> std::result::Result<RommCollection, RommQueryError> {
    // FastAPI reads `is_favorite` from the query string and the collection's
    // fields from the form body, so the flag stays out of the form.
    let response = romm_request(
        credentials,
        reqwest::Method::POST,
        "/api/collections?is_favorite=true",
    )
    .form(&[("name", FAVORITE_COLLECTION_NAME), ("description", "")])
    .send()
    .await
    .map_err(|error| RommQueryError::Failed(error.into()))?;
    if !response.status().is_success() {
        return Err(romm_status_error(response.status()));
    }
    response
        .json::<RommCollection>()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))
}

/// Sets RomM's per-user "play later" flag for one game, returning the new
/// state. One call: RomM stores it as a field of the user's row for the game.
pub async fn set_backlogged(
    settings: &SettingsRepository,
    rom_id: i64,
    backlogged: bool,
) -> std::result::Result<bool, RommQueryError> {
    let credentials = romm_credentials(settings).await?;
    let response = romm_request(
        &credentials,
        reqwest::Method::PUT,
        &format!("/api/roms/{rom_id}/props"),
    )
    .json(&serde_json::json!({ "backlogged": backlogged }))
    .send()
    .await
    .map_err(|error| RommQueryError::Failed(error.into()))?;
    if !response.status().is_success() {
        return Err(romm_status_error(response.status()));
    }
    Ok(backlogged)
}

/// RomM serves per-ROM artwork from this URL prefix, with everything after it
/// being the file's path under `RESOURCES_BASE_PATH`.
const ROMM_RESOURCES_URL_PREFIX: &str = "/assets/romm/resources/";

/// RomM's bundled per-platform console art. Unlike per-ROM artwork this is not
/// on any mounted path: it ships inside RomM's own container image.
const ROMM_PLATFORM_ART_URL_PREFIX: &str = "/assets/platforms/";

pub async fn asset(
    settings: &SettingsRepository,
    paths: &RommPaths,
    path: &str,
) -> std::result::Result<RommAsset, RommQueryError> {
    let path = normalized_romm_asset_path(path).map_err(RommQueryError::Failed)?;
    // Per-ROM artwork is read straight off the host's resources mount: RomM
    // serves those same bytes from that same disk, so going through its HTTP
    // API only adds a round trip and a dependency on the container being up.
    if let Some(relative) = resources_relative_path(&path) {
        return read_resources_asset(paths, relative).await;
    }
    // What is left is the bundled console art, which only RomM can serve.
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let response = http()
        .get(format!("{}{}", credentials.base_url, path))
        .bearer_auth(&credentials.token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(RommQueryError::Failed(anyhow::anyhow!(
            "RomM artwork request returned {status}"
        )));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if !content_type.starts_with("image/") {
        return Err(RommQueryError::Failed(anyhow::anyhow!(
            "RomM artwork response was not an image"
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?
        .to_vec();
    Ok(RommAsset {
        content_type,
        bytes,
    })
}

/// The file path behind a RomM resources URL, relative to the resources root.
///
/// RomM reports artwork as `/assets/romm/resources/<relative>?ts=<updated_at>`;
/// the query string only exists to bust a cache, so it is dropped here. `None`
/// for anything else, which after normalization is the bundled console art.
fn resources_relative_path(path: &str) -> Option<&str> {
    let relative = path.strip_prefix(ROMM_RESOURCES_URL_PREFIX)?;
    Some(relative.split('?').next().unwrap_or_default())
}

/// Reads one artwork file off RomM's resources mount. The extension decides the
/// content type, and a non-image extension is refused rather than served as an
/// image, mirroring the check the API path applies to RomM's own answer.
fn artwork_content_type(file: &Path) -> Option<&'static str> {
    let extension = file.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        _ => return None,
    })
}

async fn read_resources_asset(
    paths: &RommPaths,
    relative: &str,
) -> std::result::Result<RommAsset, RommQueryError> {
    let file = paths.resources_root.join(relative);
    let not_an_image = || {
        RommQueryError::Failed(anyhow::anyhow!(
            "RomM artwork {} is not an image",
            file.display()
        ))
    };
    let content_type = artwork_content_type(&file)
        .ok_or_else(not_an_image)?
        .to_owned();
    let bytes = tokio::fs::read(&file).await.map_err(|error| {
        RommQueryError::Failed(anyhow::anyhow!(
            "could not read RomM artwork {}: {error}",
            file.display()
        ))
    })?;
    Ok(RommAsset {
        content_type,
        bytes,
    })
}

async fn query_romm(
    credentials: &RommCredentials,
) -> std::result::Result<Vec<RommPlatform>, RommQueryError> {
    let response = http()
        .get(format!("{}/api/platforms", credentials.base_url))
        .header(reqwest::header::ACCEPT, "application/json")
        .bearer_auth(&credentials.token)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    let status = response.status();
    if !status.is_success() {
        let error = anyhow::anyhow!("RomM returned {status}");
        // Diagnostics polls this check periodically; the UI carries a degraded
        // state without turning one unavailable RomM server into a log flood.
        tracing::debug!(base_url = %credentials.base_url, %error, "RomM console check failed");
        return Err(RommQueryError::Failed(error));
    }
    let platforms = response
        .json::<Vec<RommPlatform>>()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    tracing::debug!(
        base_url = %credentials.base_url,
        console_count = platforms.len(),
        "RomM console check completed"
    );
    Ok(platforms)
}

fn normalized_romm_asset_path(value: &str) -> anyhow::Result<String> {
    let path = value.trim();
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    // Two RomM asset roots are allowed: per-ROM artwork under
    // `/assets/romm/resources/` and the bundled per-platform console art under
    // `/assets/platforms/`. Everything else is rejected so the proxy cannot be
    // walked into arbitrary server paths.
    let allowed = path.starts_with(ROMM_RESOURCES_URL_PREFIX)
        || path.starts_with(ROMM_PLATFORM_ART_URL_PREFIX);
    if !allowed || path.contains("..") || path.contains('#') || path.contains("//") {
        anyhow::bail!("invalid RomM artwork path");
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn romm_user_state_and_collections_parse_from_romm_shapes() {
        // Field names and nesting only; the values come from a real RomM
        // response, so a rename upstream fails here rather than silently
        // reading every game as unfavorited and un-backlogged.
        let user: super::RommRomUser = serde_json::from_str(
            r#"{"id": -1, "backlogged": true, "now_playing": false, "hidden": false, "is_main_sibling": false, "last_played": null}"#,
        )
        .expect("rom_user parses");
        assert!(user.backlogged);

        // A user with no row on the game gets RomM's all-default stub, which
        // must still parse rather than failing the whole detail read.
        let stub: super::RommRomUser = serde_json::from_str(r#"{"id": -1}"#).expect("stub parses");
        assert!(!stub.backlogged);

        let collections: Vec<super::RommCollection> = serde_json::from_str(
            r#"[
                {"id": 1, "name": "Favorites", "is_favorite": true, "rom_ids": [4014, 4015], "rom_count": 2},
                {"id": 2, "name": "RPGs", "is_favorite": false, "rom_ids": []}
            ]"#,
        )
        .expect("collections parse");
        let favorite = collections
            .iter()
            .find(|collection| collection.is_favorite)
            .expect("a favorite collection");
        assert_eq!(favorite.id, 1);
        assert_eq!(favorite.rom_ids, vec![4014, 4015]);
    }

    #[test]
    fn romm_release_dates_normalise_milliseconds_and_seconds() {
        use super::RommGameMetadata;

        // RomM's merged `metadatum` reports milliseconds (791_596_800_000 is the
        // value an actual RomM install returns for Zoop) while its raw IGDB
        // payload reports the same instant in seconds. Both must land on 1995.
        let milliseconds = RommGameMetadata {
            first_release_date: Some(791_596_800_000),
            ..Default::default()
        };
        assert_eq!(milliseconds.release_year(), Some(1995));
        assert_eq!(milliseconds.release_date().as_deref(), Some("1995-02-01"));

        let seconds = RommGameMetadata {
            first_release_date: Some(791_596_800),
            ..Default::default()
        };
        assert_eq!(seconds.release_year(), Some(1995));

        assert_eq!(RommGameMetadata::default().release_year(), None);
        assert_eq!(RommGameMetadata::default().release_date(), None);
    }

    #[test]
    fn accepts_only_romm_managed_resource_paths() {
        assert_eq!(
            super::normalized_romm_asset_path("/assets/romm/resources/roms/1/cover.webp?ts=1")
                .unwrap(),
            "/assets/romm/resources/roms/1/cover.webp?ts=1"
        );
        assert!(super::normalized_romm_asset_path("/api/roms").is_err());
        assert!(super::normalized_romm_asset_path("/resources/roms/1/cover.webp").is_err());
        assert!(super::normalized_romm_asset_path("https://example.com/cover.webp").is_err());
        assert!(super::normalized_romm_asset_path("/assets/romm/resources/../secret").is_err());
    }

    #[test]
    fn resources_urls_map_to_the_resources_mount_and_console_art_does_not() {
        // The `?ts=` suffix is a cache-buster RomM adds to the URL, not part of
        // the file's path, so it is dropped on the way to the disk.
        assert_eq!(
            super::resources_relative_path("/assets/romm/resources/roms/1/cover/big.png?ts=1"),
            Some("roms/1/cover/big.png")
        );
        assert_eq!(
            super::resources_relative_path("/assets/romm/resources/roms/1/cover/big.png"),
            Some("roms/1/cover/big.png")
        );
        // Bundled console art lives in RomM's image, so it has no disk path.
        assert_eq!(
            super::resources_relative_path("/assets/platforms/snes.svg"),
            None
        );
    }

    #[test]
    fn artwork_content_type_comes_from_the_file_extension() {
        use std::path::Path;

        assert_eq!(
            super::artwork_content_type(Path::new("a.png")),
            Some("image/png")
        );
        assert_eq!(
            super::artwork_content_type(Path::new("a.jpeg")),
            Some("image/jpeg")
        );
        assert_eq!(
            super::artwork_content_type(Path::new("a.PNG")),
            Some("image/png")
        );
        assert_eq!(
            super::artwork_content_type(Path::new("a.webp")),
            Some("image/webp")
        );
        assert_eq!(
            super::artwork_content_type(Path::new("a.ico")),
            Some("image/x-icon")
        );
        assert_eq!(super::artwork_content_type(Path::new("a.txt")), None);
        assert_eq!(super::artwork_content_type(Path::new("a")), None);
    }

    #[test]
    fn allows_platform_artwork_under_the_asset_proxy() {
        // RomM's bundled console art lives under a different root than per-ROM
        // artwork, so the proxy allowlist has to include it explicitly.
        assert_eq!(
            super::normalized_romm_asset_path("/assets/platforms/snes.svg").unwrap(),
            "/assets/platforms/snes.svg"
        );
        assert!(super::normalized_romm_asset_path("/assets/platforms/../secret").is_err());
        assert!(super::normalized_romm_asset_path("/assets/other/nes.svg").is_err());
    }

    #[test]
    fn platform_artwork_prefers_the_slug_then_the_filesystem_slug_then_default() {
        let platform = super::RommPlatform {
            id: 7,
            name: "Super Nintendo".to_owned(),
            display_name: None,
            rom_count: 3,
            slug: Some("snes".to_owned()),
            fs_slug: Some("super-nintendo".to_owned()),
            url_logo: None,
            artwork_paths: Vec::new(),
        };

        assert_eq!(
            super::platform_artwork_paths(&platform),
            vec![
                "/assets/platforms/snes.svg",
                "/assets/platforms/snes.ico",
                "/assets/platforms/super-nintendo.svg",
                "/assets/platforms/super-nintendo.ico",
                "/assets/platforms/default.ico",
            ]
        );
    }

    #[test]
    fn platform_artwork_deduplicates_matching_slugs_and_trims_case() {
        let platform = super::RommPlatform {
            id: 7,
            name: "Game Boy".to_owned(),
            display_name: None,
            rom_count: 1,
            slug: Some("GB".to_owned()),
            fs_slug: Some("gb".to_owned()),
            url_logo: None,
            artwork_paths: Vec::new(),
        };

        assert_eq!(
            super::platform_artwork_paths(&platform),
            vec![
                "/assets/platforms/gb.svg",
                "/assets/platforms/gb.ico",
                "/assets/platforms/default.ico",
            ]
        );
    }
}
