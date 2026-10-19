//! Keeping what the dashboard and the library are drawn from fresh, and the way
//! back when they are not.
//!
//! Each read is made once at startup, and a daemon that is not answering yet - the
//! normal case when both start together - used to leave the dashboard on its
//! fallback shelf for the whole session, with only a log line to say why.

use std::sync::Arc;
use std::time::Duration;

use cosmic::app::Task;

use crate::providers::GameProvider;
use crate::providers::daemon::DaemonProvider;

use super::{App, Message, Page};

/// How long a failed read waits before it is tried again.
const DATA_RETRY_INTERVAL: Duration = Duration::from_secs(10);

/// Which reads are in a retry chain, so a read that keeps failing is retried by
/// that one chain rather than by one more for every request that finds it down.
#[derive(Default)]
pub(super) struct Retries {
    pub(super) catalog: bool,
    pub(super) consoles: bool,
    pub(super) collections: bool,
}

impl App {
    /// Fetches the whole catalog after `delay`.
    pub(super) fn load_catalog(&self, delay: Duration) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                tokio::time::sleep(delay).await;
                let provider = DaemonProvider::with_client(client);
                GameProvider::discover(&provider)
                    .await
                    .map(|records| {
                        records
                            .into_iter()
                            .map(|record| Arc::new(record.into_desktop_entry()))
                            .collect()
                    })
                    .map_err(|error| format!("{error:?}"))
            },
            |result| cosmic::Action::App(Message::CatalogLoaded(result)),
        )
    }

    /// A catalog read failed: try again after a pause. The chain lives until a
    /// read succeeds, which is what the flag says.
    pub(super) fn catalog_failed(&mut self) -> Task<Message> {
        self.retries.catalog = true;
        self.load_catalog(DATA_RETRY_INTERVAL)
    }

    /// A console list read failed: try again after a pause.
    pub(super) fn consoles_failed(&mut self) -> Task<Message> {
        self.retries.consoles = true;
        self.load_consoles_after(DATA_RETRY_INTERVAL)
    }

    /// A collections read failed: try again after a pause.
    pub(super) fn collections_failed(&mut self) -> Task<Message> {
        self.retries.collections = true;
        self.load_collections_after(DATA_RETRY_INTERVAL)
    }

    /// What landing on the dashboard asks for: its rails read again, and the
    /// consoles if none have ever arrived. A read already in a retry chain is
    /// left to it.
    pub(super) fn refresh_dashboard(&self) -> Task<Message> {
        let mut reads = Vec::new();
        if !self.retries.collections {
            reads.push(self.load_collections());
        }
        if self.console_platforms.is_empty() && !self.retries.consoles {
            reads.push(self.load_consoles());
        }
        Task::batch(reads)
    }

    /// Puts the dashboard back as the daemon composes it: every read again, the
    /// cursor on the first card, and the dashboard on screen.
    ///
    /// The built-in rails live in the daemon, which restores them on read; what
    /// this client can get wrong is only what it holds, so this is what it
    /// throws away.
    pub(super) fn restore_dashboard(&mut self) -> Task<Message> {
        self.switch_page(Page::Dashboard);
        self.cursors.ring = None;
        self.bar_cursor = None;
        self.dashboard_cursor = Default::default();
        let mut reads = Vec::new();
        if !self.retries.catalog {
            reads.push(self.load_catalog(Duration::ZERO));
        }
        if !self.retries.consoles {
            reads.push(self.load_consoles());
        }
        if !self.retries.collections {
            reads.push(self.load_collections());
        }
        Task::batch(reads)
    }
}
