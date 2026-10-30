use std::sync::Arc;

use chrono::Utc;
pub use hearthdeck_protocol::{ProviderHealth, ProviderKind, ProviderStatus, ServerEvent};
use tokio::sync::broadcast;

use crate::{
    activity::ActivityStore,
    auth::AuthRepository,
    catalog::CatalogStore,
    categorizer::CategorizationService,
    collections::CollectionStore,
    config::Config,
    database::Database,
    discovery::DiscoveryService,
    enrichment::EnrichmentService,
    romm_link::DeviceLinkState,
    settings::SettingsRepository,
    stremio::{LinkState, StremioRepository},
};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub auth: AuthRepository,
    pub activity: ActivityStore,
    pub catalog: CatalogStore,
    pub collections: CollectionStore,
    pub settings: SettingsRepository,
    /// The Stremio account link: the stored session, plus the one link a user may
    /// be waiting on. The stored session is a credential and is read only by the
    /// provider; the pending link lives in memory, because a link is approved
    /// within minutes or not at all.
    pub stremio: StremioRepository,
    pub stremio_link: LinkState,
    /// The RomM device approval a user may be waiting on.
    pub romm_link: DeviceLinkState,
    pub discovery: Option<DiscoveryService>,
    pub enrichment: Option<EnrichmentService>,
    /// Always present on Linux. It stays an `Option` for the same reason
    /// discovery does — a build or platform without it is not an error — but
    /// nothing has to be configured to have it, and no checkpoint is loaded
    /// until a user asks for a scan.
    pub categorization: Option<CategorizationService>,
    pub events: broadcast::Sender<ServerEvent>,
}

impl AppState {
    pub fn sessions(&self) -> crate::sessions::SessionService<'_> {
        crate::sessions::SessionService {
            bridge_socket: &self.config.bridge_socket_path,
            activity: &self.activity,
            discovery: self.discovery.as_ref(),
            events: &self.events,
        }
    }

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

pub fn starting_provider_health(id: &str, kind: ProviderKind) -> ProviderHealth {
    ProviderHealth {
        id: id.to_owned(),
        kind,
        status: ProviderStatus::Starting,
        record_count: None,
        last_attempt_at: None,
        last_success_at: None,
        last_error: None,
    }
}

pub fn record_provider_started(health: &mut ProviderHealth) {
    health.status = ProviderStatus::Refreshing;
    health.last_attempt_at = Some(Utc::now().to_rfc3339());
    health.last_error = None;
}

pub fn record_provider_success(health: &mut ProviderHealth, record_count: usize) {
    health.status = ProviderStatus::Ready;
    health.record_count = Some(record_count);
    let now = Utc::now().to_rfc3339();
    health.last_attempt_at = Some(now.clone());
    health.last_success_at = Some(now);
    health.last_error = None;
}

pub fn record_provider_failure(health: &mut ProviderHealth, error: &anyhow::Error) {
    health.status = ProviderStatus::Degraded;
    health.last_attempt_at = Some(Utc::now().to_rfc3339());
    health.last_error = Some(error.to_string());
}

impl AppState {
    pub fn new(config: Config, database: Database) -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            config,
            auth: AuthRepository::new(database.pool().clone()),
            activity: ActivityStore::new(database.pool().clone()),
            catalog: CatalogStore::new(database.pool().clone()),
            collections: CollectionStore::new(database.pool().clone()),
            settings: SettingsRepository::new(database.pool().clone()),
            stremio: StremioRepository::new(database.pool().clone()),
            stremio_link: LinkState::new(),
            romm_link: DeviceLinkState::new(),
            discovery: None,
            enrichment: None,
            categorization: None,
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

    pub fn with_categorization(mut self, categorization: CategorizationService) -> Self {
        self.categorization = Some(categorization);
        self
    }
}

pub type SharedState = Arc<AppState>;

#[cfg(test)]
mod tests {
    use super::{
        ProviderKind, ProviderStatus, record_provider_failure, record_provider_started,
        record_provider_success, starting_provider_health,
    };

    #[test]
    fn provider_failure_keeps_the_last_success_and_a_retry_clears_the_error() {
        let mut health = starting_provider_health("desktop-apps", ProviderKind::Discovery);
        assert!(matches!(health.status, ProviderStatus::Starting));
        record_provider_started(&mut health);
        assert!(matches!(health.status, ProviderStatus::Refreshing));
        record_provider_success(&mut health, 3);
        assert!(matches!(health.status, ProviderStatus::Ready));
        assert_eq!(health.record_count, Some(3));
        assert!(health.last_success_at.is_some());
        let last_success = health.last_success_at.clone();

        record_provider_failure(&mut health, &anyhow::anyhow!("provider unavailable"));
        assert!(matches!(health.status, ProviderStatus::Degraded));
        assert_eq!(health.last_success_at, last_success);
        assert_eq!(health.record_count, Some(3));
        assert_eq!(health.last_error.as_deref(), Some("provider unavailable"));

        record_provider_started(&mut health);
        assert!(matches!(health.status, ProviderStatus::Refreshing));
        assert!(health.last_error.is_none());
        assert_eq!(health.last_success_at, last_success);
    }
}
