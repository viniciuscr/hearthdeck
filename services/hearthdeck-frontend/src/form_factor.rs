//! Resolved daemon form factors and local user overrides.

use crate::library_item::LibraryItem;
pub use hearthdeck_protocol::FormFactor;

pub fn set(entry: &mut LibraryItem, form: FormFactor) {
    entry.form_factor = Some(form);
}

#[must_use]
pub fn of(entry: &LibraryItem) -> FormFactor {
    entry.form_factor.unwrap_or(FormFactor::Desktop)
}

#[must_use]
pub fn category(form: FormFactor) -> String {
    form.category_value().to_owned()
}

#[cfg(test)]
mod tests {
    use super::{FormFactor, of, set};
    use crate::library_item::LibraryItem;

    #[test]
    fn user_override_replaces_the_daemons_form_without_category_tags() {
        let mut app = LibraryItem {
            form_factor: Some(FormFactor::Desktop),
            ..LibraryItem::default()
        };
        set(&mut app, FormFactor::Couch);
        assert_eq!(of(&app), FormFactor::Couch);
        assert!(app.categories.is_empty());
    }

    #[test]
    fn category_text_cannot_override_the_typed_form() {
        let app = LibraryItem {
            categories: vec!["hearthdeck-form:couch".to_owned()],
            form_factor: Some(FormFactor::Desktop),
            ..LibraryItem::default()
        };
        assert_eq!(of(&app), FormFactor::Desktop);
        assert!(app.matches_category("HEARTHDECK-FORM:DESKTOP"));
        assert!(!app.matches_category("hearthdeck-form:couch"));
    }
}
