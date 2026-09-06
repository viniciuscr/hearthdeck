use serde::Serialize;
use serde_json::{Map, Value, json};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use tracing::info;

#[derive(Clone)]
pub struct CatalogStore {
    pool: SqlitePool,
}

impl CatalogStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Replaces the records owned by one discovery provider in a single
    /// transaction. One provider can never delete or overwrite another source.
    pub async fn replace_source(
        &self,
        source_id: &str,
        records: Vec<CatalogRecord>,
    ) -> Result<(), sqlx::Error> {
        let record_count = records.len();
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM library_items WHERE source_id = ?")
            .bind(source_id)
            .execute(&mut *transaction)
            .await?;
        for record in records {
            sqlx::query(
                r#"
                INSERT INTO library_items (id, source_id, title, kind, launch_id, icon, metadata_json, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET source_id = excluded.source_id,
                  title = excluded.title, kind = excluded.kind,
                  icon = excluded.icon, launch_id = excluded.launch_id,
                  metadata_json = excluded.metadata_json, updated_at = excluded.updated_at
                "#,
            )
            .bind(record.id)
            .bind(source_id)
            .bind(record.title)
            .bind(record.kind)
            .bind(record.launch_id)
            .bind(record.icon)
            .bind(record.metadata.to_string())
            .bind(record.updated_at)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        info!(source_id, record_count, "catalog source replaced");
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<CatalogItem>, sqlx::Error> {
        let rows = sqlx::query(
            r#"
            SELECT
              item.id, item.source_id, item.title, item.kind, item.launch_id, item.icon,
              item.metadata_json,
              enrichment.provider_id AS enrichment_provider_id,
              enrichment.payload_json AS enrichment_payload_json
            FROM library_items AS item
            LEFT JOIN catalog_enrichments AS enrichment ON enrichment.rowid = (
              SELECT candidate.rowid
              FROM catalog_enrichments AS candidate
              WHERE candidate.application_id = item.launch_id
              ORDER BY candidate.priority DESC, candidate.updated_at DESC
              LIMIT 1
            )
            ORDER BY item.title
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(catalog_item_from_row).collect())
    }

    /// Replaces every application alias owned by a metadata provider in one
    /// transaction. Discovery records remain untouched.
    pub async fn replace_enrichment_source(
        &self,
        provider_id: &str,
        records: Vec<EnrichmentRecord>,
    ) -> Result<(), sqlx::Error> {
        let record_count = records.len();
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM catalog_enrichments WHERE provider_id = ?")
            .bind(provider_id)
            .execute(&mut *transaction)
            .await?;
        for record in records {
            for application_id in record.application_ids {
                sqlx::query(
                    r#"
                    INSERT INTO catalog_enrichments (provider_id, application_id, priority, payload_json, updated_at)
                    VALUES (?, ?, ?, ?, ?)
                    ON CONFLICT(provider_id, application_id) DO UPDATE SET
                      priority = excluded.priority,
                      payload_json = excluded.payload_json,
                      updated_at = excluded.updated_at
                    "#,
                )
                .bind(provider_id)
                .bind(application_id)
                .bind(record.priority)
                .bind(record.payload.to_string())
                .bind(&record.updated_at)
                .execute(&mut *transaction)
                .await?;
            }
        }
        transaction.commit().await?;
        info!(
            provider_id,
            record_count, "catalog enrichment source replaced"
        );
        Ok(())
    }

    pub async fn get(&self, item_id: &str) -> Result<Option<CatalogItem>, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT
              item.id, item.source_id, item.title, item.kind, item.launch_id, item.icon,
              item.metadata_json,
              enrichment.provider_id AS enrichment_provider_id,
              enrichment.payload_json AS enrichment_payload_json
            FROM library_items AS item
            LEFT JOIN catalog_enrichments AS enrichment ON enrichment.rowid = (
              SELECT candidate.rowid
              FROM catalog_enrichments AS candidate
              WHERE candidate.application_id = item.launch_id
              ORDER BY candidate.priority DESC, candidate.updated_at DESC
              LIMIT 1
            )
            WHERE item.id = ?
            "#,
        )
        .bind(item_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(catalog_item_from_row))
    }
}

fn catalog_item_from_row(row: SqliteRow) -> CatalogItem {
    let discovery =
        serde_json::from_str(&row.get::<String, _>("metadata_json")).unwrap_or(Value::Null);
    let enrichment: Option<Value> = row
        .get::<Option<String>, _>("enrichment_payload_json")
        .and_then(|payload| serde_json::from_str(&payload).ok());
    let enrichment_provider: Option<String> = row.get("enrichment_provider_id");
    let title: String = row.get("title");
    CatalogItem {
        id: row.get("id"),
        source_id: row.get("source_id"),
        title: title.clone(),
        kind: row.get("kind"),
        launch_id: row.get("launch_id"),
        icon: row
            .get::<Option<String>, _>("icon")
            .or_else(|| metadata_string(enrichment.as_ref(), "icon")),
        metadata: merged_metadata(&title, &discovery, enrichment.as_ref(), enrichment_provider),
    }
}

