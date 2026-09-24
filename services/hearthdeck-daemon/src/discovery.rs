pub mod providers;

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use tracing::{Instrument, info, info_span};

use crate::{
    catalog::{CatalogRecord, CatalogStore},
    provider_worker::{WorkerSet, WorkerSetBuilder},
    state::{ProviderHealth, ProviderKind, ServerEvent},
};

/// The outcome of asking a provider to refresh.
///
/// Defined alongside the worker machinery so discovery and enrichment share one
/// type, and re-exported here under the name discovery's callers already use.
pub use crate::provider_worker::RefreshRequest;

#[async_trait]
pub trait DiscoveryProvider: Send + Sync {
    /// Immutable provider ID. This becomes the catalog `source_id` exposed to
    /// clients, so changing it would orphan previously discovered records.
    fn source_id(&self) -> &'static str;
    /// `None` means the provider only runs when a caller explicitly refreshes
    /// it. Providers with a duration get independent periodic schedules.
    fn refresh_interval(&self) -> Option<Duration>;
    async fn discover(&self) -> anyhow::Result<Vec<CatalogRecord>>;
}

#[derive(Clone)]
pub struct DiscoveryService {
    workers: WorkerSet,
}

impl DiscoveryService {
    pub fn start(
        providers: Vec<Arc<dyn DiscoveryProvider>>,
        catalog: CatalogStore,
        events: tokio::sync::broadcast::Sender<ServerEvent>,
    ) -> Self {
        let mut workers = WorkerSetBuilder::new("discovery");
        for provider in providers {
            let catalog = catalog.clone();
            let events = events.clone();
            workers.spawn(
                provider.source_id(),
                ProviderKind::Discovery,
                provider.refresh_interval(),
                move || {
                    let provider = provider.clone();
                    let catalog = catalog.clone();
                    let events = events.clone();
                    let span = info_span!("discovery.run", source_id = provider.source_id());
                    async move {
                        discover_source(provider.as_ref(), &catalog, &events)
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

    pub async fn request(&self, source_id: &str) -> RefreshRequest {
        self.workers.request(source_id).await
    }
}

async fn discover_source(
    provider: &dyn DiscoveryProvider,
    catalog: &CatalogStore,
    events: &tokio::sync::broadcast::Sender<ServerEvent>,
) -> anyhow::Result<usize> {
    let started_at = std::time::Instant::now();
    info!(source_id = provider.source_id(), "discovery started");
    let records = provider.discover().await?;
    let record_count = records.len();
    catalog
        .replace_source(provider.source_id(), records)
        .await?;
    let _ = events.send(ServerEvent::LibraryChanged {
        source_id: provider.source_id().to_owned(),
        record_count,
    });
    info!(
        source_id = provider.source_id(),
        record_count,
        duration_ms = started_at.elapsed().as_millis() as u64,
        "discovery completed"
    );
    Ok(record_count)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;
    use tempfile::tempdir;
    use tokio::time::timeout;

    use super::{DiscoveryProvider, DiscoveryService, RefreshRequest};
    use crate::{
        catalog::{CatalogRecord, CatalogStore},
        database::Database,
    };

    struct FakeProvider(Arc<AtomicUsize>);

    #[async_trait]
    impl DiscoveryProvider for FakeProvider {
        fn source_id(&self) -> &'static str {
            "fake"
        }
        fn refresh_interval(&self) -> Option<Duration> {
            None
        }
        async fn discover(&self) -> anyhow::Result<Vec<CatalogRecord>> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(vec![CatalogRecord {
                id: "fake:item".to_owned(),
                title: "Fake item".to_owned(),
                kind: "application".to_owned(),
                launch_id: None,
                icon: None,
                metadata: serde_json::Value::Null,
                updated_at: "2026-01-01T00:00:00Z".to_owned(),
            }])
        }
    }

    #[tokio::test]
    async fn coalesces_duplicate_provider_refreshes() {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let (events, mut receiver) = tokio::sync::broadcast::channel(4);
        let runs = Arc::new(AtomicUsize::new(0));
        let service = DiscoveryService::start(
            vec![Arc::new(FakeProvider(runs.clone()))],
            CatalogStore::new(database.pool().clone()),
            events,
        );

        assert_eq!(service.request("fake").await, RefreshRequest::Queued);
        assert_eq!(
            service.request("fake").await,
            RefreshRequest::AlreadyScheduled
        );
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(runs.load(Ordering::Relaxed), 1);
    }

    struct PanickingProvider;

    #[async_trait]
    impl DiscoveryProvider for PanickingProvider {
        fn source_id(&self) -> &'static str {
            "panicking"
        }
        fn refresh_interval(&self) -> Option<Duration> {
            None
        }
        async fn discover(&self) -> anyhow::Result<Vec<CatalogRecord>> {
            panic!("simulated discovery provider crash");
        }
    }

    #[tokio::test]
    async fn a_panicking_provider_does_not_permanently_disable_its_worker() {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let (events, _receiver) = tokio::sync::broadcast::channel(4);
        let service = DiscoveryService::start(
            vec![Arc::new(PanickingProvider)],
            CatalogStore::new(database.pool().clone()),
            events,
        );

        assert_eq!(service.request("panicking").await, RefreshRequest::Queued);

        // Before the fix this never resolves: the panic kills the worker
        // task, `running` is stuck true, and every request below returns
        // `AlreadyScheduled` forever.
        let recovered = timeout(Duration::from_secs(2), async {
            loop {
                match service.request("panicking").await {
                    RefreshRequest::AlreadyScheduled => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    other => break other,
                }
            }
        })
        .await
        .expect("worker never recovered from the provider panic");

        assert_eq!(recovered, RefreshRequest::Queued);
    }
}
