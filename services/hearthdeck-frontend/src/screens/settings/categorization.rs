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

/// One control the section offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The opt-in itself: turning it on is what installs and activates the
    /// model, and turning it off restores everything the model changed.
    Toggle,
    /// Classify the library with the model.
    Categorize,
    /// Publish the last run's rails as dashboard collections, without calling
    /// the model again.
    CreateCollections,
    /// Give the tabs back to the ones derived from the entries themselves.
    Reset,
    /// Free the downloaded checkpoint.
    RemoveModel,
}

/// What the last action did, for the section to say in its own status text.
///
/// The toast layer that would normally carry these outcomes does not exist yet,
/// so the section's own status line is where the user reads them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// The last run's rails were published: this many dashboard collections.
    Created(usize),
    /// The publish was accepted, but the last run implies no dashboard
    /// collection - which is not the same as "updated none", and says so.
    NoCollections,
    /// An action was refused, with the daemon's own reason.
    Failed(String),
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
/// Each is added when the thing it acts on exists: the switch is the way in and
/// is always drawn, the model actions need the opt-in, the two that read the
/// report need a report to read, and the removal needs a checkpoint this client
/// downloaded - the deployment's own model is never ours to delete.
fn actions(snapshot: &CategorizationSnapshot) -> Vec<Action> {
    let mut actions = vec![Action::Toggle];
    if !snapshot.enabled {
        return actions;
    }
    actions.push(Action::Categorize);
    if snapshot.report.is_some() {
        actions.push(Action::CreateCollections);
        actions.push(Action::Reset);
    }
    if matches!(snapshot.status.model, ModelStatus::Downloaded { .. }) {
        actions.push(Action::RemoveModel);
    }
    actions
}

/// Whether pressing `action` would be accepted in the state `panel` describes.
/// Until the pad's focus walk reaches this screen, the shell uses this to
/// decide whether a control is drawn enabled; a control that could not be
/// confirmed is drawn disabled rather than focused and dead.
///
/// Being on offer is not enough on its own: a scan in flight holds back
/// everything that reads or writes the checkpoint, while the switch and the
/// restore are the daemon's to honour either way.
pub fn pressable(action: Action, panel: &Panel<'_>) -> bool {
    let Section::Ready { snapshot, actions } = section(panel) else {
        return false;
    };
    if !actions.contains(&action) {
        return false;
    }
    !snapshot.status.running || matches!(action, Action::Toggle | Action::Reset)
}

#[cfg(test)]
mod tests {
    use super::{Action, Panel, Section, pressable, section};
    use crate::providers::daemon::{
        CategorizationPhase, CategorizationSnapshot, CategorizationStatus, ModelStatus, ScanReport,
    };
    use serde::de::IgnoredAny;

