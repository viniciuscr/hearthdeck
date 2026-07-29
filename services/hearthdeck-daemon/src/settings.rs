use anyhow::{Result, anyhow};
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

    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Aurora => "aurora",
            Self::Ember => "ember",
            Self::Indigo => "indigo",
            Self::Noir => "noir",
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

    fn as_str(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::EdgeWash => "edge_wash",
            Self::QuietGrid => "quiet_grid",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct UserSettings {
    pub theme_mode: ThemeMode,
    pub backdrop_mode: BackdropMode,
    pub revision: i64,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub enum SettingsUpdate {
    Saved(UserSettings),
    Conflict(UserSettings),
}

#[derive(Clone, Copy, Debug)]
pub struct SettingsChange {
    pub theme_mode: Option<ThemeMode>,
    pub backdrop_mode: Option<BackdropMode>,
}

impl SettingsChange {
    pub fn is_empty(self) -> bool {
        self.theme_mode.is_none() && self.backdrop_mode.is_none()
    }
}

impl SettingsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self) -> Result<UserSettings> {
        let row = sqlx::query(
            "SELECT theme_mode, backdrop_mode, revision, updated_at FROM user_settings WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await?;
        settings_from_row(&row)
    }

    pub async fn update(
        &self,
        change: SettingsChange,
        expected_revision: Option<i64>,
    ) -> Result<SettingsUpdate> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT theme_mode, backdrop_mode, revision, updated_at FROM user_settings WHERE id = 1",
        )
        .fetch_one(&mut *transaction)
        .await?;
        let current = settings_from_row(&row)?;
        let theme_mode = change.theme_mode.unwrap_or(current.theme_mode);
        let backdrop_mode = change.backdrop_mode.unwrap_or(current.backdrop_mode);

        if expected_revision.is_some_and(|revision| revision != current.revision)
            && (current.theme_mode != theme_mode || current.backdrop_mode != backdrop_mode)
        {
            return Ok(SettingsUpdate::Conflict(current));
        }
        if current.theme_mode == theme_mode && current.backdrop_mode == backdrop_mode {
            transaction.commit().await?;
            return Ok(SettingsUpdate::Saved(current));
        }

        let updated_at = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE user_settings SET theme_mode = ?, backdrop_mode = ?, revision = revision + 1, updated_at = ? WHERE id = 1",
        )
        .bind(theme_mode.as_str())
        .bind(backdrop_mode.as_str())
        .bind(updated_at)
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query(
            "SELECT theme_mode, backdrop_mode, revision, updated_at FROM user_settings WHERE id = 1",
        )
        .fetch_one(&mut *transaction)
        .await?;
        let saved = settings_from_row(&row)?;
        transaction.commit().await?;
        Ok(SettingsUpdate::Saved(saved))
    }
}

fn settings_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<UserSettings> {
    let theme_mode = row.get::<String, _>("theme_mode");
    let backdrop_mode = row.get::<String, _>("backdrop_mode");
    Ok(UserSettings {
        theme_mode: ThemeMode::parse(&theme_mode)
            .ok_or_else(|| anyhow!("invalid stored theme mode: {theme_mode}"))?,
        backdrop_mode: BackdropMode::parse(&backdrop_mode)
            .ok_or_else(|| anyhow!("invalid stored backdrop mode: {backdrop_mode}"))?,
        revision: row.get("revision"),
        updated_at: row.get("updated_at"),
    })
}
