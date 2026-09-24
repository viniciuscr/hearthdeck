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
            CREATE TABLE IF NOT EXISTS romm_settings (
              id INTEGER PRIMARY KEY CHECK (id = 1),
              base_url TEXT NOT NULL,
              token TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS hearthdeck_schema_migrations (
              version INTEGER PRIMARY KEY
            );
                        CREATE TABLE IF NOT EXISTS launch_activity (
                            item_id TEXT PRIMARY KEY,
                            snapshot_json TEXT NOT NULL,
                            last_launched_at TEXT NOT NULL,
                            launch_count INTEGER NOT NULL DEFAULT 1
                        );
            -- One row per play, not per item. `launch_activity` above is a cache of
            -- this for the dashboard's shelf; this is the record, and the only place
            -- a duration can come from. `duration_seconds` is NULL, not 0, for a play
            -- whose end was never observed: a gap in a total beats a wrong number.
            CREATE TABLE IF NOT EXISTS play_sessions (
              session_id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              source_id TEXT NOT NULL,
              started_at TEXT NOT NULL,
              ended_at TEXT,
              outcome TEXT NOT NULL CHECK (outcome IN ('running', 'closed', 'unclosed')),
              duration_seconds INTEGER
            );
            CREATE INDEX IF NOT EXISTS play_sessions_by_item
              ON play_sessions (item_id, started_at DESC);
            -- Partial, because this table grows and the running rows are never more
            -- than the one play the bridge allows at a time.
            CREATE INDEX IF NOT EXISTS play_sessions_running
              ON play_sessions (outcome) WHERE outcome = 'running';
            CREATE TABLE IF NOT EXISTS collections (
              slug TEXT PRIMARY KEY,
              kind TEXT NOT NULL,
              rule TEXT,
              role TEXT NOT NULL DEFAULT 'none',
              position INTEGER NOT NULL,
              created_at TEXT NOT NULL,
              owner TEXT NOT NULL DEFAULT 'user',
              name TEXT
            );
            CREATE TABLE IF NOT EXISTS collection_items (
              collection_slug TEXT NOT NULL REFERENCES collections(slug) ON DELETE CASCADE,
              item_id TEXT NOT NULL,
              name TEXT NOT NULL,
              icon TEXT,
              position INTEGER NOT NULL,
              created_at TEXT NOT NULL,
              PRIMARY KEY (collection_slug, item_id)
            );
            INSERT OR IGNORE INTO collections (slug, kind, rule, role, position, created_at, owner, name) VALUES
              ('last-played', 'rule', 'last_played', 'none', 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 'system', NULL),
              ('favorites', 'membership', NULL, 'favorite', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 'user', NULL),
              ('play-later', 'membership', NULL, 'play_later', 2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 'user', NULL);
            -- One row, the newest report: a categorization describes the library as
            -- it stood when the scan ran, so a stale one is worthless.
            CREATE TABLE IF NOT EXISTS categorization_reports (
              id INTEGER PRIMARY KEY CHECK (id = 1),
              generated_at TEXT NOT NULL,
              categorizer TEXT NOT NULL,
              payload_json TEXT NOT NULL,
              updated_at TEXT NOT NULL
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
        // Collections gained an owner: who may edit one. The seeded rows predate
        // it, and their owner is the only thing about them that is not already
        // implied by their slug and role.
        let _ =
            sqlx::query("ALTER TABLE collections ADD COLUMN owner TEXT NOT NULL DEFAULT 'user'")
                .execute(&self.pool)
                .await;
        let _ = sqlx::query("ALTER TABLE collections ADD COLUMN name TEXT")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("UPDATE collections SET owner = 'system' WHERE slug = 'last-played'")
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
        // Columns added after `user_settings` was first defined have to be added
        // here, below the versioned rebuilds: those rebuild the table from a fixed
        // column list, so a column added before them is dropped again on any
        // database that has not yet applied them.
        let _ = sqlx::query(
            "ALTER TABLE user_settings ADD COLUMN categorization_enabled INTEGER NOT NULL DEFAULT 0",
        )
        .execute(&self.pool)
        .await;
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
