use serde::Serialize;
use sqlx::{Row, SqlitePool};
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
        Ok(rows
            .into_iter()
            .map(|row| {
                let discovery = serde_json::from_str(&row.get::<String, _>("metadata_json"))
                    .unwrap_or(serde_json::Value::Null);
                let enrichment: Option<serde_json::Value> = row
                    .get::<Option<String>, _>("enrichment_payload_json")
                    .and_then(|payload| serde_json::from_str(&payload).ok());
                CatalogItem {
                    id: row.get("id"),
                    source_id: row.get("source_id"),
                    title: row.get("title"),
                    kind: row.get("kind"),
                    launch_id: row.get("launch_id"),
                    icon: row.get("icon"),
                    metadata: serde_json::json!({
                        "discovery": discovery,
                        "enrichment": enrichment,
                        "enrichment_provider": row.get::<Option<String>, _>("enrichment_provider_id"),
                    }),
                }
            })
            .collect())
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

    pub async fn launch_id_for(&self, item_id: &str) -> Result<Option<String>, sqlx::Error> {
        let row = sqlx::query("SELECT launch_id FROM library_items WHERE id = ?")
            .bind(item_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|row| row.get("launch_id")))
    }

    pub async fn source_id_for(&self, item_id: &str) -> Result<Option<String>, sqlx::Error> {
        let row = sqlx::query("SELECT source_id FROM library_items WHERE id = ?")
            .bind(item_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|row| row.get("source_id")))
    }
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

#[derive(Serialize)]
pub struct CatalogItem {
    pub id: String,
    pub source_id: String,
    pub title: String,
    pub kind: String,
    pub launch_id: Option<String>,
    pub icon: Option<String>,
    pub metadata: serde_json::Value,
}
