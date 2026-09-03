pub mod daemon;
pub mod desktop_apps;
pub mod flatpak;
pub mod heroic;
pub mod lutris;
pub mod service;

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use cosmic::desktop::{DesktopEntryData, fde};
use serde::{Deserialize, Serialize};

/// A provider-agnostic record representing a discovered application or game.
///
/// Providers produce `Vec<GameRecord>`. The app converts these into
/// `DesktopEntryData` for display alongside XDG `.desktop` entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameRecord {
    /// Unique identifier, prefixed by source (e.g. `"heroic:epic:Fortnite"`).
    pub id: String,
    pub name: String,
    pub exec: Option<String>,
    pub icon: Option<String>,
    pub path: Option<PathBuf>,
    pub categories: Vec<String>,
    pub terminal: bool,
    pub prefers_dgpu: bool,
    /// Which provider discovered this record.
    pub source: String,
    /// Provider-specific metadata (store name, runner, version, etc.).
    pub metadata: serde_json::Value,
}

impl GameRecord {
    /// Convert into a `DesktopEntryData` for integration with the existing UI.
    pub fn into_desktop_entry(self) -> DesktopEntryData {
        let icon_source = self
            .icon
            .as_deref()
            .map(|icon| {
                if icon.starts_with("http://") || icon.starts_with("https://") {
                    // HTTP URLs can't be resolved as icon theme names.
                    // Use the standard applications-games icon as fallback.
                    // TODO: download and cache remote icons.
                    fde::IconSource::Name("applications-games".to_string())
                } else {
                    fde::IconSource::Path(PathBuf::from(icon))
                }
            })
            .unwrap_or(fde::IconSource::Name(String::new()));

        DesktopEntryData {
            id: self.id,
            name: self.name,
            wm_class: None,
            exec: self.exec,
            icon: icon_source,
            path: self.path,
            categories: self.categories,
            desktop_actions: Vec::new(),
            mime_types: Vec::new(),
            prefers_dgpu: self.prefers_dgpu,
            terminal: self.terminal,
        }
    }
}

/// A game provider discovers installed games or applications from a specific
/// source (Heroic Games Launcher, Steam, Lutris, etc.).
///
/// # Implementation notes
///
/// - `source_id` must be stable across sessions; it becomes the record's
///   `source` field and is used for deduplication.
/// - `refresh_interval` controls automatic periodic refresh. Return `None`
///   for providers that only run on manual refresh.
/// - `discover` should be idempotent and return the full current state.
#[async_trait]
pub trait GameProvider: Send + Sync {
    /// Stable provider identifier.
    fn source_id(&self) -> &'static str;

    /// Optional periodic refresh interval.
    fn refresh_interval(&self) -> Option<Duration>;

    /// Discover all items from this provider.
    async fn discover(&self) -> anyhow::Result<Vec<GameRecord>>;
}

/// Health status of a provider.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderHealth {
    pub source_id: String,
    pub status: ProviderStatus,
    pub record_count: Option<usize>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Starting,
    Refreshing,
    Ready,
    Degraded,
}
