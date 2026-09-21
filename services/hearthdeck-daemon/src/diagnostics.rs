use std::{
    collections::VecDeque,
    process::Stdio,
    sync::{Mutex, OnceLock},
};

use chrono::{DateTime, Datelike, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::debug;

use crate::settings::{RommCredentials, SettingsRepository};

const LOG_LINE_LIMIT: usize = 200;
const LOG_MESSAGE_LIMIT: usize = 600;
const ROMM_LOG_LIMIT: usize = 40;
const MAX_ROM_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024 * 1024;

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
    /// asset proxy until one exists. Populated from `romm_platforms`, so it
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

#[derive(Clone, Debug, Deserialize)]
pub struct RommGame {
    pub id: i64,
    pub platform_id: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub fs_name_no_tags: String,
    /// Filename actually stored on RomM's disk, including tags/extension.
    /// Used to build the ROM content download URL for a RetroArch launch;
    /// distinct from `fs_name_no_tags`, which is only for display.
    #[serde(default)]
    pub fs_name: Option<String>,
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
    /// per page; `romm_games` asks RomM to collapse each sibling group into
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
    /// the favorite collection instead (see [`romm_favorite`]).
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

#[derive(Serialize)]
pub struct DiagnosticsSnapshot {
    pub generated_at: String,
    pub services: Vec<ServiceStatus>,
    pub romm: RommDiagnostic,
    pub logs: LogTail,
}

#[derive(Serialize)]
pub struct ServiceStatus {
    pub id: &'static str,
    pub unit: &'static str,
    pub state: String,
    pub detail: String,
}

#[derive(Serialize)]
pub struct RommDiagnostic {
    pub configured: bool,
    pub status: &'static str,
    pub base_url: Option<String>,
    pub console_count: Option<usize>,
    pub checked_at: String,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct LogTail {
    pub available: bool,
    pub error: Option<String>,
    pub entries: Vec<LogEntry>,
}

/// Stable identifier for where a log line came from. The COSMIC frontend
/// groups the diagnostics log tail into tabs keyed on this value: `daemon`
/// and `bridge` come from their respective systemd journals, `api` is the
/// daemon's own per-request access log (normally noisy, so it's split out
/// of the general `daemon` tab instead of being dropped), and `romm` is
/// synthesized locally from the periodic RomM connectivity check since RomM
/// itself is an external server with no journal we can tail.
#[derive(Serialize)]
pub struct LogEntry {
    pub timestamp: Option<String>,
    pub source: String,
    pub level: String,
    pub message: String,
}

pub async fn snapshot(settings: &SettingsRepository) -> DiagnosticsSnapshot {
    let (services, logs, romm) =
        tokio::join!(service_statuses(), recent_logs(), romm_diagnostic(settings));
    DiagnosticsSnapshot {
        generated_at: Utc::now().to_rfc3339(),
        services,
        romm,
        logs,
    }
}

pub async fn romm_platforms(
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

pub async fn romm_games(
    settings: &SettingsRepository,
    platform_id: Option<i64>,
    search_term: Option<&str>,
    limit: u32,
    offset: u32,
) -> std::result::Result<RommGamePage, RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let mut request = reqwest::Client::new()
        .get(format!("{}/api/roms", credentials.base_url))
        .query(&[
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
            ("with_char_index", "false".to_owned()),
            ("with_filter_values", "false".to_owned()),
            ("with_rom_id_index", "false".to_owned()),
            // Collapse a game's variants (regions, revisions, discs) into one
            // entry, with the rest carried in `sibling_roms`. Without this
            // RomM pages each file separately, so a multi-disc title would
            // occupy several grid tiles. RomM keeps `total` in terms of these
            // collapsed groups, so pagination stays correct.
            ("group_by_meta_id", "true".to_owned()),
            ("order_by", "name".to_owned()),
            ("order_dir", "asc".to_owned()),
        ]);
    if let Some(platform_id) = platform_id {
        request = request.query(&[("platform_ids", platform_id.to_string())]);
    }
    if let Some(search_term) = search_term {
        request = request.query(&[("search_term", search_term)]);
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
        .json::<RommGamePage>()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))
}

/// Fetches a single ROM by ID, for the RetroArch launch path: `romm_games`
/// only returns the fields needed for browsing, and does not carry the
/// on-disk filename a launch needs to build a download URL.
pub async fn romm_rom(
    settings: &SettingsRepository,
    rom_id: i64,
) -> std::result::Result<RommGame, RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let response = reqwest::Client::new()
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
    reqwest::Client::new()
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
pub async fn romm_favorite(
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
pub async fn romm_set_favorite(
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
pub async fn romm_set_backlogged(
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

/// Downloads a ROM into a temporary cache file ahead of a RetroArch launch.
/// A multi-disc set is handled one disc at a time: RomM exposes each disc as
/// its own rom and the client launches the chosen sibling, so this only ever
/// fetches a single content file. An `.m3u` playlist spanning several on-disk
/// files is still out of scope.
pub async fn download_rom_content(
    settings: &SettingsRepository,
    rom_id: i64,
    fs_name: &str,
    destination: &std::path::Path,
) -> std::result::Result<(), RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let url = rom_content_url(&credentials.base_url, rom_id, fs_name)?;
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(&credentials.token)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(RommQueryError::Failed(anyhow::anyhow!(
            "RomM returned {status} downloading rom content"
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ROM_DOWNLOAD_BYTES)
    {
        return Err(RommQueryError::Failed(anyhow::anyhow!(
            "RomM content exceeds the 16 GiB download limit"
        )));
    }

    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    let mut downloaded = 0_u64;
    let chunks = response.bytes_stream();
    tokio::pin!(chunks);
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|error| RommQueryError::Failed(error.into()))?;
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .filter(|length| *length <= MAX_ROM_DOWNLOAD_BYTES)
            .ok_or_else(|| {
                RommQueryError::Failed(anyhow::anyhow!(
                    "RomM content exceeds the 16 GiB download limit"
                ))
            })?;
        file.write_all(&chunk)
            .await
            .map_err(|error| RommQueryError::Failed(error.into()))?;
    }
    file.sync_all()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))
}

/// Builds the RomM content-download URL for a rom. Kept as a separate
/// function so the exact path shape is unit-testable without a network call:
/// the RetroArch launch path depends on this matching RomM's
/// `/api/roms/{id}/content/{file_name}` route exactly.
///
/// The base path deliberately has no trailing slash after `content`: the url
/// crate's `path_segments_mut().push` keeps an existing empty final segment,
/// which would serialize the path as `/content//<file>` and 404 against
/// RomM's single-slash route.
fn rom_content_url(
    base_url: &str,
    rom_id: i64,
    fs_name: &str,
) -> std::result::Result<reqwest::Url, RommQueryError> {
    let mut url = reqwest::Url::parse(&format!("{base_url}/api/roms/{rom_id}/content"))
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    url.path_segments_mut()
        .map_err(|_| RommQueryError::Failed(anyhow::anyhow!("invalid RomM content URL")))?
        .push(fs_name);
    Ok(url)
}

pub async fn romm_asset(
    settings: &SettingsRepository,
    path: &str,
) -> std::result::Result<RommAsset, RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let path = normalized_romm_asset_path(path).map_err(RommQueryError::Failed)?;
    let response = reqwest::Client::new()
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

async fn romm_diagnostic(settings: &SettingsRepository) -> RommDiagnostic {
    let checked_at = Utc::now().to_rfc3339();
    let configured = match settings.romm().await {
        Ok(configured) => configured,
        Err(error) => {
            let message = format!("Could not read RomM settings: {error}");
            push_romm_log("error", message.clone());
            return RommDiagnostic {
                configured: false,
                status: "degraded",
                base_url: None,
                console_count: None,
                checked_at,
                error: Some(message),
            };
        }
    };
    let Some(configured) = configured else {
        push_romm_log("info", "RomM is not configured.".to_owned());
        return RommDiagnostic {
            configured: false,
            status: "not_configured",
            base_url: None,
            console_count: None,
            checked_at,
            error: None,
        };
    };
    match romm_platforms(settings).await {
        Ok(platforms) => {
            push_romm_log(
                "info",
                format!(
                    "Connected to {} ({} consoles available)",
                    configured.base_url,
                    platforms.len()
                ),
            );
            RommDiagnostic {
                configured: true,
                status: "ready",
                base_url: Some(configured.base_url),
                console_count: Some(platforms.len()),
                checked_at,
                error: None,
            }
        }
        Err(error) => {
            let message = match error {
                RommQueryError::NotConfigured => "RomM is not configured".to_owned(),
                RommQueryError::Failed(error) => error.to_string(),
            };
            push_romm_log(
                "error",
                format!("Could not reach {}: {message}", configured.base_url),
            );
            RommDiagnostic {
                configured: true,
                status: "degraded",
                base_url: Some(configured.base_url),
                console_count: None,
                checked_at,
                error: Some(message),
            }
        }
    }
}

fn romm_log_buffer() -> &'static Mutex<VecDeque<LogEntry>> {
    static BUFFER: OnceLock<Mutex<VecDeque<LogEntry>>> = OnceLock::new();
    BUFFER.get_or_init(|| Mutex::new(VecDeque::with_capacity(ROMM_LOG_LIMIT)))
}

/// Records an event from the periodic RomM connectivity check so the
/// diagnostics log tail has real content for the "RomM" tab, even though
/// RomM runs as an external server with no local journal to tail. Repeated
/// identical messages (e.g. "still connected" on every 5s poll) collapse
/// into the existing entry instead of flooding the tab with duplicates.
fn push_romm_log(level: &str, message: String) {
    let Ok(mut buffer) = romm_log_buffer().lock() else {
        return;
    };
    if buffer.back().is_some_and(|entry| entry.message == message) {
        return;
    }
    if buffer.len() >= ROMM_LOG_LIMIT {
        buffer.pop_front();
    }
    buffer.push_back(LogEntry {
        timestamp: Some(Utc::now().to_rfc3339()),
        source: "romm".to_owned(),
        level: level.to_owned(),
        message,
    });
}

fn recent_romm_logs() -> Vec<LogEntry> {
    let Ok(buffer) = romm_log_buffer().lock() else {
        return Vec::new();
    };
    buffer
        .iter()
        .rev()
        .map(|entry| LogEntry {
            timestamp: entry.timestamp.clone(),
            source: entry.source.clone(),
            level: entry.level.clone(),
            message: entry.message.clone(),
        })
        .collect()
}

async fn query_romm(
    credentials: &RommCredentials,
) -> std::result::Result<Vec<RommPlatform>, RommQueryError> {
    let response = reqwest::Client::new()
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
    let allowed =
        path.starts_with("/assets/romm/resources/") || path.starts_with("/assets/platforms/");
    if !allowed || path.contains("..") || path.contains('#') || path.contains("//") {
        anyhow::bail!("invalid RomM artwork path");
    }
    Ok(path)
}

async fn service_statuses() -> Vec<ServiceStatus> {
    let (session, daemon, bridge_socket, bridge, romm_container) = tokio::join!(
        service_status("session", "hearthdeck.target", false),
        service_status("daemon", "hearthdeck-daemon.service", false),
        service_status("bridge_socket", "hearthdeck-bridge.socket", false),
        service_status("bridge", "hearthdeck-bridge.service", true),
        // Optional: the packaged unit is skipped when its compose file is
        // absent. This is safe to query unconditionally.
        service_status("romm_container", "romm.service", false),
    );
    vec![session, daemon, bridge_socket, bridge, romm_container]
}

async fn service_status(id: &'static str, unit: &'static str, on_demand: bool) -> ServiceStatus {
    let output = Command::new("systemctl")
        .args([
            "--user",
            "show",
            unit,
            "--property=ActiveState",
            "--property=SubState",
            "--value",
        ])
        .stdin(Stdio::null())
        .output()
        .await;
    let Ok(output) = output else {
        return ServiceStatus {
            id,
            unit,
            state: "unavailable".to_owned(),
            detail: "Could not query the user service manager.".to_owned(),
        };
    };
    if !output.status.success() {
        return ServiceStatus {
            id,
            unit,
            state: "unavailable".to_owned(),
            detail: "The user service manager did not return this unit.".to_owned(),
        };
    }
    let output = String::from_utf8_lossy(&output.stdout);
    let states = output
        .lines()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let active_state = states.first().copied().unwrap_or("unknown");
    let sub_state = states.get(1).copied().unwrap_or("unknown");
    ServiceStatus {
        id,
        unit,
        state: active_state.to_owned(),
        detail: if on_demand && active_state == "inactive" {
            "On demand; starts when the daemon needs a host request.".to_owned()
        } else {
            format!("{active_state} ({sub_state})")
        },
    }
}

/// The `systemd --user` unit [`restart_romm_service`] acts on. A named constant,
/// never built from request data: this is a narrowly scoped action on one
/// specific unit, not a generic "restart any unit" capability.
const ROMM_SERVICE_UNIT: &str = "romm.service";

/// The fixed argv that restarts it, kept as data so its scope is assertable in a
/// test without a systemd user session present - and without restarting a real
/// RomM stack as a side effect of running the test suite.
const ROMM_RESTART_ARGS: [&str; 3] = ["--user", "restart", ROMM_SERVICE_UNIT];

/// Maps a finished `systemctl restart` onto the caller's result. A missing unit
/// fails the same way `systemctl` itself reports it, surfaced to the caller
/// rather than silently ignored. `systemd` can also fail with nothing on stderr,
/// so that case still yields a message naming the unit that was rejected.
fn romm_restart_result(success: bool, stderr: &[u8]) -> anyhow::Result<()> {
    if success {
        return Ok(());
    }
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    Err(if message.is_empty() {
        anyhow::anyhow!("systemd rejected the {ROMM_SERVICE_UNIT} restart")
    } else {
        anyhow::anyhow!("systemd rejected the {ROMM_SERVICE_UNIT} restart: {message}")
    })
}

/// Restarts the optional RomM systemd unit
/// (`deploy/systemd/romm.service`). The unit name is a fixed
/// constant, never caller-supplied: this is a narrowly scoped action on one
/// specific service, not a generic "restart any unit" capability. A missing
/// unit fails the same way `systemctl` itself reports it, surfaced to the
/// caller rather than silently ignored.
pub async fn restart_romm_service() -> anyhow::Result<()> {
    let output = Command::new("systemctl")
        .args(ROMM_RESTART_ARGS)
        .stdin(Stdio::null())
        .output()
        .await?;
    romm_restart_result(output.status.success(), &output.stderr)
}

async fn recent_logs() -> LogTail {
    let romm_logs = recent_romm_logs();
    let output = Command::new("journalctl")
        .args([
            "--user",
            "--no-pager",
            "--output=json",
            "--reverse",
            "--lines=400",
            "--unit=hearthdeck-daemon.service",
            "--unit=hearthdeck-bridge.service",
            // Optional: the unit may be skipped and have no journal entries.
            "--unit=romm.service",
        ])
        .stdin(Stdio::null())
        .output()
        .await;
    let Ok(output) = output else {
        // The journal itself is unavailable, but RomM connectivity events are
        // synthesized locally and don't depend on it, so they still show up
        // in their own tab.
        return LogTail {
            available: false,
            error: Some("Could not start journalctl for Hearthdeck services.".to_owned()),
            entries: romm_logs,
        };
    };
    if !output.status.success() {
        return LogTail {
            available: false,
            error: Some("The user journal is unavailable for Hearthdeck services.".to_owned()),
            entries: romm_logs,
        };
    }
    let mut entries: Vec<LogEntry> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_journal_entry)
        .take(LOG_LINE_LIMIT)
        .collect();
    entries.extend(romm_logs);
    LogTail {
        available: true,
        error: None,
        entries,
    }
}

