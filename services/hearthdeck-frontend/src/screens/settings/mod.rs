//! The settings screen: what Hearthdeck is allowed to do to the library.
//!
//! A sibling destination like the dashboard, and built on libcosmic's own
//! settings surface - a column of titled sections, each a list of rows with its
//! control on the right - so it reads as a COSMIC settings page rather than a
//! screen that hand-drew rows to look like one.

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::Length;
use cosmic::theme;
use cosmic::theme::Button;
use cosmic::widget::{
    ListColumn, button, column, container, list, row, scrollable, settings, space, text, toggler,
};

use crate::app::Message;
use crate::fl;
use crate::providers::daemon::{CategorizationPhase, CategorizationSnapshot, ModelStatus};
use crate::style::{
    SETTINGS_MAX_WIDTH, TEXT_BODY, TEXT_CAPTION, TEXT_HEADER, TEXT_LARGE, TEXT_TITLE,
    content_horizontal_padding, destructive_button_class, error_text_class,
    primary_action_button_class, standard_button_class,
};
use crate::widgets::top_bar;

pub mod categorization;
pub mod romm;
pub mod stremio_link;

/// What the first scan costs. Named once, so the sentence that warns about it
/// cannot drift from the number.
const DOWNLOAD_SIZE: &str = "~850 MB";

/// What the screen draws.
pub struct Settings<'a> {
    /// The daemon's answer, once it has landed. Until then the screen says it is
    /// checking, rather than showing a switch in a state nobody reported.
    pub categorization: Option<&'a CategorizationSnapshot>,
    /// Whether the deployment provides categorization at all, from /v1/health.
    /// None until the health reply lands.
    pub capable: Option<bool>,
    /// Why the last action was refused, for the section's own status text.
    pub notice: Option<&'a str>,
    /// The Stremio link flow: the linked account, the code being approved
    /// elsewhere, and whether a request is in flight. The flow owns all of it;
    /// this section reads it and names the request each control makes.
    pub stremio: &'a stremio_link::Flow,
    pub romm: &'a romm::Form,
}

/// The page: the shared top bar, then the sections.
#[must_use]
pub fn view<'a>(
    state: &Settings<'a>,
    bar: &top_bar::TopBar<'_>,
    on_navigate: fn(top_bar::Destination) -> Message,
) -> Element<'a, Message> {
    let Spacing {
        space_l, space_xl, ..
    } = theme::spacing();
    let padding = content_horizontal_padding();

    let sections = column![
        text::title1(fl!("settings-title")).size(TEXT_TITLE),
        scrollable(settings::view_column(vec![
            library_and_dashboard(),
            categorization(state),
            stremio(state),
            romm::view(state.romm)
        ]))
        .width(Length::Fill)
        .height(Length::Fill)
        .scrollbar_width(0)
        .scroller_width(0),
    ]
    .spacing(space_l)
    .max_width(SETTINGS_MAX_WIDTH)
    .height(Length::Fill);
    // One centered column, like COSMIC Settings: rows stay readable on a wide TV.
    let sections = container(sections)
        .center_x(Length::Fill)
        .padding([space_xl, padding]);

    // The bar sits where the dashboard's does: a sibling destination wears the
    // same chrome in the same place, so moving between them moves nothing.
    column![
        container(top_bar::view(bar, on_navigate)).padding([space_l, padding, 0, padding]),
        sections,
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// The library's own tabs and the dashboard's rails: the controls that put them
/// back as Hearthdeck derives them.
///
/// Its own section, and always on offer: both can go wrong whether or not the
/// local model is installed, or even supported, and the way out must not depend
/// on a feature the user may never have turned on.
fn library_and_dashboard<'a>() -> Element<'a, Message> {
    let Spacing { space_s, .. } = theme::spacing();
    let buttons = row![
        action_button(
            fl!("categories-restore"),
            Message::RestoreDefaultCategories,
            standard_button_class(),
            true,
        ),
        action_button(
            fl!("dashboard-restore"),
            Message::RestoreDashboard,
            standard_button_class(),
            true,
        ),
    ]
    .spacing(space_s);
    titled_section(fl!("categories-heading"))
        .add(body_line(fl!("categories-intro")))
        .add(button_row(buttons))
        .into()
}

// ---------------------------------------------------------------------------
// The page's own building blocks
//
// libcosmic's settings widgets are sized for a laptop: `section` titles at 14px
// and `item` labels at 14px over a 12px description. A television is read from
// a sofa, so the page keeps libcosmic's structure - the card, the dividers, the
// whole-row press behaviour - and supplies the type scale.
// ---------------------------------------------------------------------------

/// A section with its title in the page's own type scale.
///
/// The rows get a little more vertical air than the 8px default: the taller
/// labels would otherwise sit nearly on the dividers between them.
fn titled_section<'a>(title: String) -> settings::Section<'a, Message> {
    let Spacing {
        space_s, space_m, ..
    } = theme::spacing();
    settings::section::with_column(ListColumn::default().list_item_padding([space_s, space_m]))
        .header(text::title4(title).size(TEXT_HEADER))
}

/// One settings row: its label on the left, its control against the right edge.
///
/// Built on [`settings::item_row`], which is the spacing and alignment
/// `settings::item` uses, because `item` bakes its desktop label size into the
/// row with no way to override it. The horizontal spacer is the same one
/// `settings::item` puts between the two, so a wide control (an input) fills the
/// space the label does not need.
fn item<'a>(label: String, control: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    settings::item_row(vec![
        text::body(label).size(TEXT_LARGE).into(),
        space::horizontal().into(),
        control.into(),
    ])
    .into()
}

