//! The decision interface used by categorization engines.

pub use hearthdeck_protocol::{AppCategorization, AppTraits, CategoryMatch, DecisionProvenance};

use super::error::Result;
use super::model::AppProfile;
use super::taxonomy::Taxonomy;

/// Probability above which a yes/no trait is treated as true.
pub const TRAIT_THRESHOLD: f64 = 0.5;

/// Engines run on the scanner's blocking worker because inference is CPU-bound.
pub trait Categorizer: Send + Sync {
    fn id(&self) -> &'static str;

    fn categorize(&self, app: &AppProfile, taxonomy: &Taxonomy) -> Result<AppCategorization>;
}
