use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::theme;
use cosmic::theme::Button;
use cosmic::widget::{list, row, toggler};

use crate::fl;
use crate::providers::daemon::{CategorizationPhase, CategorizationSnapshot, ModelStatus};
use crate::style::{primary_action_button_class, standard_button_class};

use super::super::{
    action_button, body_line, button_row, caption_line, described_item, error_line, titled_section,
};
use super::{Action, Message, Panel, Section, State, pressable, section};

const DOWNLOAD_SIZE: &str = "~850 MB";

pub fn view(state: &State) -> Element<'_, Message> {
    let Spacing { space_s, .. } = theme::spacing();
    let panel = Panel {
        capable: state.capable,
        snapshot: state.snapshot.as_ref(),
    };
    let card = titled_section(fl!("categorization-heading"));
    let (snapshot, actions) = match section(&panel) {
        Section::Unsupported => {
            return card
                .add(body_line(fl!("categorization-unsupported")))
                .add(caption_line(fl!("categorization-unsupported-hint")))
                .into();
        }
        Section::Checking => {
            return card
                .add(body_line(fl!("categorization-reading-status")))
                .into();
        }
        Section::Ready { snapshot, actions } => (snapshot, actions),
    };

    // The whole row is pressable so the pad need not land on the toggle itself.
    let mut card = card
        .add(
            list::button(described_item(
                fl!("categorization-toggle"),
                fl!("categorization-intro", size = DOWNLOAD_SIZE),
                toggler(snapshot.enabled).on_toggle(Message::SetEnabled),
            ))
            .on_press(action_message(Action::Toggle, snapshot)),
        )
        .add(body_line(status_line(snapshot)))
        .add_maybe(
            state
                .notice
                .as_deref()
                .map(|reason| error_line(fl!("categorization-failed", reason = reason))),
        );
    card = card.add_maybe(
        snapshot
            .enabled
            .then(|| caption_line(model_line(&snapshot.status.model))),
    );
    card = card.add_maybe(snapshot.report.as_ref().map(|report| {
        caption_line(fl!(
            "categorization-report",
            apps = report.app_count,
            categories = report.categories.len(),
            unclassified = report.unclassified.len()
        ))
    }));

    let mut buttons = row![].spacing(space_s);
    let mut has_buttons = false;
    for action in actions {
        if action == Action::Toggle {
            continue;
        }
        buttons = buttons.push(action_button(
            action_label(action),
            action_message(action, snapshot),
            action_class(action),
            pressable(action, &panel),
        ));
        has_buttons = true;
    }
    // An empty row would still claim spacing beneath the switch.
    if has_buttons {
        card = card.add(button_row(buttons));
    }
    card.into()
}

fn action_class(action: Action) -> Button {
    match action {
        Action::PublishRails | Action::DetectCouch => primary_action_button_class(),
        Action::Toggle => standard_button_class(),
    }
}

fn action_message(action: Action, snapshot: &CategorizationSnapshot) -> Message {
    match action {
        Action::Toggle => Message::SetEnabled(!snapshot.enabled),
        Action::PublishRails => Message::PublishRails,
        Action::DetectCouch => Message::DetectCouch,
    }
}

fn action_label(action: Action) -> String {
    match action {
        Action::Toggle => fl!("categorization-toggle"),
        Action::PublishRails => fl!("categorization-publish-rails"),
        Action::DetectCouch => fl!("categorization-detect-couch"),
    }
}

fn status_line(snapshot: &CategorizationSnapshot) -> String {
    let status = &snapshot.status;
    if status.running {
        return match status.phase {
            CategorizationPhase::Downloading => fl!("categorization-phase-downloading"),
            CategorizationPhase::Loading => fl!("categorization-phase-loading"),
            CategorizationPhase::Scanning => fl!(
                "categorization-phase-scanning",
                completed = status.completed,
                total = status.total
            ),
            CategorizationPhase::Idle => fl!("categorization-reading-status"),
        };
    }
    // "Ready" would suggest an enabled feature, so off takes precedence.
    if !snapshot.enabled {
        return fl!("categorization-off");
    }
    match status.last_error.as_deref() {
        Some(reason) => fl!("categorization-failed", reason = reason),
        None => match status.last_completed_at.as_deref() {
            Some(when) => fl!("categorization-last-run", when = when),
            None => fl!("categorization-ready"),
        },
    }
}

fn model_line(model: &ModelStatus) -> String {
    match model {
        ModelStatus::Absent => fl!("categorization-model-absent"),
        ModelStatus::Downloaded { bytes, path } => fl!(
            "categorization-model-downloaded",
            size = crate::system_status::human_size(*bytes),
            path = path.display().to_string()
        ),
        ModelStatus::External { path } => fl!(
            "categorization-model-external",
            path = path.display().to_string()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn the_off_switch_says_the_feature_is_off() {
        assert_eq!(status_line(&snapshot(false)), fl!("categorization-off"));
    }

    #[test]
    fn a_completed_scan_reads_as_the_last_run() {
        let mut answered = snapshot(true);
        answered.status.last_completed_at = Some("2026-10-07T10:00:00Z".to_owned());
        assert_eq!(
            status_line(&answered),
            fl!("categorization-last-run", when = "2026-10-07T10:00:00Z")
        );
    }

    #[test]
    fn controls_emit_the_local_event_for_the_action_shown() {
        for enabled in [false, true] {
            assert!(matches!(
                action_message(Action::Toggle, &snapshot(enabled)),
                Message::SetEnabled(on) if on == !enabled
            ));
        }
        assert!(matches!(
            action_message(Action::PublishRails, &snapshot(true)),
            Message::PublishRails
        ));
        assert!(matches!(
            action_message(Action::DetectCouch, &snapshot(true)),
            Message::DetectCouch
        ));
    }
}
