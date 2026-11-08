//! The TMDB credential, and the market the new-arrivals feed asks TMDB about.
//!
//! The dashboard's banner needs titles somebody can *watch now*, and the catalogs
//! Stremio's addon protocol offers cannot answer that: a Cinemeta year bucket is a
//! release calendar, where a film still in cinemas carries a `released` date like
//! any other and no field says whether it can be played. TMDB answers the question
//! directly — a film has left the cinemas once it has a digital or physical
//! release date in the past for a market — so this module holds the key that
//! unlocks that read, plus the locale-derived market to ask it about.
//!
//! The key is a secret with no expiry, like the Stremio `auth_key`: it is written
//! to the database file that is narrowed to its owner, read by the discovery
//! provider and nothing else, never logged, and never returned by a handler — the
//! settings endpoint serves only whether one is stored.

mod control;

pub(crate) use control::{Control, ControlError};

use std::env;

use anyhow::{Result, bail};
use chrono::Utc;
use serde::Serialize;
use sqlx::{Row, SqlitePool};

/// The market a host that declares no locale is assumed to be in. TMDB requires a
/// region for the release-type filter, so there has to be a default; the US is the
/// market TMDB's own release data is most complete for.
const DEFAULT_REGION: &str = "US";

/// The language a host that declares no locale is assumed to speak.
const DEFAULT_LANGUAGE: &str = "en-US";

#[derive(Clone)]
pub struct TmdbRepository {
    pool: SqlitePool,
}

