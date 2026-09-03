use std::time::Duration;

use async_trait::async_trait;
use cosmic::desktop;

use super::{GameProvider, GameRecord};

#[allow(dead_code)]
pub struct DesktopAppsProvider {
    locale: Vec<String>,
    include_flatpak: bool,
    xdg_current_desktop: Option<String>,
}

#[allow(dead_code)]
impl DesktopAppsProvider {
    pub fn new(
        locale: Vec<String>,
        include_flatpak: bool,
        xdg_current_desktop: Option<String>,
    ) -> Self {
        Self {
            locale,
            include_flatpak,
            xdg_current_desktop,
        }
    }
}

#[async_trait]
impl GameProvider for DesktopAppsProvider {
    fn source_id(&self) -> &'static str {
        "desktop"
    }

    fn refresh_interval(&self) -> Option<Duration> {
        None
    }

    async fn discover(&self) -> anyhow::Result<Vec<GameRecord>> {
        let locale_ref: &[String] = &self.locale;
        let entries: Vec<GameRecord> = desktop::load_applications(
            locale_ref,
            self.include_flatpak,
            self.xdg_current_desktop.as_deref(),
        )
        .filter(|d| d.exec.is_some() && !is_game_launcher(d))
        .map(|de| GameRecord {
            id: de.id,
            name: de.name,
            exec: de.exec,
            icon: match &de.icon {
                cosmic::desktop::fde::IconSource::Name(n) => Some(n.clone()),
                cosmic::desktop::fde::IconSource::Path(p) => Some(p.to_string_lossy().into_owned()),
            },
            path: de.path,
            categories: de.categories,
            terminal: de.terminal,
            prefers_dgpu: de.prefers_dgpu,
            source: "desktop".to_owned(),
            metadata: serde_json::Value::Null,
        })
        .collect();
        Ok(entries)
    }
}

#[allow(dead_code)]
fn is_game_launcher(entry: &desktop::DesktopEntryData) -> bool {
    let id = entry.id.to_lowercase();
    let exec = entry.exec.as_deref().unwrap_or_default().to_lowercase();
    let haystack = format!("{} {}", id, exec);
    ["steam", "lutris", "heroic", "bottles", "minigalaxy"]
        .iter()
        .any(|launcher| haystack.contains(launcher))
}
