//! What the categorizer has learned about an item that the catalog does not carry
//! on its own.
//!
//! The catalog is rewritten by every discovery refresh, so a verdict kept beside
//! the record would be lost with it. Verdicts live here instead, keyed by item id,
//! and are laid over the record when it is served. Each carries the hash of the
//! input it was made from and the model that made it, which is what lets a later
//! pass ask only about what changed.

use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::Utc;
use hearthdeck_ai::categorization::AppProfile;
use hearthdeck_ai::categorization::prompt::PROMPT_VERSION;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

/// The trait that says whether an application suits a couch or a desktop.
pub const FORM_FACTOR: &str = "form_factor";
pub const COUCH: &str = "couch";
pub const DESKTOP: &str = "desktop";

/// How sure the model has to be before an application is called couch-friendly.
/// Above the engine's own 0.5 yes/no line on purpose: a wrong "couch" puts a
/// desktop tool on the front page, while a missed one only leaves it a tab away.
const COUCH_CONFIDENCE: f64 = 0.7;

/// What a model's probability that an application suits the couch comes to.
#[must_use]
pub fn form_factor_for(probability: f64) -> &'static str {
    if probability >= COUCH_CONFIDENCE {
        COUCH
    } else {
        DESKTOP
    }
}

/// Which questions and which engine a verdict was made under. A verdict from
/// another stamp is stale: asked again, never trusted.
#[must_use]
pub fn stamp(categorizer: &str) -> String {
    format!("{categorizer}:{PROMPT_VERSION}")
}

/// Everything a decision read about an application, as one hash. When it
/// changes, the verdict made from the old one no longer describes the record.
#[must_use]
pub fn input_hash(profile: &AppProfile) -> String {
    let bytes = serde_json::to_vec(profile).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

/// One application's form-factor verdict, ready to be stored.
pub struct FormFactorVerdict {
    pub item_id: String,
    pub value: &'static str,
    pub confidence: f64,
    pub input_hash: String,
}

#[derive(Clone)]
pub struct TraitStore {
    pool: SqlitePool,
}

impl TraitStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The input hash each item's verdict under `stamp` was made from.
    pub async fn current(&self, stamp: &str) -> Result<HashMap<String, String>> {
        let rows = sqlx::query(
            "SELECT item_id, input_hash FROM item_traits WHERE trait = ? AND model = ?",
        )
        .bind(FORM_FACTOR)
        .bind(stamp)
        .fetch_all(&self.pool)
        .await
        .context("read the current form-factor verdicts")?;
        Ok(rows
            .into_iter()
            .map(|row| (row.get("item_id"), row.get("input_hash")))
            .collect())
    }

    /// Stores the verdicts, replacing an item's earlier one, in one transaction.
    pub async fn write_form_factors(
        &self,
        stamp: &str,
        verdicts: &[FormFactorVerdict],
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await.context("begin verdict write")?;
        for verdict in verdicts {
            sqlx::query(
                r#"
                INSERT INTO item_traits (item_id, trait, value, confidence, input_hash, model, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(item_id, trait) DO UPDATE SET
                  value = excluded.value,
                  confidence = excluded.confidence,
                  input_hash = excluded.input_hash,
                  model = excluded.model,
                  updated_at = excluded.updated_at
                "#,
            )
            .bind(&verdict.item_id)
            .bind(FORM_FACTOR)
            .bind(verdict.value)
            .bind(verdict.confidence)
            .bind(&verdict.input_hash)
            .bind(stamp)
            .bind(Utc::now().to_rfc3339())
            .execute(&mut *transaction)
            .await
            .with_context(|| format!("write the verdict for {}", verdict.item_id))?;
        }
        transaction.commit().await.context("commit verdict write")?;
        Ok(())
    }

    /// Drops the verdicts of items the library no longer holds.
    pub async fn prune(&self) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM item_traits WHERE item_id NOT IN (SELECT id FROM library_items)",
        )
        .execute(&self.pool)
        .await
        .context("prune verdicts of removed items")?;
        Ok(result.rows_affected())
    }

    /// Forgets everything the model decided, which is what turning it off does.
    pub async fn clear(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM item_traits")
            .execute(&self.pool)
            .await
            .context("clear the verdicts")?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use hearthdeck_ai::categorization::AppProfile;
    use tempfile::tempdir;

    use super::{
        COUCH, DESKTOP, FormFactorVerdict, TraitStore, form_factor_for, input_hash, stamp,
    };
    use crate::catalog::{CatalogRecord, CatalogStore};
    use crate::database::Database;

    async fn stores() -> (TraitStore, CatalogStore, tempfile::TempDir) {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        (
            TraitStore::new(database.pool().clone()),
            CatalogStore::new(database.pool().clone()),
            directory,
        )
    }

    fn verdict(id: &str, value: &'static str) -> FormFactorVerdict {
        FormFactorVerdict {
            item_id: id.to_owned(),
            value,
            confidence: 0.9,
            input_hash: format!("hash-{id}"),
        }
    }

    #[test]
    fn only_a_confident_yes_is_couch() {
        assert_eq!(form_factor_for(0.95), COUCH);
        assert_eq!(form_factor_for(0.7), COUCH);
        assert_eq!(form_factor_for(0.6), DESKTOP);
    }

    #[test]
    fn a_change_in_what_was_read_changes_the_hash() {
        let mut app = AppProfile::new("kodi.desktop", "Kodi");
        let before = input_hash(&app);
        assert_eq!(before, input_hash(&app.clone()));

        app.description = Some("A media centre".to_owned());

        assert_ne!(before, input_hash(&app));
        assert_ne!(stamp("laya"), stamp("heuristic"));
    }

    #[tokio::test]
    async fn a_verdict_is_current_only_under_the_stamp_that_made_it() {
        let (store, _catalog, _directory) = stores().await;
        store
            .write_form_factors("laya:2", &[verdict("kodi", COUCH)])
            .await
            .unwrap();

        assert_eq!(
            store.current("laya:2").await.unwrap().get("kodi"),
            Some(&"hash-kodi".to_owned())
        );
        assert!(store.current("laya:3").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn pruning_drops_only_the_verdicts_of_removed_items() {
        let (store, catalog, _directory) = stores().await;
        catalog
            .replace_source(
                "desktop",
                vec![CatalogRecord {
                    id: "kodi".into(),
                    title: "Kodi".into(),
                    kind: "application".into(),
                    launch_id: None,
                    icon: None,
                    metadata: serde_json::json!({}),
                    updated_at: "2026-10-19T00:00:00Z".into(),
                }],
            )
            .await
            .unwrap();
        store
            .write_form_factors(
                "laya:2",
                &[verdict("kodi", COUCH), verdict("gone", DESKTOP)],
            )
            .await
            .unwrap();

        assert_eq!(store.prune().await.unwrap(), 1);

        let kept = store.current("laya:2").await.unwrap();
        assert!(kept.contains_key("kodi") && !kept.contains_key("gone"));
        assert_eq!(store.clear().await.unwrap(), 1);
    }
}