fn parse_journal_entry(line: &str) -> Option<LogEntry> {
    let record = serde_json::from_str::<Value>(line).ok()?;
    let raw_message = record.get("MESSAGE")?.as_str()?;
    let unit = record
        .get("_SYSTEMD_UNIT")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source = log_source(unit, raw_message);
    // display_message's redaction is tuned for Hearthdeck's own structured
    // JSON log lines, where a stray local path or bearer token in an error
    // string would be an accidental leak. romm.service's lines are plain
    // Podman Compose output: paths, image references, and container
    // names are the entire point of reading them, not secrets, and that
    // process never has access to Hearthdeck's own tokens. Redacting them
    // the same way would turn the one tab meant to show a real
    // WorkingDirectory/image-pull error into a wall of "[path]".
    let message = if source == "romm" {
        truncate_plain(raw_message)
    } else {
        display_message(raw_message)
    };
    let timestamp = record
        .get("__REALTIME_TIMESTAMP")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(DateTime::from_timestamp_micros)
        .map(|timestamp| timestamp.to_rfc3339());
    let level = record
        .get("PRIORITY")
        .and_then(Value::as_str)
        .map(priority_label)
        .unwrap_or_else(|| "info".to_owned());
    Some(LogEntry {
        timestamp,
        source,
        level,
        message,
    })
}

