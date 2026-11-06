use std::path::Path;

use anyhow::{Context, Result, ensure};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use tracing::warn;

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
        let pool = SqlitePool::connect_with(options).await?;
        // The pool is connected, so the file exists — SQLite creates it before
        // the first statement runs, and this must happen before the first write
        // so the WAL siblings inherit the narrowed mode too.
        restrict_to_owner(path).await;
        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut *transaction)
            .await?;
        ensure!(
            matches!(version, 0 | 1),
            "unsupported database schema version {version}"
        );
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
              updated_at TEXT NOT NULL,
              categorization_enabled INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS romm_settings (
              id INTEGER PRIMARY KEY CHECK (id = 1),
              base_url TEXT NOT NULL,
              token TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            -- The Stremio account link. One row, and the only thing in it that is
            -- not a timestamp is the session key the link flow produced.
            --
            -- `auth_key` is a bearer token with no expiry and no refresh token, so
            -- it is read by the provider and nothing else: no handler returns it,
            -- no log line names it, and the settings screen is served only the
            -- timestamps below. It is also the reason the database file itself is
            -- narrowed to its owner (see `restrict_to_owner`).
            --
            -- There is no user or account column because the link flow has none to
            -- give: `/api/read` answers with a key and nothing else, so the link
            -- time is all there is to show.
            CREATE TABLE IF NOT EXISTS stremio_settings (
              id INTEGER PRIMARY KEY CHECK (id = 1),
              auth_key TEXT NOT NULL,
              linked_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
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
            -- What the categorizer decided about one item, kept apart from the
            -- catalog because every discovery refresh rewrites that. The hash and
            -- the model stamp are what let a later pass ask only about what changed.
            CREATE TABLE IF NOT EXISTS item_traits (
              item_id TEXT NOT NULL,
              trait TEXT NOT NULL,
              value TEXT NOT NULL,
              confidence REAL NOT NULL,
              input_hash TEXT NOT NULL,
              model TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              PRIMARY KEY (item_id, trait)
            );
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
        .execute(&mut *transaction)
        .await?;
        // Only the current deployed schema is adopted; historical upgrades are retired.
        for statement in [
            "SELECT source_id, launch_id FROM library_items LIMIT 0",
            "SELECT backdrop_mode, revision, categorization_enabled FROM user_settings LIMIT 0",
            "SELECT owner, name FROM collections LIMIT 0",
        ] {
            sqlx::query(statement)
                .execute(&mut *transaction)
                .await
                .context("database predates the supported baseline; restore a current database before starting")?;
        }
        sqlx::query(
            "INSERT OR IGNORE INTO user_settings (id, theme_mode, backdrop_mode, revision, updated_at) VALUES (1, 'noir', 'solid', 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DROP TABLE IF EXISTS hearthdeck_schema_migrations; PRAGMA user_version = 1")
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        crate::collections::seed_builtin(&self.pool).await?;
        Ok(())
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// Narrows the database file to its owner.
///
/// The database holds bearer-equivalent credentials — the RomM token, and now the
/// Stremio session — while SQLite creates the file subject to the umask, which a
/// default login leaves at `0644`, readable by every account on the box. The WAL
/// and SHM siblings take the database file's mode when they are created, so doing
/// this before the first write covers them as well.
///
/// A filesystem that refuses the change is reported and tolerated rather than
/// fatal: refusing to start over a permission quirk on a mounted share would be a
/// worse outcome than a database that is more readable than intended.
async fn restrict_to_owner(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let owner_only = std::fs::Permissions::from_mode(0o600);
    if let Err(error) = tokio::fs::set_permissions(path, owner_only).await {
        warn!(
            path = %path.display(),
            %error,
            "could not restrict the database file to its owner"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::Database;

    #[tokio::test]
    async fn rejects_prebaseline_databases_without_partial_schema_changes() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();

        sqlx::query(
            "CREATE TABLE collections (\
             slug TEXT PRIMARY KEY, kind TEXT NOT NULL, rule TEXT, \
             role TEXT NOT NULL DEFAULT 'none', position INTEGER NOT NULL, \
             created_at TEXT NOT NULL)",
        )
        .execute(database.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO collections (slug, kind, rule, role, position, created_at) \
             VALUES ('last-played', 'rule', 'last_played', 'none', 0, '2020-01-01T00:00:00Z')",
        )
        .execute(database.pool())
        .await
        .unwrap();

        let error = database.migrate().await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("predates the supported baseline")
        );
        let tables: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
                .fetch_all(database.pool())
                .await
                .unwrap();
        assert_eq!(tables, ["collections"]);
        let slug: String = sqlx::query_scalar("SELECT slug FROM collections")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(slug, "last-played");
    }

    /// A database created before Stremio support gains its table on the next
    /// migrate, keeps a session it already holds across a re-migration, and
    /// refuses a second row.
    #[tokio::test]
    async fn adds_the_stremio_table_to_an_existing_database() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();

        // Stand in for a database migrated before this table existed: its
        // neighbours are there, `stremio_settings` is not.
        sqlx::query(
            "CREATE TABLE romm_settings (\
             id INTEGER PRIMARY KEY CHECK (id = 1), base_url TEXT NOT NULL, \
             token TEXT NOT NULL, updated_at TEXT NOT NULL)",
        )
        .execute(database.pool())
        .await
        .unwrap();

        database.migrate().await.unwrap();

        let present: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'stremio_settings'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(present, 1);

        sqlx::query(
            "INSERT INTO stremio_settings (id, auth_key, linked_at, updated_at) \
             VALUES (1, 'linked_key', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .execute(database.pool())
        .await
        .unwrap();

        // The upgrade path for a database that already migrated: a no-op.
        database.migrate().await.unwrap();

        let auth_key: Option<String> =
            sqlx::query_scalar("SELECT auth_key FROM stremio_settings WHERE id = 1")
                .fetch_optional(database.pool())
                .await
                .unwrap();
        assert_eq!(auth_key.as_deref(), Some("linked_key"));

        // `CHECK (id = 1)` is what makes this a single-session table.
        let second_row = sqlx::query(
            "INSERT INTO stremio_settings (id, auth_key, linked_at, updated_at) \
             VALUES (2, 'other_key', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .execute(database.pool())
        .await;
        assert!(second_row.is_err(), "a second session row must be refused");
    }

    /// The database holds bearer-equivalent credentials, so it is owner-only.
    #[cfg(unix)]
    #[tokio::test]
    async fn restricts_the_database_file_to_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hearthdeck.db");
        let database = Database::connect(&path).await.unwrap();
        database.migrate().await.unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "the database should be readable only by its owner"
        );
    }

    #[tokio::test]
    async fn a_fresh_baseline_is_idempotent_without_a_legacy_ledger() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();

        database.migrate().await.unwrap();
        // Running again is the upgrade path for a database that already migrated;
        // it must be a no-op rather than an error.
        database.migrate().await.unwrap();

        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(version, 1);
        let obsolete: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('user_settings_rebuilt', 'hearthdeck_schema_migrations')",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(obsolete, 0);
        let settings: (String, String, i64, bool) = sqlx::query_as(
            "SELECT theme_mode, backdrop_mode, revision, categorization_enabled FROM user_settings",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(settings, ("noir".into(), "solid".into(), 0, false));
        let builtins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM collections")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(builtins, 5);
    }

    #[tokio::test]
    async fn adopts_the_current_deployed_settings_without_rebuilding_or_losing_values() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE user_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1), theme_mode TEXT NOT NULL,
                backdrop_mode TEXT NOT NULL DEFAULT 'solid', revision INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL, categorization_enabled INTEGER NOT NULL DEFAULT 0
             );
             INSERT INTO user_settings VALUES (1, 'light', 'edge_wash', 42, 'saved', 1);
             CREATE TABLE hearthdeck_schema_migrations (version INTEGER PRIMARY KEY);
             INSERT INTO hearthdeck_schema_migrations VALUES (1), (2);
             CREATE TABLE romm_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1), base_url TEXT NOT NULL,
                token TEXT NOT NULL, updated_at TEXT NOT NULL
             );
             INSERT INTO romm_settings VALUES (1, 'http://romm', 'saved-token', 'saved');
             CREATE TABLE collections (
                slug TEXT PRIMARY KEY, kind TEXT NOT NULL, rule TEXT,
                role TEXT NOT NULL DEFAULT 'none', position INTEGER NOT NULL,
                created_at TEXT NOT NULL, owner TEXT NOT NULL DEFAULT 'user', name TEXT
             );
             INSERT INTO collections VALUES ('my-list', 'membership', NULL, 'none', 6, 'saved', 'user', 'My list');
             CREATE TABLE collection_items (
                collection_slug TEXT NOT NULL REFERENCES collections(slug) ON DELETE CASCADE,
                item_id TEXT NOT NULL, name TEXT NOT NULL, icon TEXT,
                position INTEGER NOT NULL, created_at TEXT NOT NULL,
                PRIMARY KEY (collection_slug, item_id)
             );
             INSERT INTO collection_items VALUES ('my-list', 'game', 'Saved game', NULL, 0, 'saved');",
        )
        .execute(database.pool())
        .await
        .unwrap();
        let root: i64 =
            sqlx::query_scalar("SELECT rootpage FROM sqlite_master WHERE name = 'user_settings'")
                .fetch_one(database.pool())
                .await
                .unwrap();
        database.migrate().await.unwrap();
        database.migrate().await.unwrap();
        let settings: (String, String, i64, String, bool) = sqlx::query_as(
            "SELECT theme_mode, backdrop_mode, revision, updated_at, categorization_enabled FROM user_settings",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(
            settings,
            ("light".into(), "edge_wash".into(), 42, "saved".into(), true)
        );
        let current_root: i64 =
            sqlx::query_scalar("SELECT rootpage FROM sqlite_master WHERE name = 'user_settings'")
                .fetch_one(database.pool())
                .await
                .unwrap();
        assert_eq!(root, current_root);
        let token: String = sqlx::query_scalar("SELECT token FROM romm_settings WHERE id = 1")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(token, "saved-token");
        let saved_item: (String, String) = sqlx::query_as(
            "SELECT c.name, i.item_id FROM collections c JOIN collection_items i ON i.collection_slug = c.slug WHERE c.slug = 'my-list'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(saved_item, ("My list".into(), "game".into()));
    }

    #[tokio::test]
    async fn refuses_future_schema_versions_without_changing_them() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        sqlx::query("PRAGMA user_version = 2")
            .execute(database.pool())
            .await
            .unwrap();
        assert!(
            database
                .migrate()
                .await
                .unwrap_err()
                .to_string()
                .contains("unsupported database schema version 2")
        );
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(version, 2);
    }
}