/// A row whose label carries a sentence beneath it, for a control that needs
/// explaining: the shape `settings::item::builder` draws, at the page's sizes.
fn described_item<'a>(
    label: String,
    description: String,
    control: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let Spacing { space_xxs, .. } = theme::spacing();
    settings::item_row(vec![
        column![
            text::body(label).size(TEXT_LARGE),
            text::body(description).size(TEXT_BODY),
        ]
        .spacing(space_xxs)
        .width(Length::Fill)
        .into(),
        control.into(),
    ])
    .into()
}

/// A line of the section's own words: a state, a notice, an explanation.
fn body_line<'a>(line: String) -> Element<'a, Message> {
    text::body(line).size(TEXT_BODY).into()
}

/// A quieter line: fine print the section only reports (a model path, a count).
fn caption_line<'a>(line: String) -> Element<'a, Message> {
    text::caption(line).size(TEXT_CAPTION).into()
}

/// A line that reports a failure: the theme's destructive color, so it does not
/// read as one more quiet note.
fn error_line<'a>(line: String) -> Element<'a, Message> {
    text::body(line)
        .size(TEXT_BODY)
        .class(error_text_class())
        .into()
}

/// A section's opening row: where things stand, with the why beneath it in the
/// quieter style, the way COSMIC Settings describes a row.
fn summary<'a>(status: String, intro: String) -> Element<'a, Message> {
    let Spacing { space_xxs, .. } = theme::spacing();
    column![
        text::body(status).size(TEXT_LARGE),
        text::body(intro).size(TEXT_BODY),
    ]
    .spacing(space_xxs)
    .into()
}

/// A section's buttons, set against its right edge like a COSMIC dialog's.
fn button_row<'a>(buttons: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(buttons).align_right(Length::Fill).into()
}