    /// A categorization answer with nothing but the switch set.
    fn snapshot(enabled: bool) -> CategorizationSnapshot {
        CategorizationSnapshot {
            enabled,
            status: CategorizationStatus {
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

    /// Give the answer a last run to read: the two publish-and-restore buttons
    /// have nothing to act on before one exists.
    fn with_report(snapshot: &mut CategorizationSnapshot) {
        snapshot.report = Some(ScanReport {
            app_count: 3,
            categories: vec![IgnoredAny],
            unclassified: vec![IgnoredAny],
        });
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

    /// An installation that cannot run smart categorization says so plainly and
    /// offers nothing: a control that cannot work is worse than no control.
    #[test]
    fn an_installation_that_cannot_categorize_offers_nothing() {
        let answered = snapshot(true);
        assert!(
            matches!(
                section(&panel(Some(false), Some(&answered))),
                Section::Unsupported
            ),
            "the feature does not exist here, whatever the user wants"
        );
        assert!(
            matches!(section(&panel(Some(false), None)), Section::Unsupported),
            "and the state read cannot make it exist"
        );
    }

    /// Nothing is drawn in a state nobody reported: until the health reply and
    /// the daemon's answer have both landed, the section says it is checking.
    #[test]
    fn the_controls_wait_for_the_reads_to_land() {
        let answered = snapshot(true);
        assert!(
            matches!(section(&panel(None, Some(&answered))), Section::Checking),
            "the health reply has not said whether the feature exists"
        );
        assert!(
            matches!(section(&panel(Some(true), None)), Section::Checking),
            "and the daemon's own state has not been read either"
        );
    }

    /// Before anything is turned on there is only the switch to draw: the
    /// model actions have nothing to act on and must not pretend otherwise.
    #[test]
    fn the_switch_alone_is_offered_before_anything_is_turned_on() {
        let answered = snapshot(false);
        assert_eq!(
            controls(&panel(Some(true), Some(&answered))),
            vec![Action::Toggle],
            "enabled=false: only the way in is on offer"
        );
    }

    /// Publishing the last run and restoring the tabs both read the report, so
    /// neither appears before the library has been categorized once.
    #[test]
    fn publishing_and_restoring_wait_for_a_report() {
        let mut answered = snapshot(true);
        assert_eq!(
            controls(&panel(Some(true), Some(&answered))),
            vec![Action::Toggle, Action::Categorize],
            "no report: nothing to publish and nothing to undo"
        );

        with_report(&mut answered);
        assert_eq!(
            controls(&panel(Some(true), Some(&answered))),
            vec![
                Action::Toggle,
                Action::Categorize,
                Action::CreateCollections,
                Action::Reset
            ],
            "a report gives both something to read"
        );
    }

    /// The checkpoint can only be freed once one is on disk: the deployment's
    /// own model is never Hearthdeck's to delete.
    #[test]
    fn remove_model_waits_for_a_downloaded_checkpoint() {
        let mut answered = snapshot(true);
        let cases = [
            (ModelStatus::Absent, false, "nothing on disk to remove"),
            (
                ModelStatus::External {
                    path: "/opt/models".into(),
                },
                false,
                "the deployment's own model is not ours to delete",
            ),
            (
                ModelStatus::Downloaded {
                    bytes: 64,
                    path: "/cache/model".into(),
                },
                true,
                "a checkpoint we downloaded is ours to free",
            ),
        ];
        for (model, offered, why) in cases {
            let label = format!("{model:?}");
            answered.status.model = model;
            assert_eq!(
                controls(&panel(Some(true), Some(&answered))).contains(&Action::RemoveModel),
                offered,
                "model {label}: {why}"
            );
        }
    }

    /// The switch stays live while a scan runs - the daemon lets a running scan
    /// finish either way - but the actions that need the checkpoint are off
    /// until the run is done.
    #[test]
    fn the_switch_stays_live_during_a_scan_but_the_model_actions_do_not() {
        let mut answered = snapshot(true);
        with_report(&mut answered);
        answered.status.running = true;
        answered.status.model = ModelStatus::Downloaded {
            bytes: 64,
            path: "/cache/model".into(),
        };
        let running = panel(Some(true), Some(&answered));

        assert!(
            pressable(Action::Toggle, &running),
            "the switch is not the scan's to hold"
        );
        assert!(
            pressable(Action::Reset, &running),
            "restore defaults is a local change and stays available"
        );
        for action in [
            Action::Categorize,
            Action::CreateCollections,
            Action::RemoveModel,
        ] {
            assert!(
                !pressable(action, &running),
                "{action:?} needs the checkpoint to be free"
            );
        }
    }

    /// Nothing but the switch is pressable before the opt-in: the model actions
    /// act on something that does not exist yet.
    #[test]
    fn nothing_but_the_switch_is_pressable_before_the_opt_in() {
        let answered = snapshot(false);
        let off = panel(Some(true), Some(&answered));

        assert!(pressable(Action::Toggle, &off), "the switch is the way in");
        for action in [
            Action::Categorize,
            Action::CreateCollections,
            Action::Reset,
            Action::RemoveModel,
        ] {
            assert!(!pressable(action, &off), "{action:?} needs the opt-in");
        }
    }

    /// On an installation that cannot run the feature, nothing here can be
    /// confirmed - not even the switch, which would only fail.
    #[test]
    fn an_unsupported_installation_makes_nothing_pressable() {
        let answered = snapshot(true);
        let unsupported = panel(Some(false), Some(&answered));
        for action in [
            Action::Toggle,
            Action::Categorize,
            Action::CreateCollections,
            Action::Reset,
            Action::RemoveModel,
        ] {
            assert!(
                !pressable(action, &unsupported),
                "{action:?} cannot work here, so it cannot be pressed"
            );
        }
    }
}
