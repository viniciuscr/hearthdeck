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

fn content_kind(categories: &[String]) -> &'static str {
    if categories
        .iter()
        .any(|category| category.eq_ignore_ascii_case("Game"))
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
            .map(|application| CatalogRecord {
                id: format!("desktop:{}", application.application_id),
                title: application.name,
                kind: content_kind(&application.categories).to_owned(),
                launch_id: Some(application.application_id),
                icon: application.icon,
                metadata: serde_json::json!({
                    "categories": application.categories,
                    "comment": application.comment,
                }),
                updated_at: updated_at.clone(),
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
            content_kind(&["Game".to_owned(), "ActionGame".to_owned()]),
            "game"
        );
        assert_eq!(content_kind(&["Utility".to_owned()]), "application");
    }
}
