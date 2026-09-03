use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use serde_json::Value;

use super::{GameProvider, GameRecord};

const LUTRIS_SOURCE: &str = "lutris";

/// Discovers games installed through Lutris.
pub struct LutrisProvider {
    config_directory: PathBuf,
}

impl LutrisProvider {
    pub fn from_system() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|path| path.join(".config")));

        let config_directory = config_home
            .map(|path| path.join("lutris"))
            .unwrap_or_else(|| PathBuf::from("/etc/xdg/lutris"));

        Self { config_directory }
    }
}

#[async_trait]
impl GameProvider for LutrisProvider {
    fn source_id(&self) -> &'static str {
        LUTRIS_SOURCE
    }

    fn refresh_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(5 * 60)) // 5 minute refresh
    }

    async fn discover(&self) -> anyhow::Result<Vec<GameRecord>> {
        let mut records = Vec::new();

        // Check if lutris config directory exists
        if !self.config_directory.exists() {
            return Ok(records);
        }

        // Look for games in the games directory
        let games_dir = self.config_directory.join("games");
        if games_dir.exists() {
            for entry in std::fs::read_dir(&games_dir)? {
                if let Ok(entry) = entry
                    && let Some(name) = entry.file_name().to_str()
                {
                    // Look for game metadata
                    let game_config = games_dir.join(name).join("game.json");
                    if game_config.exists()
                        && let Ok(contents) = std::fs::read_to_string(&game_config)
                        && let Ok(game_data) = serde_json::from_str::<Value>(&contents)
                    {
                        // Extract relevant information from the game config
                        let name = game_data
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or(name)
                            .to_string();

                        let slug = game_data
                            .get("slug")
                            .and_then(Value::as_str)
                            .unwrap_or(name.as_str())
                            .to_string();

                        let runner = game_data
                            .get("runner")
                            .and_then(Value::as_str)
                            .map(|r| r.to_string());

                        let exe = game_data
                            .get("exe")
                            .and_then(Value::as_str)
                            .map(|e| e.to_string());

                        let icon = game_data
                            .get("icon")
                            .and_then(Value::as_str)
                            .map(|i| i.to_string());

                        // Try to find a valid executable
                        let exec = exe;

                        records.push(GameRecord {
                            id: format!("lutris:{slug}"),
                            name,
                            exec,
                            icon,
                            path: None, // Lutris uses runner-based launching
                            categories: vec!["Game".to_string()], // Default to game category
                            terminal: false,
                            prefers_dgpu: false,
                            source: LUTRIS_SOURCE.to_owned(),
                            metadata: serde_json::json!({
                                "runner": runner,
                                "type": "lutris_game",
                            }),
                        });
                    }
                }
            }
        }

        Ok(records)
    }
}
