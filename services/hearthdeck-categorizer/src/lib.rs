//! Laya-backed categorization for the Hearthdeck library.
//!
//! The frontend currently decides what an app *is* with hand-maintained tables:
//! a list of streaming-service names, a list of emulator names, and a whitelist
//! of freedesktop categories mapped one-to-one onto tabs. That is the "shady"
//! matching this crate replaces. Here, a small local model ([`laya`]) answers a
//! fixed set of questions about each app, and the answers are aggregated into
//! the sections and category tabs a library should have.
//!
//! # Shape of a run
//!
//! 1. [`AppProfile`] — flatten a catalog record into the fields a decision may
//!    use, plus whatever a [`AppResearcher`] can find online.
//! 2. [`Categorizer`] — one engine answers "is this a game, an emulator, a
//!    streaming client, and which category does it belong in". Two ship here:
//!    [`HeuristicCategorizer`], which reproduces today's behavior, and
//!    [`LayaCategorizer`], which asks the model.
//! 3. [`LibraryScanner`] — runs the engine over the whole library off the async
//!    runtime, aggregates the verdicts into [`CategoryProposal`]s, and returns a
//!    serializable [`ScanReport`].
//!
//! A scan is deliberately one-shot. `LayaCategorizer::load` pulls a checkpoint
//! into memory and holds it until it is dropped, which is why the engine is
//! built for a scan and released after it, rather than kept next to the daemon.
//!
//! ```
//! use std::sync::Arc;
//! use hearthdeck_categorizer::{AppProfile, HeuristicCategorizer, LibraryScanner};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let scanner = LibraryScanner::new(Arc::new(HeuristicCategorizer::new()));
//! let apps = vec![AppProfile::new("netflix.desktop", "Netflix")];
//! let report = tokio::runtime::Runtime::new()?.block_on(scanner.scan(apps))?;
//!
//! println!("{} tabs to create", report.recommended_categories().count());
//! # Ok(())
//! # }
//! ```
//!
//! # Why the questions are fixed
//!
//! Laya is an encoder, not a generator: it scores a list of candidate answers
//! instead of writing text. So "which categories should exist" is split in two —
//! [`Taxonomy`] supplies the candidates, and [`ScanReport::recommended_categories`]
//! decides which of them earned a tab. An app that fits no candidate raises
//! [`AppCategorization::needs_category`], which is the signal that the taxonomy
//! itself should grow.

pub mod decision;
pub mod error;
pub mod heuristic;
pub mod model;
pub mod prompt;
pub mod research;
pub mod scan;
pub mod taxonomy;

#[cfg(feature = "laya")]
pub mod laya_engine;

pub use decision::{
    AppCategorization, AppTraits, Categorizer, CategoryMatch, DecisionProvenance, TRAIT_THRESHOLD,
};
pub use error::{CategorizerError, Result};
pub use heuristic::HeuristicCategorizer;
pub use model::{AppKind, AppProfile};
pub use research::{AppResearcher, NoopResearcher};
pub use scan::{
    CategoryProposal, LibraryScanner, ScanFailure, ScanOptions, ScanPhase, ScanProgress,
    ScanReport, UnclassifiedApp,
};
pub use taxonomy::{CategoryDef, Section, Taxonomy};

#[cfg(feature = "laya")]
pub use laya_engine::{LayaCategorizer, LayaConfig, LayaModel};
