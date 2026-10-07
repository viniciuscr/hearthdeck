//! The Stremio half of the settings screen: linking an account, and letting it go.
//!
//! Nothing on this screen ever holds a credential. Linking is a code the user
//! approves on a device where they are already signed in, so there is no password
//! to type on a television and none to store; the daemon keeps the session the link
//! produces, and this screen is only ever told whether one exists.
//!
//! The controls follow the same shape as the categorization half: one list that the
//! renderer, the gamepad's focus walk and the confirm handler all read, so the
//! drawn controls and the handled ones cannot drift apart.

use std::sync::LazyLock;

use cosmic::Element;
use cosmic::iced::{Alignment, Length};
use cosmic::theme;
use cosmic::theme::Button;
use cosmic::widget::{Id, button, column, container, row, text};

use crate::app::Message;
use crate::fl;
use crate::providers::daemon::StremioConnection;
use crate::style::{TEXT_HEADER, destructive_button_class, primary_action_button_class};

static LINK_ID: LazyLock<Id> = LazyLock::new(|| Id::new("settings-stremio-link"));
static UNLINK_ID: LazyLock<Id> = LazyLock::new(|| Id::new("settings-stremio-unlink"));

/// One control this section offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Ask the daemon for a code, and show it.
    Link,
    /// Forget the account, and the rows it published.
    Unlink,
}

impl Action {
    const ALL: [Action; 2] = [Action::Link, Action::Unlink];

    /// The widget id the control carries, so focus can be addressed to it.
    pub fn id(self) -> &'static Id {
        match self {
            Self::Link => &LINK_ID,
            Self::Unlink => &UNLINK_ID,
        }
    }

    fn label(self) -> String {
        match self {
            Self::Link => fl!("stremio-link"),
            Self::Unlink => fl!("stremio-unlink"),
        }
    }

    /// The message the application handles when this is pressed.
    pub fn message(self) -> Message {
        match self {
            Self::Link => Message::StartStremioLink,
            Self::Unlink => Message::UnlinkStremio,
        }
    }

    fn class(self) -> Button {
        match self {
            Self::Link => primary_action_button_class(),
            Self::Unlink => destructive_button_class(),
        }
    }
}

/// The link a user is in the middle of approving.
#[derive(Clone, Debug)]
pub struct Pending {
    pub code: String,
    pub link: String,
    pub seconds_left: u64,
}

/// What the section shows. Gathered by the caller, like the categorization panel.
pub struct Panel {
    /// The linked account. `None` covers both "not linked" and "not read yet",
    /// which `reading` tells apart.
    pub connection: Option<StremioConnection>,
    /// A read is in flight, so the status line says so instead of claiming that
    /// nothing is linked.
    pub reading: bool,
    /// The link being approved, if any.
    pub pending: Option<Pending>,
    /// A request is in flight, so nothing should be pressed twice.
    pub busy: bool,
    /// The last failure, in the daemon's or Stremio's own words.
    pub error: Option<String>,
}

/// The controls on offer, in drawing order.
///
/// Linking stays available even once an account is linked: re-linking is how a
/// session Stremio has revoked gets repaired without anybody needing support. It is
/// also the way out of a code that has been on screen too long — pressing it again
/// asks for a fresh one, which replaces the pending link the daemon is holding.
pub fn actions(panel: &Panel) -> Vec<Action> {
    let mut actions = vec![Action::Link];
    if panel.connection.is_some() {
        actions.push(Action::Unlink);
    }
    actions
}

/// The action a widget id stands for, or `None` when the id belongs to something
/// else. Widget ids are unique across the app, which is how the confirm handler
/// recognises its own controls.
pub fn action(id: &Id) -> Option<Action> {
    Action::ALL.into_iter().find(|action| action.id() == id)
}

/// Whether pressing `action` would be accepted in the state `panel` describes.
pub fn pressable(action: Action, panel: &Panel) -> bool {
    match action {
        Action::Link => !panel.busy,
        // Only offered when there is something to forget, and never while another
        // request is still in flight.
        Action::Unlink => !panel.busy && panel.connection.is_some(),
    }
}

