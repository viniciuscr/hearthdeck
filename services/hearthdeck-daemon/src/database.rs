use std::path::Path;

use anyhow::Result;
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn connect(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        Ok(Self {
            pool: SqlitePool::connect_with(options).await?,
        })
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS paired_clients (
              id TEXT PRIMARY KEY,
              name TEXT NOT NULL,
              token_hash TEXT NOT NULL UNIQUE,
              created_at TEXT NOT NULL,
              last_seen_at TEXT
            );
            CREATE TABLE IF NOT EXISTS pairing_sessions (
              code_hash TEXT PRIMARY KEY,
              expires_at TEXT NOT NULL,
              consumed_at TEXT
            );
            CREATE TABLE IF NOT EXISTS library_items (
              id TEXT PRIMARY KEY,
              source_id TEXT NOT NULL DEFAULT 'legacy',
              title TEXT NOT NULL,
              kind TEXT NOT NULL,
              launch_id TEXT,
              icon TEXT,
              metadata_json TEXT NOT NULL DEFAULT '{}',
              updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS catalog_enrichments (
              provider_id TEXT NOT NULL,
              application_id TEXT NOT NULL,
              priority INTEGER NOT NULL,
              payload_json TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              PRIMARY KEY (provider_id, application_id)
            );
            CREATE INDEX IF NOT EXISTS catalog_enrichments_lookup
              ON catalog_enrichments (application_id, priority DESC, updated_at DESC);
            CREATE TABLE IF NOT EXISTS user_settings (
              id INTEGER PRIMARY KEY CHECK (id = 1),
              theme_mode TEXT NOT NULL,
              backdrop_mode TEXT NOT NULL DEFAULT 'solid',
              revision INTEGER NOT NULL DEFAULT 0,
              updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS hearthdeck_schema_migrations (
              version INTEGER PRIMARY KEY
            );
            "#,
        )
        .execute(&self.pool)
        .await?;
        // Existing development databases predate source ownership. SQLite has
        // no `ADD COLUMN IF NOT EXISTS`, so a duplicate-column error is the
        // expected no-op case after the first migration.
        let _ = sqlx::query(
            "ALTER TABLE library_items ADD COLUMN source_id TEXT NOT NULL DEFAULT 'legacy'",
        )
        .execute(&self.pool)
        .await;
        let _ = sqlx::query("ALTER TABLE library_items ADD COLUMN launch_id TEXT")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query(
            "ALTER TABLE user_settings ADD COLUMN backdrop_mode TEXT NOT NULL DEFAULT 'solid' CHECK (backdrop_mode IN ('solid', 'edge_wash', 'quiet_grid'))",
        )
        .execute(&self.pool)
        .await;
        let migration =
            sqlx::query("INSERT OR IGNORE INTO hearthdeck_schema_migrations (version) VALUES (1)")
                .execute(&self.pool)
                .await?;
        if migration.rows_affected() == 1 {
            let mut transaction = self.pool.begin().await?;
            sqlx::query(
                "CREATE TABLE user_settings_rebuilt (id INTEGER PRIMARY KEY CHECK (id = 1), theme_mode TEXT NOT NULL, backdrop_mode TEXT NOT NULL DEFAULT 'solid', revision INTEGER NOT NULL DEFAULT 0, updated_at TEXT NOT NULL)",
            )
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "INSERT INTO user_settings_rebuilt (id, theme_mode, backdrop_mode, revision, updated_at) SELECT id, theme_mode, backdrop_mode, revision, updated_at FROM user_settings",
            )
            .execute(&mut *transaction)
            .await?;
            sqlx::query("DROP TABLE user_settings")
                .execute(&mut *transaction)
                .await?;
            sqlx::query("ALTER TABLE user_settings_rebuilt RENAME TO user_settings")
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
        }
        let migration =
            sqlx::query("INSERT OR IGNORE INTO hearthdeck_schema_migrations (version) VALUES (2)")
                .execute(&self.pool)
                .await?;
        if migration.rows_affected() == 1 {
            let mut transaction = self.pool.begin().await?;
            sqlx::query(
                "CREATE TABLE user_settings_rebuilt (id INTEGER PRIMARY KEY CHECK (id = 1), theme_mode TEXT NOT NULL, backdrop_mode TEXT NOT NULL DEFAULT 'solid', revision INTEGER NOT NULL DEFAULT 0, updated_at TEXT NOT NULL)",
            )
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "INSERT INTO user_settings_rebuilt (id, theme_mode, backdrop_mode, revision, updated_at) SELECT id, theme_mode, backdrop_mode, revision, updated_at FROM user_settings",
            )
            .execute(&mut *transaction)
            .await?;
            sqlx::query("DROP TABLE user_settings")
                .execute(&mut *transaction)
                .await?;
            sqlx::query("ALTER TABLE user_settings_rebuilt RENAME TO user_settings")
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
        }
        sqlx::query(
            "INSERT OR IGNORE INTO user_settings (id, theme_mode, backdrop_mode, revision, updated_at) VALUES (1, 'noir', 'solid', 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
