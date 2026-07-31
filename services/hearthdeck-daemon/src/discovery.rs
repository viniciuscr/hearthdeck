pub mod providers;

use std::{any::Any, collections::HashMap, panic::AssertUnwindSafe, sync::Arc, time::Duration};

use async_trait::async_trait;
use futures_util::FutureExt;
use tokio::sync::{Mutex, mpsc};
use tracing::{Instrument, error, info, info_span, warn};

use crate::{
    catalog::{CatalogRecord, CatalogStore},
    state::{ProviderHealth, ProviderKind, ServerEvent},
};

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
    workers: Arc<HashMap<&'static str, ProviderWorker>>,
}

#[derive(Clone)]
struct ProviderWorker {
    sender: mpsc::Sender<()>,
    state: Arc<Mutex<RefreshState>>,
    health: Arc<Mutex<ProviderHealth>>,
}

#[derive(Default)]
struct RefreshState {
    running: bool,
    queued: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshRequest {
    Queued,
    AlreadyScheduled,
    UnknownProvider,
}

impl DiscoveryService {
    pub fn start(
        providers: Vec<Arc<dyn DiscoveryProvider>>,
        catalog: CatalogStore,
        events: tokio::sync::broadcast::Sender<ServerEvent>,
    ) -> Self {
        let mut workers = HashMap::new();
        for provider in providers {
            let source_id = provider.source_id();
            let (sender, mut receiver) = mpsc::channel(1);
            let state = Arc::new(Mutex::new(RefreshState::default()));
            let health = Arc::new(Mutex::new(ProviderHealth::starting(
                source_id,
                ProviderKind::Discovery,
            )));
            let worker = ProviderWorker {
                sender: sender.clone(),
                state: state.clone(),
                health: health.clone(),
            };
            workers.insert(source_id, worker);
            info!(
                source_id,
                scheduled = provider.refresh_interval().is_some(),
                "discovery provider registered"
            );

            let worker_events = events.clone();
            let worker_catalog = catalog.clone();
            let worker_provider = provider.clone();
            let worker_state = state.clone();
            let worker_health = health.clone();
            tokio::spawn(async move {
                while receiver.recv().await.is_some() {
                    {
                        let mut state = worker_state.lock().await;
                        state.queued = false;
                        state.running = true;
                    }
                    worker_health.lock().await.record_started();
                    let span = info_span!("discovery.run", source_id);
                    // ponytail: a provider is third-party-ish code we don't control (a
                    // future Steam/GOG scraper can misbehave). catch_unwind keeps a
                    // panic from killing this worker task, which would otherwise close
                    // its mpsc channel and leave `running` stuck true forever, so every
                    // later refresh request silently reports "already scheduled".
                    match AssertUnwindSafe(
                        discover_source(worker_provider.as_ref(), &worker_catalog, &worker_events)
                            .instrument(span),
                    )
                    .catch_unwind()
                    .await
                    {
                        Ok(Ok(record_count)) => {
                            worker_health.lock().await.record_success(record_count);
                        }
                        Ok(Err(error)) => {
                            worker_health.lock().await.record_failure(&error);
                            error!(source_id, %error, "discovery provider failed");
                        }
                        Err(panic) => {
                            let error = anyhow::anyhow!(panic_message(&panic).to_owned());
                            worker_health.lock().await.record_failure(&error);
                            error!(source_id, %error, "discovery provider panicked");
                        }
                    }
                    worker_state.lock().await.running = false;
                }
            });

            if let Some(interval) = provider.refresh_interval() {
                let interval_sender = sender;
                let interval_state = state;
                tokio::spawn(async move {
                    let mut timer = tokio::time::interval(interval);
                    loop {
                        timer.tick().await;
                        enqueue(&interval_sender, &interval_state).await;
                    }
                });
            }
        }
        Self {
            workers: Arc::new(workers),
        }
    }

    pub async fn request_all(&self) {
        for source_id in self.workers.keys() {
            self.request(source_id).await;
        }
    }

    pub async fn provider_health(&self) -> Vec<ProviderHealth> {
        let mut health = Vec::with_capacity(self.workers.len());
        for worker in self.workers.values() {
            health.push(worker.health.lock().await.clone());
        }
        health.sort_by(|left, right| left.id.cmp(&right.id));
        health
    }

    pub async fn request(&self, source_id: &str) -> RefreshRequest {
        let Some(worker) = self.workers.get(source_id) else {
            warn!(%source_id, "requested unknown discovery provider");
            return RefreshRequest::UnknownProvider;
        };
        enqueue(&worker.sender, &worker.state).await
    }
}

async fn enqueue(sender: &mpsc::Sender<()>, state: &Mutex<RefreshState>) -> RefreshRequest {
    let mut state = state.lock().await;
    if state.running || state.queued {
        info!("discovery work already scheduled");
        return RefreshRequest::AlreadyScheduled;
    }
    state.queued = true;
    if sender.send(()).await.is_err() {
        state.queued = false;
        error!("discovery provider worker is unavailable");
        return RefreshRequest::UnknownProvider;
    }
    RefreshRequest::Queued
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

fn panic_message(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic payload")
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