pub fn view(panel: &Panel) -> Element<'static, Message> {
    let spacing = theme::spacing();
    let mut body = column![
        text(fl!("stremio-heading")).size(TEXT_HEADER),
        text::body(fl!("stremio-intro")),
    ]
    .spacing(spacing.space_s)
    .width(Length::Fill);

    // The code replaces the status line rather than sitting under it: while a link
    // is pending, the code is the only thing on this section worth reading.
    match &panel.pending {
        Some(pending) => body = body.push(pending_block(pending)),
        None => body = body.push(text::body(status_line(panel))),
    }

    if let Some(error) = &panel.error {
        body = body.push(text::caption(fl!("stremio-failed", reason = error.clone())));
    }

    let mut controls = row![].spacing(spacing.space_s).align_y(Alignment::Center);
    for action in actions(panel) {
        controls = controls.push(action_button(action, panel));
    }
    body.push(controls).into()
}

/// The code, the link, and how long is left.
///
/// The code is drawn large on purpose: this is read off a television and typed into
/// a phone, which is the whole reason the flow exists instead of a password field.
fn pending_block(pending: &Pending) -> Element<'static, Message> {
    let spacing = theme::spacing();
    container(
        column![
            text::body(fl!("stremio-pending", seconds = pending.seconds_left)),
            text(pending.code.clone()).size(crate::style::TEXT_TITLE),
            text::caption(pending.link.clone()),
        ]
        .spacing(spacing.space_xs),
    )
    .padding(spacing.space_s)
    .width(Length::Fill)
    .into()
}

fn status_line(panel: &Panel) -> String {
    if panel.reading {
        return fl!("stremio-reading");
    }
    match &panel.connection {
        Some(connection) => fl!("stremio-linked", when = connection.linked_at.clone()),
        None => fl!("stremio-not-linked"),
    }
}

fn action_button(action: Action, panel: &Panel) -> Element<'static, Message> {
    let press = pressable(action, panel).then(|| action.message());
    button::custom(text::body(action.label()))
        .id(action.id().clone())
        .padding(theme::spacing().space_s)
        .class(action.class())
        .on_press_maybe(press)
        .into()
}

#[cfg(test)]
mod tests {
    use super::{Action, Panel, action, actions, pressable};
    use crate::providers::daemon::StremioConnection;

    fn connection() -> StremioConnection {
        StremioConnection {
            linked_at: "2026-10-07T10:00:00Z".to_owned(),
        }
    }

    fn panel(connection: Option<StremioConnection>, busy: bool) -> Panel {
        Panel {
            connection,
            reading: false,
            pending: None,
            busy,
            error: None,
        }
    }

    /// Unlinking is offered only when there is an account to forget, and linking is
    /// always offered so a revoked session can be repaired.
    #[test]
    fn unlink_appears_only_for_a_linked_account() {
        assert_eq!(actions(&panel(None, false)), vec![Action::Link]);
        assert_eq!(
            actions(&panel(Some(connection()), false)),
            vec![Action::Link, Action::Unlink]
        );
    }

    /// A request in flight cannot be started twice, whichever control is focused.
    #[test]
    fn nothing_is_pressable_while_a_request_is_in_flight() {
        let idle = panel(Some(connection()), false);
        assert!(pressable(Action::Link, &idle));
        assert!(pressable(Action::Unlink, &idle));

        let busy = panel(Some(connection()), true);
        assert!(!pressable(Action::Link, &busy));
        assert!(!pressable(Action::Unlink, &busy));
    }

    /// An unlinked account has nothing to press but Link, so the confirm handler's
    /// lookup has to agree with the list the row was drawn from.
    #[test]
    fn an_unlinked_account_cannot_press_unlink() {
        let unlinked = panel(None, false);
        assert!(!pressable(Action::Unlink, &unlinked));
        assert_eq!(action(Action::Unlink.id()), Some(Action::Unlink));
    }
}
