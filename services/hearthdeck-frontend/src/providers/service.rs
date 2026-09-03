use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{RwLock, broadcast, mpsc};

use super::{GameProvider, GameRecord, ProviderHealth, ProviderStatus};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ProviderEvent {
    RecordsChanged {
        source_id: String,
        record_count: usize,
    },
    ProviderFailed {
        source_id: String,
        error: String,
    },
}

#[allow(dead_code)]
pub struct ProviderService {
    refresh_tx: mpsc::Sender<String>,
    health: Arc<RwLock<Vec<ProviderHealth>>>,
    pub events: broadcast::Sender<ProviderEvent>,
}

#[allow(dead_code)]
impl ProviderService {
    pub fn start(providers: Vec<Arc<dyn GameProvider>>) -> (Self, mpsc::Receiver<Vec<GameRecord>>) {
        let (records_tx, records_rx) = mpsc::channel(4);
        let (refresh_tx, mut refresh_rx) = mpsc::channel(16);
        let (events_tx, _) = broadcast::channel(64);

        let health = Arc::new(RwLock::new(
            providers
                .iter()
                .map(|p| ProviderHealth {
                    source_id: p.source_id().to_owned(),
                    status: ProviderStatus::Starting,
                    record_count: None,
                    last_error: None,
                })
                .collect(),
        ));
        let cached = Arc::new(RwLock::new(HashMap::new()));

        let service = Self {
            refresh_tx,
            health: health.clone(),
            events: events_tx.clone(),
        };

        // Initial discovery.
        let h1 = health.clone();
        let c1 = cached.clone();
        let e1 = events_tx.clone();
        let t1 = records_tx.clone();
        let p1 = providers.clone();
        tokio::spawn(async move {
            for provider in &p1 {
                run_discovery(provider.as_ref(), &h1, &c1, &e1).await;
            }
            let _ = t1.send(merge_records(&c1).await).await;
        });

        // Orchestrator: manual refresh + periodic ticks.
        let h2 = health;
        let c2 = cached;
        let e2 = events_tx;
        let t2 = records_tx;
        tokio::spawn(async move {
            // Compute the minimum refresh interval across all providers.
            let min_interval = providers
                .iter()
                .filter_map(|p| p.refresh_interval())
                .min()
                .unwrap_or(Duration::from_secs(300));

            loop {
                tokio::select! {
                    Some(source_id) = refresh_rx.recv() => {
                        if let Some(provider) = providers.iter().find(|p| p.source_id() == source_id) {
                            run_discovery(provider.as_ref(), &h2, &c2, &e2).await;
                            let _ = t2.send(merge_records(&c2).await).await;
                        }
                    }
                    _ = tokio::time::sleep(min_interval) => {
                        for provider in &providers {
                            if provider.refresh_interval().is_some() {
                                run_discovery(provider.as_ref(), &h2, &c2, &e2).await;
                            }
                        }
                        let _ = t2.send(merge_records(&c2).await).await;
                    }
                }
            }
        });

        (service, records_rx)
    }

    pub async fn refresh(&self, source_id: &str) {
        let _ = self.refresh_tx.send(source_id.to_owned()).await;
    }

    pub async fn refresh_all(&self) {
        let ids: Vec<String> = self
            .health
            .read()
            .await
            .iter()
            .map(|h| h.source_id.clone())
            .collect();
        for id in ids {
            let _ = self.refresh_tx.send(id).await;
        }
    }

    pub async fn health(&self) -> Vec<ProviderHealth> {
        self.health.read().await.clone()
    }
}

async fn run_discovery(
    provider: &dyn GameProvider,
    health: &Arc<RwLock<Vec<ProviderHealth>>>,
    cached: &Arc<RwLock<HashMap<String, Vec<GameRecord>>>>,
    events: &broadcast::Sender<ProviderEvent>,
) {
    let source_id = provider.source_id().to_owned();

    {
        let mut h = health.write().await;
        if let Some(entry) = h.iter_mut().find(|e| e.source_id == source_id) {
            entry.status = ProviderStatus::Refreshing;
            entry.last_error = None;
        }
    }

    match provider.discover().await {
        Ok(records) => {
            let count = records.len();
            cached.write().await.insert(source_id.clone(), records);
            {
                let mut h = health.write().await;
                if let Some(entry) = h.iter_mut().find(|e| e.source_id == source_id) {
                    entry.status = ProviderStatus::Ready;
                    entry.record_count = Some(count);
                }
            }
            let _ = events.send(ProviderEvent::RecordsChanged {
                source_id,
                record_count: count,
            });
        }
        Err(error) => {
            let error_msg = error.to_string();
            {
                let mut h = health.write().await;
                if let Some(entry) = h.iter_mut().find(|e| e.source_id == source_id) {
                    entry.status = ProviderStatus::Degraded;
                    entry.last_error = Some(error_msg.clone());
                }
            }
            let _ = events.send(ProviderEvent::ProviderFailed {
                source_id,
                error: error_msg,
            });
        }
    }
}

async fn merge_records(cached: &Arc<RwLock<HashMap<String, Vec<GameRecord>>>>) -> Vec<GameRecord> {
    let cache = cached.read().await;
    let mut all: Vec<GameRecord> = cache.values().flatten().cloned().collect();
    all.sort_by(|a, b| a.name.cmp(&b.name));
    all
}
