use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
use serde_json::Value;

use super::{GameProvider, GameRecord};

const BOTTLES_SOURCE: &str = "bottles";

/// Discovers games installed through Bottles (Steam-like wine runner).
pub struct BottlesProvider {
    config_directory: PathBuf,
}

impl BottlesProvider {
    pub fn from_system() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|path| path.join(".config")));

        let config_directory = config_home
            .map(|path| path.join("bottles"))
            .unwrap_or_else(|| PathBuf::from("/etc/xdg/bottles"));

        Self {
            config_directory,
        }
    }
}

#[async_trait]
impl GameProvider for BottlesProvider {
    fn source_id(&self) -> &'static str {
        BOTTLES_SOURCE
    }

    fn refresh_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(5 * 60)) // 5 minute refresh
    }

    async fn discover(&self) -> anyhow::Result<Vec<GameRecord>> {
        let mut records = Vec::new();

        // Check if bottles config directory exists
        if !self.config_directory.exists() {
            return Ok(records);
        }

        // Look for bottles configuration
        let bottles_dir = self.config_directory.join("bottles");
        if bottles_dir.exists() {
            for entry in std::fs::read_dir(&bottles_dir)? {
                if let Ok(entry) = entry {
                    if let Some(name) = entry.file_name().to_str() {
                        // Look for bottle configuration
                        let bottle_config = bottles_dir.join(name).join("bottle.json");
                        if bottle_config.exists() {
                            if let Ok(contents) = std::fs::read_to_string(&bottle_config) {
                                if let Ok(bottle_data) = serde_json::from_str::<Value>(&contents) {
                                    // Extract relevant information from the bottle config
                                    let name = bottle_data.get("name")
                                        .and_then(Value::as_str)
                                        .unwrap_or(name)
                                        .to_string();

                                    let runner = bottle_data.get("runner")
                                        .and_then(Value::as_str)
                                        .map(|r| r.to_string());

                                    // Bottles typically has a list of programs
                                    let programs = bottle_data.get("programs")
                                        .and_then(Value::as_array)
                                        .unwrap_or(&Vec::new());

                                    for program in programs {
                                        if let Some(program_name) = program.get("name").and_then(Value::as_str) {
                                            let exec = program.get("exec")
                                                .and_then(Value::as_str)
                                                .map(|e| e.to_string());

                                            let icon = program.get("icon")
                                                .and_then(Value::as_str)
                                                .map(|i| i.to_string());

                                            records.push(GameRecord {
                                                id: format!("bottles:{name}:{program_name}"),
                                                name: program_name.to_string(),
                                                exec,
                                                icon,
                                                path: None,  // Bottles uses wine-based launching
                                                categories: vec!["Game".to_string()],  // Default to game category
                                                terminal: false,
                                                prefers_dgpu: false,
                                                source: BOTTLES_SOURCE.to_owned(),
                                                metadata: serde_json::json!({
                                                    "bottle": name,
                                                    "runner": runner,
                                                    "type": "bottles_program",
                                                }),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(records)
    }
}
