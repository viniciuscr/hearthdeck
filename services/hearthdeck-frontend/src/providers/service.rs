//! Runs the game providers in the background and streams their merged records.
//!
//! A caller starts the service and reads records from the returned receiver; the
//! discovery tasks themselves are detached, so holding the service does nothing
//! and it is returned only to give `start` a value to pair with the receiver.
//! Providers that declare a `refresh_interval` are re-run on that cadence and
//! their records re-merged.
//!
//! There is deliberately no event or health channel here. An earlier version had
//! one, but nothing consumed it — the daemon owns provider health now, and the UI
//! reads it from `/v1/health` — so it was removed rather than kept as a second,
//! unused source of truth.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};
use tracing::warn;

use super::{GameProvider, GameRecord};

pub struct ProviderService;

impl ProviderService {
    pub fn start(providers: Vec<Arc<dyn GameProvider>>) -> (Self, mpsc::Receiver<Vec<GameRecord>>) {
        let (records_tx, records_rx) = mpsc::channel(4);
        let (refresh_tx, mut refresh_rx) = mpsc::channel(16);
        let cached = Arc::new(RwLock::new(HashMap::new()));

        for provider in &providers {
            let Some(interval) = provider.refresh_interval() else {
                continue;
            };
            let source_id = provider.source_id().to_owned();
            let refresh_tx = refresh_tx.clone();
            tokio::spawn(async move {
                let start = tokio::time::Instant::now() + interval;
                let mut timer = tokio::time::interval_at(start, interval);
                loop {
                    timer.tick().await;
                    if refresh_tx.send(source_id.clone()).await.is_err() {
                        break;
                    }
                }
            });
        }

        // Initial discovery.
        let c1 = cached.clone();
        let t1 = records_tx.clone();
        let p1 = providers.clone();
        tokio::spawn(async move {
            for provider in &p1 {
                run_discovery(provider.as_ref(), &c1).await;
            }
            let _ = t1.send(merge_records(&c1).await).await;
        });

        // Orchestrator: periodic ticks. The original `refresh_tx` is dropped when
        // no provider schedules an interval, which closes the channel and ends this
        // task — correct, since there is then nothing to refresh.
        let c2 = cached;
        let t2 = records_tx;
        tokio::spawn(async move {
            while let Some(source_id) = refresh_rx.recv().await {
                if let Some(provider) = providers.iter().find(|p| p.source_id() == source_id) {
                    run_discovery(provider.as_ref(), &c2).await;
                    let _ = t2.send(merge_records(&c2).await).await;
                }
            }
        });

        (Self, records_rx)
    }
}

async fn run_discovery(
    provider: &dyn GameProvider,
    cached: &Arc<RwLock<HashMap<String, Vec<GameRecord>>>>,
) {
    let source_id = provider.source_id().to_owned();
    match provider.discover().await {
        Ok(records) => {
            cached.write().await.insert(source_id, records);
        }
        Err(error) => {
            // Nothing to record this against any more; the log is the only place a
            // provider that cannot answer is visible from here.
            warn!(source_id, %error, "provider discovery failed");
        }
    }
}

async fn merge_records(cached: &Arc<RwLock<HashMap<String, Vec<GameRecord>>>>) -> Vec<GameRecord> {
    let cache = cached.read().await;
    let mut all: Vec<GameRecord> = cache.values().flatten().cloned().collect();
    all.sort_by(|a, b| a.name.cmp(&b.name));
    all
}
