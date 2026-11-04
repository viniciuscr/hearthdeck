use futures_util::{StreamExt, stream};
use hearthdeck_ai::categorization::{Taxonomy, rail_app_ids};
use tracing::debug;

use super::{Collection, CollectionItem};
use crate::{
    activity::{ActivityStore, RecentActivity},
    catalog::CatalogStore,
    categorizer::CategorizationService,
    discovery::providers::stremio_arrivals,
    romm,
    settings::SettingsRepository,
    stremio,
};

pub(crate) const ENTRY_ID_PREFIX: &str = "hearthdeck:";
const ITEM_LIMIT: u16 = 20;
const ITEM_ICON_CONCURRENCY: usize = 8;

pub(crate) struct RuleService<'a> {
    pub catalog: &'a CatalogStore,
    pub activity: &'a ActivityStore,
    pub categorization: Option<&'a CategorizationService>,
    pub settings: &'a SettingsRepository,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RuleError {
    #[error("could not read collection catalog: {0}")]
    Catalog(#[from] sqlx::Error),
    #[error("could not read collection activity: {0:#}")]
    Activity(#[source] anyhow::Error),
    #[error("could not read categorization report: {0:#}")]
    Categorization(#[source] anyhow::Error),
}

impl RuleService<'_> {
    pub async fn resolve(
        &self,
        collection: &Collection,
    ) -> Result<Option<Vec<CollectionItem>>, RuleError> {
        let items = match (collection.owner.as_str(), collection.rule.as_deref()) {
            (super::OWNER_LAYA, _) => self.modeled_items(collection).await?,
            (super::OWNER_STREMIO, Some(super::RULE_CONTINUE_WATCHING)) => {
                self.continue_watching_items().await?
            }
            (super::OWNER_STREMIO, Some(super::RULE_NEW_ARRIVALS)) => {
                self.new_arrival_items().await?
            }
            (super::OWNER_SYSTEM, Some(super::RULE_LAST_PLAYED)) => {
                self.last_played_items().await?
            }
            _ => return Ok(None),
        };
        Ok(Some(items))
    }

    async fn modeled_items(
        &self,
        collection: &Collection,
    ) -> Result<Vec<CollectionItem>, RuleError> {
        let Some(rail_id) = collection
            .rule
            .as_deref()
            .and_then(|rule| super::owned_rule_id(super::OWNER_LAYA, rule))
        else {
            return Ok(Vec::new());
        };
        let Some(categorization) = self.categorization else {
            return Ok(Vec::new());
        };
        let Some(report) = categorization
            .latest_report()
            .await
            .map_err(RuleError::Categorization)?
        else {
            return Ok(Vec::new());
        };
        let mut items = Vec::new();
        for app_id in rail_app_ids(&report, &Taxonomy::baseline(), rail_id) {
            let (name, icon) = match self.catalog.get(&app_id).await {
                Ok(Some(item)) => (item.title, item.icon),
                Ok(None) => (String::new(), None),
                Err(error) => {
                    debug!(%error, app_id, "could not resolve modeled rail item");
                    (String::new(), None)
                }
            };
            items.push(CollectionItem {
                item_id: format!("{ENTRY_ID_PREFIX}{app_id}"),
                name,
                icon,
                background: None,
                logo: None,
                description: None,
                played: None,
            });
        }
        Ok(items)
    }

    async fn last_played_items(&self) -> Result<Vec<CollectionItem>, RuleError> {
        Ok(self
            .activity
            .recent_games(u32::from(ITEM_LIMIT))
            .await
            .map_err(RuleError::Activity)?
            .into_iter()
            .map(played_collection_item)
            .collect())
    }

    async fn continue_watching_items(&self) -> Result<Vec<CollectionItem>, RuleError> {
        let records = self
            .catalog
            .list_source(stremio::SOURCE_ID, usize::from(ITEM_LIMIT))
            .await?;
        debug!(
            source_id = stremio::SOURCE_ID,
            items = records.len(),
            "continue watching rail composed"
        );
        Ok(records
            .into_iter()
            .filter(|record| matches!(record.kind.as_str(), "movie" | "series"))
            .map(|record| CollectionItem {
                item_id: format!("{ENTRY_ID_PREFIX}{}", record.id),
                name: record.title,
                icon: record.icon,
                background: None,
                logo: None,
                description: None,
                played: None,
            })
            .collect())
    }

    async fn new_arrival_items(&self) -> Result<Vec<CollectionItem>, RuleError> {
        let records = self
            .catalog
            .list_source(stremio_arrivals::SOURCE_ID, usize::from(ITEM_LIMIT))
            .await?;
        Ok(records
            .into_iter()
            .map(|record| {
                let art = |key: &str| {
                    record
                        .metadata
                        .get("discovery")
                        .and_then(|metadata| metadata.get(key))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                };
                CollectionItem {
                    item_id: format!("{ENTRY_ID_PREFIX}{}", record.id),
                    name: record.title,
                    icon: record.icon,
                    background: art("background"),
                    logo: art("logo"),
                    description: art("description"),
                    played: None,
                }
            })
            .collect())
    }

    pub async fn with_item_icons(&self, items: Vec<CollectionItem>) -> Vec<CollectionItem> {
        stream::iter(items)
            .map(|mut item| async move {
                if item.icon.is_none() {
                    item.icon = self.stored_item_icon(&item.item_id).await;
                }
                item
            })
            .buffered(ITEM_ICON_CONCURRENCY)
            .collect()
            .await
    }

    async fn stored_item_icon(&self, item_id: &str) -> Option<String> {
        if let Some(rom_id) = rom_id_from_item(item_id) {
            return match romm::rom(self.settings, rom_id).await {
                Ok(game) => hearthdeck_protocol::RetroGame::from(&game).cover_path,
                Err(error) => {
                    debug!(?error, rom_id, "could not resolve a rail item's RomM cover");
                    None
                }
            };
        }
        let catalog_id = catalog_id_from_item(item_id)?;
        match self.catalog.get(catalog_id).await {
            Ok(item) => item.and_then(|item| item.icon),
            Err(error) => {
                debug!(
                    ?error,
                    item_id, "could not resolve a rail item's catalog icon"
                );
                None
            }
        }
    }
}

fn played_collection_item(play: RecentActivity) -> CollectionItem {
    CollectionItem {
        item_id: play.entry.id.clone(),
        name: play.entry.title.clone(),
        icon: play.entry.icon.clone(),
        background: None,
        logo: None,
        description: None,
        played: Some(play),
    }
}

pub(crate) fn catalog_id_from_item(item_id: &str) -> Option<&str> {
    item_id
        .strip_prefix(ENTRY_ID_PREFIX)
        .filter(|id| !id.is_empty())
}

pub(crate) fn rom_id_from_item(item_id: &str) -> Option<i64> {
    item_id.strip_prefix("romm:")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{activity::ActivityEntry, database::Database, settings::SettingsRepository};

    #[test]
    fn a_resolved_play_rides_along_with_its_collection_item() {
        let item = played_collection_item(RecentActivity {
            entry: ActivityEntry {
                id: "romm:7".into(),
                title: "Chrono Trigger".into(),
                icon: Some("/covers/chrono.png".into()),
                categories: vec!["Game".into()],
                kind: Some("game".into()),
                source: "romm".into(),
                metadata: serde_json::json!({"release_year": 1995}),
            },
            last_launched_at: "2026-09-23T10:00:00Z".into(),
            launch_count: 3,
        });
        assert_eq!(item.item_id, "romm:7");
        assert_eq!(item.name, "Chrono Trigger");
        let played = item.played.expect("the play travels with the item");
        assert_eq!(played.entry.kind.as_deref(), Some("game"));
        assert_eq!(played.entry.source, "romm");
        assert_eq!(played.launch_count, 3);
    }

    #[tokio::test]
    async fn unknown_rules_and_curated_collections_are_not_replaced() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("rules.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let catalog = CatalogStore::new(database.pool().clone());
        let activity = ActivityStore::new(database.pool().clone());
        let store = super::super::CollectionStore::new(database.pool().clone());
        let settings = SettingsRepository::new(database.pool().clone());
        let service = RuleService {
            catalog: &catalog,
            activity: &activity,
            categorization: None,
            settings: &settings,
        };
        let mut collection = store.collection("favorites").await.unwrap().unwrap();
        assert!(service.resolve(&collection).await.unwrap().is_none());
        collection.rule = Some(super::super::RULE_CONTINUE_WATCHING.into());
        assert!(
            service.resolve(&collection).await.unwrap().is_none(),
            "owner controls rule dispatch"
        );
        collection.owner = super::super::OWNER_STREMIO.into();
        collection.rule = Some("unknown".into());
        assert!(service.resolve(&collection).await.unwrap().is_none());
        collection.owner = super::super::OWNER_LAYA.into();
        collection.rule = Some("laya:watch".into());
        assert!(
            service
                .resolve(&collection)
                .await
                .unwrap()
                .unwrap()
                .is_empty(),
            "a disabled categorizer contributes no items"
        );
    }

    #[tokio::test]
    async fn catalog_failures_keep_their_typed_source() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("rules.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let catalog = CatalogStore::new(database.pool().clone());
        let activity = ActivityStore::new(database.pool().clone());
        let store = super::super::CollectionStore::new(database.pool().clone());
        let settings = SettingsRepository::new(database.pool().clone());
        let collection = store
            .collection(super::super::RULE_NEW_ARRIVALS)
            .await
            .unwrap()
            .unwrap();
        let last_played = store.collection("last-played").await.unwrap().unwrap();
        database.pool().close().await;
        let service = RuleService {
            catalog: &catalog,
            activity: &activity,
            categorization: None,
            settings: &settings,
        };
        assert!(matches!(
            service.resolve(&collection).await,
            Err(RuleError::Catalog(sqlx::Error::PoolClosed))
        ));
        assert!(matches!(
            service.resolve(&last_played).await,
            Err(RuleError::Activity(error)) if matches!(error.downcast_ref::<sqlx::Error>(), Some(sqlx::Error::PoolClosed))
        ));
    }
}
