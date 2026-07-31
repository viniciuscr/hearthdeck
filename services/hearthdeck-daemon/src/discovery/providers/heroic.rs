use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(target_os = "linux")]
use std::{collections::HashSet, env};

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::{catalog::CatalogRecord, discovery::DiscoveryProvider};

const HEROIC_SOURCE: &str = "heroic";

/// Discovers games installed through Heroic without reading any account or
/// authentication data. Heroic remains responsible for runner configuration
/// and launches the game through its URI handler.
pub struct HeroicInstalledProvider {
    config_directories: Vec<PathBuf>,
}

impl HeroicInstalledProvider {
    #[cfg(target_os = "linux")]
    pub fn from_system() -> Self {
        Self {
            config_directories: heroic_config_directories(),
        }
    }

    #[cfg(test)]
    fn with_config_directories(config_directories: Vec<PathBuf>) -> Self {
        Self { config_directories }
    }
}

#[async_trait]
impl DiscoveryProvider for HeroicInstalledProvider {
    fn source_id(&self) -> &'static str {
        HEROIC_SOURCE
    }

    fn refresh_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(5 * 60))
    }

    async fn discover(&self) -> anyhow::Result<Vec<CatalogRecord>> {
        let updated_at = Utc::now().to_rfc3339();
        let mut records = HashMap::new();
        for directory in &self.config_directories {
            for record in discover_directory(directory, &updated_at).await? {
                records.entry(record.id.clone()).or_insert(record);
            }
        }
        let mut records: Vec<_> = records.into_values().collect();
        records.sort_by(|left, right| left.title.cmp(&right.title));
        Ok(records)
    }
}

async fn discover_directory(
    directory: &Path,
    updated_at: &str,
) -> anyhow::Result<Vec<CatalogRecord>> {
    let mut records = discover_epic(directory, updated_at).await?;
    records.extend(discover_gog(directory, updated_at).await?);
    Ok(records)
}

async fn discover_epic(directory: &Path, updated_at: &str) -> anyhow::Result<Vec<CatalogRecord>> {
    let legendary_directory = directory.join("legendaryConfig/legendary");
    let Some(installed) = read_json_if_exists(&legendary_directory.join("installed.json")).await?
    else {
        return Ok(Vec::new());
    };
    let Some(installed) = installed.as_object() else {
        anyhow::bail!("Heroic Epic installed.json must contain an object")
    };

    let mut records = Vec::new();
    for (key, install) in installed {
        if install.get("is_dlc").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let application_id = string_at(install, &["app_name"]).unwrap_or_else(|| key.to_owned());
        if !valid_application_id(&application_id) {
            continue;
        }
        let metadata = read_json_if_exists(
            &legendary_directory
                .join("metadata")
                .join(format!("{application_id}.json")),
        )
        .await?
        .unwrap_or(Value::Null);
        let catalog_metadata = metadata.get("metadata").unwrap_or(&metadata);
        let title = string_at(catalog_metadata, &["title"])
            .or_else(|| string_at(&metadata, &["app_title"]))
            .or_else(|| string_at(install, &["title"]))
            .unwrap_or_else(|| application_id.clone());
        let description = string_at(catalog_metadata, &["description"])
            .or_else(|| string_at(catalog_metadata, &["shortDescription"]));
        let developer = string_at(catalog_metadata, &["developer"]);
        let categories = epic_categories(catalog_metadata);
        let artwork = epic_artwork(catalog_metadata);
        records.push(heroic_record(
            "epic",
            "legendary",
            application_id,
            title,
            description,
            developer,
            categories,
            artwork,
            string_at(install, &["version"]),
            string_at(install, &["platform"]),
            install.get("install_size").and_then(Value::as_u64),
            catalog_metadata
                .get("customAttributes")
                .and_then(|attributes| attributes.get("CloudSaveFolder"))
                .and_then(|attribute| attribute.get("value"))
                .and_then(Value::as_str)
                .is_some(),
            updated_at,
        ));
    }
    Ok(records)
}

