use std::time::Duration;

use async_trait::async_trait;
use cosmic::desktop;

use super::{GameProvider, GameRecord};

const FLATPAK_SOURCE: &str = "flatpak";

/// Discovers Flatpak applications.
pub struct FlatpakProvider {
    locale: Vec<String>,
}

impl FlatpakProvider {
    pub fn new(locale: Vec<String>) -> Self {
        Self { locale }
    }
}

#[async_trait]
impl GameProvider for FlatpakProvider {
    fn source_id(&self) -> &'static str {
        FLATPAK_SOURCE
    }

    fn refresh_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(10 * 60)) // 10 minute refresh
    }

    async fn discover(&self) -> anyhow::Result<Vec<GameRecord>> {
        let mut records = Vec::new();

        // Use cosmic's built-in flatpak support to find installed apps
        // The function we can use is load_applications but with a specific filter
        let flatpak_apps = desktop::load_applications(
            self.locale.as_slice(),
            true, // include_no_display (for flatpak apps)
            None, // no specific desktop filter needed
        );

        for app in flatpak_apps {
            if let Some(exec) = &app.exec {
                let is_game = app
                    .categories
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case("game"));
                if !is_game {
                    continue;
                }
                records.push(GameRecord {
                    id: format!("flatpak:{}", app.id),
                    name: app.name,
                    exec: Some(exec.clone()),
                    icon: match &app.icon {
                        cosmic::desktop::fde::IconSource::Name(n) => Some(n.clone()),
                        cosmic::desktop::fde::IconSource::Path(p) => {
                            Some(p.to_string_lossy().into_owned())
                        }
                    },
                    path: app.path,
                    categories: app.categories,
                    terminal: app.terminal,
                    prefers_dgpu: app.prefers_dgpu,
                    source: FLATPAK_SOURCE.to_owned(),
                    metadata: serde_json::json!({
                        "type": "flatpak_application",
                    }),
                });
            }
        }

        Ok(records)
    }
}
