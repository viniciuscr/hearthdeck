use crate::form_factor::FormFactor;
use cosmic::desktop::fde::IconSource;

/// Compatibility encoding for saved store filters, not an item category.
pub const STORE_FILTER_PREFIX: &str = "hearthdeck-store:";

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
}

impl LibraryItem {
    /// Matches saved category filters against typed metadata when applicable.
    #[must_use]
    pub fn matches_category(&self, category: &str) -> bool {
        if category
            .get(..STORE_FILTER_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(STORE_FILTER_PREFIX))
        {
            return self.store.as_ref().is_some_and(|store| {
                store.to_lowercase() == category[STORE_FILTER_PREFIX.len()..].to_lowercase()
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

    #[must_use]
    pub fn matches_facet(&self, value: &str) -> bool {
        if let Some(store) = value.strip_prefix(STORE_FILTER_PREFIX) {
            self.store.as_deref() == Some(store)
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LibraryItem;

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
}
