//! Dashboard collections: the curated lists the dashboard is composed from, plus
//! the built-in ones whose contents are derived rather than stored.
//!
//! Membership lives here and nowhere else. Each stored item carries the display
//! name and icon it had when it was added, so a rail can render it without
//! resolving the item against a provider. That is the fix for the frontend-only
//! favorite list this replaces: it could only draw ids it could still find in
//! memory, so a favorited RomM game disappeared from the rail whenever its
//! console section had not been loaded.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Serialize;
use sqlx::{Row, SqlitePool};

/// Items are curated by a user or a service; they live in `collection_items`.
pub const KIND_MEMBERSHIP: &str = "membership";
/// Items are derived from other data on every read; the table holds no rows for
/// these collections.
pub const KIND_RULE: &str = "rule";

/// A rule collection holding the most recently launched items.
pub const RULE_LAST_PLAYED: &str = "last_played";

#[derive(Clone, Debug, Serialize)]
pub struct Collection {
    pub slug: String,
    /// [`KIND_MEMBERSHIP`] or [`KIND_RULE`].
    pub kind: String,
    /// The derivation for a rule collection; absent for a membership one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    /// `favorite` or `play_later` for the collections a screen can toggle by
    /// role; `none` for every other collection.
    pub role: String,
    /// Composition order on the dashboard.
    pub position: i64,
    pub items: Vec<CollectionItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CollectionItem {
    pub item_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Clone)]
pub struct CollectionStore {
    pool: SqlitePool,
}

impl CollectionStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Every collection in composition order, with its stored items.
    ///
    /// Rule collections come back with empty `items`: only the caller knows
    /// where the data each rule reads from lives.
    pub async fn list(&self) -> Result<Vec<Collection>> {
        let collections = sqlx::query(
            "SELECT slug, kind, rule, role, position FROM collections ORDER BY position, slug",
        )
        .fetch_all(&self.pool)
        .await
        .context("list collections")?;
        let items = sqlx::query(
            "SELECT collection_slug, item_id, name, icon FROM collection_items \
             ORDER BY collection_slug, position, created_at",
        )
        .fetch_all(&self.pool)
        .await
        .context("list collection items")?;

        let mut by_collection: HashMap<String, Vec<CollectionItem>> = HashMap::new();
        for row in items {
            by_collection
                .entry(row.get("collection_slug"))
                .or_default()
                .push(CollectionItem {
                    item_id: row.get("item_id"),
                    name: row.get("name"),
                    icon: row.get("icon"),
                });
        }

        Ok(collections
            .into_iter()
            .map(|row| {
                let slug: String = row.get("slug");
                let items = by_collection.remove(&slug).unwrap_or_default();
                Collection {
                    slug,
                    kind: row.get("kind"),
                    rule: row.get("rule"),
                    role: row.get("role"),
                    position: row.get("position"),
                    items,
                }
            })
            .collect())
    }

    pub async fn collection(&self, slug: &str) -> Result<Option<Collection>> {
        Ok(self.list().await?.into_iter().find(|one| one.slug == slug))
    }

    /// Whether `slug` exists and takes curated items. `None` is an unknown
    /// collection; `Some(false)` is one whose contents are derived, so adding to
    /// it would be silently lost.
    pub async fn accepts_items(&self, slug: &str) -> Result<Option<bool>> {
        let row = sqlx::query("SELECT kind FROM collections WHERE slug = ?")
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .context("read collection kind")?;
        Ok(row.map(|row| row.get::<String, _>("kind") == KIND_MEMBERSHIP))
    }

    /// Adds an item, or refreshes the stored name and icon of one already there.
    /// Re-adding keeps the item's original position, so a like and an unlike
    /// followed by another like cannot reorder a rail.
    pub async fn add_item(&self, slug: &str, item: &CollectionItem) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO collection_items (collection_slug, item_id, name, icon, position, created_at)
            VALUES (
              ?, ?, ?, ?,
              (SELECT COALESCE(MAX(position), -1) + 1 FROM collection_items WHERE collection_slug = ?),
              strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            )
            ON CONFLICT(collection_slug, item_id) DO UPDATE SET
              name = excluded.name,
              icon = excluded.icon
            "#,
        )
        .bind(slug)
        .bind(&item.item_id)
        .bind(&item.name)
        .bind(&item.icon)
        .bind(slug)
        .execute(&self.pool)
        .await
        .context("add collection item")?;
        Ok(())
    }

    /// Removes an item, answering whether it was there.
    pub async fn remove_item(&self, slug: &str, item_id: &str) -> Result<bool> {
        let removed =
            sqlx::query("DELETE FROM collection_items WHERE collection_slug = ? AND item_id = ?")
                .bind(slug)
                .bind(item_id)
                .execute(&self.pool)
                .await
                .context("remove collection item")?;
        Ok(removed.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{CollectionItem, CollectionStore, KIND_MEMBERSHIP, KIND_RULE, RULE_LAST_PLAYED};
    use crate::database::Database;

    async fn store() -> CollectionStore {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        CollectionStore::new(database.pool().clone())
    }

    fn item(id: &str, name: &str) -> CollectionItem {
        CollectionItem {
            item_id: id.to_owned(),
            name: name.to_owned(),
            icon: None,
        }
    }

    #[tokio::test]
    async fn the_built_in_collections_are_seeded_in_composition_order() {
        let collections = store().await.list().await.unwrap();
        let slugs: Vec<&str> = collections.iter().map(|one| one.slug.as_str()).collect();

        assert_eq!(slugs, vec!["last-played", "favorites", "play-later"]);
        assert_eq!(collections[0].kind, KIND_RULE);
        assert_eq!(collections[0].rule.as_deref(), Some(RULE_LAST_PLAYED));
        assert_eq!(collections[1].kind, KIND_MEMBERSHIP);
        assert_eq!(collections[1].role, "favorite");
        assert_eq!(collections[2].role, "play_later");
        assert!(collections.iter().all(|one| one.items.is_empty()));
    }

    #[tokio::test]
    async fn curated_items_round_trip_in_the_order_they_were_added() {
        let store = store().await;
        store
            .add_item("favorites", &item("romm:4014", "Zoop"))
            .await
            .unwrap();
        store
            .add_item("favorites", &item("org.example.App", "Writer"))
            .await
            .unwrap();
        // Re-adding refreshes the snapshot without moving the item.
        store
            .add_item("favorites", &item("romm:4014", "Zoop (USA)"))
            .await
            .unwrap();

        let favorites = store.collection("favorites").await.unwrap().unwrap();
        let ids: Vec<&str> = favorites
            .items
            .iter()
            .map(|one| one.item_id.as_str())
            .collect();
        assert_eq!(ids, vec!["romm:4014", "org.example.App"]);
        assert_eq!(favorites.items[0].name, "Zoop (USA)");

        assert!(store.remove_item("favorites", "romm:4014").await.unwrap());
        assert!(!store.remove_item("favorites", "romm:4014").await.unwrap());
        let favorites = store.collection("favorites").await.unwrap().unwrap();
        assert_eq!(favorites.items.len(), 1);
    }

    #[tokio::test]
    async fn derived_collections_refuse_curated_items_and_unknown_ones_do_not_exist() {
        let store = store().await;

        assert_eq!(store.accepts_items("favorites").await.unwrap(), Some(true));
        // A rule collection is read-only: its contents are recomputed on read.
        assert_eq!(
            store.accepts_items("last-played").await.unwrap(),
            Some(false)
        );
        assert_eq!(store.accepts_items("nope").await.unwrap(), None);
    }
}
