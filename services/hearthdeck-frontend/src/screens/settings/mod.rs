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
    content_horizontal_padding, error_text_class, standard_button_class,
};
use crate::widgets::top_bar;

pub mod categorization;
pub mod readiness;
pub mod romm;
pub mod stremio_link;

pub fn romm_message(message: romm::ViewMessage) -> Message {
    match message {
        romm::ViewMessage::Form(message) => Message::Romm(message),
        romm::ViewMessage::TextFieldFocused(field) => Message::TextFieldFocused(field),
        romm::ViewMessage::TextFieldBlurred(field) => Message::TextFieldBlurred(field),
        romm::ViewMessage::TextFieldSubmitted(field) => Message::TextFieldSubmitted(field),
    }
}

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
            stremio_link::view(state.stremio).map(Message::Stremio),
            romm::view(state.romm).map(romm_message),
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
fn item<'a, M: 'a>(label: String, control: impl Into<Element<'a, M>>) -> Element<'a, M> {
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
fn summary<'a, M: 'a>(status: String, intro: String) -> Element<'a, M> {
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

/// One action in the section's button row, so padding cannot drift apart as
/// more are added beside it. The class is passed in rather than fixed here:
/// each control wears the class its role earns.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_entry::Field;

    #[test]
    fn romm_card_and_dialog_keep_the_global_text_entry_events() {
        for field in [Field::RommUrl, Field::RommUsername, Field::RommPassword] {
            assert!(matches!(
                romm_message(romm::ViewMessage::TextFieldFocused(field)),
                Message::TextFieldFocused(actual) if actual == field
            ));
            assert!(matches!(
                romm_message(romm::ViewMessage::TextFieldBlurred(field)),
                Message::TextFieldBlurred(actual) if actual == field
            ));
            assert!(matches!(
                romm_message(romm::ViewMessage::TextFieldSubmitted(field)),
                Message::TextFieldSubmitted(actual) if actual == field
            ));
        }
    }

    #[test]
    fn romm_form_events_keep_credentials_redacted_through_the_page_boundary() {
        let event =
            romm::ViewMessage::Form(romm::Message::PasswordChanged("private-secret".into()));
        assert!(!format!("{event:?}").contains("private-secret"));
        let message = romm_message(event);
        assert!(!format!("{message:?}").contains("private-secret"));
        assert!(matches!(
            message,
            Message::Romm(romm::Message::PasswordChanged(password)) if password == "private-secret"
        ));
        assert!(matches!(
            romm_message(romm::ViewMessage::Form(romm::Message::SignInPressed)),
            Message::Romm(romm::Message::SignInPressed)
        ));
        assert!(matches!(
            romm_message(romm::ViewMessage::Form(romm::Message::SignInClosed)),
            Message::Romm(romm::Message::SignInClosed)
        ));
    }
}
