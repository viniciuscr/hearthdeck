use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use hearthdeck_protocol::{ApplicationSession, BridgeRequest, BridgeResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, SqlitePool};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// How often the daemon looks at the bridge while something is playing.
///
/// This is also the resolution of a recorded duration: the play ended somewhere
/// between the two ticks that bracket it.
const SESSION_WATCH_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct ActivityStore {
    pool: SqlitePool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ActivityEntry {
    pub id: String,
    pub title: String,
    pub icon: Option<String>,
    pub categories: Vec<String>,
    /// Catalog content kind (`game` / `application`), stored so the client can
    /// split "Recently Played" into games and apps without provider knowledge.
    /// `None` only for snapshots written before this field existed.
    #[serde(default)]
    pub kind: Option<String>,
    pub source: String,
    pub metadata: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RecentActivity {
    #[serde(flatten)]
    pub entry: ActivityEntry,
    pub last_launched_at: String,
    pub launch_count: i64,
}

/// How a play ended, which is also how much is known about it.
///
/// `Unclosed` is the honest answer for a play whose end the daemon never saw — a
/// crash, a power cut, a restart — and it carries no duration rather than a
/// guess. The strings are the table's, and its `CHECK` is the authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionOutcome {
    Running,
    Closed,
    Unclosed,
}

impl SessionOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Closed => "closed",
            Self::Unclosed => "unclosed",
        }
    }
}

impl ActivityStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Records that a launch was accepted: the aggregate row the dashboard reads,
    /// and one row in the play journal.
    ///
    /// One call for both, on purpose. Recording a play is something every launch
    /// path has to remember, and a path that forgets is a play that never happened
    /// as far as the statistics are concerned — so there is one method to call
    /// instead of two, and the two rows go in together or not at all.
    pub async fn session_started(
        &self,
        entry: ActivityEntry,
        session: &ApplicationSession,
    ) -> Result<()> {
        let snapshot = serde_json::to_string(&entry).context("serialize launch activity")?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("begin launch transaction")?;
        sqlx::query(
            r#"
            INSERT INTO launch_activity (item_id, snapshot_json, last_launched_at, launch_count)
            VALUES (?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 1)
            ON CONFLICT(item_id) DO UPDATE SET
              snapshot_json = excluded.snapshot_json,
              last_launched_at = excluded.last_launched_at,
              launch_count = launch_activity.launch_count + 1
            "#,
        )
        .bind(&entry.id)
        .bind(snapshot)
        .execute(&mut *transaction)
        .await
        .context("record launch activity")?;
        // `DO NOTHING` rather than an upsert: the same session id recorded twice
        // is the same play, and its start must not move.
        sqlx::query(
            r#"
            INSERT INTO play_sessions (session_id, item_id, source_id, started_at, outcome)
            VALUES (?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?)
            ON CONFLICT(session_id) DO NOTHING
            "#,
        )
        .bind(&session.id)
        .bind(&entry.id)
        .bind(&session.source_id)
        .bind(SessionOutcome::Running.as_str())
        .execute(&mut *transaction)
        .await
        .context("record play session")?;
        transaction
            .commit()
            .await
            .context("commit launch transaction")?;
        Ok(())
    }

    /// Closes a play, with the duration that has passed since it started.
    ///
    /// Idempotent: only the first close of a running session writes anything, so
    /// a press of Stop and the watcher noticing the same exit cannot both take
    /// credit for it.
    pub async fn session_ended(&self, session_id: &str, outcome: SessionOutcome) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE play_sessions
            SET ended_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                outcome = ?,
                duration_seconds = CAST(
                  (julianday('now') - julianday(started_at)) * 86400 AS INTEGER
                )
            WHERE session_id = ? AND outcome = 'running'
            "#,
        )
        .bind(outcome.as_str())
        .bind(session_id)
        .execute(&self.pool)
        .await
        .context("close play session")?;
        Ok(())
    }

    /// Session ids of plays that have not been closed.
    pub async fn running_sessions(&self) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT session_id FROM play_sessions WHERE outcome = 'running' ORDER BY started_at",
        )
        .fetch_all(&self.pool)
        .await
        .context("list running play sessions")?;
        Ok(rows.iter().map(|row| row.get("session_id")).collect())
    }

    /// Closes whatever the previous process left running, and reports how many.
    ///
    /// A row still open at startup belongs to a daemon that is gone: the machine
    /// slept, the service was restarted, the power went. When that play ended is
    /// not knowable, so it is closed without a duration instead of with one.
    pub async fn sweep_running_sessions(&self) -> Result<u64> {
        let result = sqlx::query("UPDATE play_sessions SET outcome = ? WHERE outcome = 'running'")
            .bind(SessionOutcome::Unclosed.as_str())
            .execute(&self.pool)
            .await
            .context("close play sessions left open")?;
        Ok(result.rows_affected())
    }

    pub async fn recent(&self, limit: u32) -> Result<Vec<RecentActivity>> {
        let rows = sqlx::query(
            r#"
            SELECT snapshot_json, last_launched_at, launch_count
            FROM launch_activity
            ORDER BY last_launched_at DESC
            LIMIT ?
            "#,
        )
        .bind(i64::from(limit.clamp(1, 50)))
        .fetch_all(&self.pool)
        .await
        .context("list recent launch activity")?;

        rows.into_iter()
            .map(|row| {
                let entry = serde_json::from_str(&row.get::<String, _>("snapshot_json"))
                    .context("deserialize launch activity")?;
                Ok(RecentActivity {
                    entry,
                    last_launched_at: row.get("last_launched_at"),
                    launch_count: row.get("launch_count"),
                })
            })
            .collect()
    }
}

