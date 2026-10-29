//! The input record: everything the categorizer is allowed to know about one
//! application before a scan.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a discovery provider says a record is.
///
/// Providers only ever emit `game` or `application` today, so unknown strings
/// deserialize to [`AppKind::Other`] instead of failing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppKind {
    Game,
    #[default]
    Application,
    #[serde(other)]
    Other,
}

impl AppKind {
    /// Read the `kind` a discovery provider stored. Mirrors the `Deserialize`
    /// impl, for callers that already have a `&str`.
    pub fn parse(value: &str) -> Self {
        match value {
            "game" => Self::Game,
            "application" => Self::Application,
            _ => Self::Other,
        }
    }
}

/// A flattened application record.
///
/// This is deliberately a superset of what any single source knows: discovery
/// fills title, categories and exec, AppStream-style metadata fills summary and
/// description.
/// The categorizer never assumes a field is present, because a scan has to run
/// over a library where most entries are only partially described.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppProfile {
    /// Stable identifier used to write the categorization back to its record.
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<AppKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer: Option<String>,
    /// What a launcher hands to the system to start this app.
    ///
    /// That is a command line when a source exposes one, and otherwise the
    /// identifier the launcher itself resolves — a desktop file id, or
    /// `runner:game` for a game launcher. It is only ever used as an identity
    /// signal ([`AppProfile::signal_terms`]), never executed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec: Option<String>,
    /// Store or launcher the record came from, e.g. `steam` or `flathub`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    /// Console platform, when the record is a ROM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Discovery source id, used to keep records of one provider together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// Freedesktop or provider categories, verbatim.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub urls: BTreeMap<String, String>,
}

impl AppProfile {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            ..Self::default()
        }
    }

    /// Build a profile from the merged metadata JSON the daemon serves.
    ///
    /// Only top-level keys are read. `summary` falls back to `comment`, which is
    /// what a desktop entry calls the same field.
    pub fn from_metadata(
        id: impl Into<String>,
        title: impl Into<String>,
        kind: Option<AppKind>,
        metadata: &Value,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind,
            summary: string_at(metadata, "summary").or_else(|| string_at(metadata, "comment")),
            description: string_at(metadata, "description"),
            developer: string_at(metadata, "developer"),
            exec: string_at(metadata, "exec"),
            store: string_at(metadata, "store"),
            platform: string_at(metadata, "platform"),
            source_id: string_at(metadata, "source_id"),
            categories: string_list_at(metadata, "categories"),
            keywords: string_list_at(metadata, "keywords"),
            urls: string_map_at(metadata, "urls"),
        }
    }

    /// The id and exec line, lowercased, as one haystack. Discovery
    /// heuristics key off this.
    pub fn signal_terms(&self) -> String {
        format!("{} {}", self.id, self.exec.as_deref().unwrap_or_default()).to_lowercase()
    }

    pub fn has_category(&self, name: &str) -> bool {
        self.categories
            .iter()
            .any(|category| category.eq_ignore_ascii_case(name))
    }
}

fn string_at(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

fn string_list_at(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn string_map_at(value: &Value, key: &str) -> BTreeMap<String, String> {
    value
        .get(key)
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|text| (key.clone(), text.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{AppKind, AppProfile};
    use serde_json::json;

    #[test]
    fn unknown_kinds_deserialize_to_other() {
        assert_eq!(
            serde_json::from_value::<AppKind>(json!("game")).unwrap(),
            AppKind::Game
        );
        assert_eq!(
            serde_json::from_value::<AppKind>(json!("rom")).unwrap(),
            AppKind::Other
        );
    }

    #[test]
    fn metadata_comment_becomes_summary() {
        let profile = AppProfile::from_metadata(
            "netflix.desktop",
            "Netflix",
            Some(AppKind::Application),
            &json!({
                "comment": "Watch TV shows and movies",
                "categories": ["AudioVideo", "Player"],
                "urls": {"homepage": "https://netflix.com"},
            }),
        );

        assert_eq!(
            profile.summary.as_deref(),
            Some("Watch TV shows and movies")
        );
        assert_eq!(profile.categories, vec!["AudioVideo", "Player"]);
        assert_eq!(
            profile.urls.get("homepage").map(String::as_str),
            Some("https://netflix.com")
        );
    }
}
