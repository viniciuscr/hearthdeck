//! The one background refresh worker, shared by discovery and enrichment.
//!
//! Discovery and enrichment differ only in what a refresh *runs* and how it is
//! labelled. Everything around that — the coalescing queue, the health record,
//! the periodic timer, and the panic guard — is identical, and it lives here
//! once so the two cannot drift.
//!
//! They did drift. When the machinery was copied, the enrichment copy lost the
//! `catch_unwind` guard that discovery had. A panicking metadata provider then
//! ended its own worker task, which closed the channel and left `running` stuck
//! true, so every later refresh request silently reported "already scheduled"
//! and that provider never ran again for the life of the process. One copy is
//! the fix.

use std::{
    any::Any, collections::HashMap, future::Future, panic::AssertUnwindSafe, sync::Arc,
    time::Duration,
};

use futures_util::FutureExt;
use tokio::sync::{Mutex, mpsc};
use tracing::{error, info, warn};

use crate::state::{ProviderHealth, ProviderKind};

/// The outcome of asking a worker to refresh. Shared by both services; each
/// re-exports it under the name its callers already use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshRequest {
    Queued,
    AlreadyScheduled,
    UnknownProvider,
}

#[derive(Default)]
struct RefreshState {
    running: bool,
    queued: bool,
}

#[derive(Clone)]
struct Worker {
    sender: mpsc::Sender<()>,
    state: Arc<Mutex<RefreshState>>,
    health: Arc<Mutex<ProviderHealth>>,
}

/// The completed set of workers, cheap to clone and share.
#[derive(Clone)]
pub(crate) struct WorkerSet {
    label: &'static str,
    workers: Arc<HashMap<&'static str, Worker>>,
}

/// Registers providers and spawns their workers, then freezes into a
/// [`WorkerSet`]. Building is separate from using so the map can be filled
/// before it is shared behind an `Arc`.
pub(crate) struct WorkerSetBuilder {
    label: &'static str,
    workers: HashMap<&'static str, Worker>,
}

impl WorkerSetBuilder {
    /// `label` is the word used in this set's log messages, e.g. `"discovery"`.
    pub(crate) fn new(label: &'static str) -> Self {
        Self {
            label,
            workers: HashMap::new(),
        }
    }

    /// Registers one provider and starts its worker, plus a periodic timer when
    /// it asked for one.
    ///
    /// `run` is called once per refresh and returns the number of records the
    /// refresh touched. It must be `'static`: capture the provider as an `Arc`
    /// and clone it into the returned future, which is what lets the worker task
    /// own a fresh future per run rather than borrowing the provider.
    pub(crate) fn spawn<F, Fut>(
        &mut self,
        id: &'static str,
        kind: ProviderKind,
        refresh_interval: Option<Duration>,
        run: F,
    ) where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<usize>> + Send + 'static,
    {
        let label = self.label;
        let (sender, mut receiver) = mpsc::channel(1);
        let state = Arc::new(Mutex::new(RefreshState::default()));
        let health = Arc::new(Mutex::new(ProviderHealth::starting(id, kind)));
        self.workers.insert(
            id,
            Worker {
                sender: sender.clone(),
                state: state.clone(),
                health: health.clone(),
            },
        );
        info!(
            id,
            scheduled = refresh_interval.is_some(),
            "{label} provider registered"
        );

        let worker_state = state.clone();
        let worker_health = health.clone();
        let run = Arc::new(run);
        tokio::spawn(async move {
            while receiver.recv().await.is_some() {
                {
                    let mut state = worker_state.lock().await;
                    state.queued = false;
                    state.running = true;
                }
                worker_health.lock().await.record_started();
                // A provider is third-party-ish code we do not control (a future
                // Steam/GOG scraper can misbehave). catch_unwind keeps a panic from
                // ending this worker task, which would close its channel and leave
                // `running` stuck true forever, so every later request silently
                // reports "already scheduled".
                match AssertUnwindSafe((run)()).catch_unwind().await {
                    Ok(Ok(record_count)) => {
                        worker_health.lock().await.record_success(record_count);
                    }
                    Ok(Err(error)) => {
                        worker_health.lock().await.record_failure(&error);
                        error!(id, %error, "{label} provider failed");
                    }
                    Err(panic) => {
                        let error = anyhow::anyhow!(panic_message(&panic).to_owned());
                        worker_health.lock().await.record_failure(&error);
                        error!(id, %error, "{label} provider panicked");
                    }
                }
                worker_state.lock().await.running = false;
            }
        });

        if let Some(interval) = refresh_interval {
            let interval_sender = sender;
            let interval_state = state;
            tokio::spawn(async move {
                let mut timer = tokio::time::interval(interval);
                loop {
                    timer.tick().await;
                    enqueue(&interval_sender, &interval_state, label).await;
                }
            });
        }
    }

    pub(crate) fn build(self) -> WorkerSet {
        WorkerSet {
            label: self.label,
            workers: Arc::new(self.workers),
        }
    }
}

impl WorkerSet {
    pub(crate) async fn request_all(&self) {
        for id in self.workers.keys() {
            self.request(id).await;
        }
    }

    pub(crate) async fn provider_health(&self) -> Vec<ProviderHealth> {
        let mut health = Vec::with_capacity(self.workers.len());
        for worker in self.workers.values() {
            health.push(worker.health.lock().await.clone());
        }
        health.sort_by(|left, right| left.id.cmp(&right.id));
        health
    }

    pub(crate) async fn request(&self, id: &str) -> RefreshRequest {
        let Some(worker) = self.workers.get(id) else {
            warn!(%id, "requested unknown {} provider", self.label);
            return RefreshRequest::UnknownProvider;
        };
        enqueue(&worker.sender, &worker.state, self.label).await
    }
}

/// Queues one refresh unless the worker is already running or has one queued.
///
/// The lock is held across the `send` so a request cannot observe `running ==
/// false` and queue behind another that is about to flip it — the coalescing is
/// the point, and a refresh that ran twice for one request is exactly what the
/// frontend's polling would amplify.
async fn enqueue(
    sender: &mpsc::Sender<()>,
    state: &Mutex<RefreshState>,
    label: &'static str,
) -> RefreshRequest {
    let mut state = state.lock().await;
    if state.running || state.queued {
        info!("{label} work already scheduled");
        return RefreshRequest::AlreadyScheduled;
    }
    state.queued = true;
    if sender.send(()).await.is_err() {
        state.queued = false;
        error!("{label} provider worker is unavailable");
        return RefreshRequest::UnknownProvider;
    }
    RefreshRequest::Queued
}

fn panic_message(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic payload")
}
