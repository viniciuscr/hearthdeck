//! The model-backed categorizer.
//!
//! It turns an application into the model's state, asks the categorization
//! questions, and reads the answers back into an [`AppCategorization`]. The
//! checkpoint and the runtime live in [`crate::ai`]; what is specific to
//! categorization — the questions and how a reply is interpreted — lives in
//! [`super::prompt`].
//!
//! Loading a checkpoint is the expensive part, in both time and memory, which is
//! why this type is built once per scan and dropped afterwards rather than kept
//! resident next to the daemon.

use tracing::warn;

use crate::ai::{self, Laya};

use super::decision::{AppCategorization, Categorizer, TRAIT_THRESHOLD};
use super::error::Result;
use super::model::AppProfile;
use super::prompt::{build_questions, build_state, interpret};
use super::taxonomy::Taxonomy;

/// A loaded checkpoint, used as a [`Categorizer`].
pub struct LayaCategorizer {
    model: Laya,
}

impl LayaCategorizer {
    pub fn load(config: &ai::LayaConfig) -> Result<Self> {
        Ok(Self {
            model: Laya::load(config)?,
        })
    }

    /// Which checkpoint is loaded, for logs and provenance.
    pub fn model(&self) -> &str {
        self.model.model()
    }
}

impl Categorizer for LayaCategorizer {
    fn id(&self) -> &'static str {
        "laya"
    }

    fn categorize(&self, app: &AppProfile, taxonomy: &Taxonomy) -> Result<AppCategorization> {
        let prediction = self
            .model
            .predict(build_state(app), build_questions(taxonomy))?;
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
            TRAIT_THRESHOLD,
        ))
    }
}
