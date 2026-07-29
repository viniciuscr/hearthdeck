use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use hearthdeck_protocol::{BridgeRequest, BridgeResponse};

use crate::{bridge, catalog::CatalogRecord, discovery::DiscoveryProvider};

pub struct MacosAppsProvider {
    bridge_socket_path: PathBuf,
}

impl MacosAppsProvider {
    pub fn new(bridge_socket_path: PathBuf) -> Self {
        Self { bridge_socket_path }
    }
}

#[async_trait]
impl DiscoveryProvider for MacosAppsProvider {
    fn source_id(&self) -> &'static str {
        "macos-apps"
    }

    fn refresh_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(30 * 60))
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
            .map(|application| CatalogRecord {
                id: format!("macos:{}", application.application_id),
                title: application.name,
                kind: "application".to_owned(),
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
