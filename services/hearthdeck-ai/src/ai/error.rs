//! Errors raised while loading or running the local model.

use thiserror::Error;

/// Result alias for the model interface.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// The checkpoint could not be downloaded, opened or built.
    #[error("could not load a Laya checkpoint: {0}")]
    Model(String),

    /// The checkpoint ran but rejected the prompt, ran out of memory, or
    /// produced a non-finite answer.
    #[error("Laya inference failed: {0}")]
    Inference(String),
}
