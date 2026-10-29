//! Categorization results shared by the scanner, daemon and clients.

use serde::{Deserialize, Serialize};

use crate::Section;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionProvenance {
    Laya,
    Heuristic,
}

/// Raw probabilities; an absent answer remains unknown.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppTraits {
    pub game: Option<f64>,
    pub watch_service: Option<f64>,
    pub emulator: Option<f64>,
    #[serde(default)]
    pub couch: Option<f64>,
}

impl AppTraits {
    pub fn is_game(&self, threshold: f64) -> bool {
        self.game.unwrap_or(0.0) >= threshold
    }

    pub fn is_watch_service(&self, threshold: f64) -> bool {
        self.watch_service.unwrap_or(0.0) >= threshold
    }

    pub fn is_emulator(&self, threshold: f64) -> bool {
        self.emulator.unwrap_or(0.0) >= threshold
    }

    pub fn is_couch_friendly(&self, threshold: f64) -> bool {
        self.couch.unwrap_or(0.0) >= threshold
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CategoryMatch {
    pub slug: String,
    pub name: String,
    pub confidence: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppCategorization {
    pub app_id: String,
    pub title: String,
    pub section: Section,
    pub section_confidence: f64,
    pub categories: Vec<CategoryMatch>,
    pub needs_category: bool,
    pub traits: AppTraits,
    pub provenance: DecisionProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanFailure {
    pub app_id: String,
    pub error: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnclassifiedApp {
    pub app_id: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CategoryProposal {
    pub slug: String,
    pub name: String,
    pub section: Section,
    pub app_ids: Vec<String>,
    pub mean_confidence: f64,
    pub recommended: bool,
}

/// The scanner's persisted report and the daemon's categorization payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanReport {
    pub generated_at: String,
    pub categorizer: String,
    pub app_count: usize,
    pub assignments: Vec<AppCategorization>,
    pub failures: Vec<ScanFailure>,
    pub categories: Vec<CategoryProposal>,
    pub unclassified: Vec<UnclassifiedApp>,
}

impl ScanReport {
    pub fn recommended_categories(&self) -> impl Iterator<Item = &CategoryProposal> {
        self.categories
            .iter()
            .filter(|category| category.recommended)
    }

    pub fn categorizations_for(&self, app_id: &str) -> Option<&AppCategorization> {
        self.assignments
            .iter()
            .find(|assignment| assignment.app_id == app_id)
    }
}

#[cfg(test)]
mod tests {
    use super::{AppTraits, ScanReport};

    #[test]
    fn a_missing_probability_reads_as_a_no() {
        let empty = AppTraits::default();
        assert!(!empty.is_game(0.5));
        assert!(!empty.is_watch_service(0.5));
        assert!(!empty.is_emulator(0.5));
        assert!(!empty.is_couch_friendly(0.5));
    }

    #[test]
    fn scan_reports_preserve_wire_fields_and_read_older_assignments() {
        let json = serde_json::json!({
            "generated_at": "2026-10-29T00:00:00Z",
            "categorizer": "heuristic",
            "app_count": 1,
            "assignments": [{
                "app_id": "game", "title": "Game", "section": "pc_games",
                "section_confidence": 1.0, "categories": [], "needs_category": false,
                "traits": {"game": 1.0, "watch_service": null, "emulator": 0.0},
                "provenance": "heuristic"
            }],
            "failures": [{"app_id": "broken", "error": "unreadable"}],
            "categories": [{
                "slug": "games", "name": "Games", "section": "pc_games",
                "app_ids": ["game"], "mean_confidence": 1.0, "recommended": true
            }],
            "unclassified": [{"app_id": "unknown", "title": "Unknown"}]
        });
        let report: ScanReport = serde_json::from_value(json.clone()).unwrap();
        let game = report.categorizations_for("game").unwrap();
        assert_eq!(game.traits.couch, None);
        assert_eq!(game.rationale, None);
        assert_eq!(report.categorizations_for("missing"), None);
        assert_eq!(
            report.recommended_categories().next().unwrap().slug,
            "games"
        );
        let mut expected = json;
        expected["assignments"][0]["traits"]["couch"] = serde_json::Value::Null;
        assert_eq!(serde_json::to_value(report).unwrap(), expected);
    }
}
