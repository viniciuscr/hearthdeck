use std::sync::Arc;

use tokio::sync::broadcast;

use crate::{
    auth::AuthRepository, catalog::CatalogStore, config::Config, database::Database,
    discovery::DiscoveryService, enrichment::EnrichmentService,
};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub auth: AuthRepository,
    pub catalog: CatalogStore,
    pub discovery: Option<DiscoveryService>,
    pub enrichment: Option<EnrichmentService>,
    pub events: broadcast::Sender<ServerEvent>,
}

impl AppState {
    pub fn new(config: Config, database: Database) -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            config,
            auth: AuthRepository::new(database.pool().clone()),
            catalog: CatalogStore::new(database.pool().clone()),
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
