use std::net::IpAddr;

use anyhow::{Result, anyhow, bail};
use chrono::Utc;
use serde::Serialize;
use sqlx::{Row, SqlitePool};

#[derive(Clone)]
pub struct SettingsRepository {
    pool: SqlitePool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    System,
    Aurora,
    Ember,
    Indigo,
    Noir,
}

impl ThemeMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "aurora" => Some(Self::Aurora),
            "ember" => Some(Self::Ember),
            "indigo" => Some(Self::Indigo),
            "noir" => Some(Self::Noir),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackdropMode {
    Solid,
    EdgeWash,
    QuietGrid,
}

impl BackdropMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "solid" => Some(Self::Solid),
            "edge_wash" => Some(Self::EdgeWash),
            "quiet_grid" => Some(Self::QuietGrid),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct UserSettings {
    pub theme_mode: ThemeMode,
    pub backdrop_mode: BackdropMode,
    /// The user's opt-in for smart categorization. Off until they say so, and
    /// nothing is downloaded, scanned or replaced while it is off.
    pub categorization_enabled: bool,
    pub revision: i64,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RommSettings {
    pub base_url: String,
    pub configured: bool,
    pub updated_at: String,
}

impl SettingsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self) -> Result<UserSettings> {
        let row = sqlx::query(
            "SELECT theme_mode, backdrop_mode, categorization_enabled, revision, updated_at FROM user_settings WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await?;
        settings_from_row(&row)
    }

    /// Records the user's answer on smart categorization.
    ///
    /// There is no two-way editing here to reconcile: the flag is the daemon's
    /// gate on a capability, and a half-applied toggle would be worse than a lost one.
    pub async fn set_categorization_enabled(&self, enabled: bool) -> Result<UserSettings> {
        let updated_at = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE user_settings SET categorization_enabled = ?, revision = revision + 1, updated_at = ? WHERE id = 1",
        )
        .bind(enabled)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;
        self.get().await
    }

    pub async fn romm(&self) -> Result<Option<RommSettings>> {
        let row = sqlx::query("SELECT base_url, updated_at FROM romm_settings WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| RommSettings {
            base_url: row.get("base_url"),
            configured: true,
            updated_at: row.get("updated_at"),
        }))
    }

    pub async fn romm_credentials(&self) -> Result<Option<RommCredentials>> {
        let row = sqlx::query("SELECT base_url, token FROM romm_settings WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| RommCredentials {
            base_url: row.get("base_url"),
            token: row.get("token"),
        }))
    }

    pub async fn save_romm(&self, base_url: &str, token: &str) -> Result<RommSettings> {
        let base_url = normalize_romm_url(base_url)?;
        let token = token.trim();
        if token.is_empty() || token.chars().count() > 2048 {
            bail!("RomM token must contain 1 to 2048 characters");
        }
        let updated_at = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO romm_settings (id, base_url, token, updated_at) VALUES (1, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET base_url = excluded.base_url, token = excluded.token, updated_at = excluded.updated_at",
        )
        .bind(&base_url)
        .bind(token)
        .bind(&updated_at)
        .execute(&self.pool)
        .await?;
        Ok(RommSettings {
            base_url,
            configured: true,
            updated_at,
        })
    }

    pub async fn clear_romm(&self) -> Result<()> {
        sqlx::query("DELETE FROM romm_settings WHERE id = 1")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct RommCredentials {
    pub base_url: String,
    pub token: String,
}

pub(crate) fn normalize_romm_url(value: &str) -> Result<String> {
    let base_url = value.trim().trim_end_matches('/');
    if base_url.is_empty() || base_url.chars().count() > 2048 {
        bail!("RomM URL must contain 1 to 2048 characters");
    }
    let url = reqwest::Url::parse(base_url).map_err(|_| anyhow!("RomM URL must be absolute"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("RomM URL must be an absolute HTTP(S) URL");
    }
    let host = url.host_str().expect("validated URL host");
    if host != "localhost" && !host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()) {
        bail!("RomM URL must use a loopback host");
    }
    Ok(base_url.to_owned())
}

fn settings_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<UserSettings> {
    let theme_mode = row.get::<String, _>("theme_mode");
    let backdrop_mode = row.get::<String, _>("backdrop_mode");
    Ok(UserSettings {
        theme_mode: ThemeMode::parse(&theme_mode)
            .ok_or_else(|| anyhow!("invalid stored theme mode: {theme_mode}"))?,
        backdrop_mode: BackdropMode::parse(&backdrop_mode)
            .ok_or_else(|| anyhow!("invalid stored backdrop mode: {backdrop_mode}"))?,
        categorization_enabled: row.get("categorization_enabled"),
        revision: row.get("revision"),
        updated_at: row.get("updated_at"),
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::SettingsRepository;
    use crate::database::Database;

    #[tokio::test]
    async fn stores_romm_credentials_without_exposing_the_token() {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let settings = SettingsRepository::new(database.pool().clone());

        let saved = settings
            .save_romm("http://127.0.0.1:8080/", "rmm_secret")
            .await
            .unwrap();
        let public = settings.romm().await.unwrap().unwrap();
        let credentials = settings.romm_credentials().await.unwrap().unwrap();

        assert_eq!(saved.base_url, "http://127.0.0.1:8080");
        assert!(public.configured);
        assert_eq!(public.base_url, "http://127.0.0.1:8080");
        assert_eq!(credentials.token, "rmm_secret");
    }

    #[tokio::test]
    async fn rejects_non_loopback_romm_urls() {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let settings = SettingsRepository::new(database.pool().clone());

        let error = settings
            .save_romm("https://romm.example.com", "rmm_secret")
            .await
            .unwrap_err();

        assert!(error.to_string().contains("loopback"));
    }
}
