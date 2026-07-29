pub mod providers;

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::sync::{Mutex, mpsc};
use tracing::{Instrument, error, info, info_span, warn};

use crate::{
    catalog::{CatalogStore, EnrichmentRecord},
    state::ServerEvent,
};

#[async_trait]
pub trait MetadataProvider: Send + Sync {
    /// Stable ID that records metadata provenance in SQLite and API responses.
    fn provider_id(&self) -> &'static str;
    fn refresh_interval(&self) -> Option<Duration>;
    async fn enrich(&self) -> anyhow::Result<Vec<EnrichmentRecord>>;
}

#[derive(Clone)]
pub struct EnrichmentService {
    workers: Arc<HashMap<&'static str, ProviderWorker>>,
}

#[derive(Clone)]
struct ProviderWorker {
    sender: mpsc::Sender<()>,
    state: Arc<Mutex<RefreshState>>,
}

#[derive(Default)]
struct RefreshState {
    running: bool,
    queued: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnrichmentRequest {
    Queued,
    AlreadyScheduled,
    UnknownProvider,
}

impl EnrichmentService {
    pub fn start(
        providers: Vec<Arc<dyn MetadataProvider>>,
        catalog: CatalogStore,
        events: tokio::sync::broadcast::Sender<ServerEvent>,
    ) -> Self {
        let mut workers = HashMap::new();
        for provider in providers {
            let provider_id = provider.provider_id();
            let (sender, mut receiver) = mpsc::channel(1);
            let state = Arc::new(Mutex::new(RefreshState::default()));
            workers.insert(
                provider_id,
                ProviderWorker {
                    sender: sender.clone(),
                    state: state.clone(),
                },
            );
            info!(
                provider_id,
                scheduled = provider.refresh_interval().is_some(),
                "metadata provider registered"
            );

            let worker_catalog = catalog.clone();
            let worker_events = events.clone();
            let worker_provider = provider.clone();
            let worker_state = state.clone();
            tokio::spawn(async move {
                while receiver.recv().await.is_some() {
                    {
                        let mut state = worker_state.lock().await;
                        state.queued = false;
                        state.running = true;
                    }
                    let span = info_span!("metadata.enrich", provider_id);
                    if let Err(error) =
                        enrich_source(worker_provider.as_ref(), &worker_catalog, &worker_events)
                            .instrument(span)
                            .await
                    {
                        error!(provider_id, %error, "metadata provider failed");
                    }
                    worker_state.lock().await.running = false;
                }
            });

            if let Some(interval) = provider.refresh_interval() {
                let interval_state = state;
                tokio::spawn(async move {
                    let mut timer = tokio::time::interval(interval);
                    loop {
                        timer.tick().await;
                        enqueue(&sender, &interval_state).await;
                    }
                });
            }
        }
        Self {
            workers: Arc::new(workers),
        }
    }

    pub async fn request_all(&self) {
        for provider_id in self.workers.keys() {
            self.request(provider_id).await;
        }
    }

    pub async fn request(&self, provider_id: &str) -> EnrichmentRequest {
        let Some(worker) = self.workers.get(provider_id) else {
            warn!(provider_id, "unknown metadata provider requested");
            return EnrichmentRequest::UnknownProvider;
        };
        enqueue(&worker.sender, &worker.state).await
    }
}

async fn enqueue(sender: &mpsc::Sender<()>, state: &Mutex<RefreshState>) -> EnrichmentRequest {
    let mut state = state.lock().await;
    if state.running || state.queued {
        return EnrichmentRequest::AlreadyScheduled;
    }
    state.queued = true;
    if sender.send(()).await.is_err() {
        state.queued = false;
        error!("metadata provider worker is unavailable");
        return EnrichmentRequest::UnknownProvider;
    }
    EnrichmentRequest::Queued
}

async fn enrich_source(
    provider: &dyn MetadataProvider,
    catalog: &CatalogStore,
    events: &tokio::sync::broadcast::Sender<ServerEvent>,
) -> anyhow::Result<()> {
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
    Ok(())
}
