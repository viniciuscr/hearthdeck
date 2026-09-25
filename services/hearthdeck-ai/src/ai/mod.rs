//! The shared interface to the local model.
//!
//! Every functionality in this crate that needs the model goes through here.
//! This is the only place that knows how to load a Laya checkpoint and run one
//! constrained prediction; it has no opinion about what is being asked or how an
//! answer is used. A functionality supplies a state string and a map of
//! questions, and reads the answers back.
//!
//! Laya is an encoder, not a generator: it scores a list of candidate answers
//! rather than writing text, so a prediction here is a set of scores, not prose.
//! Loading a checkpoint is the expensive part — most of a gigabyte and seconds
//! of CPU — which is why a caller builds one [`Laya`] per unit of work and drops
//! it afterwards rather than keeping it resident.

pub mod error;

#[cfg(feature = "laya")]
mod laya;

pub use error::{Error, Result};

#[cfg(feature = "laya")]
pub use laya::{Laya, LayaConfig, LayaModel, Prediction};
