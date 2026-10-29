//! The smart-categories section's controls: what is on offer, and when.
//!
//! Kept as data rather than as buttons built in one place, because the same
//! list decides three things that must not drift apart: what the section
//! draws, which of it can be pressed, and - once the pad walks this screen -
//! what a press on a focused control means. The section is offered only where
//! the deployment provides the feature at all; a control that cannot work is
//! worse than no control.

use crate::providers::daemon::{CategorizationSnapshot, ModelStatus};

/// What the section draws.
pub struct Panel<'a> {
    /// Whether the deployment provides categorization at all. `Some(false)`
    /// means the feature does not exist for this client, whatever the user
    /// wants; `None` means the health reply has not arrived yet.
    pub capable: Option<bool>,
    /// The daemon's state, or `None` before the first read comes back.
    pub snapshot: Option<&'a CategorizationSnapshot>,
}

/// What the section draws for the state [`Panel`] describes.
#[derive(Debug)]
pub enum Section<'a> {
    /// This installation cannot run smart categorization at all.
    Unsupported,
    /// One of the reads has not landed. Nothing is claimed and nothing is
    /// offered, rather than drawing controls in a state nobody reported.
    Checking,
    /// The feature is here: the answer to draw, and the controls on offer.
    Ready {
        /// The daemon's own answer, which every line on screen is read off.
        snapshot: &'a CategorizationSnapshot,
        /// The controls, in drawing order.
        actions: Vec<Action>,
    },
}

/// One control the section offers. Each model action is its own button: the
/// switch only installs the model, and nothing is classified until one is pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The opt-in itself: turning it on downloads the model, and turning it off
    /// removes the model and restores everything it changed.
    Toggle,
    /// Classify the library and publish its dashboard rails (Watch, ...).
    PublishRails,
    /// Ask the model which applications are couch-friendly (the Couch tab).
    DetectCouch,
}

/// What the section draws for the state [`Panel`] describes.
///
/// The deployment's answer outranks the daemon's: where the feature does not
/// exist, no state read can make it exist, so the check comes first.
pub fn section<'a>(panel: &Panel<'a>) -> Section<'a> {
    if panel.capable == Some(false) {
        return Section::Unsupported;
    }
    // Nothing is claimed in a state nobody reported, so both reads have to have
    // landed before anything is drawn.
    let (Some(snapshot), Some(_)) = (panel.snapshot, panel.capable) else {
        return Section::Checking;
    };
    Section::Ready {
        snapshot,
        actions: actions(snapshot),
    }
}

/// The controls `snapshot` earns, in drawing order.
///
/// The switch is the way in and is always drawn; the model actions need the
/// opt-in and a model on disk to run.
fn actions(snapshot: &CategorizationSnapshot) -> Vec<Action> {
    let mut actions = vec![Action::Toggle];
    if snapshot.enabled {
        actions.push(Action::PublishRails);
        if !matches!(snapshot.status.model, ModelStatus::Absent) {
            actions.push(Action::DetectCouch);
        }
    }
    actions
}

/// Whether pressing `action` would be accepted in the state `panel` describes.
/// Until the pad's focus walk reaches this screen, the shell uses this to
/// decide whether a control is drawn enabled; a control that could not be
/// confirmed is drawn disabled rather than focused and dead.
///
/// Being on offer is not enough on its own: a scan in flight holds back
/// everything that reads or writes the checkpoint, while the switch is the
/// daemon's to honour either way.
pub fn pressable(action: Action, panel: &Panel<'_>) -> bool {
    let Section::Ready { snapshot, actions } = section(panel) else {
        return false;
    };
    if !actions.contains(&action) {
        return false;
    }
    !snapshot.status.running || action == Action::Toggle
}

#[cfg(test)]
mod tests {
    use super::{Action, Panel, Section, pressable, section};
    use crate::providers::daemon::{CategorizationPhase, CategorizationSnapshot, ModelStatus};

    /// A categorization answer with nothing but the switch set.
    fn snapshot(enabled: bool) -> CategorizationSnapshot {
        CategorizationSnapshot {
            enabled,
            status: hearthdeck_protocol::CategorizationStatus {
                running: false,
                phase: CategorizationPhase::Idle,
                completed: 0,
                total: 0,
                model: ModelStatus::Absent,
                last_error: None,
                last_completed_at: None,
            },
            report: None,
        }
    }

    fn downloaded() -> ModelStatus {
        ModelStatus::Downloaded {
            bytes: 64,
            path: "/cache/model".into(),
        }
    }

    fn panel<'a>(capable: Option<bool>, snapshot: Option<&'a CategorizationSnapshot>) -> Panel<'a> {
        Panel { capable, snapshot }
    }

    /// The controls on offer for `panel`, panicking when the section is not
    /// offering any - so every failure names what was drawn instead.
    fn controls(panel: &Panel<'_>) -> Vec<Action> {
        match section(panel) {
            Section::Ready { actions, .. } => actions,
            other => panic!("expected the controls to be on offer, got {other:?}"),
        }
    }

    #[test]
    fn an_installation_that_cannot_categorize_offers_nothing() {
        let answered = snapshot(true);
        assert!(matches!(
            section(&panel(Some(false), Some(&answered))),
            Section::Unsupported
        ));
        assert!(matches!(
            section(&panel(Some(false), None)),
            Section::Unsupported
        ));
    }

    #[test]
    fn the_controls_wait_for_the_reads_to_land() {
        let answered = snapshot(true);
        assert!(matches!(
            section(&panel(None, Some(&answered))),
            Section::Checking
        ));
        assert!(matches!(
            section(&panel(Some(true), None)),
            Section::Checking
        ));
    }

    /// The switch only installs the model: no model action is offered until the
    /// model is on disk, and each one is its own button after that.
    #[test]
    fn each_model_action_is_a_button_once_the_model_is_installed() {
        let mut answered = snapshot(false);
        answered.status.model = downloaded();
        assert_eq!(
            controls(&panel(Some(true), Some(&answered))),
            vec![Action::Toggle],
            "off: only the way in"
        );

        answered.enabled = true;
        answered.status.model = ModelStatus::Absent;
        assert_eq!(
            controls(&panel(Some(true), Some(&answered))),
            vec![Action::Toggle, Action::PublishRails],
            "on but still downloading: rails need no model, couch detection does"
        );

        answered.status.model = downloaded();
        assert_eq!(
            controls(&panel(Some(true), Some(&answered))),
            vec![Action::Toggle, Action::PublishRails, Action::DetectCouch]
        );
    }

    #[test]
    fn the_switch_stays_live_during_a_job_but_the_model_actions_do_not() {
        let mut answered = snapshot(true);
        answered.status.model = downloaded();
        answered.status.running = true;
        let running = panel(Some(true), Some(&answered));

        assert!(pressable(Action::Toggle, &running));
        for action in [Action::PublishRails, Action::DetectCouch] {
            assert!(!pressable(action, &running), "{action:?} waits for the job");
        }
    }

    #[test]
    fn an_unsupported_installation_makes_nothing_pressable() {
        let answered = snapshot(true);
        let unsupported = panel(Some(false), Some(&answered));
        for action in [Action::Toggle, Action::PublishRails, Action::DetectCouch] {
            assert!(!pressable(action, &unsupported), "{action:?}");
        }
    }
}