fn truncate_plain(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() > LOG_MESSAGE_LIMIT {
        let mut truncated = trimmed.chars().take(LOG_MESSAGE_LIMIT).collect::<String>();
        truncated.push_str("...");
        truncated
    } else {
        trimmed.to_owned()
    }
}

fn display_message(raw: &str) -> String {
    let Ok(Value::Object(event)) = serde_json::from_str::<Value>(raw) else {
        return truncate_and_redact(raw);
    };
    let message = event
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("structured service event");
    let details = [
        "source_id",
        "provider_id",
        "record_count",
        "console_count",
        "installed_game_count",
        "status_code",
        "item_id",
        "base_url",
        "error",
    ]
    .into_iter()
    .filter_map(|key| event.get(key).map(|value| (key, value)))
    .map(|(key, value)| format!("{key}={}", value_to_text(value)))
    .collect::<Vec<_>>();
    let rendered = if details.is_empty() {
        message.to_owned()
    } else {
        format!("{message} ({})", details.join(", "))
    };
    truncate_and_redact(&rendered)
}

fn value_to_text(value: &Value) -> String {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| value.to_string())
}

/// Assigns a stable source id per journal line. The daemon's own per-request
/// access log (`"request completed"`, previously dropped entirely as noise)
/// is split into its own `api` source instead of being merged with general
/// `daemon` events, so the two can be viewed as separate log tabs. Lines
/// from the optional `romm.service` unit (deploy/systemd/romm.service,
/// Podman Compose's own start/stop output) join the same `romm` tab the
/// synthesized RomM connectivity-check messages already use (`push_romm_log`),
/// rather than getting a separate tab, since both are "what's going on with
/// RomM" from the user's point of view.
fn log_source(unit: &str, raw_message: &str) -> String {
    match unit {
        "hearthdeck-bridge.service" => "bridge",
        "romm.service" => "romm",
        _ if structured_message(raw_message).as_deref() == Some("request completed") => "api",
        _ => "daemon",
    }
    .to_owned()
}

