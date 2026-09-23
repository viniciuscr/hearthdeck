//! The decision interface, and the record one application's verdict comes back as.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::model::AppProfile;
use crate::taxonomy::{Section, Taxonomy};

/// Probability above which a yes/no trait is treated as true.
pub const TRAIT_THRESHOLD: f64 = 0.5;

/// Which engine produced a verdict. Carried on every record so a scan's output
/// can be compared against the heuristic it replaces, and so the UI can label a
/// guess it should not trust blindly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionProvenance {
    Laya,
    Heuristic,
}

/// The yes/no questions every engine answers about an application, kept as raw
/// probabilities so callers can pick their own confidence bar.
///
/// These are the three predicates the frontend currently hardcodes: a
/// hand-maintained list of streaming services, a hand-maintained list of
/// emulators, and the `Game` freedesktop category.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppTraits {
    /// P(this is a video game).
    pub game: Option<f64>,
    /// P(this exists to watch streaming or TV content).
    pub watch_service: Option<f64>,
    /// P(this is a console emulator or retro-game frontend).
    pub emulator: Option<f64>,
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
}

/// One category the model placed an application in, with its probability.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CategoryMatch {
    pub slug: String,
    pub name: String,
    pub confidence: f64,
}

/// The verdict for one application.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppCategorization {
    pub app_id: String,
    pub title: String,
    pub section: Section,
    /// Probability supporting `section`.
    pub section_confidence: f64,
    pub categories: Vec<CategoryMatch>,
    /// True when the app belongs in Applications but no candidate category fit.
    /// This is the signal that the taxonomy needs a new entry.
    pub needs_category: bool,
    pub traits: AppTraits,
    pub provenance: DecisionProvenance,
    /// Short human explanation, filled by the deterministic engine and by any
    /// engine that had to override the model's own answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// A way to decide what an application is.
///
/// Implementations must be cheap to share across threads: a scan holds one
/// behind an `Arc` and calls it once per application. The method is synchronous
/// because inference is CPU-bound; the scanner runs it on a blocking worker.
pub trait Categorizer: Send + Sync {
    /// Stable id recorded as provenance in the scan report.
    fn id(&self) -> &'static str;

    fn categorize(&self, app: &AppProfile, taxonomy: &Taxonomy) -> Result<AppCategorization>;
}

#[cfg(test)]
mod tests {
    use super::AppTraits;

    #[test]
    fn a_missing_probability_reads_as_a_no() {
        let empty = AppTraits::default();
        assert!(!empty.is_game(0.5));
        assert!(!empty.is_watch_service(0.5));
        assert!(!empty.is_emulator(0.5));
    }
}
