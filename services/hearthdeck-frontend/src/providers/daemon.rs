use async_trait::async_trait;
use futures::{StreamExt, future::join_all, stream};
use hearthdeck_protocol::{ApplicationSession, InputProfile};
use serde::{Deserialize, Serialize};
use std::{
    env,
    hash::{DefaultHasher, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{OnceCell, mpsc};

use super::{GameProvider, GameRecord};
use crate::app_group::romm_console_category;

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

/// Response from the /v1/health endpoint.
#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct HealthResponse {
    pub version: String,
    pub lan_enabled: bool,
    pub transport: String,
    pub providers: Vec<ProviderHealthInfo>,
    pub capabilities: HostCapabilities,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct ProviderHealthInfo {
    pub id: String,
    pub status: String,
    pub record_count: Option<u64>,
    pub last_attempt_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct HostCapabilities {
    pub launch: bool,
    pub application_sessions: bool,
    pub install_requests: bool,
    pub retro_launch: bool,
}

/// Catalog item returned by the daemon's /v1/library endpoint.
#[derive(Debug, Deserialize, Serialize)]
pub struct CatalogItem {
    pub id: String,
    pub source_id: String,
    pub title: String,
    pub kind: String,
    pub launch_id: Option<String>,
    pub icon: Option<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct RecentActivityItem {
    id: String,
    title: String,
    icon: Option<String>,
    categories: Vec<String>,
    source: String,
    metadata: serde_json::Value,
}

/// Server event received via WebSocket.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    LibraryChanged,
    ApplicationSessionChanged { session: Option<ApplicationSession> },
    InstallRequested { item_id: String },
}

/// Retro game from the daemon's /v1/retro/roms endpoint.
#[derive(Clone, Debug, Deserialize)]
pub struct RetroGame {
    pub id: i64,
    pub platform_id: i64,
    pub title: String,
    pub summary: Option<String>,
    pub cover_path: Option<String>,
    pub screenshot_paths: Vec<String>,
    pub genres: Vec<String>,
    pub release_year: Option<i32>,
}

/// Retro platform from the daemon's /v1/retro/consoles endpoint.
#[derive(Clone, Debug, Deserialize)]
pub struct RetroPlatform {
    pub id: i64,
    pub name: String,
    pub display_name: Option<String>,
    pub rom_count: u64,
}

impl RetroPlatform {
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }
}

impl DaemonClient {
    /// Creates a new daemon client with the given configuration.
    pub fn new(config: DaemonConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            token: Arc::new(OnceCell::new()),
        }
    }

    /// Returns the base URL for API requests.
    fn api_url(&self, path: &str) -> String {
        format!("{}{}", self.config.base_url, path)
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
            return Err(DaemonError::UnexpectedStatus(response.status()));
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
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }
        response
            .json::<PairingCompleteResponse>()
            .await
            .map(|pairing| pairing.token)
            .map_err(DaemonError::Deserialization)
    }

    /// Checks if the daemon is reachable and returns its health status.
    #[allow(dead_code)]
    pub async fn health(&self) -> Result<HealthResponse, DaemonError> {
        let response = self
            .http
            .get(self.api_url("/v1/health"))
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(DaemonError::UnexpectedStatus(response.status()));
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
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    pub async fn recent_records(&self, limit: u32) -> Result<Vec<GameRecord>, DaemonError> {
        let response = self
            .http
            .get(self.api_url("/v1/activity/recent"))
            .query(&[("limit", limit)])
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;
        if !response.status().is_success() {
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }
        let items: Vec<RecentActivityItem> = response
            .json()
            .await
            .map_err(DaemonError::Deserialization)?;

        Ok(stream::iter(items.into_iter().map(|item| {
            let client = self.clone();
            async move {
                let icon = if item.source == "romm" {
                    client.cache_retro_cover_path(item.icon.as_deref()).await
                } else {
                    item.icon.clone()
                };
                recent_activity_to_game_record(item, icon)
            }
        }))
        .buffered(8)
        .collect()
        .await)
    }

    /// Triggers a rescan of all discovery and enrichment providers.
    #[allow(dead_code)]
    pub async fn rescan_library(&self) -> Result<(), DaemonError> {
        let response = self
            .http
            .post(self.api_url("/v1/library/rescan"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }

        Ok(())
    }

    /// Launches an application by its catalog item ID.
    pub async fn launch_app(
        &self,
        id: &str,
        input_profile: InputProfile,
    ) -> Result<ApplicationSession, DaemonError> {
        let response = self
            .http
            .post(self.api_url(&format!("/v1/apps/{}/launch", id)))
            .query(&[("input_profile", input_profile)])
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(DaemonError::UnexpectedStatus(response.status()));
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
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Stops a running application session.
    #[allow(dead_code)]
    pub async fn stop_session(&self, session_id: &str) -> Result<(), DaemonError> {
        let response = self
            .http
            .post(self.api_url(&format!("/v1/sessions/{}/stop", session_id)))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }

        Ok(())
    }

    /// Lists available retro consoles from RomM.
    pub async fn list_retro_consoles(&self) -> Result<Vec<RetroPlatform>, DaemonError> {
        let response = self
            .http
            .get(self.api_url("/v1/retro/consoles"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Lists retro ROMs, optionally filtered by platform.
    pub async fn list_retro_roms(
        &self,
        platform_id: Option<i64>,
        search: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<RetroGamesResponse, DaemonError> {
        let mut url = self.api_url("/v1/retro/roms");
        let mut params = Vec::new();

        if let Some(platform_id) = platform_id {
            params.push(format!("platform_id={}", platform_id));
        }
        if let Some(search) = search.filter(|s| !s.is_empty()) {
            params.push(format!("q={}", urlencoding::encode(search)));
        }
        params.push(format!("limit={}", limit));
        params.push(format!("offset={}", offset));

        if !params.is_empty() {
            url = format!("{}?{}", url, params.join("&"));
        }

        let response = self
            .http
            .get(&url)
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    pub async fn list_retro_records(
        &self,
        platform_id: Option<i64>,
        limit: u32,
        offset: u32,
    ) -> Result<RetroRecordPage, DaemonError> {
        let page = self
            .list_retro_roms(platform_id, None, limit, offset)
            .await?;
        let records = stream::iter(page.items.into_iter().map(|game| {
            let client = self.clone();
            async move {
                let icon = client.cache_retro_cover(&game).await;
                retro_game_to_game_record(game, icon)
            }
        }))
        .buffered(8)
        .collect()
        .await;
        Ok(RetroRecordPage {
            items: records,
            total: page.total,
            offset: page.offset,
        })
    }

    async fn cache_retro_cover(&self, game: &RetroGame) -> Option<String> {
        self.cache_retro_cover_path(game.cover_path.as_deref())
            .await
    }

    async fn cache_retro_cover_path(&self, path: Option<&str>) -> Option<String> {
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

    /// Launches a retro ROM by its ID.
    pub async fn launch_retro_rom(
        &self,
        rom_id: i64,
        input_profile: InputProfile,
    ) -> Result<ApplicationSession, DaemonError> {
        let response = self
            .http
            .post(self.api_url(&format!("/v1/retro/roms/{}/launch", rom_id)))
            .query(&[("input_profile", input_profile)])
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Connects to the daemon's event stream via polling.
    /// Returns a channel receiver that yields server events.
    #[allow(dead_code)]
    pub async fn connect_events(&self) -> Result<mpsc::Receiver<ServerEvent>, DaemonError> {
        let (tx, rx) = mpsc::channel(32);

        // Spawn a task that polls for events
        let client = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                // Poll active session changes
                match client.active_session().await {
                    Ok(session) => {
                        let event = ServerEvent::ApplicationSessionChanged { session };
                        if tx.send(event).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        // Daemon might be unavailable, continue polling
                    }
                }
            }
        });

        Ok(rx)
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

    // Determine if this prefers dGPU based on metadata
    let prefers_dgpu = metadata
        .get("prefers_dgpu")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    GameRecord {
        id: format!("hearthdeck:{}", item.id),
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

pub fn retro_game_to_game_record(game: RetroGame, icon: Option<String>) -> GameRecord {
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

    GameRecord {
        id: format!("romm:{}", game.id),
        name: game.title,
        exec: Some(game.id.to_string()),
        icon,
        path: None,
        categories: vec!["Game".to_string(), romm_console_category(game.platform_id)],
        terminal: false,
        prefers_dgpu: false,
        source: "RomM".to_string(),
        metadata: serde_json::Value::Object(metadata),
    }
}

fn recent_activity_to_game_record(item: RecentActivityItem, icon: Option<String>) -> GameRecord {
    let prefers_dgpu = item
        .metadata
        .get("prefers_dgpu")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    GameRecord {
        id: item.id,
        name: item.title,
        exec: None,
        icon,
        path: None,
        categories: item.categories,
        terminal: false,
        prefers_dgpu,
        source: item.source,
        metadata: item.metadata,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("connection failed: {0}")]
    Connection(reqwest::Error),

    #[error("unexpected status: {0}")]
    UnexpectedStatus(reqwest::StatusCode),

    #[error("deserialization failed: {0}")]
    Deserialization(reqwest::Error),

    #[error("invalid daemon base URL: {0}")]
    InvalidBaseUrl(url::ParseError),

    #[error("invalid pairing token: {0}")]
    InvalidToken(reqwest::header::InvalidHeaderValue),

    #[error("automatic pairing requires a loopback daemon")]
    PairingRequiresLoopback,

    #[error("daemon not available")]
    #[allow(dead_code)]
    Unavailable,
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

    /// Returns a reference to the underlying daemon client.
    #[allow(dead_code)]
    pub fn client(&self) -> &DaemonClient {
        &self.client
    }
}

#[async_trait]
impl GameProvider for DaemonProvider {
    fn source_id(&self) -> &'static str {
        "hearthdeck-daemon"
    }

    fn refresh_interval(&self) -> Option<std::time::Duration> {
        // Daemon handles its own refresh, so we poll less frequently
        Some(std::time::Duration::from_secs(30))
    }

    async fn discover(&self) -> anyhow::Result<Vec<GameRecord>> {
        let items = self
            .client
            .fetch_library()
            .await
            .map_err(|e| anyhow::anyhow!("failed to fetch library: {}", e))?;

        tracing::info!(item_count = items.len(), "fetched catalog from daemon");

        let items = join_all(items.into_iter().map(|mut item| async move {
            if let Some(url) = item
                .icon
                .as_deref()
                .filter(|icon| icon.starts_with("http://") || icon.starts_with("https://"))
                .map(str::to_owned)
            {
                item.icon = tokio::task::spawn_blocking(move || cache_icon(&url))
                    .await
                    .unwrap_or(None);
            }
            item
        }))
        .await;

        Ok(items.into_iter().map(catalog_item_to_game_record).collect())
    }
}

fn cache_icon(url: &str) -> Option<String> {
    let cache_dir = icon_cache_dir();
    let _ = std::fs::create_dir_all(&cache_dir);
    let extension = url::Url::parse(url)
        .ok()
        .and_then(|parsed| {
            Path::new(parsed.path())
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
        })
        .filter(|extension| matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "svg" | "webp"))
        .unwrap_or_else(|| "png".to_string());
    let mut hasher = DefaultHasher::new();
    hasher.write(url.as_bytes());
    let cached = cache_dir.join(format!("{:016x}.{extension}", hasher.finish()));

    if cached.metadata().is_ok_and(|metadata| metadata.len() >= 16) {
        return Some(cached.to_string_lossy().into_owned());
    }

    let response = ureq::get(url).call().ok()?;
    let mut body = Vec::new();
    response
        .into_body()
        .into_reader()
        .read_to_end(&mut body)
        .ok()?;
    if body.len() < 16 {
        return None;
    }
    let mut file = std::fs::File::create(&cached).ok()?;
    file.write_all(&body).ok()?;
    Some(cached.to_string_lossy().into_owned())
}

fn cached_icon(key: &str) -> Option<String> {
    let mut hasher = DefaultHasher::new();
    hasher.write(key.as_bytes());
    let stem = format!("{:016x}", hasher.finish());
    ["jpg", "png", "webp"]
        .into_iter()
        .map(|extension| icon_cache_dir().join(format!("{stem}.{extension}")))
        .find(|path| path.metadata().is_ok_and(|metadata| metadata.len() >= 16))
        .map(|path| path.to_string_lossy().into_owned())
}

fn cache_icon_bytes(key: &str, extension: &str, bytes: &[u8]) -> Option<String> {
    if bytes.len() < 16 {
        return None;
    }
    let cache_dir = icon_cache_dir();
    std::fs::create_dir_all(&cache_dir).ok()?;
    let mut hasher = DefaultHasher::new();
    hasher.write(key.as_bytes());
    let cached = cache_dir.join(format!("{:016x}.{extension}", hasher.finish()));
    std::fs::File::create(&cached).ok()?.write_all(bytes).ok()?;
    Some(cached.to_string_lossy().into_owned())
}

fn image_extension(content_type: Option<&str>) -> &'static str {
    match content_type {
        Some(value) if value.eq_ignore_ascii_case("image/png") => "png",
        Some(value) if value.eq_ignore_ascii_case("image/webp") => "webp",
        _ => "jpg",
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
        CatalogItem, RecentActivityItem, RetroGame, catalog_item_to_game_record,
        recent_activity_to_game_record, retro_game_to_game_record,
    };

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
                screenshot_paths: Vec::new(),
                genres: vec!["RPG".to_string()],
                release_year: Some(1994),
            },
            Some("/tmp/example.jpg".to_string()),
        );

        assert_eq!(record.id, "romm:42");
        assert_eq!(record.exec.as_deref(), Some("42"));
        assert_eq!(record.categories, vec!["Game", "hearthdeck-console:7"]);
        assert_eq!(record.icon.as_deref(), Some("/tmp/example.jpg"));
        assert_eq!(record.metadata["genres"], serde_json::json!(["RPG"]));
    }

    #[test]
    fn recent_activity_keeps_its_launchable_frontend_id() {
        let record = recent_activity_to_game_record(
            RecentActivityItem {
                id: "romm:42".into(),
                title: "Example ROM".into(),
                icon: Some("/assets/example.jpg".into()),
                categories: vec!["Game".into(), "hearthdeck-console:7".into()],
                source: "romm".into(),
                metadata: serde_json::json!({"release_year": 1994}),
            },
            Some("/tmp/example.jpg".into()),
        );

        assert_eq!(record.id, "romm:42");
        assert_eq!(record.icon.as_deref(), Some("/tmp/example.jpg"));
        assert_eq!(record.categories[1], "hearthdeck-console:7");
    }
}
