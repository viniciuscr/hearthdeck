//! The Laya-backed model: load a checkpoint, run one prediction.
//!
//! This module owns the checkpoint and the runtime, and nothing about what is
//! being asked. [`crate::categorization`] is the first functionality built on
//! it; a later one reuses the same [`Laya`] by supplying its own state and
//! questions.

use std::path::PathBuf;

use laya::Agent;
use laya::agent::{AgentBuilder, parse_dtype};
use laya::router::ModelName;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::error::{Error, Result};

/// Which pretrained checkpoint to answer with.
///
/// The two differ only in the language they can read; the multilingual one is
/// finer to load, so English is the default. Picking by language is a model
/// choice, not a per-record one, because a caller loads exactly one checkpoint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayaModel {
    #[default]
    English,
    Multilingual,
}

impl LayaModel {
    fn resolve(self) -> ModelName {
        match self {
            Self::English => ModelName::English,
            Self::Multilingual => ModelName::Multilingual,
        }
    }

    pub fn as_str(self) -> &'static str {
        self.resolve().as_str()
    }
}

/// How to load a checkpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayaConfig {
    pub model: LayaModel,
    /// A local checkpoint directory. When set, nothing is downloaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_path: Option<PathBuf>,
    /// `float32`, `float16` (the default) or `bfloat16`.
    #[serde(default = "default_dtype")]
    pub dtype: String,
    /// Inputs per forward pass.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

impl Default for LayaConfig {
    fn default() -> Self {
        Self {
            model: LayaModel::default(),
            model_path: None,
            dtype: default_dtype(),
            batch_size: default_batch_size(),
        }
    }
}

fn default_dtype() -> String {
    "float16".to_owned()
}

fn default_batch_size() -> usize {
    16
}

/// What one prediction came back with.
pub struct Prediction {
    /// One entry per question id, in whatever shape Laya answers that question
    /// type with. Reading it is the caller's job, because only the caller knows
    /// what it asked.
    pub answers: Map<String, Value>,
    /// True when the state was cut short to fit the token budget, so the answer
    /// rests on less evidence than the caller supplied.
    pub truncated: bool,
}

/// A loaded checkpoint.
pub struct Laya {
    agent: Agent,
    model: String,
}

impl Laya {
    pub fn load(config: &LayaConfig) -> Result<Self> {
        let builder = AgentBuilder::new()
            .dtype(parse_dtype(&config.dtype).map_err(model_error)?)
            .batch_size(config.batch_size)
            // A functionality asks the same questions of every record, so caching
            // the tokenized question prefixes is what keeps a run over a large
            // library linear in the number of records rather than in prompt size.
            .cache_prompts(true);

        let model = config.model.resolve();
        let agent = match &config.model_path {
            Some(path) => builder.build(path).map_err(model_error)?,
            None => {
                let (repo, subfolder) = model.bundle_location();
                Agent::from_pretrained_with(repo, subfolder, None, builder).map_err(model_error)?
            }
        };

        Ok(Self {
            agent,
            model: model.as_str().to_owned(),
        })
    }

    /// Which checkpoint is loaded, for logs and provenance.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Answer a state against a map of questions in one forward pass.
    ///
    /// The state is a string rather than JSON so the caller controls field
    /// order — Laya truncates from the right, so what identifies a record best
    /// has to come first. Both arguments are owned because a caller builds them
    /// fresh per record.
    pub fn predict(&self, state: String, questions: Map<String, Value>) -> Result<Prediction> {
        let prediction = self
            .agent
            .predict(&Value::String(state), &Value::Object(questions))
            .map_err(|error| Error::Inference(error.to_string()))?;
        Ok(Prediction {
            answers: prediction.answers,
            truncated: prediction.truncated,
        })
    }
}

fn model_error(error: laya::Error) -> Error {
    Error::Model(error.to_string())
}