/// Watches for plays that have finished, and closes their rows.
///
/// The bridge knows when a session ends — it checks the systemd unit when asked —
/// but nothing asks once the frontend's overlay is gone, so a game closed from the
/// couch used to leave no trace of having stopped. This asks on a slow tick for as
/// long as something is running, and not at all otherwise.
///
/// The returned handle is deliberately droppable: dropping it detaches the task,
/// which is what a daemon that never stops wanting this should do.
pub fn spawn_session_watcher(activity: ActivityStore, bridge_socket: PathBuf) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(SESSION_WATCH_INTERVAL).await;
            let running = match activity.running_sessions().await {
                Ok(running) => running,
                Err(error) => {
                    warn!(%error, "could not list running plays");
                    continue;
                }
            };
            if running.is_empty() {
                continue;
            }
            let request =
                crate::bridge::request(&bridge_socket, BridgeRequest::ActiveApplicationSession);
            let active = match request.await {
                Ok(BridgeResponse::ApplicationSession { session }) => session.map(|it| it.id),
                Ok(_) => {
                    warn!("bridge answered the wrong way about the active session");
                    continue;
                }
                // A bridge that cannot answer says nothing about whether a play
                // ended, so nothing is closed on its account.
                Err(error) => {
                    debug!(%error, "bridge unreachable while watching a play");
                    continue;
                }
            };
            for session_id in finished_sessions(&running, active.as_deref()) {
                match activity
                    .session_ended(&session_id, SessionOutcome::Closed)
                    .await
                {
                    Ok(()) => info!(%session_id, "play finished"),
                    Err(error) => warn!(%error, %session_id, "could not close a finished play"),
                }
            }
        }
    })
}

