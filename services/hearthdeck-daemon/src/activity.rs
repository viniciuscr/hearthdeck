use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

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

impl ActivityStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn record_launch(&self, entry: ActivityEntry) -> Result<()> {
        let snapshot = serde_json::to_string(&entry).context("serialize launch activity")?;
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
        .execute(&self.pool)
        .await
        .context("record launch activity")?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use super::{ActivityEntry, ActivityStore};
    use crate::database::Database;

    #[tokio::test]
    async fn repeated_launch_updates_snapshot_and_count() {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let activity = ActivityStore::new(database.pool().clone());
        let mut entry = ActivityEntry {
            id: "romm:42".into(),
            title: "Original title".into(),
            icon: None,
            categories: vec!["Game".into()],
            source: "romm".into(),
            metadata: json!({"platform_id": 7}),
        };

        activity.record_launch(entry.clone()).await.unwrap();
        entry.title = "Updated title".into();
        activity.record_launch(entry).await.unwrap();

        let recent = activity.recent(6).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].entry.title, "Updated title");
        assert_eq!(recent[0].launch_count, 2);
    }
}
