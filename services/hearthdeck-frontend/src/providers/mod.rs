pub mod daemon;
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
    pub fn into_desktop_entry(mut self) -> DesktopEntryData {
        let is_game = self
            .categories
            .iter()
            .any(|category| category.eq_ignore_ascii_case("game"));
        if is_game
            && let Some(store) = self
                .metadata
                .get("store")
                .and_then(serde_json::Value::as_str)
        {
            self.categories
                .retain(|category| !category.eq_ignore_ascii_case(store));
            self.categories.push(format!("hearthdeck-store:{store}"));
        }

        let fallback_icon = if is_game {
            "applications-games"
        } else {
            "application-x-executable"
        };
        let icon_source = self
            .icon
            .as_deref()
            .map(|icon| {
                if icon.starts_with("http://") || icon.starts_with("https://") {
                    fde::IconSource::Name(fallback_icon.to_string())
                } else if PathBuf::from(icon).is_absolute() {
                    fde::IconSource::Path(PathBuf::from(icon))
                } else {
                    fde::IconSource::Name(icon.to_string())
                }
            })
            .unwrap_or_else(|| fde::IconSource::Name(fallback_icon.to_string()));

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

#[cfg(test)]
mod tests {
    use super::GameRecord;
    use cosmic::desktop::fde::IconSource;

    fn record(icon: Option<&str>) -> GameRecord {
        GameRecord {
            id: "test".into(),
            name: "Test".into(),
            exec: None,
            icon: icon.map(str::to_owned),
            path: None,
            categories: vec!["Utility".into()],
            terminal: false,
            prefers_dgpu: false,
            source: "test".into(),
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn resolves_icon_names_and_absolute_paths() {
        assert!(matches!(
            record(Some("org.example.App")).into_desktop_entry().icon,
            IconSource::Name(name) if name == "org.example.App"
        ));
        assert!(matches!(
            record(Some("/usr/share/icons/example.png"))
                .into_desktop_entry()
                .icon,
            IconSource::Path(path) if path == std::path::Path::new("/usr/share/icons/example.png")
        ));
    }

    #[test]
    fn missing_application_icon_uses_fallback() {
        assert!(matches!(
            record(None).into_desktop_entry().icon,
            IconSource::Name(name) if name == "application-x-executable"
        ));
    }

    #[test]
    fn game_store_becomes_a_tab_category() {
        let mut game = record(None);
        game.categories = vec!["Game".into(), "Epic Games".into()];
        game.metadata = serde_json::json!({"store": "Epic Games"});

        assert_eq!(
            game.into_desktop_entry().categories,
            vec!["Game", "hearthdeck-store:Epic Games"]
        );
    }
}