/// The running plays the bridge is no longer reporting.
///
/// The whole of the decision, apart from its own function so it can be tested
/// without a bridge: the bridge enforces one session at a time, so anything it is
/// not reporting has stopped.
fn finished_sessions(running: &[String], active: Option<&str>) -> Vec<String> {
    running
        .iter()
        .filter(|session_id| Some(session_id.as_str()) != active)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use hearthdeck_protocol::{ApplicationSession, ApplicationSessionState};
    use serde_json::json;
    use sqlx::{Row, SqlitePool};
    use tempfile::tempdir;

    use super::{ActivityEntry, ActivityStore, SessionOutcome, finished_sessions};
    use crate::database::Database;

    fn entry() -> ActivityEntry {
        ActivityEntry {
            id: "romm:42".into(),
            title: "Original title".into(),
            icon: None,
            categories: vec!["Game".into()],
            kind: Some("game".into()),
            source: "romm".into(),
            metadata: json!({"platform_id": 7}),
        }
    }

    fn session(id: &str) -> ApplicationSession {
        ApplicationSession {
            id: id.into(),
            source_id: "retroarch".into(),
            application_id: "romm:42".into(),
            state: ApplicationSessionState::Running,
        }
    }

    async fn store() -> (tempfile::TempDir, ActivityStore, SqlitePool) {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let pool = database.pool().clone();
        (directory, ActivityStore::new(pool.clone()), pool)
    }

    /// A journal row's outcome, when it ended, and how long it took.
    async fn journal(pool: &SqlitePool, session_id: &str) -> (String, Option<String>, Option<i64>) {
        let row = sqlx::query(
            "SELECT outcome, ended_at, duration_seconds FROM play_sessions WHERE session_id = ?",
        )
        .bind(session_id)
        .fetch_one(pool)
        .await
        .unwrap();
        (
            row.get("outcome"),
            row.get("ended_at"),
            row.get("duration_seconds"),
        )
    }

    async fn journal_len(pool: &SqlitePool) -> i64 {
        sqlx::query("SELECT COUNT(*) AS count FROM play_sessions")
            .fetch_one(pool)
            .await
            .unwrap()
            .get("count")
    }

    #[tokio::test]
    async fn a_launch_records_the_shelf_row_and_a_row_in_the_journal() {
        let (_directory, activity, pool) = store().await;

        activity
            .session_started(entry(), &session("first"))
            .await
            .unwrap();
        activity
            .session_started(entry(), &session("second"))
            .await
            .unwrap();

        // The shelf row is still one row per item, as it must be.
        let recent = activity.recent(6).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].launch_count, 2);
        // The journal is one row per play, which is the point of it: two rows
        // where the old table could only remember the last one.
        assert_eq!(journal_len(&pool).await, 2);
        assert_eq!(
            journal(&pool, "first").await.0,
            SessionOutcome::Running.as_str()
        );
        assert_eq!(activity.running_sessions().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn the_same_session_id_is_one_play_however_often_it_is_recorded() {
        let (_directory, activity, pool) = store().await;

        activity
            .session_started(entry(), &session("only"))
            .await
            .unwrap();
        activity
            .session_started(entry(), &session("only"))
            .await
            .unwrap();

        assert_eq!(journal_len(&pool).await, 1);
    }

    #[tokio::test]
    async fn a_finished_play_is_closed_with_its_duration_exactly_once() {
        let (_directory, activity, pool) = store().await;
        activity
            .session_started(entry(), &session("played"))
            .await
            .unwrap();

        activity
            .session_ended("played", SessionOutcome::Closed)
            .await
            .unwrap();
        let (outcome, ended_at, duration) = journal(&pool, "played").await;
        assert_eq!(outcome, SessionOutcome::Closed.as_str());
        assert!(ended_at.is_some(), "a finished play knows when it ended");
        assert!(duration.is_some_and(|seconds| seconds >= 0));

        // Stop pressed and the watcher noticing the same exit must not both write:
        // a second close leaves the first one's numbers alone.
        activity
            .session_ended("played", SessionOutcome::Closed)
            .await
            .unwrap();
        assert_eq!(
            journal(&pool, "played").await,
            (outcome, ended_at, duration)
        );
        assert!(activity.running_sessions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_sweep_closes_what_a_previous_run_left_open_without_a_duration() {
        let (_directory, activity, pool) = store().await;
        activity
            .session_started(entry(), &session("interrupted"))
            .await
            .unwrap();

        assert_eq!(activity.sweep_running_sessions().await.unwrap(), 1);

        let (outcome, ended_at, duration) = journal(&pool, "interrupted").await;
        assert_eq!(outcome, SessionOutcome::Unclosed.as_str());
        // Unknown is not zero: the total must show a gap, not a phantom play.
        assert!(ended_at.is_none());
        assert!(duration.is_none());
        assert!(activity.running_sessions().await.unwrap().is_empty());
    }

    #[test]
    fn finished_sessions_are_the_ones_the_bridge_stopped_reporting() {
        let running = vec!["a".to_owned(), "b".to_owned()];

        assert!(finished_sessions(&running, Some("a")).eq(&["b"]));
        // Nothing active: everything that was running has stopped.
        assert!(finished_sessions(&running, None).eq(&["a", "b"]));
        assert!(finished_sessions(&running, Some("b")).eq(&["a"]));
        assert!(finished_sessions(&[], None).is_empty());
    }

    #[tokio::test]
    async fn repeated_launch_updates_snapshot_and_count() {
        let (_directory, activity, _pool) = store().await;
        let mut entry = entry();

        activity
            .session_started(entry.clone(), &session("first"))
            .await
            .unwrap();
        entry.title = "Updated title".into();
        activity
            .session_started(entry, &session("second"))
            .await
            .unwrap();

        let recent = activity.recent(6).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].entry.title, "Updated title");
        assert_eq!(recent[0].launch_count, 2);
    }
}
