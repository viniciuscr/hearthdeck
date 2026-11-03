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
    ListColumn, button, column, container, row, scrollable, settings, space, text,
};

use crate::app::Message;
use crate::fl;
use crate::style::{
    SETTINGS_MAX_WIDTH, TEXT_BODY, TEXT_CAPTION, TEXT_HEADER, TEXT_LARGE, TEXT_TITLE,
    content_horizontal_padding, destructive_button_class, error_text_class,
    primary_action_button_class, standard_button_class,
};
use crate::widgets::top_bar;

pub mod categorization;
pub mod readiness;
pub mod romm;
pub mod stremio_link;

/// What the screen draws.
pub struct Settings<'a> {
    pub categorization: &'a categorization::State,
    /// The Stremio link flow: the linked account, the code being approved
    /// elsewhere, and whether a request is in flight. The flow owns all of it;
    /// this section reads it and names the request each control makes.
    pub stremio: &'a stremio_link::Flow,
    pub romm: &'a romm::Form,
    /// The emulator-readiness card's state: what can play and what the last
    /// BIOS sync did.
    pub readiness: &'a readiness::Flow,
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
            categorization::view(state.categorization).map(Message::Categorization),
            stremio(state),
            romm::view(state.romm),
            readiness::view(state.readiness).map(Message::Readiness)
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

/// Where the library's tabs and the dashboard's rails come from, and the
/// control that puts the rails back as Hearthdeck derives them.
///
/// Its own section, and always on offer: the rails can go wrong whether or not
/// the local model is installed, or even supported, and the way out must not
/// depend on a feature the user may never have turned on.
fn library_and_dashboard<'a>() -> Element<'a, Message> {
    let Spacing { space_s, .. } = theme::spacing();
    let buttons = row![action_button(
        fl!("dashboard-restore"),
        Message::RestoreDashboard,
        standard_button_class(),
        true,
    )]
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
fn titled_section<'a, M: Clone + 'static>(title: String) -> settings::Section<'a, M> {
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
fn described_item<'a, M: 'a>(
    label: String,
    description: String,
    control: impl Into<Element<'a, M>>,
) -> Element<'a, M> {
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
fn body_line<'a, M: 'a>(line: String) -> Element<'a, M> {
    text::body(line).size(TEXT_BODY).into()
}

/// A quieter line: fine print the section only reports (a model path, a count).
fn caption_line<'a, M: 'a>(line: String) -> Element<'a, M> {
    text::caption(line).size(TEXT_CAPTION).into()
}

/// A line that reports a failure: the theme's destructive color, so it does not
/// read as one more quiet note.
fn error_line<'a, M: 'a>(line: String) -> Element<'a, M> {
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
fn button_row<'a, M: 'a>(buttons: impl Into<Element<'a, M>>) -> Element<'a, M> {
    container(buttons).align_right(Length::Fill).into()
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
        stremio_link::Action::Link => Message::Stremio(stremio_link::Message::LinkPressed),
        stremio_link::Action::Unlink => Message::Stremio(stremio_link::Message::UnlinkPressed),
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
fn action_button<'a, M: Clone + 'a>(
    label: String,
    message: M,
    class: Button,
    enabled: bool,
) -> Element<'a, M> {
    let Spacing { space_s, .. } = theme::spacing();
    button::custom(text::body(label).size(TEXT_BODY))
        .padding([space_s, space_s])
        .class(class)
        .on_press_maybe(enabled.then_some(message))
        .into()
}

/// The same rule for the Stremio controls: linking is the affirmative action,
/// unlinking removes the account.
fn stremio_action_class(action: stremio_link::Action) -> Button {
    match action {
        stremio_link::Action::Link => primary_action_button_class(),
        stremio_link::Action::Unlink => destructive_button_class(),
    }
}