async fn discover_gog(directory: &Path, updated_at: &str) -> anyhow::Result<Vec<CatalogRecord>> {
    let Some(installed) = read_json_if_exists(&directory.join("gog_store/installed.json")).await?
    else {
        return Ok(Vec::new());
    };
    let Some(installed) = installed.get("installed").and_then(Value::as_array) else {
        anyhow::bail!("Heroic GOG installed.json must contain an installed array")
    };
    let library = gog_library(&directory.join("store_cache/gog_library.json")).await?;

    let mut records = Vec::new();
    for install in installed {
        if install.get("is_dlc").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(application_id) = string_at(install, &["appName"]) else {
            continue;
        };
        if !valid_application_id(&application_id) {
            continue;
        }
        let cached = library.get(&application_id);
        let info = match string_at(install, &["install_path"]) {
            Some(path) => {
                read_json_if_exists(
                    &PathBuf::from(path).join(format!("goggame-{application_id}.info")),
                )
                .await?
            }
            None => None,
        }
        .unwrap_or(Value::Null);
        let title = cached
            .and_then(|game| string_at(game, &["title"]))
            .or_else(|| string_at(&info, &["name"]))
            .unwrap_or_else(|| application_id.clone());
        let description = cached.and_then(|game| {
            string_at(game, &["extra", "about", "description"])
                .or_else(|| string_at(game, &["extra", "about", "shortDescription"]))
        });
        let developer = cached.and_then(|game| string_at(game, &["developer"]));
        let categories = cached
            .map(|game| string_list_at(game, &["extra", "genres"]))
            .filter(|categories| !categories.is_empty())
            .unwrap_or_else(|| vec!["Game".to_owned()]);
        let artwork = cached.and_then(|game| {
            string_at(game, &["art_square"])
                .or_else(|| string_at(game, &["art_cover"]))
                .or_else(|| string_at(game, &["art_icon"]))
        });
        records.push(heroic_record(
            "gog",
            "gog",
            application_id,
            title,
            description,
            developer,
            categories,
            artwork,
            string_at(install, &["version"]),
            string_at(install, &["platform"]),
            install.get("install_size").and_then(Value::as_u64),
            cached
                .and_then(|game| game.get("cloud_save_enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            updated_at,
        ));
    }
    Ok(records)
}

#[allow(clippy::too_many_arguments)]
fn heroic_record(
    store_id: &str,
    runner: &str,
    application_id: String,
    title: String,
    description: Option<String>,
    developer: Option<String>,
    categories: Vec<String>,
    artwork: Option<String>,
    version: Option<String>,
    platform: Option<String>,
    install_size_bytes: Option<u64>,
    cloud_saves: bool,
    updated_at: &str,
) -> CatalogRecord {
    let store = match store_id {
        "epic" => "Epic Games",
        "gog" => "GOG",
        _ => store_id,
    };
    CatalogRecord {
        id: format!("heroic:{store_id}:{application_id}"),
        title,
        kind: "game".to_owned(),
        launch_id: Some(format!("{runner}:{application_id}")),
        icon: artwork,
        metadata: serde_json::json!({
            "description": description,
            "developer": developer,
            "categories": categories,
            "store": store,
            "runner": runner,
            "version": version,
            "platform": platform,
            "install_size_bytes": install_size_bytes,
            "cloud_saves": cloud_saves,
            "provenance": "heroic",
        }),
        updated_at: updated_at.to_owned(),
    }
}

async fn gog_library(path: &Path) -> anyhow::Result<HashMap<String, Value>> {
    let Some(cache) = read_json_if_exists(path).await? else {
        return Ok(HashMap::new());
    };
    let Some(games) = cache.get("games").and_then(Value::as_array) else {
        return Ok(HashMap::new());
    };
    Ok(games
        .iter()
        .filter_map(|game| string_at(game, &["app_name"]).map(|id| (id, game.clone())))
        .collect())
}

async fn read_json_if_exists(path: &Path) -> anyhow::Result<Option<Value>> {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => Ok(Some(serde_json::from_str(&contents).map_err(|error| {
            anyhow::anyhow!("could not parse Heroic data at {}: {error}", path.display())
        })?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    path.iter()
        .try_fold(value, |value, key| value.get(key))?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn string_list_at(value: &Value, path: &[&str]) -> Vec<String> {
    path.iter()
        .try_fold(value, |value, key| value.get(key))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn epic_categories(metadata: &Value) -> Vec<String> {
    let categories = metadata
        .get("categories")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|category| string_at(category, &["path"]))
        .collect::<Vec<_>>();
    if categories.is_empty() {
        vec!["Game".to_owned()]
    } else {
        categories
    }
}

fn epic_artwork(metadata: &Value) -> Option<String> {
    let images = metadata.get("keyImages")?.as_array()?;
    for kind in [
        "DieselGameBoxTall",
        "OfferImageTall",
        "DieselGameBox",
        "OfferImageWide",
    ] {
        if let Some(image) = images
            .iter()
            .find(|image| image.get("type").and_then(Value::as_str) == Some(kind))
            && let Some(url) = string_at(image, &["url"])
        {
            return Some(url);
        }
    }
    None
}

fn valid_application_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, b'.' | b'_' | b'-')
        })
}

#[cfg(target_os = "linux")]
fn heroic_config_directories() -> Vec<PathBuf> {
    let home = env::var_os("HOME").map(PathBuf::from);
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|path| path.join(".config")));
    let mut directories = Vec::new();
    let mut seen = HashSet::new();
    for directory in [
        config_home.map(|path| path.join("heroic")),
        home.as_ref()
            .map(|path| path.join(".var/app/com.heroicgameslauncher.hgl/config/heroic")),
        home.map(|path| path.join("snap/heroic/common/.config/heroic")),
    ]
    .into_iter()
    .flatten()
    {
        if seen.insert(directory.clone()) {
            directories.push(directory);
        }
    }
    directories
}

