use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::Length;
use cosmic::theme;
use cosmic::theme::Button;
use cosmic::widget::{column, container, row, text};

use crate::fl;
use crate::style::{TEXT_TITLE, destructive_button_class, primary_action_button_class};

use super::super::{
    action_button, body_line, button_row, caption_line, error_line, summary, titled_section,
};
use super::{Action, Flow, Message, Pending, actions, pressable};

pub fn view(flow: &Flow) -> Element<'_, Message> {
    let Spacing { space_s, .. } = theme::spacing();
    let section = titled_section(fl!("stremio-heading"));

    // The pending code replaces the account line until approval finishes.
    let mut section = match flow.pending.as_ref() {
        Some(pending) => section
            .add(body_line(fl!("stremio-intro")))
            .add(pending_block(pending)),
        None => section.add(summary(status_line(flow), fl!("stremio-intro"))),
    };
    section = section.add_maybe(
        flow.error
            .as_deref()
            .map(|error| error_line(fl!("stremio-failed", reason = error))),
    );
    let mut buttons = row![].spacing(space_s);
    for action in actions(flow) {
        buttons = buttons.push(action_button(
            action_label(action),
            action_message(action),
            action_class(action),
            pressable(action, flow),
        ));
    }
    section.add(button_row(buttons)).into()
}

fn pending_block(pending: &Pending) -> Element<'_, Message> {
    let Spacing {
        space_xs, space_s, ..
    } = theme::spacing();
    container(
        column![
            body_line(fl!("stremio-pending", seconds = pending.seconds_left)),
            text::title1(pending.code.as_str()).size(TEXT_TITLE),
            caption_line(pending.link.clone()),
        ]
        .spacing(space_xs),
    )
    .padding(space_s)
    .width(Length::Fill)
    .class(theme::Container::Primary)
    .into()
}

fn status_line(flow: &Flow) -> String {
    if flow.reading {
        return fl!("stremio-reading");
    }
    // Expiry explains the last attempt even if an older account is still linked.
    if flow.expired {
        return fl!("stremio-link-expired");
    }
    match flow.connection.as_ref() {
        Some(connection) => fl!("stremio-linked", when = connection.linked_at.clone()),
        None => fl!("stremio-not-linked"),
    }
}

fn action_message(action: Action) -> Message {
    match action {
        Action::Link => Message::LinkPressed,
        Action::Unlink => Message::UnlinkPressed,
    }
}

fn action_label(action: Action) -> String {
    match action {
        Action::Link => fl!("stremio-link"),
        Action::Unlink => fl!("stremio-unlink"),
    }
}

fn action_class(action: Action) -> Button {
    match action {
        Action::Link => primary_action_button_class(),
        Action::Unlink => destructive_button_class(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::daemon::StremioConnection;

    #[test]
    fn checking_and_expiry_take_precedence_over_the_last_linked_account() {
        let mut flow = Flow {
            connection: Some(StremioConnection {
                linked_at: "2026-11-03T10:00:00Z".into(),
            }),
            reading: true,
            expired: true,
            ..Flow::default()
        };
        assert_eq!(status_line(&flow), fl!("stremio-reading"));
        flow.reading = false;
        assert_eq!(status_line(&flow), fl!("stremio-link-expired"));
        flow.expired = false;
        assert_eq!(
            status_line(&flow),
            fl!("stremio-linked", when = "2026-11-03T10:00:00Z")
        );
        flow.connection = None;
        assert_eq!(status_line(&flow), fl!("stremio-not-linked"));
    }

    #[test]
    fn controls_emit_the_local_event_for_the_action_shown() {
        assert!(matches!(action_message(Action::Link), Message::LinkPressed));
        assert!(matches!(
            action_message(Action::Unlink),
            Message::UnlinkPressed
        ));
    }
}