fn structured_message(raw: &str) -> Option<String> {
    let Value::Object(event) = serde_json::from_str::<Value>(raw).ok()? else {
        return None;
    };
    event
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn priority_label(priority: &str) -> String {
    match priority {
        "0" | "1" | "2" | "3" => "error".to_owned(),
        "4" => "warning".to_owned(),
        "5" | "6" => "info".to_owned(),
        _ => "debug".to_owned(),
    }
}

fn truncate_and_redact(value: &str) -> String {
    let mut redacted = String::new();
    let mut redact_next = false;
    for word in value.split_inclusive(char::is_whitespace) {
        let trimmed = word.trim();
        let is_token =
            trimmed.starts_with("rmm_") || trimmed.starts_with("hearthdeck_") || redact_next;
        let is_path = trimmed.contains('/');
        redact_next = trimmed.eq_ignore_ascii_case("bearer");
        if is_token || is_path {
            redacted.push_str(if is_path { "[path]" } else { "[redacted]" });
            if word.ends_with(char::is_whitespace) {
                redacted.push(' ');
            }
        } else {
            redacted.push_str(word);
        }
    }
    if redacted.chars().count() > LOG_MESSAGE_LIMIT {
        let mut truncated = redacted.chars().take(LOG_MESSAGE_LIMIT).collect::<String>();
        truncated.push_str("...");
        truncated
    } else {
        redacted
    }
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
        use crate::diagnostics::RommGameMetadata;

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
    fn romm_compose_service_is_optional_and_session_managed() {
        let service = include_str!("../../../deploy/systemd/romm.service");
        let log_service = include_str!("../../../deploy/systemd/hearthdeck-log.service");
        let deploy_target = include_str!("../../../deploy/systemd/hearthdeck.target");
        let package_target = include_str!("../../../packaging/arch/hearthdeck.target");
        let kiosk_session = include_str!("../../../packaging/arch/hearthdeck-session");
        let acceptance = include_str!("../../../scripts/linux-acceptance");
        let package = include_str!("../../../packaging/arch/PKGBUILD");
        let install = include_str!("../../../packaging/arch/hearthdeck.install");
        let justfile = include_str!("../../../justfile");

        assert!(
            service
                .contains("Environment=ROMM_COMPOSE_FILE=/mnt/external/romM/podman-compose.yaml")
        );
        assert!(service.contains("EnvironmentFile=-%h/.config/hearthdeck/romm.env"));
        assert!(service.contains("ExecCondition=/usr/bin/test -f ${ROMM_COMPOSE_FILE}"));
        assert!(service.contains("Starting RomM Podman Compose stack"));
        assert!(
            service.contains("podman-compose -f ${ROMM_COMPOSE_FILE} $ROMM_COMPOSE_ARGS up -d")
        );
        assert!(service.contains("podman-compose -f ${ROMM_COMPOSE_FILE} $ROMM_COMPOSE_ARGS down"));
        assert!(service.contains("SyslogIdentifier=romm"));
        assert!(service.contains("PartOf=hearthdeck.target"));
        // `$ROMM_COMPOSE_ARGS` (no braces) is what makes systemd split the value
        // into separate arguments, and drop the argument entirely when the
        // variable is unset - a `${ROMM_COMPOSE_ARGS}` spelling would pass one
        // empty argument and break the no-override case.
        assert!(service.contains("$ROMM_COMPOSE_ARGS"));
        assert!(!service.contains("${ROMM_COMPOSE_ARGS}"));
        // No restart policy: `podman-compose up` recreates containers to converge,
        // so retrying a failed up tears down a stack that was already running.
        // Asserted on whole lines so the explanatory comment above is allowed to
        // name the directive.
        assert!(
            !service.lines().any(|line| line.starts_with("Restart=")),
            "romm.service must not auto-restart: retrying a failed `podman-compose up` recreates containers"
        );
        // romm.service's Documentation= must resolve to a file the package ships.
        assert!(service.contains("Documentation=file:///usr/share/doc/hearthdeck/ROMM.md"));
        assert!(package.contains("docs/retroarch-integration.md"));
        assert!(package.contains("usr/share/doc/hearthdeck/ROMM.md"));
        // podman's default networking opens /dev/net/tun, mirroring the uinput
        // module the controller broker already needs.
        assert!(install.contains("modprobe tun"));
        assert!(log_service.contains("StandardOutput=append:%h/hearthdeck.log"));
        assert!(log_service.contains("ExecStartPre=/usr/bin/truncate --size=0 %h/hearthdeck.log"));
        assert!(log_service.contains("ExecStartPre=/usr/bin/chmod 600 %h/hearthdeck.log"));
        assert!(log_service.contains("--identifier=hearthdeck-session"));
        assert!(log_service.contains("--identifier=cosmic-test-session"));
        assert!(log_service.contains("--identifier=hearthdeck-daemon"));
        assert!(log_service.contains("--identifier=hearthdeck-bridge"));
        assert!(log_service.contains("--identifier=hearthdeck-input"));
        assert!(log_service.contains("--identifier=hearthdeck-overlay"));
        assert!(log_service.contains("--identifier=romm"));
        assert!(kiosk_session.contains("systemd-cat -t hearthdeck-session"));
        assert!(
            deploy_target.contains(
                "Wants=hearthdeck-log.service hearthdeck-bridge.socket hearthdeck-daemon.service hearthdeck-input.service romm.service"
            )
        );
        assert!(
            package_target.contains(
                "Wants=hearthdeck-log.service hearthdeck-bridge.socket hearthdeck-daemon.service hearthdeck-input.service romm.service"
            )
        );
        assert!(package.contains("deploy/systemd/hearthdeck-log.service"));
        assert!(package.contains("deploy/systemd/romm.service"));
        assert!(package.contains("deploy/systemd/romm.env.example"));
        assert!(package.contains("scripts/linux-acceptance"));
        assert!(acceptance.contains("systemctl --user restart hearthdeck.target"));
        assert!(acceptance.contains("http://127.0.0.1:38400/v1/health"));
        assert!(
            acceptance.contains("podman-compose -f \"$romm_compose_file\" $romm_compose_args ps")
        );
        // The acceptance check must reconstruct the same podman-compose argv the
        // unit runs, overrides included, or it verifies a stack nobody started.
        assert!(acceptance.contains("ROMM_COMPOSE_ARGS"));
        assert!(acceptance.contains("stat -c '%a' \"$log_file\""));
        assert!(justfile.contains("./scripts/linux-acceptance --require-romm"));
        assert!(justfile.contains("cp deploy/systemd/hearthdeck-log.service"));
        assert!(justfile.contains("cp deploy/systemd/romm.service"));
    }

    #[test]
    fn renders_structured_journal_messages_for_the_diagnostics_view() {
        let entry = super::parse_journal_entry(
            r#"{"MESSAGE":"{\"level\":\"INFO\",\"message\":\"discovery completed\",\"source_id\":\"heroic\",\"record_count\":2}","PRIORITY":"6","_SYSTEMD_UNIT":"hearthdeck-daemon.service","__REALTIME_TIMESTAMP":"1760000000000000"}"#,
        )
        .unwrap();

        assert_eq!(entry.source, "daemon");
        assert_eq!(entry.level, "info");
        assert_eq!(
            entry.message,
            "discovery completed (source_id=heroic, record_count=2)"
        );
    }

    #[test]
    fn splits_request_access_lines_into_their_own_api_source() {
        let entry = super::parse_journal_entry(
            r#"{"MESSAGE":"{\"level\":\"INFO\",\"message\":\"request completed\",\"status_code\":200}","PRIORITY":"6","_SYSTEMD_UNIT":"hearthdeck-daemon.service","__REALTIME_TIMESTAMP":"1760000000000000"}"#,
        )
        .unwrap();

        assert_eq!(entry.source, "api");
    }

    #[test]
    fn labels_bridge_journal_lines_with_the_bridge_source() {
        let entry = super::parse_journal_entry(
            r#"{"MESSAGE":"{\"level\":\"INFO\",\"message\":\"bridge ready\"}","PRIORITY":"6","_SYSTEMD_UNIT":"hearthdeck-bridge.service","__REALTIME_TIMESTAMP":"1760000000000000"}"#,
        )
        .unwrap();

        assert_eq!(entry.source, "bridge");
    }

    #[test]
    fn labels_romm_service_journal_lines_with_the_romm_source_unredacted() {
        // Plain Podman Compose stdout, not Hearthdeck's own structured JSON
        // log shape, and full of legitimate paths/image refs that a real
        // configuration failure needs to stay readable.
        let entry = super::parse_journal_entry(
            r#"{"MESSAGE":"Error: WorkingDirectory '/home/alex/mnt/external/romM' not found","PRIORITY":"3","_SYSTEMD_UNIT":"romm.service","__REALTIME_TIMESTAMP":"1760000000000000"}"#,
        )
        .unwrap();

        assert_eq!(entry.source, "romm");
        assert_eq!(
            entry.message,
            "Error: WorkingDirectory '/home/alex/mnt/external/romM' not found"
        );
    }

    #[test]
    fn redacts_credentials_before_returning_log_messages() {
        let message =
            super::truncate_and_redact("token rmm_secret_value bearer hearthdeck_client_secret");

        assert_eq!(message, "token [redacted] bearer [redacted]");
    }

    #[test]
    fn redacts_local_paths_before_returning_log_messages() {
        let message =
            super::truncate_and_redact("could not read /home/alex/.config/heroic/installed.json");

        assert_eq!(message, "could not read [path]");
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

    #[test]
    fn content_url_has_a_single_slash_between_content_and_the_rom_file() {
        // Regression: the download URL used to serialize as
        // `/api/roms/{id}/content//<file>` (an empty path segment kept by
        // `path_segments_mut().push` after a trailing slash), which RomM
        // answers with 404 because its route is `/content/{file_name}`.
        let url = super::rom_content_url(
            "http://127.0.0.1:8080",
            289,
            "Advance Guardian Heroes (NA).gba",
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:8080/api/roms/289/content/Advance%20Guardian%20Heroes%20(NA).gba"
        );
    }

    #[test]
    fn romm_restart_targets_the_one_fixed_unit() {
        // Locks in scope: this may only ever restart the single fixed unit, and
        // its name may not become a request parameter (the same discipline the
        // install-request boundary uses).
        assert_eq!(
            super::ROMM_RESTART_ARGS,
            ["--user", "restart", "romm.service"]
        );
    }

    #[test]
    fn romm_restart_surfaces_systemd_failures_instead_of_swallowing_them() {
        assert!(super::romm_restart_result(true, b"").is_ok());

        let reported = super::romm_restart_result(
            false,
            b"Failed to restart romm.service: Unit romm.service not found.\n",
        )
        .unwrap_err()
        .to_string();
        assert!(
            reported.contains("Unit romm.service not found"),
            "{reported}"
        );

        // systemd can fail with nothing useful on stderr; the message must still
        // name the unit that was rejected.
        let bare = super::romm_restart_result(false, b"  \n")
            .unwrap_err()
            .to_string();
        assert!(bare.contains("romm.service"), "{bare}");
    }
}