impl TmdbRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The credential-free view the settings endpoint serves.
    pub async fn settings(&self) -> Result<Option<TmdbSettings>> {
        let row = sqlx::query("SELECT updated_at FROM tmdb_settings WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| TmdbSettings {
            configured: true,
            updated_at: row.get("updated_at"),
        }))
    }

    /// The key itself, for the provider only.
    pub async fn credentials(&self) -> Result<Option<TmdbCredentials>> {
        let row = sqlx::query("SELECT api_key FROM tmdb_settings WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| TmdbCredentials {
            api_key: row.get("api_key"),
        }))
    }

    pub async fn save(&self, api_key: &str) -> Result<TmdbSettings> {
        let api_key = normalize_api_key(api_key)?;
        let updated_at = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO tmdb_settings (id, api_key, updated_at) VALUES (1, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET api_key = excluded.api_key, \
             updated_at = excluded.updated_at",
        )
        .bind(&api_key)
        .bind(&updated_at)
        .execute(&self.pool)
        .await?;
        Ok(TmdbSettings {
            configured: true,
            updated_at,
        })
    }

    pub async fn clear(&self) -> Result<()> {
        sqlx::query("DELETE FROM tmdb_settings WHERE id = 1")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct TmdbCredentials {
    pub api_key: String,
}

/// What the settings endpoint may return: that a key is stored, when it was
/// stored, and nothing that could be replayed.
#[derive(Clone, Debug, Serialize)]
pub struct TmdbSettings {
    pub configured: bool,
    pub updated_at: String,
}

/// Accepted spellings of the credential, trimmed and checked.
///
/// TMDB hands out two: a 32-character v3 key read as a query parameter, and a
/// longer v4 read token — a JWT — read as a bearer header. Both are accepted, so
/// the check is about shape rather than an exact length: a value must be long
/// enough to be one of them and must not carry the whitespace a copied line picks
/// up, because either would be stored and then fail on every refresh with nothing
/// in the log to say the paste went wrong.
pub(crate) fn normalize_api_key(api_key: &str) -> Result<String> {
    let key = api_key.trim();
    if key.is_empty() {
        bail!("the TMDB API key must not be empty");
    }
    if key.chars().count() > 2048 {
        bail!("the TMDB API key must be at most 2048 characters");
    }
    if key.contains(char::is_whitespace) {
        bail!("the TMDB API key must not contain whitespace");
    }
    if key.chars().any(char::is_control) {
        bail!("the TMDB API key must not contain control characters");
    }
    if key.chars().count() < 16 {
        bail!("the TMDB API key is too short to be one");
    }
    Ok(key.to_owned())
}

/// The market the arrivals feed asks about, as an ISO 3166-1 alpha-2 code.
pub fn region() -> String {
    declared_locale()
        .and_then(|locale| locale_parts(&locale))
        .and_then(|(_, region)| region)
        .unwrap_or_else(|| DEFAULT_REGION.to_owned())
}

/// The language TMDB answers titles in, as a tag TMDB knows (`pt-BR`).
pub fn language() -> String {
    declared_locale()
        .and_then(|locale| locale_parts(&locale))
        .map(|(language, region)| match region {
            Some(region) => format!("{language}-{region}"),
            None => language,
        })
        .unwrap_or_else(|| DEFAULT_LANGUAGE.to_owned())
}

/// The first locale the host declares.
///
/// The POSIX order is what decides it: `LC_ALL` overrides every category,
/// `LC_TIME` is the nearest category a date is read from, and `LANG` is the
/// fallback. A host that sets none of them gets the defaults above rather than an
/// error — the banner still works, in the market TMDB dates most completely.
fn declared_locale() -> Option<String> {
    ["LC_ALL", "LC_TIME", "LANG"]
        .into_iter()
        .filter_map(|name| env::var(name).ok())
        .find(|value| !value.trim().is_empty())
}

/// `pt_BR.UTF-8` → (`pt`, `Some("BR")`).
///
/// A locale value is `language[_territory][.codeset][@modifier]`, and `C`,
/// `POSIX` and an empty value declare no market at all — which is why the caller
/// falls back rather than treating them as an unknown country.
fn locale_parts(locale: &str) -> Option<(String, Option<String>)> {
    let bare = locale.split(['.', '@']).next()?.trim();
    let mut parts = bare.split(['_', '-']);
    let language = parts.next()?;
    if language.len() != 2
        || !language
            .chars()
            .all(|character| character.is_ascii_alphabetic())
    {
        return None;
    }
    let region = parts
        .next()
        .filter(|region| {
            region.len() == 2
                && region
                    .chars()
                    .all(|character| character.is_ascii_alphabetic())
        })
        .map(str::to_ascii_uppercase);
    Some((language.to_ascii_lowercase(), region))
}

#[cfg(test)]
mod tests {
    use super::{TmdbRepository, language, locale_parts, normalize_api_key, region};
    use crate::database::Database;

    #[tokio::test]
    async fn a_stored_key_is_never_served_back_and_a_cleared_one_is_gone() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let repository = TmdbRepository::new(database.pool().clone());

        assert!(repository.settings().await.unwrap().is_none());
        assert!(repository.credentials().await.unwrap().is_none());

        let key = "0123456789abcdef0123456789abcdef";
        let saved = repository.save(key).await.unwrap();
        assert!(saved.configured);
        assert!(
            !serde_json::to_string(&saved).unwrap().contains(key),
            "the settings view must not carry the key"
        );
        assert_eq!(
            repository.credentials().await.unwrap().unwrap().api_key,
            key
        );

        repository.clear().await.unwrap();
        assert!(repository.settings().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn saving_again_replaces_the_key_rather_than_adding_a_row() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let repository = TmdbRepository::new(database.pool().clone());

        repository.save("the-first-key-0123456789").await.unwrap();
        repository.save("the-second-key-0123456789").await.unwrap();

        assert_eq!(
            repository.credentials().await.unwrap().unwrap().api_key,
            "the-second-key-0123456789"
        );
    }

    #[test]
    fn a_key_is_accepted_only_when_it_could_work_as_a_credential() {
        assert_eq!(
            normalize_api_key("  0123456789abcdef0123456789abcdef\n").unwrap(),
            "0123456789abcdef0123456789abcdef"
        );
        // A v4 read token is longer and carries dots.
        assert!(normalize_api_key("eyJhbGciOiJIUzI1NiJ9.eyJhdWQiOiJ4In0.signature").is_ok());
        assert!(normalize_api_key("").is_err());
        assert!(normalize_api_key("too-short").is_err());
        assert!(normalize_api_key("0123456789abcdef two-words").is_err());
    }

    #[test]
    fn a_locale_names_the_language_and_the_market() {
        assert_eq!(
            locale_parts("pt_BR.UTF-8"),
            Some(("pt".to_owned(), Some("BR".to_owned())))
        );
        assert_eq!(
            locale_parts("de_AT@euro"),
            Some(("de".to_owned(), Some("AT".to_owned())))
        );
        assert_eq!(
            locale_parts("en-US"),
            Some(("en".to_owned(), Some("US".to_owned())))
        );
        assert_eq!(locale_parts("fr"), Some(("fr".to_owned(), None)));
        assert_eq!(locale_parts("C"), None);
        assert_eq!(locale_parts("POSIX"), None);
        assert_eq!(locale_parts(""), None);
    }

    #[test]
    fn the_market_falls_back_when_the_host_declares_none() {
        // Whatever this host sets, both answers must be a market-shaped value:
        // the fallback is what keeps a bare container from asking TMDB for the
        // film market of nowhere.
        let region = region();
        assert_eq!(region.len(), 2, "region: {region}");
        assert!(
            region
                .chars()
                .all(|character| character.is_ascii_uppercase())
        );

        let language = language();
        let tag = language.split('-').next().unwrap_or_default();
        assert_eq!(tag.len(), 2, "language: {language}");
    }
}