#[cfg(test)]
mod tests {
    use super::{DiscoveryProvider, HeroicInstalledProvider};

    #[tokio::test]
    async fn discovers_installed_epic_and_gog_games_without_authentication_data() {
        let directory = tempfile::tempdir().unwrap();
        let heroic = directory.path();
        tokio::fs::create_dir_all(heroic.join("legendaryConfig/legendary/metadata"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(heroic.join("gog_store"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(heroic.join("store_cache"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(heroic.join("games/cyberpunk"))
            .await
            .unwrap();
        tokio::fs::write(
            heroic.join("legendaryConfig/legendary/installed.json"),
            r#"{"Fortnite":{"app_name":"Fortnite","title":"Fallback title","version":"1.2.3","platform":"Windows","install_size":42,"is_dlc":false},"Dlc":{"app_name":"Dlc","is_dlc":true}}"#,
        )
        .await
        .unwrap();
        tokio::fs::write(
            heroic.join("legendaryConfig/legendary/metadata/Fortnite.json"),
            r#"{"app_title":"Fortnite","metadata":{"title":"Fortnite","description":"Battle royale","developer":"Epic Games","categories":[{"path":"games"}],"keyImages":[{"type":"DieselGameBoxTall","url":"https://example.test/fortnite.jpg"}],"customAttributes":{"CloudSaveFolder":{"value":"saves"}}}}"#,
        )
        .await
        .unwrap();
        tokio::fs::write(
            heroic.join("gog_store/installed.json"),
            r#"{"installed":[{"appName":"1091500","install_path":"REPLACE","version":"2.2","platform":"windows","install_size":123,"is_dlc":false}]}"#.replace("REPLACE", &heroic.join("games/cyberpunk").to_string_lossy()),
        )
        .await
        .unwrap();
        tokio::fs::write(
            heroic.join("games/cyberpunk/goggame-1091500.info"),
            r#"{"name":"Cyberpunk 2077"}"#,
        )
        .await
        .unwrap();
        tokio::fs::write(
            heroic.join("store_cache/gog_library.json"),
            r#"{"games":[{"app_name":"1091500","title":"Cyberpunk 2077","developer":"CD PROJEKT RED","art_square":"https://example.test/cyberpunk.jpg","cloud_save_enabled":true,"extra":{"about":{"description":"Night City","shortDescription":""},"genres":["RPG"]}}]}"#,
        )
        .await
        .unwrap();

        let records = HeroicInstalledProvider::with_config_directories(vec![heroic.to_owned()])
            .discover()
            .await
            .unwrap();

        assert_eq!(records.len(), 2);
        let epic = records
            .iter()
            .find(|record| record.id == "heroic:epic:Fortnite")
            .unwrap();
        assert_eq!(epic.launch_id.as_deref(), Some("legendary:Fortnite"));
        assert_eq!(epic.metadata["store"], "Epic Games");
        assert_eq!(epic.metadata["developer"], "Epic Games");
        assert_eq!(
            epic.icon.as_deref(),
            Some("https://example.test/fortnite.jpg")
        );
        let gog = records
            .iter()
            .find(|record| record.id == "heroic:gog:1091500")
            .unwrap();
        assert_eq!(gog.launch_id.as_deref(), Some("gog:1091500"));
        assert_eq!(gog.metadata["store"], "GOG");
        assert_eq!(gog.metadata["categories"], serde_json::json!(["RPG"]));
    }
}