/// The smart-categories section: the local model that sorts the library, and
/// every switch and action it has.
///
/// Every line is the daemon's own answer - a scan's phase, the model it has on
/// disk, whether the last one failed - so the screen never claims more than it
/// was told. Nothing heavy runs until the switch is on, which is the promise the
/// intro makes and the reason the off state says so plainly.
fn categorization<'a>(state: &Settings<'a>) -> Element<'a, Message> {
    let Spacing { space_s, .. } = theme::spacing();
    let panel = categorization::Panel {
        capable: state.capable,
        snapshot: state.categorization,
    };

    let section = titled_section(fl!("categorization-heading"));

    let (snapshot, actions) = match categorization::section(&panel) {
        // A feature this installation does not have is said plainly, with what it
        // would need - nothing on offer, because nothing here could work.
        categorization::Section::Unsupported => {
            return section
                .add(body_line(fl!("categorization-unsupported")))
                .add(caption_line(fl!("categorization-unsupported-hint")))
                .into();
        }
        categorization::Section::Checking => {
            return section
                .add(body_line(fl!("categorization-reading-status")))
                .into();
        }
        categorization::Section::Ready { snapshot, actions } => (snapshot, actions),
    };

    // The switch is a whole pressable row, so the controller can flip it from
    // anywhere on the line, not only by landing on the toggle itself.
    let mut section = section
        .add(
            list::button(described_item(
                fl!("categorization-toggle"),
                fl!("categorization-intro", size = DOWNLOAD_SIZE),
                toggler(snapshot.enabled).on_toggle(Message::SetCategorization),
            ))
            .on_press(Message::SetCategorization(!snapshot.enabled)),
        )
        .add(body_line(status_line(snapshot)))
        .add_maybe(state.notice.map(notice_line));

    // Where the model lives is the user's business only while the feature is
    // on; in the off state the promise is that nothing runs, and a model line
    // with no way to act on it would be noise.
    section = section.add_maybe(
        snapshot
            .enabled
            .then(|| caption_line(model_line(&snapshot.status.model))),
    );

    section = section.add_maybe(snapshot.report.as_ref().map(|report| {
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
        // The switch is its own row above, so it is the one offer that is not a
        // button down here (which is why the `action_*` maps still cover it:
        // the match must stay exhaustive).
        if action == categorization::Action::Toggle {
            continue;
        }
        buttons = buttons.push(action_button(
            action_label(action),
            action_message(action, snapshot),
            action_class(action),
            categorization::pressable(action, &panel),
        ));
        has_buttons = true;
    }

    // A row with nothing in it still claims its spacing; when the switch is
    // the only offer, the section simply ends above it.
    if has_buttons {
        section = section.add(button_row(buttons));
    }
    section.into()
}

/// The Stremio section: the account, the code being approved on another device,
/// and the two controls that move between them.
///
/// Nothing here ever holds a credential - the code is approved where the user is
/// already signed in, and the daemon keeps the session the link produces - so
/// this section only ever says whether a session exists. Every line is the
/// flow's own state: a status line here would be a second place guessing at what
/// the daemon said.
fn stremio<'a>(state: &Settings<'a>) -> Element<'a, Message> {
    let Spacing { space_s, .. } = theme::spacing();
    let flow = state.stremio;

    let section = titled_section(fl!("stremio-heading"));

    // The code replaces the status line rather than sitting under it: while a
    // link is pending, the code is the only thing on this section worth reading.
    let mut section = match flow.pending.as_ref() {
        Some(pending) => section
            .add(body_line(fl!("stremio-intro")))
            .add(pending_block(pending)),
        None => section.add(summary(stremio_status_line(flow), fl!("stremio-intro"))),
    };

    // The failure is the daemon's or Stremio's own words, and it sits beside the
    // state rather than replacing it: a refused request did not change what the
    // account is.
    section = section.add_maybe(
        flow.error
            .as_deref()
            .map(|error| error_line(fl!("stremio-failed", reason = error))),
    );

    let mut buttons = row![].spacing(space_s);
    for action in stremio_link::actions(flow) {
        buttons = buttons.push(action_button(
            stremio_action_label(action),
            stremio_action_message(action),
            stremio_action_class(action),
            stremio_link::pressable(action, flow),
        ));
    }

    section.add(button_row(buttons)).into()
}

/// The code, the link that carries it, and how long is left. The code is drawn
/// large on purpose: this is read off a television and typed into a phone, which
/// is the whole reason the flow exists instead of a password field.
fn pending_block<'a>(pending: &stremio_link::Pending) -> Element<'a, Message> {
    let Spacing {
        space_xs, space_s, ..
    } = theme::spacing();
    container(
        column![
            body_line(fl!("stremio-pending", seconds = pending.seconds_left)),
            text::title1(pending.code.clone()).size(TEXT_TITLE),
            caption_line(pending.link.clone()),
        ]
        .spacing(space_xs),
    )
    .padding(space_s)
    .width(Length::Fill)
    .class(theme::Container::Primary)
    .into()
}

/// What the section says when no code is on screen: an account, a read in
/// flight, an expired code, or nothing linked.
fn stremio_status_line(flow: &stremio_link::Flow) -> String {
    // Checking outranks the last answer: the section must not claim what the
    // account was while it is asking what it is.
    if flow.reading {
        return fl!("stremio-reading");
    }
    // An expired code is what the user last acted on, so it is the news this
    // line carries until the next request - "no account linked" is true but
    // does not say what happened to the code they were typing.
    if flow.expired {
        return fl!("stremio-link-expired");
    }
    match flow.connection.as_ref() {
        Some(connection) => fl!("stremio-linked", when = connection.linked_at.clone()),
        None => fl!("stremio-not-linked"),
    }
}

/// What pressing `action` asks the shell for. The decision is the flow's; this
/// only names the request.
fn stremio_action_message(action: stremio_link::Action) -> Message {
    match action {
        stremio_link::Action::Link => Message::StartStremioLink,
        stremio_link::Action::Unlink => Message::UnlinkStremio,
    }
}

