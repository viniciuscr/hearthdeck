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

    /// One source's records, most recently changed first, up to `limit`.
    ///
    /// A rail composed from a provider's records needs a defined order, and the
    /// order the provider published them in is not stored — the record's own
    /// `updated_at` is, and for the Stremio records that is the account record's
    /// modification time, which is what Stremio itself sorts its Continue Watching
    /// row by.
    pub async fn list_source(
        &self,
        source_id: &str,
        limit: usize,
    ) -> Result<Vec<CatalogItem>, sqlx::Error> {
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
            WHERE item.source_id = ?
            ORDER BY item.updated_at DESC, item.id
            LIMIT ?
            "#,
        )
        .bind(source_id)
        .bind(limit as i64)
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
    let source_id: String = row.get("source_id");
    let metadata = merged_metadata(&title, &discovery, enrichment.as_ref(), enrichment_provider);
    let kind = served_kind(&source_id, &row.get::<String, _>("kind"));
    CatalogItem {
        id: row.get("id"),
        source_id,
        title,
        kind,
        launch_id: row.get("launch_id"),
        icon: row
            .get::<Option<String>, _>("icon")
            .or_else(|| metadata_string(enrichment.as_ref(), "icon")),
        metadata,
    }
}

/// The provider whose records are the only ones served as games.
const HEROIC_SOURCE: &str = "heroic";

/// The kind for a record the library plays.
const GAME: &str = "game";

/// The kind for a record the library runs.
///
/// These two are the daemon's own vocabulary for what it serves, so they live here
/// rather than being borrowed from the categorizer: the categorizer reads records,
/// it does not own what the catalog serves.
const APPLICATION: &str = "application";

/// The kind a record is served with.
///
/// The product files a PC game only when Heroic discovered it: Heroic is the
/// one provider in this crate that finds the Epic and GOG games the library can
/// launch, so nothing else is a game. A desktop entry describes a program the
/// library runs rather than a game it plays, however its own `Categories` list
/// is spelled — Goverlay configures MangoHud and vkBasalt overlays and declares
/// `Game` — and a streaming record is a film or a series. Settling the served
/// kind by the source instead of by the categories is what keeps a tool out of
/// the games section, and it does so without leaning on AppStream enrichment,
/// which does not match on every machine. A standalone desktop game such as
/// SuperTux therefore leaves the section as well, a trade-off the product
/// accepted.
fn served_kind(source_id: &str, claimed: &str) -> String {
    if claimed.eq_ignore_ascii_case(GAME) && source_id != HEROIC_SOURCE {
        APPLICATION.to_owned()
    } else {
        claimed.to_owned()
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
        "source": metadata_string(Some(discovery), "source"),
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

pub use hearthdeck_protocol::CatalogItem;

#[derive(Clone)]
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

#[cfg(test)]
mod tests {
    use sqlx::Row;
    use tempfile::tempdir;

    use super::{CatalogRecord, CatalogStore, EnrichmentRecord, merged_metadata};
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

    /// The serving rule since the desktop games were removed: a PC game is a
    /// game only when Heroic discovered it. A desktop entry that declares the
    /// `Game` category is still a program — Goverlay only configures game
    /// overlays, and SuperTux is a game the library does not run through
    /// Heroic — so only a Heroic record is served as a game.
    #[tokio::test]
    async fn serves_a_game_only_when_heroic_discovered_it() {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let catalog = CatalogStore::new(database.pool().clone());

        let desktop_record =
            |application_id: &str, title: &str, categories: &[&str]| CatalogRecord {
                id: format!("desktop:{application_id}"),
                title: title.to_owned(),
                kind: "game".to_owned(),
                launch_id: Some(application_id.to_owned()),
                icon: None,
                metadata: serde_json::json!({ "categories": categories }),
                updated_at: "2026-01-01T00:00:00Z".to_owned(),
            };
        catalog
            .replace_source(
                "desktop-apps",
                vec![
                    desktop_record(
                        "io.github.benjamimgois.goverlay.desktop",
                        "Goverlay",
                        &["Game"],
                    ),
                    desktop_record("super-tux.desktop", "SuperTux", &["Game", "ActionGame"]),
                ],
            )
            .await
            .unwrap();
        catalog
            .replace_source(
                "heroic",
                vec![CatalogRecord {
                    id: "heroic:epic:Fortnite".to_owned(),
                    title: "Fortnite".to_owned(),
                    kind: "game".to_owned(),
                    launch_id: Some("epic:Fortnite".to_owned()),
                    icon: None,
                    metadata: serde_json::json!({ "categories": ["Action"], "store": "Epic" }),
                    updated_at: "2026-01-01T00:00:00Z".to_owned(),
                }],
            )
            .await
            .unwrap();

        let items = catalog.list().await.unwrap();
        let kind = |id: &str| {
            items
                .iter()
                .find(|item| item.id == id)
                .unwrap_or_else(|| panic!("{id} is missing from the library"))
                .kind
                .clone()
        };

        // The tool: a desktop entry that claims a game, but nothing the desktop
        // provider discovers is a game any more.
        assert_eq!(
            kind("desktop:io.github.benjamimgois.goverlay.desktop"),
            "application"
        );
        // The standalone game: it left the games section with the rest of the
        // desktop entries, the accepted trade-off.
        assert_eq!(kind("desktop:super-tux.desktop"), "application");
        // The only games that remain are the ones Heroic found.
        assert_eq!(kind("heroic:epic:Fortnite"), "game");
    }

    #[test]
    fn merges_desktop_entry_data_into_a_minimum_metadata_baseline() {
        let metadata = merged_metadata(
            "Example",
            &serde_json::json!({
                "comment": "A useful application",
                "categories": ["Utility"],
                "source": "flatpak",
            }),
            None,
            None,
        );

        assert_eq!(metadata["summary"], "A useful application");
        assert_eq!(metadata["categories"], serde_json::json!(["Utility"]));
        assert_eq!(metadata["provenance"], "desktop-entry");
        assert_eq!(metadata["urls"], serde_json::json!({}));
        // Where the launcher was installed survives the merge: the client
        // filters applications by it.
        assert_eq!(metadata["source"], "flatpak");
    }
}
