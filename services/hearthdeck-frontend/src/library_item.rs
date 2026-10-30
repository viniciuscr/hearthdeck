use crate::form_factor::FormFactor;
use cosmic::desktop::fde::IconSource;

/// Compatibility encoding for saved store filters, not an item category.
pub const STORE_FILTER_PREFIX: &str = "hearthdeck-store:";
pub const SOURCE_FILTER_PREFIX: &str = "hearthdeck-source:";
pub const CONSOLE_FILTER_PREFIX: &str = "hearthdeck-console:";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleMetadata {
    pub platform_id: i64,
    pub genres: Vec<String>,
    pub regions: Vec<String>,
}

/// A library entry owned by Hearthdeck, not the desktop shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryItem {
    pub id: String,
    pub name: String,
    pub exec: Option<String>,
    pub icon: IconSource,
    pub categories: Vec<String>,
    pub wm_class: Option<String>,
    pub form_factor: Option<FormFactor>,
    pub store: Option<String>,
    pub sources: Vec<String>,
    pub console: Option<ConsoleMetadata>,
}

impl LibraryItem {
    /// Matches saved category filters against typed metadata when applicable.
    #[must_use]
    pub fn matches_category(&self, category: &str) -> bool {
        if let Some((prefix, value)) = category.split_once(':') {
            match prefix.to_ascii_lowercase().as_str() {
                "hearthdeck-console" => {
                    return self
                        .console
                        .as_ref()
                        .is_some_and(|console| console.platform_id.to_string() == value);
                }
                "hearthdeck-genre" | "hearthdeck-region" => {
                    return self.console.as_ref().is_some_and(|console| {
                        let values = if prefix.eq_ignore_ascii_case("hearthdeck-genre") {
                            &console.genres
                        } else {
                            &console.regions
                        };
                        values
                            .iter()
                            .any(|entry| entry.to_lowercase() == value.to_lowercase())
                    });
                }
                _ => {}
            }
        }
        if category
            .get(..STORE_FILTER_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(STORE_FILTER_PREFIX))
        {
            return self.store.as_ref().is_some_and(|store| {
                store.to_lowercase() == category[STORE_FILTER_PREFIX.len()..].to_lowercase()
            });
        }
        if category
            .get(..SOURCE_FILTER_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(SOURCE_FILTER_PREFIX))
        {
            return self.sources.iter().any(|source| {
                source.to_lowercase() == category[SOURCE_FILTER_PREFIX.len()..].to_lowercase()
            });
        }
        match FormFactor::from_category(category) {
            Some(form) => self.form_factor == Some(form),
            None => self
                .categories
                .iter()
                .any(|value| value.to_lowercase() == category.to_lowercase()),
        }
    }

    #[must_use]
    pub fn store_filter_value(&self) -> Option<String> {
        self.store
            .as_ref()
            .map(|store| format!("{STORE_FILTER_PREFIX}{store}"))
    }

    pub fn source_filter_values(&self) -> impl Iterator<Item = String> + '_ {
        self.sources
            .iter()
            .map(|source| format!("{SOURCE_FILTER_PREFIX}{source}"))
    }

    #[must_use]
    pub fn matches_facet(&self, value: &str) -> bool {
        if let Some(platform) = value.strip_prefix(CONSOLE_FILTER_PREFIX) {
            self.console
                .as_ref()
                .is_some_and(|console| console.platform_id.to_string() == platform)
        } else if let Some(genre) = value.strip_prefix("hearthdeck-genre:") {
            self.console
                .as_ref()
                .is_some_and(|console| console.genres.iter().any(|value| value == genre))
        } else if let Some(region) = value.strip_prefix("hearthdeck-region:") {
            self.console
                .as_ref()
                .is_some_and(|console| console.regions.iter().any(|value| value == region))
        } else if let Some(store) = value.strip_prefix(STORE_FILTER_PREFIX) {
            self.store.as_deref() == Some(store)
        } else if let Some(source) = value.strip_prefix(SOURCE_FILTER_PREFIX) {
            self.sources.iter().any(|value| value == source)
        } else if FormFactor::from_category(value).is_some() {
            self.matches_category(value)
        } else {
            self.categories.iter().any(|category| category == value)
        }
    }
}

