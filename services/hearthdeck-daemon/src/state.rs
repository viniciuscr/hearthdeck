use std::sync::Arc;

use chrono::Utc;
use tokio::sync::broadcast;

use crate::{
    auth::AuthRepository, catalog::CatalogStore, config::Config, database::Database,
    discovery::DiscoveryService, enrichment::EnrichmentService, settings::SettingsRepository,
};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub auth: AuthRepository,
    pub catalog: CatalogStore,
    pub settings: SettingsRepository,
    pub discovery: Option<DiscoveryService>,
    pub enrichment: Option<EnrichmentService>,
    pub events: broadcast::Sender<ServerEvent>,
}

impl AppState {
    pub async fn provider_health(&self) -> Vec<ProviderHealth> {
        let mut providers = Vec::new();
        if let Some(discovery) = &self.discovery {
            providers.extend(discovery.provider_health().await);
        }
        if let Some(enrichment) = &self.enrichment {
            providers.extend(enrichment.provider_health().await);
        }
        providers.sort_by(|left, right| left.id.cmp(&right.id));
        providers
    }
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Discovery,
    Metadata,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Starting,
    Ready,
    Degraded,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ProviderHealth {
    pub id: String,
    pub kind: ProviderKind,
    pub status: ProviderStatus,
    pub record_count: Option<usize>,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
}

impl ProviderHealth {
    pub fn starting(id: &str, kind: ProviderKind) -> Self {
        Self {
            id: id.to_owned(),
            kind,
            status: ProviderStatus::Starting,
            record_count: None,
            last_success_at: None,
            last_error: None,
        }
    }

    pub fn record_success(&mut self, record_count: usize) {
        self.status = ProviderStatus::Ready;
        self.record_count = Some(record_count);
        self.last_success_at = Some(Utc::now().to_rfc3339());
        self.last_error = None;
    }

    pub fn record_failure(&mut self, error: &anyhow::Error) {
        self.status = ProviderStatus::Degraded;
        self.last_error = Some(error.to_string());
    }
}

impl AppState {
    pub fn new(config: Config, database: Database) -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            config,
            auth: AuthRepository::new(database.pool().clone()),
            catalog: CatalogStore::new(database.pool().clone()),
            settings: SettingsRepository::new(database.pool().clone()),
            discovery: None,
            enrichment: None,
            events,
        }
    }

    pub fn with_discovery(mut self, discovery: DiscoveryService) -> Self {
        self.discovery = Some(discovery);
        self
    }

    pub fn with_enrichment(mut self, enrichment: EnrichmentService) -> Self {
        self.enrichment = Some(enrichment);
        self
    }
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    LibraryChanged {
        source_id: String,
        record_count: usize,
    },
    MetadataChanged {
        provider_id: String,
        record_count: usize,
    },
    ActionCompleted {
        item_id: String,
    },
}

pub type SharedState = Arc<AppState>;
