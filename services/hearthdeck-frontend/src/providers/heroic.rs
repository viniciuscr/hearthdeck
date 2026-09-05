use std::{
    collections::HashMap,
    env,
    hash::{DefaultHasher, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
use serde_json::Value;

use super::{GameProvider, GameRecord};

const HEROIC_SOURCE: &str = "heroic";

/// Discovers games installed through Heroic Games Launcher without reading
/// any account or authentication data. Heroic remains responsible for runner
/// configuration and launches the game through its URI handler.
pub struct HeroicProvider {
    config_directories: Vec<PathBuf>,
}

impl HeroicProvider {
    pub fn from_system() -> Self {
        Self {
            config_directories: heroic_config_directories(),
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn with_config_directories(config_directories: Vec<PathBuf>) -> Self {
        Self { config_directories }
    }
}

#[async_trait]
impl GameProvider for HeroicProvider {
    fn source_id(&self) -> &'static str {
        HEROIC_SOURCE
    }

    fn refresh_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(5 * 60))
    }

    async fn discover(&self) -> anyhow::Result<Vec<GameRecord>> {
        let mut records = HashMap::new();
        for directory in &self.config_directories {
            for record in discover_directory(directory)? {
                records.entry(record.id.clone()).or_insert(record);
            }
        }
        let mut records: Vec<_> = records.into_values().collect();
        records.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(records)
    }
}

fn discover_directory(directory: &Path) -> anyhow::Result<Vec<GameRecord>> {
    let mut records = discover_epic(directory)?;
    records.extend(discover_gog(directory)?);
    Ok(records)
}

fn discover_epic(directory: &Path) -> anyhow::Result<Vec<GameRecord>> {
    let legendary_directory = directory.join("legendaryConfig/legendary");
    let Some(installed) = read_json_if_exists(&legendary_directory.join("installed.json"))? else {
        return Ok(Vec::new());
    };
    let Some(installed) = installed.as_object() else {
        anyhow::bail!("Heroic Epic installed.json must contain an object");
    };

    let _game_info = epic_game_info(&directory.join("store_cache/legendary_gameinfo.json"))?;
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
        )?
        .unwrap_or(Value::Null);
        let catalog_metadata = metadata.get("metadata").unwrap_or(&metadata);
        let title = string_at(catalog_metadata, &["title"])
            .or_else(|| string_at(&metadata, &["app_title"]))
            .or_else(|| string_at(install, &["title"]))
            .unwrap_or_else(|| application_id.clone());
        let mut categories = epic_categories(catalog_metadata);
        categories.push("Epic Games".to_owned());
        let artwork = epic_artwork(catalog_metadata).and_then(|url| cache_icon(&url));
        let version = string_at(install, &["version"]);
        let platform = string_at(install, &["platform"]);

        let mut metadata = serde_json::json!({
            "store": "Epic Games",
            "runner": "legendary",
        });
        if let Some(desc) = string_at(catalog_metadata, &["description"])
            .or_else(|| string_at(catalog_metadata, &["shortDescription"]))
        {
            metadata["description"] = serde_json::Value::String(desc);
        }
        if let Some(developer) = string_at(catalog_metadata, &["developer"]) {
            metadata["developer"] = serde_json::Value::String(developer);
        }
        if let Some(version) = version {
            metadata["version"] = serde_json::Value::String(version);
        }
        if let Some(platform) = platform {
            metadata["platform"] = serde_json::Value::String(platform);
        }
        if let Some(size) = install.get("install_size").and_then(Value::as_u64) {
            metadata["install_size_bytes"] = serde_json::json!(size);
        }

        records.push(GameRecord {
            id: format!("heroic:epic:{application_id}"),
            name: title,
            exec: Some(format!(
                "xdg-open heroic://launch/legendary/{application_id}"
            )),
            icon: artwork,
            path: None,
            categories,
            terminal: false,
            prefers_dgpu: false,
            source: HEROIC_SOURCE.to_owned(),
            metadata,
        });
    }
    Ok(records)
}

