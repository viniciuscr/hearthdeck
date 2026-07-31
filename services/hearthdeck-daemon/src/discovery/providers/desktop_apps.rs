use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use hearthdeck_protocol::{BridgeRequest, BridgeResponse};

use crate::{bridge, catalog::CatalogRecord, discovery::DiscoveryProvider};

pub struct DesktopAppsProvider {
    bridge_socket_path: PathBuf,
}

impl DesktopAppsProvider {
    pub fn new(bridge_socket_path: PathBuf) -> Self {
        Self { bridge_socket_path }
    }
}

const GAME_LAUNCHER_IDS: &[&str] = &[
    "steam.desktop",
    "com.valvesoftware.steam.desktop",
    "lutris.desktop",
    "net.lutris.lutris.desktop",
    "heroic.desktop",
    "com.heroicgameslauncher.hgl.desktop",
    "hearthdeck.desktop",
    "dev.hearthdeck.hearthdeck.desktop",
    "com.hearthdeck.hearthdeck.desktop",
];
const GAME_LAUNCHER_NAMES: &[&str] = &[
    "steam",
    "lutris",
    "heroic",
    "heroic games launcher",
    "hearthdeck",
];

fn content_kind(application_id: &str, name: &str, categories: &[String]) -> &'static str {
    let normalized_id = application_id.to_ascii_lowercase();
    let normalized_name = name.trim().to_ascii_lowercase();
    if categories
        .iter()
        .any(|category| category.eq_ignore_ascii_case("Game"))
        && !GAME_LAUNCHER_IDS.contains(&normalized_id.as_str())
        && !GAME_LAUNCHER_NAMES.contains(&normalized_name.as_str())
    {
        "game"
    } else {
        "application"
    }
}

#[async_trait]
impl DiscoveryProvider for DesktopAppsProvider {
    fn source_id(&self) -> &'static str {
        "desktop-apps"
    }

    fn refresh_interval(&self) -> Option<Duration> {
        // Desktop entries rarely change. A startup tick and a modest periodic
        // refresh keep the catalog current without filesystem polling churn.
        Some(Duration::from_secs(15 * 60))
    }

    async fn discover(&self) -> anyhow::Result<Vec<CatalogRecord>> {
        let response = bridge::request(
            &self.bridge_socket_path,
            BridgeRequest::DiscoverApplications {
                source_id: self.source_id().to_owned(),
            },
        )
        .await?;
        let BridgeResponse::Applications { applications, .. } = response else {
            anyhow::bail!("bridge returned an unexpected response")
        };
        let updated_at = Utc::now().to_rfc3339();
        Ok(applications
            .into_iter()
            .filter(|application| application.launch_scheme.as_deref() != Some("heroic"))
            .map(|application| {
                let kind = content_kind(
                    &application.application_id,
                    &application.name,
                    &application.categories,
                )
                .to_owned();
                CatalogRecord {
                    id: format!("desktop:{}", application.application_id),
                    title: application.name,
                    kind,
                    launch_id: Some(application.application_id),
                    icon: application.icon,
                    metadata: serde_json::json!({
                        "categories": application.categories,
                        "comment": application.comment,
                    }),
                    updated_at: updated_at.clone(),
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::content_kind;

    #[test]
    fn classifies_freedesktop_games_as_games() {
        assert_eq!(
            content_kind(
                "super-tux.desktop",
                "SuperTux",
                &["Game".to_owned(), "ActionGame".to_owned()]
            ),
            "game"
        );
        assert_eq!(
            content_kind("utility.desktop", "Utility", &["Utility".to_owned()]),
            "application"
        );
    }

    #[test]
    fn classifies_game_launchers_as_applications() {
        assert_eq!(
            content_kind("steam.desktop", "Steam", &["Game".to_owned()]),
            "application"
        );
        assert_eq!(
            content_kind("net.lutris.Lutris.desktop", "Lutris", &["Game".to_owned()]),
            "application"
        );
        assert_eq!(
            content_kind(
                "com.heroicgameslauncher.hgl.desktop",
                "Heroic Games Launcher",
                &["Game".to_owned()]
            ),
            "application"
        );
        assert_eq!(
            content_kind("custom.desktop", "Hearthdeck", &["Game".to_owned()]),
            "application"
        );
    }
}
