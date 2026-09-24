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

use crate::activity::RecentActivity;

/// Items are curated by a user or a service; they live in `collection_items`.
pub const KIND_MEMBERSHIP: &str = "membership";
/// Items are derived from other data on every read; the table holds no rows for
/// these collections.
pub const KIND_RULE: &str = "rule";

/// A rule collection holding the most recently launched items.
pub const RULE_LAST_PLAYED: &str = "last_played";

/// Nobody edits these but the daemon: `last-played`, and any rule the build
/// itself knows how to resolve.
pub const OWNER_SYSTEM: &str = "system";
/// A person's own collections. Only a person adds to or removes from these.
pub const OWNER_USER: &str = "user";
/// Collections an optional feature owns, such as the categorization model.
///
/// One owner per feature, so a feature writes only under its own name and can
/// neither reach a person's collections nor the built-in ones. Turning the
/// feature off removes its collections and leaves everything else alone.
pub const OWNER_LAYA: &str = "laya";

/// The rule a feature's rail is resolved by, e.g. `laya:watch`. Namespaced by
/// owner so a rule can never be mistaken for the daemon's own.
pub fn owned_rule(owner: &str, rule_id: &str) -> String {
    format!("{owner}:{rule_id}")
}

/// The rule id inside an owner's rule, or `None` when the rule belongs to
/// somebody else.
pub fn owned_rule_id<'a>(owner: &str, rule: &'a str) -> Option<&'a str> {
    rule.strip_prefix(owner)?.strip_prefix(':')
}

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
    /// Who may edit it: [`OWNER_SYSTEM`], [`OWNER_USER`], or the feature that
    /// owns it.
    pub owner: String,
    /// The label to show when the client does not know the slug, which a feature's
    /// own categories need: nobody has translated a rail that did not exist when
    /// the client shipped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub items: Vec<CollectionItem>,
}

