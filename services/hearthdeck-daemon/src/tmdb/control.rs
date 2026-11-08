use tracing::info;

use super::{TmdbRepository, TmdbSettings};
use crate::catalog::CatalogStore;
use crate::discovery::providers::arrivals;

/// The TMDB credential and the rows it published, changed together.
///
/// Clearing the key also drops the arrivals it produced: leaving them behind
/// would put a banner of titles on screen that nothing can refresh or explain,
/// which is what unlinking a Stremio account already avoids.
pub(crate) struct Control<'a> {
    pub settings: &'a TmdbRepository,
    pub catalog: &'a CatalogStore,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ControlError {
    #[error("{0}")]
    InvalidKey(#[source] anyhow::Error),
    #[error("could not read or clear the TMDB credential")]
    Storage(#[source] anyhow::Error),
    #[error("could not clear the arrival rows")]
    Catalog(#[from] sqlx::Error),
}

impl Control<'_> {
    pub(crate) async fn settings(&self) -> Result<Option<TmdbSettings>, ControlError> {
        self.settings
            .settings()
            .await
            .map_err(ControlError::Storage)
    }

    pub(crate) async fn save(&self, api_key: &str) -> Result<TmdbSettings, ControlError> {
        self.settings
            .save(api_key)
            .await
            .map_err(ControlError::InvalidKey)
    }

    pub(crate) async fn clear(&self) -> Result<(), ControlError> {
        self.settings.clear().await.map_err(ControlError::Storage)?;
        self.catalog
            .replace_source(arrivals::SOURCE_ID, Vec::new())
            .await?;
        info!("tmdb credential cleared");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::database::Database;

    async fn storage() -> (tempfile::TempDir, Database, TmdbRepository, CatalogStore) {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let settings = TmdbRepository::new(database.pool().clone());
        let catalog = CatalogStore::new(database.pool().clone());
        (directory, database, settings, catalog)
    }

    async fn seed_sources(database: &Database) {
        for source in [arrivals::SOURCE_ID, "stremio", "desktop-apps"] {
            sqlx::query(
                "INSERT INTO library_items \
                 (id, source_id, title, kind, metadata_json, updated_at) \
                 VALUES (?, ?, ?, 'movie', '{}', '2026-01-01')",
            )
            .bind(source)
            .bind(source)
            .bind(source)
            .execute(database.pool())
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn saving_a_key_stores_it_and_serves_only_the_settings_view() {
        let (_directory, _database, settings, catalog) = storage().await;
        let control = Control {
            settings: &settings,
            catalog: &catalog,
        };

        let saved = control
            .save("0123456789abcdef0123456789abcdef")
            .await
            .unwrap();

        assert!(saved.configured);
        assert!(
            !serde_json::to_string(&saved)
                .unwrap()
                .contains("0123456789abcdef")
        );
    }

    #[tokio::test]
    async fn a_key_that_could_not_work_is_refused_before_it_is_stored() {
        let (_directory, _database, settings, catalog) = storage().await;
        let control = Control {
            settings: &settings,
            catalog: &catalog,
        };

        assert!(matches!(
            control.save("short").await,
            Err(ControlError::InvalidKey(_))
        ));
        assert!(settings.credentials().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn clearing_the_key_removes_only_the_arrival_rows() {
        let (_directory, database, settings, catalog) = storage().await;
        seed_sources(&database).await;
        settings
            .save("0123456789abcdef0123456789abcdef")
            .await
            .unwrap();
        let control = Control {
            settings: &settings,
            catalog: &catalog,
        };

        control.clear().await.unwrap();
        control.clear().await.unwrap();

        assert!(control.settings().await.unwrap().is_none());
        let sources: Vec<String> =
            sqlx::query_scalar("SELECT source_id FROM library_items ORDER BY source_id")
                .fetch_all(database.pool())
                .await
                .unwrap();
        assert_eq!(sources, ["desktop-apps", "stremio"]);
    }
}
