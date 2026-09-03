use async_trait::async_trait;
use hearthdeck_protocol::ApplicationSession;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::{GameProvider, GameRecord};

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
}

/// Response from the /v1/health endpoint.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct HealthResponse {
    pub version: String,
    pub lan_enabled: bool,
    pub transport: String,
    pub providers: Vec<ProviderHealthInfo>,
    pub capabilities: HostCapabilities,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ProviderHealthInfo {
    pub id: String,
    pub status: String,
    pub record_count: Option<u64>,
    pub last_attempt_at: Option<String>,
}

#[derive(Debug, Deserialize)]
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
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct RetroGame {
    pub id: i64,
    pub platform_id: i64,
    pub title: String,
    pub summary: Option<String>,
    pub cover_path: Option<String>,
    pub cover_url: Option<String>,
    pub screenshot_paths: Vec<String>,
    pub genres: Vec<String>,
    pub release_year: Option<i32>,
}

/// Retro platform from the daemon's /v1/retro/consoles endpoint.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct RetroPlatform {
    pub id: i64,
    pub name: String,
    pub display_name: Option<String>,
}

impl DaemonClient {
    /// Creates a new daemon client with the given configuration.
    pub fn new(config: DaemonConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    /// Returns the base URL for API requests.
    fn api_url(&self, path: &str) -> String {
        format!("{}{}", self.config.base_url, path)
    }

    /// Returns authorization headers for API requests.
    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.config.token))
                .expect("invalid token"),
        );
        headers
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
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Triggers a rescan of all discovery and enrichment providers.
    #[allow(dead_code)]
    pub async fn rescan_library(&self) -> Result<(), DaemonError> {
        let response = self
            .http
            .post(self.api_url("/v1/library/rescan"))
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }

        Ok(())
    }

    /// Launches an application by its catalog item ID.
    pub async fn launch_app(&self, id: &str) -> Result<ApplicationSession, DaemonError> {
        let response = self
            .http
            .post(self.api_url(&format!("/v1/apps/{}/launch", id)))
            .headers(self.auth_headers())
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
            .headers(self.auth_headers())
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
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }

        Ok(())
    }

    /// Lists available retro consoles from RomM.
    #[allow(dead_code)]
    pub async fn list_retro_consoles(&self) -> Result<Vec<RetroPlatform>, DaemonError> {
        let response = self
            .http
            .get(self.api_url("/v1/retro/consoles"))
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Lists retro ROMs, optionally filtered by platform.
    #[allow(dead_code)]
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
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(DaemonError::Connection)?;

        if !response.status().is_success() {
            return Err(DaemonError::UnexpectedStatus(response.status()));
        }

        response.json().await.map_err(DaemonError::Deserialization)
    }

    /// Launches a retro ROM by its ID.
    #[allow(dead_code)]
    pub async fn launch_retro_rom(&self, rom_id: i64) -> Result<ApplicationSession, DaemonError> {
        let response = self
            .http
            .post(self.api_url(&format!("/v1/retro/roms/{}/launch", rom_id)))
            .headers(self.auth_headers())
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

    // Extract categories from metadata
    let categories = metadata
        .get("categories")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

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

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("connection failed: {0}")]
    Connection(reqwest::Error),

    #[error("unexpected status: {0}")]
    UnexpectedStatus(reqwest::StatusCode),

    #[error("deserialization failed: {0}")]
    Deserialization(reqwest::Error),

    #[error("daemon not available")]
    #[allow(dead_code)]
    Unavailable,
}

/// A provider that fetches games/apps from the HearthDeck daemon.
pub struct DaemonProvider {
    client: DaemonClient,
}

impl DaemonProvider {
    /// Creates a new daemon provider with the given configuration.
    pub fn new(config: DaemonConfig) -> Self {
        Self {
            client: DaemonClient::new(config),
        }
    }

    /// Creates a new daemon provider with an existing client.
    #[allow(dead_code)]
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

        Ok(items.into_iter().map(catalog_item_to_game_record).collect())
    }
}

/// Response from the /v1/retro/roms endpoint.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct RetroGamesResponse {
    pub items: Vec<RetroGame>,
    pub total: u64,
    pub limit: u32,
    pub offset: u32,
}

#[cfg(test)]
mod tests {
    use super::{CatalogItem, catalog_item_to_game_record};

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
        assert_eq!(record.categories, vec!["Utility", "Game"]);
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
}
