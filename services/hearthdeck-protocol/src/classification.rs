use serde::{Deserialize, Serialize};

use crate::Section;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormFactor {
    Couch,
    Desktop,
}

impl FormFactor {
    #[must_use]
    pub fn from_category(category: &str) -> Option<Self> {
        [Self::Couch, Self::Desktop]
            .into_iter()
            .find(|form| category.eq_ignore_ascii_case(form.category_value()))
    }

    #[must_use]
    pub fn category_value(self) -> &'static str {
        match self {
            Self::Couch => "hearthdeck-form:couch",
            Self::Desktop => "hearthdeck-form:desktop",
        }
    }

    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::Couch => Self::Desktop,
            Self::Desktop => Self::Couch,
        }
    }
}

/// Daemon-resolved library placement, independent of optional model scans.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogClassification {
    pub section: Section,
    pub streaming: bool,
    pub form_factor: Option<FormFactor>,
}

impl Default for CatalogClassification {
    fn default() -> Self {
        Self {
            section: Section::Applications,
            streaming: false,
            form_factor: Some(FormFactor::Desktop),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_has_stable_wire_names_and_does_not_default_missing_fields() {
        let value = serde_json::json!({
            "section": "applications", "streaming": true, "form_factor": "couch"
        });
        let classification: CatalogClassification = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(classification.section, Section::Applications);
        assert_eq!(classification.form_factor, Some(FormFactor::Couch));
        assert!(classification.streaming);
        assert_eq!(serde_json::to_value(classification).unwrap(), value);
        assert!(
            serde_json::from_value::<CatalogClassification>(serde_json::json!({
                "section": "applications", "form_factor": "desktop"
            }))
            .is_err()
        );
    }

    #[test]
    fn saved_form_filters_keep_case_insensitive_matching() {
        assert_eq!(
            FormFactor::from_category("HEARTHDECK-FORM:COUCH"),
            Some(FormFactor::Couch)
        );
        assert_eq!(FormFactor::from_category("hearthdeck-form:other"), None);
        assert_eq!(FormFactor::Couch.other(), FormFactor::Desktop);
    }

    #[test]
    fn catalog_responses_require_daemon_classification() {
        let value = serde_json::json!({
            "id": "test", "source_id": "custom", "title": "Test",
            "kind": "application", "launch_id": null, "icon": null, "metadata": {}
        });
        assert!(serde_json::from_value::<crate::CatalogItem>(value).is_err());
    }
}
