//! Errors raised while categorizing a library.

use thiserror::Error;

use crate::ai;

/// Result alias for the categorization functionality.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// Loading the model or running it failed. Kept separate from the
    /// functionality's own errors so a caller can tell "the model broke" from
    /// "this record or taxonomy is bad".
    #[error(transparent)]
    Ai(#[from] ai::Error),

    /// A research provider could not reach its source.
    #[error("research provider {provider} failed: {message}")]
    Research { provider: String, message: String },

    /// The caller handed in something the categorizer cannot work with, such as
    /// a taxonomy whose labels are not unique.
    #[error("invalid input: {0}")]
    Invalid(String),

    /// The decision worker panicked or was cancelled.
    #[error("categorizer worker failed: {0}")]
    Worker(String),
}
