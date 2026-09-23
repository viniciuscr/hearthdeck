//! Errors raised while categorizing a library.

use thiserror::Error;

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, CategorizerError>;

#[derive(Debug, Error)]
pub enum CategorizerError {
    /// The Laya checkpoint could not be downloaded, opened or built.
    #[error("could not load a Laya checkpoint: {0}")]
    Model(String),

    /// The checkpoint ran but rejected the prompt, ran out of memory, or
    /// produced a non-finite answer.
    #[error("Laya inference failed: {0}")]
    Inference(String),

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
