pub mod providers;

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use tracing::{Instrument, info, info_span};

use crate::{
    catalog::{CatalogStore, EnrichmentRecord},
    provider_worker::{WorkerSet, WorkerSetBuilder},
    state::{ProviderHealth, ProviderKind, ServerEvent},
};

/// The outcome of asking a provider to refresh.
///
/// Shared with discovery through the worker machinery, and re-exported here under
/// the name metadata's callers already use.
pub use crate::provider_worker::RefreshRequest as EnrichmentRequest;

#[async_trait]
pub trait MetadataProvider: Send + Sync {
    /// Stable ID that records metadata provenance in SQLite and API responses.
    fn provider_id(&self) -> &'static str;
    fn refresh_interval(&self) -> Option<Duration>;
    async fn enrich(&self) -> anyhow::Result<Vec<EnrichmentRecord>>;
}

#[derive(Clone)]
pub struct EnrichmentService {
    workers: WorkerSet,
}

impl EnrichmentService {
    pub fn start(
        providers: Vec<Arc<dyn MetadataProvider>>,
        catalog: CatalogStore,
        events: tokio::sync::broadcast::Sender<ServerEvent>,
    ) -> Self {
        let mut workers = WorkerSetBuilder::new("metadata");
        for provider in providers {
            let catalog = catalog.clone();
            let events = events.clone();
            workers.spawn(
                provider.provider_id(),
                ProviderKind::Metadata,
                provider.refresh_interval(),
                move || {
                    let provider = provider.clone();
                    let catalog = catalog.clone();
                    let events = events.clone();
                    let span = info_span!("metadata.enrich", provider_id = provider.provider_id());
                    async move {
                        enrich_source(provider.as_ref(), &catalog, &events)
                            .instrument(span)
                            .await
                    }
                },
            );
        }
        Self {
            workers: workers.build(),
        }
    }

    pub async fn request_all(&self) {
        self.workers.request_all().await;
    }

    pub async fn provider_health(&self) -> Vec<ProviderHealth> {
        self.workers.provider_health().await
    }

    pub async fn request(&self, provider_id: &str) -> EnrichmentRequest {
        self.workers.request(provider_id).await
    }
}

async fn enrich_source(
    provider: &dyn MetadataProvider,
    catalog: &CatalogStore,
    events: &tokio::sync::broadcast::Sender<ServerEvent>,
) -> anyhow::Result<usize> {
    let started_at = std::time::Instant::now();
    info!(
        provider_id = provider.provider_id(),
        "metadata enrichment started"
    );
    let records = provider.enrich().await?;
    let record_count = records.len();
    catalog
        .replace_enrichment_source(provider.provider_id(), records)
        .await?;
    let _ = events.send(ServerEvent::MetadataChanged {
        provider_id: provider.provider_id().to_owned(),
        record_count,
    });
    info!(
        provider_id = provider.provider_id(),
        record_count,
        duration_ms = started_at.elapsed().as_millis() as u64,
        "metadata enrichment completed"
    );
    Ok(record_count)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use async_trait::async_trait;
    use tempfile::tempdir;
    use tokio::time::timeout;

    use super::{EnrichmentRequest, EnrichmentService, MetadataProvider};
    use crate::{
        catalog::{CatalogStore, EnrichmentRecord},
        database::Database,
    };

    struct PanickingProvider;

    #[async_trait]
    impl MetadataProvider for PanickingProvider {
        fn provider_id(&self) -> &'static str {
            "panicking"
        }
        fn refresh_interval(&self) -> Option<Duration> {
            None
        }
        async fn enrich(&self) -> anyhow::Result<Vec<EnrichmentRecord>> {
            panic!("simulated metadata provider crash");
        }
    }

    /// This is the regression the shared worker fixes. Before it, enrichment kept
    /// its own copy of the worker loop without the `catch_unwind` discovery had,
    /// so this panic ended the worker task: `running` stayed true and every later
    /// request returned `AlreadyScheduled` for the life of the process.
    #[tokio::test]
    async fn a_panicking_metadata_provider_does_not_wedge_its_worker() {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let (events, _receiver) = tokio::sync::broadcast::channel(4);
        let service = EnrichmentService::start(
            vec![Arc::new(PanickingProvider)],
            CatalogStore::new(database.pool().clone()),
            events,
        );

        assert_eq!(
            service.request("panicking").await,
            EnrichmentRequest::Queued
        );

        let recovered = timeout(Duration::from_secs(2), async {
            loop {
                match service.request("panicking").await {
                    EnrichmentRequest::AlreadyScheduled => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    other => break other,
                }
            }
        })
        .await
        .expect("worker never recovered from the metadata provider panic");

        assert_eq!(recovered, EnrichmentRequest::Queued);
    }
}