/// A collection as the thing that owns it defines it: what to write on a scan,
/// with no knowledge of what that scan found.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedCollection {
    pub slug: String,
    pub name: Option<String>,
    /// How the members are derived, e.g. `laya:watch`.
    pub rule: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CollectionItem {
    pub item_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The play a rule derived this item from, in the same shape the activity
    /// endpoint serves. A client draws, classifies and launches it with the code
    /// it already has for a play, so a rail built from a collection needs no
    /// second lookup and no knowledge of where the rule's data came from.
    ///
    /// Absent for a curated item, which has only what it was added with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub played: Option<RecentActivity>,
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
            "SELECT slug, kind, rule, role, position, owner, name FROM collections ORDER BY position, slug",
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
                    played: None,
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
                    owner: row.get("owner"),
                    name: row.get("name"),
                    items,
                }
            })
            .collect())
    }

    pub async fn collection(&self, slug: &str) -> Result<Option<Collection>> {
        Ok(self.list().await?.into_iter().find(|one| one.slug == slug))
    }

    /// Whether `slug` exists and a person may add items to it. `None` is an
    /// unknown collection; `Some(false)` is one nobody but its owner edits — a
    /// built-in rule or a feature's own rail, where a curated item would be lost
    /// on the next scan.
    pub async fn accepts_items(&self, slug: &str) -> Result<Option<bool>> {
        let row = sqlx::query("SELECT kind, owner FROM collections WHERE slug = ?")
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .context("read collection owner")?;
        // Both halves matter: the owner says who may edit it, the kind says
        // whether its members are stored at all. A derived collection a person
        // owns is the one combination that would silently lose what was added.
        Ok(row.map(|row| {
            row.get::<String, _>("kind") == KIND_MEMBERSHIP
                && row.get::<String, _>("owner") == OWNER_USER
        }))
    }

    /// Replaces everything `owner` owns with `replace_with`, in one transaction.
    ///
    /// The only way a feature writes here, and the reason it can be given the
    /// power to create and delete rails: it can reach its own rows and nothing
    /// else, so a person's collections and the built-in ones are not its to lose.
    /// Replacement rather than merging is what makes a rail that no longer
    /// applies disappear on its own.
    pub async fn replace_owned(&self, owner: &str, replace_with: &[OwnedCollection]) -> Result<()> {
        // Far enough behind any hand-ordered collection to sort after them, and
        // spaced so the owner's own order survives a rebuild.
        const FIRST_POSITION: i64 = 100;

        let mut transaction = self
            .pool
            .begin()
            .await
            .context("begin collection replacement")?;
        sqlx::query("DELETE FROM collections WHERE owner = ?")
            .bind(owner)
            .execute(&mut *transaction)
            .await
            .context("clear owned collections")?;
        for (index, collection) in replace_with.iter().enumerate() {
            sqlx::query(
                r#"
                INSERT INTO collections (slug, kind, rule, role, position, created_at, owner, name)
                VALUES (?, ?, ?, 'none', ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?, ?)
                "#,
            )
            .bind(&collection.slug)
            .bind(KIND_RULE)
            .bind(&collection.rule)
            .bind(FIRST_POSITION + index as i64)
            .bind(owner)
            .bind(&collection.name)
            .execute(&mut *transaction)
            .await
            .with_context(|| format!("write owned collection {}", collection.slug))?;
        }
        transaction
            .commit()
            .await
            .context("commit collection replacement")?;
        Ok(())
    }

    /// Removes everything `owner` owns, which is what turning a feature off does.
    pub async fn remove_owned(&self, owner: &str) -> Result<u64> {
        let result = sqlx::query("DELETE FROM collections WHERE owner = ?")
            .bind(owner)
            .execute(&self.pool)
            .await
            .context("remove owned collections")?;
        Ok(result.rows_affected())
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

    use super::{
        CollectionItem, CollectionStore, KIND_MEMBERSHIP, KIND_RULE, OWNER_LAYA, OWNER_SYSTEM,
        OwnedCollection, RULE_LAST_PLAYED,
    };
    use crate::database::Database;

    /// Returns the store along with the temporary directory its database lives
    /// in. `SqlitePool` opens connections lazily, so the directory has to outlive
    /// the store: drop it here and the next connection finds no database file.
    async fn store() -> (CollectionStore, tempfile::TempDir) {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        (CollectionStore::new(database.pool().clone()), directory)
    }

    fn item(id: &str, name: &str) -> CollectionItem {
        CollectionItem {
            item_id: id.to_owned(),
            name: name.to_owned(),
            icon: None,
            played: None,
        }
    }

    #[tokio::test]
    async fn the_built_in_collections_are_seeded_in_composition_order() {
        let (store, _directory) = store().await;
        let collections = store.list().await.unwrap();
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
        let (store, _directory) = store().await;
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
    async fn a_feature_replaces_its_own_collections_and_nothing_else() {
        let (store, _directory) = store().await;
        let rails = |ids: &[&str]| -> Vec<OwnedCollection> {
            ids.iter()
                .map(|id| OwnedCollection {
                    slug: format!("{OWNER_LAYA}:{id}"),
                    name: Some((*id).to_owned()),
                    rule: format!("{OWNER_LAYA}:{id}"),
                })
                .collect()
        };

        store
            .replace_owned(OWNER_LAYA, &rails(&["watch", "listen"]))
            .await
            .unwrap();
        let collections = store.list().await.unwrap();
        let slugs: Vec<&str> = collections
            .iter()
            .map(|collection| collection.slug.as_str())
            .collect();
        assert!(slugs.contains(&"laya:watch"));
        assert!(slugs.contains(&"laya:listen"));
        // Another owner's rows are not this one's to lose.
        assert!(slugs.contains(&"last-played"));
        assert!(slugs.contains(&"favorites"));
        // ...and the owners came back right: the seeded system one, and the
        // feature's own.
        assert_eq!(
            collections
                .iter()
                .find(|collection| collection.slug == "last-played")
                .map(|collection| collection.owner.as_str()),
            Some(OWNER_SYSTEM)
        );
        assert_eq!(
            collections
                .iter()
                .find(|collection| collection.slug == "laya:watch")
                .map(|collection| collection.owner.as_str()),
            Some(OWNER_LAYA)
        );

        // A replacement, not a merge: the rail the feature no longer implies is
        // gone, which is how a rail disappears without anybody deleting it.
        store
            .replace_owned(OWNER_LAYA, &rails(&["watch"]))
            .await
            .unwrap();
        let slugs: Vec<String> = store
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|collection| collection.slug)
            .collect();
        assert!(slugs.contains(&"laya:watch".to_owned()));
        assert!(!slugs.contains(&"laya:listen".to_owned()));
        assert!(slugs.contains(&"last-played".to_owned()));

        // A feature's rail is derived, so a curated item added to it would be
        // lost on the next scan; the store refuses instead.
        assert_eq!(
            store.accepts_items("laya:watch").await.unwrap(),
            Some(false)
        );

        // Turning the feature off takes its rails and leaves the rest.
        assert_eq!(store.remove_owned(OWNER_LAYA).await.unwrap(), 1);
        let slugs: Vec<String> = store
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|collection| collection.slug)
            .collect();
        assert!(!slugs.contains(&"laya:watch".to_owned()));
        assert!(slugs.contains(&"favorites".to_owned()));
    }

    #[tokio::test]
    async fn derived_collections_refuse_curated_items_and_unknown_ones_do_not_exist() {
        let (store, _directory) = store().await;

        assert_eq!(store.accepts_items("favorites").await.unwrap(), Some(true));
        // A rule collection is read-only: its contents are recomputed on read.
        assert_eq!(
            store.accepts_items("last-played").await.unwrap(),
            Some(false)
        );
        assert_eq!(store.accepts_items("nope").await.unwrap(), None);
    }
}
