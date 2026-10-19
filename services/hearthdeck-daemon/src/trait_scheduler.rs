//! Keeps what the model decided about each application up to date, without
//! anybody asking and without getting in anybody's way.
//!
//! The first scan is the user's to request; this only follows it. When the
//! library changes - a new application, an update that rewrites a record - it
//! waits for the changes to stop, waits for any game to finish, and then asks the
//! model about exactly the applications whose record no longer matches the
//! verdict it holds. Everything about it is slow on purpose: a thin client is
//! better served by a verdict that arrives in ten minutes than by a stutter now.

use std::time::Duration;

use hearthdeck_protocol::{BridgeRequest, BridgeResponse};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;
use tracing::{debug, info, warn};

use crate::categorizer::{CategorizationService, TraitRun};
use crate::state::{ServerEvent, SharedState};

/// Long enough for the daemon's own startup work to finish first.
const STARTUP_DELAY: Duration = Duration::from_secs(120);
/// A catalog sync lands as a burst of events; the pass waits for the burst to end.
const DEBOUNCE: Duration = Duration::from_secs(300);
/// How long to wait before trying again when a game or a scan is in the way.
const BUSY_RECHECK: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(3600);

/// The source id the service announces its own changes under, which must not
/// start another pass.
const OWN_SOURCE: &str = "categorization";

/// Starts the scheduler. It lives as long as the daemon does.
pub fn spawn(state: SharedState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let Some(service) = state.categorization.clone() else {
            return;
        };
        run(&state, &service).await;
    })
}

async fn run(state: &SharedState, service: &CategorizationService) {
    let mut events = state.events.subscribe();
    let mut due = Some(Instant::now() + STARTUP_DELAY);
    let mut failures = 0_u32;
    loop {
        let wait = async {
            match due {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            event = events.recv() => match event {
                Ok(ServerEvent::LibraryChanged { source_id, .. }) if source_id == OWN_SOURCE => {}
                Ok(ServerEvent::LibraryChanged { .. } | ServerEvent::MetadataChanged { .. })
                | Err(RecvError::Lagged(_)) => due = Some(Instant::now() + DEBOUNCE),
                Ok(_) => {}
                Err(RecvError::Closed) => return,
            },
            () = wait => due = attempt(state, service, &mut failures).await,
        }
    }
}

/// One try. Returns when the next one is due, or `None` when there is nothing to
/// wait for until the library changes again.
async fn attempt(
    state: &SharedState,
    service: &CategorizationService,
    failures: &mut u32,
) -> Option<Instant> {
    match state.settings.get().await {
        Ok(settings) if settings.categorization_enabled => {}
        Ok(_) => return None,
        Err(error) => {
            warn!(%error, "could not read the categorization preference");
            return Some(Instant::now() + BUSY_RECHECK);
        }
    }
    if a_game_is_running(state).await {
        debug!("a game is running; the background classification waits");
        return Some(Instant::now() + BUSY_RECHECK);
    }
    match service.classify_pending().await {
        Ok(TraitRun::Busy) => Some(Instant::now() + BUSY_RECHECK),
        Ok(TraitRun::Classified(count)) => {
            info!(count, "applications classified in the background");
            *failures = 0;
            None
        }
        Ok(TraitRun::UpToDate) => {
            *failures = 0;
            None
        }
        Err(error) => {
            *failures += 1;
            let wait = backoff(*failures);
            warn!(%error, retry_in_secs = wait.as_secs(), "background classification failed");
            Some(Instant::now() + wait)
        }
    }
}

/// Whether a managed play is in progress. A bridge that cannot be reached is no
/// game: the pass is not worth holding back for a question nobody can answer.
async fn a_game_is_running(state: &SharedState) -> bool {
    matches!(
        crate::bridge::request(
            &state.config.bridge_socket_path,
            BridgeRequest::ActiveApplicationSession,
        )
        .await,
        Ok(BridgeResponse::ApplicationSession { session: Some(_) })
    )
}

/// A minute after the first failure, doubling, never more than an hour.
fn backoff(failures: u32) -> Duration {
    let doubled = BUSY_RECHECK.saturating_mul(1_u32 << failures.saturating_sub(1).min(6));
    doubled.min(MAX_BACKOFF)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::backoff;

    #[test]
    fn failures_back_off_from_a_minute_to_an_hour() {
        assert_eq!(backoff(1), Duration::from_secs(60));
        assert_eq!(backoff(2), Duration::from_secs(120));
        assert_eq!(backoff(4), Duration::from_secs(480));
        assert_eq!(backoff(30), Duration::from_secs(3600));
    }
}
