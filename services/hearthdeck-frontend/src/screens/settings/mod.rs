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
use cosmic::widget::{button, column, container, row, scrollable, settings, text, toggler};

use crate::app::Message;
use crate::fl;
use crate::providers::daemon::{CategorizationPhase, CategorizationSnapshot, ModelStatus};
use crate::style::{TEXT_BODY, TEXT_TITLE, content_horizontal_padding, standard_button_class};
use crate::widgets::top_bar;

/// What the first scan costs. Named once, so the sentence that warns about it
/// cannot drift from the number.
const DOWNLOAD_SIZE: &str = "~850 MB";

/// What the screen draws.
pub struct Settings<'a> {
    /// The daemon's answer, once it has landed. Until then the screen says it is
    /// checking, rather than showing a switch in a state nobody reported.
    pub categorization: Option<&'a CategorizationSnapshot>,
}

/// The page: the shared top bar, then the sections.
#[must_use]
pub fn view<'a>(
    state: &Settings<'a>,
    bar: &top_bar::TopBar,
    on_navigate: fn(top_bar::Destination) -> Message,
) -> Element<'a, Message> {
    let Spacing {
        space_l, space_xl, ..
    } = theme::spacing();
    let padding = content_horizontal_padding();

    let sections = column![
        text::title1(fl!("settings-title")).size(TEXT_TITLE),
        scrollable(settings::view_column(vec![categorization(
            state.categorization
        )]))
        .width(Length::Fill)
        .height(Length::Fill)
        .scrollbar_width(0)
        .scroller_width(0),
    ]
    .spacing(space_l)
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

/// The smart-categories section: the local model that sorts the library, and
/// every switch and action it has.
///
/// Every line is the daemon's own answer - a scan's phase, the model it has on
/// disk, whether the last one failed - so the screen never claims more than it
/// was told. Nothing heavy runs until the switch is on, which is the promise the
/// intro makes and the reason the off state says so plainly.
fn categorization<'a>(snapshot: Option<&'a CategorizationSnapshot>) -> Element<'a, Message> {
    let Spacing { space_s, .. } = theme::spacing();

    let Some(snapshot) = snapshot else {
        return settings::section::<Message>()
            .title(fl!("categorization-heading"))
            .add(text::body(fl!("categorization-reading-status")))
            .into();
    };

    let mut section = settings::section::<Message>()
        .title(fl!("categorization-heading"))
        .add(text::body(fl!(
            "categorization-intro",
            size = DOWNLOAD_SIZE
        )))
        .add(settings::item(
            fl!("categorization-toggle"),
            toggler(snapshot.enabled).on_toggle(Message::SetCategorization),
        ));

    if !snapshot.enabled {
        return section.add(text::body(fl!("categorization-off"))).into();
    }

    section = section.add(text::body(status_line(snapshot)));
    section = section.add(text::body(model_line(&snapshot.status.model)));

    // Nothing to press while it is already working: the phase above says what it
    // is doing, and a second scan is not something to ask for from here.
    if snapshot.status.running {
        return section.into();
    }

    let actions = row![action_button(
        scan_label(snapshot),
        Message::ScanCategorization
    )]
    .spacing(space_s);
    section.add(actions).into()
}

/// One action in the section's button row, so padding and class cannot drift
/// apart as more are added beside it.
fn action_button<'a>(label: String, message: Message) -> Element<'a, Message> {
    let Spacing { space_s, .. } = theme::spacing();
    button::custom(text::body(label).size(TEXT_BODY))
        .padding([space_s, space_s])
        .class(standard_button_class())
        .on_press(message)
        .into()
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
    match status.last_error.as_deref() {
        Some(reason) => fl!("categorization-failed", reason = reason),
        None => fl!("categorization-ready"),
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

/// The scan button reads as a first scan until there has been one, so the same
/// control does not offer to "categorize again" before it ever has.
fn scan_label(snapshot: &CategorizationSnapshot) -> String {
    if snapshot.status.last_completed_at.is_some() {
        fl!("categorization-rescan")
    } else {
        fl!("categorization-categorize")
    }
}
