use crate::form_factor::FormFactor;
use cosmic::desktop::fde::IconSource;

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
}

impl LibraryItem {
    /// Matches saved category filters, including the legacy form-factor values.
    #[must_use]
    pub fn matches_category(&self, category: &str) -> bool {
        match FormFactor::from_category(category) {
            Some(form) => self.form_factor == Some(form),
            None => self
                .categories
                .iter()
                .any(|value| value.to_lowercase() == category.to_lowercase()),
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
        }
    }
}