fn merged_metadata(
    title: &str,
    discovery: &Value,
    enrichment: Option<&Value>,
    enrichment_provider: Option<String>,
) -> Value {
    let summary = metadata_string(enrichment, "description")
        .or_else(|| metadata_string(enrichment, "summary"))
        .or_else(|| metadata_string(Some(discovery), "comment"))
        .or_else(|| metadata_string(Some(discovery), "description"))
        .unwrap_or_else(|| title.to_owned());
    let categories = metadata_string_list(enrichment, "categories")
        .or_else(|| metadata_string_list(Some(discovery), "categories"))
        .filter(|categories| !categories.is_empty())
        .unwrap_or_else(|| vec!["Other".to_owned()]);
    let urls = metadata_object(enrichment, "urls").unwrap_or_default();
    let screenshots = metadata_string_list(enrichment, "screenshots").unwrap_or_default();

    json!({
        "summary": summary,
        "description": metadata_string(enrichment, "description")
            .or_else(|| metadata_string(Some(discovery), "description")),
        "categories": categories,
        "developer": metadata_string(enrichment, "developer")
            .or_else(|| metadata_string(Some(discovery), "developer")),
        "project_license": metadata_string(enrichment, "project_license"),
        "store": metadata_string(Some(discovery), "store"),
        "runner": metadata_string(Some(discovery), "runner"),
        "version": metadata_string(Some(discovery), "version"),
        "platform": metadata_string(Some(discovery), "platform"),
        "cloud_saves": discovery.get("cloud_saves").and_then(Value::as_bool),
        "install_size_bytes": discovery.get("install_size_bytes").and_then(Value::as_u64),
        "requirements": discovery.get("requirements").cloned().unwrap_or_else(|| json!([])),
        "memory_compatibility": discovery.get("memory_compatibility").cloned(),
        "urls": urls,
        "screenshots": screenshots,
        "provenance": enrichment_provider
            .as_deref()
            .map(ToString::to_string)
            .or_else(|| metadata_string(Some(discovery), "provenance"))
            .unwrap_or_else(|| "desktop-entry".to_owned()),
        "discovery": discovery,
        "enrichment": enrichment,
        "enrichment_provider": enrichment_provider,
    })
}

fn metadata_string(metadata: Option<&Value>, key: &str) -> Option<String> {
    metadata?
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn metadata_string_list(metadata: Option<&Value>, key: &str) -> Option<Vec<String>> {
    let values = metadata?.get(key)?.as_array()?;
    let mut unique = Vec::new();
    for value in values
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !unique.iter().any(|existing: &String| existing == value) {
            unique.push(value.to_owned());
        }
    }
    Some(unique)
}

fn metadata_object(metadata: Option<&Value>, key: &str) -> Option<Map<String, Value>> {
    metadata?.get(key)?.as_object().cloned()
}

pub struct CatalogRecord {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub launch_id: Option<String>,
    pub icon: Option<String>,
    pub metadata: serde_json::Value,
    pub updated_at: String,
}

pub struct EnrichmentRecord {
    pub application_ids: Vec<String>,
    pub priority: i64,
    pub payload: serde_json::Value,
    pub updated_at: String,
}

#[derive(Clone, Serialize)]
pub struct CatalogItem {
    pub id: String,
    pub source_id: String,
    pub title: String,
    pub kind: String,
    pub launch_id: Option<String>,
    pub icon: Option<String>,
    pub metadata: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use sqlx::Row;
    use tempfile::tempdir;

    use super::{CatalogStore, EnrichmentRecord, merged_metadata};
    use crate::database::Database;

    #[tokio::test]
    async fn duplicate_metadata_aliases_replace_the_prior_record() {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let catalog = CatalogStore::new(database.pool().clone());

        catalog
            .replace_enrichment_source(
                "appstream-local",
                vec![
                    EnrichmentRecord {
                        application_ids: vec!["org.example.App.desktop".to_owned()],
                        priority: 100,
                        payload: serde_json::json!({"summary": "First"}),
                        updated_at: "2026-01-01T00:00:00Z".to_owned(),
                    },
                    EnrichmentRecord {
                        application_ids: vec!["org.example.App.desktop".to_owned()],
                        priority: 100,
                        payload: serde_json::json!({"summary": "Second"}),
                        updated_at: "2026-01-01T00:00:01Z".to_owned(),
                    },
                ],
            )
            .await
            .unwrap();

        let row = sqlx::query(
            "SELECT payload_json FROM catalog_enrichments WHERE provider_id = ? AND application_id = ?",
        )
        .bind("appstream-local")
        .bind("org.example.App.desktop")
        .fetch_one(database.pool())
        .await
        .unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(&row.get::<String, _>("payload_json")).unwrap();

        assert_eq!(payload["summary"], "Second");
    }

    #[test]
    fn merges_desktop_entry_data_into_a_minimum_metadata_baseline() {
        let metadata = merged_metadata(
            "Example",
            &serde_json::json!({
                "comment": "A useful application",
                "categories": ["Utility"],
            }),
            None,
            None,
        );

        assert_eq!(metadata["summary"], "A useful application");
        assert_eq!(metadata["categories"], serde_json::json!(["Utility"]));
        assert_eq!(metadata["provenance"], "desktop-entry");
        assert_eq!(metadata["urls"], serde_json::json!({}));
    }
}
