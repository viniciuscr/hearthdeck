//! Local-model features for the Hearthdeck library.
//!
//! The crate is a home for functionalities that need a local model to answer a
//! constrained question, each in its own module, on top of one shared interface:
//!
//! * [`ai`] — the interface to the model itself. Load a checkpoint, ask a set of
//!   questions about one state, read the answers. It knows nothing about what is
//!   being asked, so every functionality reuses it.
//! * [`categorization`] — the first functionality: decide what each installed
//!   app is and which category tabs a library should have.
//!
//! A functionality depends on [`ai`] and never the other way around. Adding one
//! means a new module beside [`categorization`] that supplies its own state,
//! questions and interpretation; nothing in [`ai`] changes.
//!
//! The `laya` feature (on by default) is what [`ai`] is built on. With it off
//! the crate still builds, and every functionality that needs the model reports
//! itself unavailable rather than pretending to answer.

pub mod ai;
pub mod categorization;
