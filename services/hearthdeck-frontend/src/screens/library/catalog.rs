//! Keeps daemon records separate from the user's local form projection.

use std::sync::Arc;

use crate::app_group::AppLibraryConfig;
use crate::library_item::LibraryItem;

#[derive(Default)]
pub struct Catalog {
    source: Vec<Arc<LibraryItem>>,
    entries: Vec<Arc<LibraryItem>>,
}

impl Catalog {
    #[must_use]
    pub fn entries(&self) -> &[Arc<LibraryItem>] {
        &self.entries
    }

    /// Retains every source update, even when an override hides its change.
    pub fn replace(&mut self, source: Vec<Arc<LibraryItem>>, config: &AppLibraryConfig) -> bool {
        self.source = source;
        self.project(config)
    }

    /// Reports whether the effective catalog changed.
    pub fn project(&mut self, config: &AppLibraryConfig) -> bool {
        let mut entries = self.source.clone();
        config.apply_form_overrides(&mut entries);
        if entries == self.entries {
            return false;
        }
        self.entries = entries;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form_factor::{self, FormFactor};

    fn entry(form: FormFactor) -> Arc<LibraryItem> {
        Arc::new(LibraryItem {
            id: "writer".into(),
            form_factor: Some(form),
            ..LibraryItem::default()
        })
    }

    #[test]
    fn hidden_source_updates_are_kept_for_override_removal() {
        let mut catalog = Catalog::default();
        let mut config = AppLibraryConfig::default();
        config.set_form_override("writer", FormFactor::Couch);
        let source = entry(FormFactor::Desktop);
        assert!(catalog.replace(vec![Arc::clone(&source)], &config));
        assert_eq!(form_factor::of(&source), FormFactor::Desktop);
        assert!(!catalog.replace(vec![entry(FormFactor::Couch)], &config));
        config.form_overrides.clear();
        assert!(!catalog.project(&config));
        assert_eq!(form_factor::of(&catalog.entries()[0]), FormFactor::Couch);
    }

    #[test]
    fn overrides_are_reversible_without_reloading_the_source() {
        let mut catalog = Catalog::default();
        let mut config = AppLibraryConfig::default();
        assert!(catalog.replace(vec![entry(FormFactor::Desktop)], &config));
        config.set_form_override("writer", FormFactor::Couch);
        assert!(catalog.project(&config));
        assert_eq!(form_factor::of(&catalog.entries()[0]), FormFactor::Couch);
        assert!(!catalog.project(&config));
        config.form_overrides.clear();
        assert!(catalog.project(&config));
        assert_eq!(form_factor::of(&catalog.entries()[0]), FormFactor::Desktop);
    }
}
