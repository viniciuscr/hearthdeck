//! The Laya-backed engine.
//!
//! Laya is a bidirectional encoder that answers constrained questions in one
//! forward pass, so the whole decision is one `predict` call per application.
//! The prompt building and answer reading live in [`crate::prompt`]; this module
//! only owns the checkpoint and the runtime.
//!
//! Loading a checkpoint is the expensive part, in both time and memory, which is
//! why this type is built once per scan and dropped afterwards rather than kept
//! resident next to the daemon.

use std::path::PathBuf;

use laya::Agent;
use laya::agent::{AgentBuilder, parse_dtype};
use laya::router::ModelName;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use crate::decision::{AppCategorization, Categorizer, TRAIT_THRESHOLD};
use crate::error::{CategorizerError, Result};
use crate::model::AppProfile;
use crate::prompt::{build_questions, build_state, interpret};
use crate::taxonomy::Taxonomy;

/// Which pretrained checkpoint to answer with.
///
/// The two differ only in the language they can read; the multilingual one is
/// finer to load, so English is the default. Picking by language is a model
/// choice, not a per-application one, because a scan loads exactly one
/// checkpoint.
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
    /// Questions per forward pass.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    /// Probability above which a yes/no trait counts as true.
    #[serde(default = "default_threshold")]
    pub threshold: f64,
}

impl Default for LayaConfig {
    fn default() -> Self {
        Self {
            model: LayaModel::default(),
            model_path: None,
            dtype: default_dtype(),
            batch_size: default_batch_size(),
            threshold: TRAIT_THRESHOLD,
        }
    }
}

fn default_dtype() -> String {
    "float16".to_owned()
}

fn default_batch_size() -> usize {
    16
}

fn default_threshold() -> f64 {
    TRAIT_THRESHOLD
}

/// A loaded checkpoint, used as a [`Categorizer`].
pub struct LayaCategorizer {
    agent: Agent,
    threshold: f64,
    model: String,
}

impl LayaCategorizer {
    pub fn load(config: &LayaConfig) -> Result<Self> {
        let builder = AgentBuilder::new()
            .dtype(parse_dtype(&config.dtype).map_err(model_error)?)
            .batch_size(config.batch_size)
            // Every application asks the same four questions, so caching the
            // tokenized question prefixes is what keeps a full scan linear in
            // the number of apps rather than in prompt size.
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
            threshold: config.threshold,
            model: model.as_str().to_owned(),
        })
    }

    /// Which checkpoint is loaded, for logs and provenance.
    pub fn model(&self) -> &str {
        &self.model
    }
}

impl Categorizer for LayaCategorizer {
    fn id(&self) -> &'static str {
        "laya"
    }

    fn categorize(&self, app: &AppProfile, taxonomy: &Taxonomy) -> Result<AppCategorization> {
        let state = build_state(app);
        let questions = build_questions(taxonomy);
        let prediction = self
            .agent
            .predict(&Value::String(state), &Value::Object(questions))
            .map_err(|error| CategorizerError::Inference(error.to_string()))?;
        if prediction.truncated {
            // The state is cut from the right, so a truncated prompt lost the
            // tail of the description. Worth surfacing: it means the answer was
            // made on less evidence than the record carries.
            warn!(
                app_id = app.id,
                "Laya truncated the state for this application"
            );
        }
        Ok(interpret(
            app,
            &prediction.answers,
            taxonomy,
            self.threshold,
        ))
    }
}

fn model_error(error: laya::Error) -> CategorizerError {
    CategorizerError::Model(error.to_string())
}