impl Default for LibraryItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            exec: None,
            icon: IconSource::Name(String::new()),
            categories: Vec::new(),
            wm_class: None,
            form_factor: None,
            store: None,
            sources: Vec::new(),
            console: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ConsoleMetadata, LibraryItem};

    #[test]
    fn console_filters_use_typed_metadata_and_keep_saved_matching_rules() {
        let item = LibraryItem {
            console: Some(ConsoleMetadata {
                platform_id: 7,
                genres: vec!["Rôle".to_owned(), "Action".to_owned()],
                regions: vec!["Japan".to_owned(), "USA".to_owned()],
            }),
            categories: vec![
                "hearthdeck-console:9".to_owned(),
                "hearthdeck-genre:Other".to_owned(),
                "hearthdeck-region:Europe".to_owned(),
            ],
            ..LibraryItem::default()
        };
        for value in [
            "HEARTHDECK-CONSOLE:7",
            "hearthdeck-genre:RÔLE",
            "hearthdeck-genre:action",
            "HEARTHDECK-REGION:JAPAN",
            "hearthdeck-region:usa",
        ] {
            assert!(item.matches_category(value), "{value}");
        }
        for value in [
            "hearthdeck-console:9",
            "hearthdeck-console:07",
            "hearthdeck-console:invalid",
            "hearthdeck-genre:Other",
            "hearthdeck-region:Europe",
        ] {
            assert!(!item.matches_category(value), "{value}");
            assert!(!item.matches_facet(value), "{value}");
        }
        for value in [
            "hearthdeck-console:7",
            "hearthdeck-genre:Rôle",
            "hearthdeck-genre:Action",
            "hearthdeck-region:Japan",
            "hearthdeck-region:USA",
        ] {
            assert!(item.matches_facet(value), "{value}");
            assert!(!LibraryItem::default().matches_category(value), "{value}");
            assert!(!LibraryItem::default().matches_facet(value), "{value}");
        }
        assert!(!item.matches_facet("hearthdeck-genre:rôle"));
        assert!(!item.matches_facet("hearthdeck-region:japan"));
        let mut changed = item.clone();
        changed
            .console
            .as_mut()
            .expect("console metadata")
            .genres
            .push("Puzzle".to_owned());
        assert_ne!(item, changed, "catalog refresh must see metadata changes");
    }

    #[test]
    fn store_filters_use_the_typed_value_not_conflicting_category_text() {
        let item = LibraryItem {
            store: Some("Stéam".to_owned()),
            categories: vec!["hearthdeck-store:GOG".to_owned()],
            ..LibraryItem::default()
        };
        assert!(item.matches_category("HEARTHDECK-STORE:STÉAM"));
        assert!(!item.matches_category("hearthdeck-store:GOG"));
        assert!(item.matches_facet("hearthdeck-store:Stéam"));
        assert!(!item.matches_facet("hearthdeck-store:stéam"));
        assert!(!item.matches_facet("hearthdeck-store:GOG"));
        assert_eq!(
            item.store_filter_value().as_deref(),
            Some("hearthdeck-store:Stéam")
        );
    }

    #[test]
    fn missing_store_does_not_match_an_empty_store_filter() {
        assert!(!LibraryItem::default().matches_category("hearthdeck-store:"));
        let item = LibraryItem {
            store: Some(String::new()),
            ..LibraryItem::default()
        };
        assert!(item.matches_category("hearthdeck-store:"));
    }

    #[test]
    fn source_filters_match_each_typed_value_not_category_text() {
        let item = LibraryItem {
            sources: vec!["custom-origin".to_owned(), "streaming".to_owned()],
            categories: vec!["hearthdeck-source:flatpak".to_owned()],
            ..LibraryItem::default()
        };
        assert!(item.matches_category("HEARTHDECK-SOURCE:CUSTOM-ORIGIN"));
        assert!(item.matches_category("hearthdeck-source:streaming"));
        assert!(!item.matches_category("hearthdeck-source:flatpak"));
        assert!(item.matches_facet("hearthdeck-source:custom-origin"));
        assert!(!item.matches_facet("hearthdeck-source:CUSTOM-ORIGIN"));
        assert!(!item.matches_facet("hearthdeck-source:flatpak"));
        assert_eq!(
            item.source_filter_values().collect::<Vec<_>>(),
            [
                "hearthdeck-source:custom-origin",
                "hearthdeck-source:streaming",
            ]
        );
        assert!(!LibraryItem::default().matches_category("hearthdeck-source:"));
    }
}