/// The control's own words.
fn stremio_action_label(action: stremio_link::Action) -> String {
    match action {
        stremio_link::Action::Link => fl!("stremio-link"),
        stremio_link::Action::Unlink => fl!("stremio-unlink"),
    }
}

/// One action in the section's button row, so padding cannot drift apart as
/// more are added beside it. The class is passed in rather than fixed here:
/// each control wears the class its role earns (the two `*_action_class` maps).
///
/// A control the state cannot honour is drawn disabled rather than dropped: the
/// row is the same row in every phase, so nothing moves under the user's hand.
fn action_button<'a>(
    label: String,
    message: Message,
    class: Button,
    enabled: bool,
) -> Element<'a, Message> {
    let Spacing { space_s, .. } = theme::spacing();
    button::custom(text::body(label).size(TEXT_BODY))
        .padding([space_s, space_s])
        .class(class)
        .on_press_maybe(enabled.then_some(message))
        .into()
}

/// What the control wears, by what it does: the model actions are the primary
/// button, and the switch stays ordinary.
fn action_class(action: categorization::Action) -> Button {
    match action {
        categorization::Action::PublishRails | categorization::Action::DetectCouch => {
            primary_action_button_class()
        }
        categorization::Action::Toggle => standard_button_class(),
    }
}

/// The same rule for the Stremio controls: linking is the affirmative action,
/// unlinking removes the account.
fn stremio_action_class(action: stremio_link::Action) -> Button {
    match action {
        stremio_link::Action::Link => primary_action_button_class(),
        stremio_link::Action::Unlink => destructive_button_class(),
    }
}

/// What pressing `action` asks the shell for. The decision is `update`'s; this
/// only names the request.
fn action_message(action: categorization::Action, snapshot: &CategorizationSnapshot) -> Message {
    match action {
        categorization::Action::Toggle => Message::SetCategorization(!snapshot.enabled),
        categorization::Action::PublishRails => Message::ScanCategorization,
        categorization::Action::DetectCouch => Message::DetectCouchFriendly,
    }
}

/// The button's own words. The switch reads as the item label above it, which is
/// why it needs a label even though its row is the toggler's.
fn action_label(action: categorization::Action) -> String {
    match action {
        categorization::Action::Toggle => fl!("categorization-toggle"),
        categorization::Action::PublishRails => fl!("categorization-publish-rails"),
        categorization::Action::DetectCouch => fl!("categorization-detect-couch"),
    }
}

/// Why the last action was refused, in the daemon's own words.
fn notice_line<'a>(reason: &str) -> Element<'a, Message> {
    error_line(fl!("categorization-failed", reason = reason))
}

/// Where the scan has got to, in the daemon's own words.
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
    // The switch is the headline: off means none of the rest is running, so
    // "ready" - which reads as "on, waiting for a scan" - would be a lie.
    if !snapshot.enabled {
        return fl!("categorization-off");
    }
    match status.last_error.as_deref() {
        Some(reason) => fl!("categorization-failed", reason = reason),
        // A scan that already ran is not "ready to run": the daemon says when
        // the last one finished, so that is the line instead of asking again.
        None => match status.last_completed_at.as_deref() {
            Some(when) => fl!("categorization-last-run", when = when),
            None => fl!("categorization-ready"),
        },
    }
}

/// Where the model is, and what it costs to keep.
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
    use super::status_line;
    use crate::fl;
    use crate::providers::daemon::{
        CategorizationPhase, CategorizationSnapshot, CategorizationStatus, ModelStatus,
    };

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

    /// The switch being off is the first thing the section says: "ready"
    /// reads as "on, waiting for a scan", which is exactly what is not true.
    #[test]
    fn the_off_switch_says_the_feature_is_off() {
        let snapshot = snapshot(false);

        assert_eq!(status_line(&snapshot), fl!("categorization-off"));
    }

    /// A scan that already ran is not "ready to run": the daemon reports when
    /// the last one finished, and the line says that instead of asking again.
    #[test]
    fn a_completed_scan_reads_as_the_last_run() {
        let mut answered = snapshot(true);
        answered.status.last_completed_at = Some("2026-10-07T10:00:00Z".to_owned());

        assert_eq!(
            status_line(&answered),
            fl!("categorization-last-run", when = "2026-10-07T10:00:00Z")
        );
    }
}