fn discover_gog(directory: &Path) -> anyhow::Result<Vec<GameRecord>> {
    let Some(installed) = read_json_if_exists(&directory.join("gog_store/installed.json"))? else {
        return Ok(Vec::new());
    };
    let Some(installed) = installed.get("installed").and_then(Value::as_array) else {
        anyhow::bail!("Heroic GOG installed.json must contain an installed array");
    };
    let library = gog_library(&directory.join("store_cache/gog_library.json"))?;

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
            Some(path) => read_json_if_exists(
                &PathBuf::from(path).join(format!("goggame-{application_id}.info")),
            )?,
            None => None,
        }
        .unwrap_or(Value::Null);
        let title = cached
            .and_then(|game| string_at(game, &["title"]))
            .or_else(|| string_at(&info, &["name"]))
            .unwrap_or_else(|| application_id.clone());
        let mut categories = cached
            .map(|game| string_list_at(game, &["extra", "genres"]))
            .filter(|categories| !categories.is_empty())
            .unwrap_or_default();
        // All GOG entries are games — ensure the "Game" category is present
        // so they appear in the PC Games section.
        if !categories.iter().any(|c| c.eq_ignore_ascii_case("game")) {
            categories.push("Game".to_owned());
        }
        categories.push("GOG".to_owned());
        let artwork = cached
            .and_then(|game| {
                string_at(game, &["art_square"])
                    .or_else(|| string_at(game, &["art_cover"]))
                    .or_else(|| string_at(game, &["art_icon"]))
            })
            .and_then(|url| cache_icon(&url));

        let mut metadata = serde_json::json!({
            "store": "GOG",
            "runner": "gog",
        });
        if let Some(desc) = cached.and_then(|game| {
            string_at(game, &["extra", "about", "description"])
                .or_else(|| string_at(game, &["extra", "about", "shortDescription"]))
        }) {
            metadata["description"] = serde_json::Value::String(desc);
        }
        if let Some(developer) = cached.and_then(|game| string_at(game, &["developer"])) {
            metadata["developer"] = serde_json::Value::String(developer);
        }
        if let Some(version) = string_at(install, &["version"]) {
            metadata["version"] = serde_json::Value::String(version);
        }
        if let Some(platform) = string_at(install, &["platform"]) {
            metadata["platform"] = serde_json::Value::String(platform);
        }
        if let Some(size) = install.get("install_size").and_then(Value::as_u64) {
            metadata["install_size_bytes"] = serde_json::json!(size);
        }

        records.push(GameRecord {
            id: format!("heroic:gog:{application_id}"),
            name: title,
            exec: Some(format!("xdg-open heroic://launch/gog/{application_id}")),
            icon: artwork,
            path: None,
            categories,
            terminal: false,
            prefers_dgpu: false,
            source: HEROIC_SOURCE.to_owned(),
            metadata,
        });
    }
    Ok(records)
}

fn heroic_config_directories() -> Vec<PathBuf> {
    let home = env::var_os("HOME").map(PathBuf::from);
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|path| path.join(".config")));
    let mut directories = Vec::new();
    let mut seen = std::collections::HashSet::new();
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

fn epic_game_info(path: &Path) -> anyhow::Result<serde_json::Map<String, Value>> {
    let Some(cache) = read_json_if_exists(path)? else {
        return Ok(serde_json::Map::new());
    };
    Ok(cache.as_object().cloned().unwrap_or_default())
}

fn epic_categories(metadata: &Value) -> Vec<String> {
    let categories: Vec<String> = metadata
        .get("categories")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|category| string_at(category, &["path"]))
        .map(|cat| {
            // Normalize Epic store categories to XDG desktop entry convention.
            if cat.eq_ignore_ascii_case("games") {
                "Game".to_owned()
            } else {
                cat
            }
        })
        .collect();
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

fn gog_library(path: &Path) -> anyhow::Result<HashMap<String, Value>> {
    let Some(cache) = read_json_if_exists(path)? else {
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

fn read_json_if_exists(path: &Path) -> anyhow::Result<Option<Value>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(Some(serde_json::from_str(&contents).map_err(|error| {
            anyhow::anyhow!("could not parse Heroic configuration data: {error}")
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

fn valid_application_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, b'.' | b'_' | b'-')
        })
}

/// Download a remote icon URL to a local cache directory and return the cached
/// file path. Returns `None` on failure (network error, etc.) so callers can
/// fall back to a generic icon.
pub(super) fn cache_icon(url: &str) -> Option<String> {
    let cache_dir = icon_cache_dir();
    let _ = std::fs::create_dir_all(&cache_dir);

    let ext = url::Url::parse(url)
        .ok()
        .and_then(|parsed| {
            Path::new(parsed.path())
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
        })
        .filter(|extension| matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "svg" | "webp"))
        .unwrap_or_else(|| "png".to_string());
    let mut hasher = DefaultHasher::new();
    hasher.write(url.as_bytes());
    let filename = format!("{:016x}.{ext}", hasher.finish());
    let cached = cache_dir.join(&filename);

    if cached.metadata().is_ok_and(|metadata| metadata.len() >= 16) {
        return Some(cached.to_string_lossy().into_owned());
    }

    log::debug!("caching icon from {url}");
    let resp = match ureq::get(url).call() {
        Ok(r) => r,
        Err(e) => {
            log::warn!("icon download failed for {url}: {e}");
            return None;
        }
    };
    let mut body = Vec::new();
    if let Err(e) = resp.into_body().into_reader().read_to_end(&mut body) {
        log::warn!("icon read failed for {url}: {e}");
        return None;
    }

    if body.len() < 16 {
        return None;
    }

    let mut file = match std::fs::File::create(&cached) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("icon cache write failed: {e}");
            return None;
        }
    };
    if let Err(e) = file.write_all(&body) {
        log::warn!("icon cache write failed: {e}");
        return None;
    }
    Some(cached.to_string_lossy().into_owned())
}

fn icon_cache_dir() -> PathBuf {
    let cache = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".cache"))
                .unwrap_or_else(|| PathBuf::from("/tmp"))
        });
    cache.join("hearthdeck").join("icons")
}
