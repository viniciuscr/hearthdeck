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
/// description, and an [`crate::AppResearcher`] can fill in whatever is left.
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

    /// The id and exec line, lowercased, as one haystack. Both discovery
    /// heuristics and a research provider key off this.
    pub fn signal_terms(&self) -> String {
        format!("{} {}", self.id, self.exec.as_deref().unwrap_or_default()).to_lowercase()
    }

    pub fn has_category(&self, name: &str) -> bool {
        self.categories
            .iter()
            .any(|category| category.eq_ignore_ascii_case(name))
    }

    /// Fold researched fields into this profile without overwriting what
    /// discovery already established. Discovery data wins because it describes
    /// what is installed, not what the upstream project publishes.
    pub fn merge_research(&mut self, other: &AppProfile) {
        fill(&mut self.summary, &other.summary);
        fill(&mut self.description, &other.description);
        fill(&mut self.developer, &other.developer);
        fill(&mut self.store, &other.store);
        fill(&mut self.platform, &other.platform);
        extend_unique(&mut self.categories, &other.categories);
        extend_unique(&mut self.keywords, &other.keywords);
        for (key, value) in &other.urls {
            self.urls
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
}

fn fill(target: &mut Option<String>, source: &Option<String>) {
    if target.is_none() {
        *target = source.clone();
    }
}

fn extend_unique(target: &mut Vec<String>, source: &[String]) {
    for value in source {
        if !target.iter().any(|existing| existing == value) {
            target.push(value.clone());
        }
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

    #[test]
    fn research_only_fills_gaps() {
        let mut profile = AppProfile::new("org.videolan.VLC.desktop", "VLC");
        profile.summary = Some("Installed copy".to_owned());
        profile.categories = vec!["AudioVideo".to_owned()];

        let researched = AppProfile {
            id: "org.videolan.VLC.desktop".to_owned(),
            title: "VLC media player".to_owned(),
            summary: Some("Upstream blurb".to_owned()),
            description: Some("A free and open source media player".to_owned()),
            developer: Some("VideoLAN".to_owned()),
            categories: vec!["AudioVideo".to_owned(), "Player".to_owned()],
            ..AppProfile::default()
        };
        profile.merge_research(&researched);

        assert_eq!(profile.summary.as_deref(), Some("Installed copy"));
        assert_eq!(
            profile.description.as_deref(),
            Some("A free and open source media player")
        );
        assert_eq!(profile.developer.as_deref(), Some("VideoLAN"));
        assert_eq!(profile.categories, vec!["AudioVideo", "Player"]);
        // The upstream title is not installed truth, so it is ignored.
        assert_eq!(profile.title, "VLC");
    }
}
