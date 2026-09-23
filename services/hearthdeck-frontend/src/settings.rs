//! The settings screen.
//!
//! Smart categorization is the one thing here that both costs something and
//! changes something: it downloads most of a gigabyte, and it takes over the
//! library's category tabs. So this screen says both of those plainly before
//! anything happens, does it only when asked, and offers the way back.

use std::sync::LazyLock;

use cosmic::Element;
use cosmic::iced::{Alignment, Length};
use cosmic::theme;
use cosmic::theme::Button;
use cosmic::widget::{Id, button, column, container, row, scrollable, text};

use crate::app::{Message, human_size};
use crate::fl;
use crate::providers::daemon::{
    CategorizationPhase, CategorizationSnapshot, ModelStatus, ScanReport,
};
use crate::style::{TEXT_TITLE, primary_action_button_class};

/// What a first download costs. Named once, so the sentence that warns about it
/// cannot drift away from the number the operator reads in the docs.
const DOWNLOAD_SIZE: &str = "~850 MB";

/// Widget ids for the buttons, so the gamepad can find them. Stable strings:
/// the focus is addressed by id across frames.
static ENABLE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("settings-categorization-enable"));
static DISABLE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("settings-categorization-disable"));
static RESCAN_ID: LazyLock<Id> = LazyLock::new(|| Id::new("settings-categorization-rescan"));
static RESET_ID: LazyLock<Id> = LazyLock::new(|| Id::new("settings-categorization-reset"));
static REMOVE_MODEL_ID: LazyLock<Id> =
    LazyLock::new(|| Id::new("settings-categorization-remove-model"));

/// One thing the screen can do.
///
/// Kept as data rather than as a row of widgets built in one place, because the
/// application needs the same list for two other jobs: moving the gamepad's
/// focus through the row, and knowing what a press on a focused button means.
/// One list means the drawn buttons and the handled ones cannot drift apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Enable,
    Disable,
    Rescan,
    Reset,
    RemoveModel,
}

impl Action {
    /// Every action. The renderer and the handler both walk the list returned by
    /// [`actions`], so this is only used to get back from a widget id to the
    /// action it stands for.
    const ALL: [Action; 5] = [
        Action::Enable,
        Action::Disable,
        Action::Rescan,
        Action::Reset,
        Action::RemoveModel,
    ];

    /// The widget id the button carries, so focus can be addressed to it.
    pub fn id(self) -> &'static Id {
        match self {
            Self::Enable => &ENABLE_ID,
            Self::Disable => &DISABLE_ID,
            Self::Rescan => &RESCAN_ID,
            Self::Reset => &RESET_ID,
            Self::RemoveModel => &REMOVE_MODEL_ID,
        }
    }

    fn label(self) -> String {
        match self {
            Self::Enable => fl!("categorization-enable"),
            Self::Disable => fl!("categorization-disable"),
            Self::Rescan => fl!("categorization-rescan"),
            Self::Reset => fl!("categorization-reset"),
            Self::RemoveModel => fl!("categorization-remove-model"),
        }
    }

    /// The message the application handles when this is pressed.
    pub fn message(self) -> Message {
        match self {
            Self::Enable => Message::EnableCategorization,
            Self::Disable => Message::DisableCategorization,
            Self::Rescan => Message::StartCategorization,
            Self::Reset => Message::ResetCategorizationTabs,
            Self::RemoveModel => Message::DeleteCategorizationModel,
        }
    }

    fn class(self) -> Button {
        match self {
            Self::Enable | Self::Rescan => primary_action_button_class(),
            Self::RemoveModel => Button::Destructive,
            Self::Disable | Self::Reset => Button::Standard,
        }
    }
}

/// The actions on offer for the state `panel` describes, in the order they are
/// drawn. Enable and Disable never appear together: which one is offered is the
/// state of the feature.
///
/// Anything that reads or writes the checkpoint is off the list while a scan is
/// running, since only one scan can hold it at a time. The buttons stay drawn
/// but unpressable, so the row does not jump about mid-scan.
pub fn actions(panel: &Panel) -> Vec<Action> {
    let Some(snapshot) = &panel.snapshot else {
        return Vec::new();
    };
    let mut actions = if snapshot.enabled {
        vec![Action::Rescan, Action::Disable]
    } else {
        vec![Action::Enable]
    };
    if snapshot.report.is_some() {
        actions.push(Action::Reset);
    }
    if matches!(snapshot.status.model, ModelStatus::Downloaded { .. }) {
        actions.push(Action::RemoveModel);
    }
    actions
}

/// The action a widget id stands for, or `None` when the id belongs to
/// something else. Widget ids are unique across the app, so this is how the
/// confirm handler recognises its own buttons.
pub fn action(id: &Id) -> Option<Action> {
    Action::ALL.into_iter().find(|action| action.id() == id)
}

/// Whether pressing `action` would be accepted in the state `panel` describes.
/// The application asks before it moves the focus onto a button, so a scan in
/// progress is skipped rather than confirming into nothing.
pub fn pressable(action: Action, panel: &Panel) -> bool {
    let busy = panel
        .snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot.status.running);
    !matches!(action, Action::Rescan | Action::RemoveModel) || !busy
}

/// What the screen shows. Gathered by the caller, so this module needs nothing
/// from the application's internals.
pub struct Panel {
    /// Whether the deployment provides categorization at all. `Some(false)` means
    /// the feature does not exist for this client, whatever the user wants;
    /// `None` means the health reply has not arrived yet.
    pub capable: Option<bool>,
    /// The daemon's state, or `None` before the first read comes back.
    pub snapshot: Option<CategorizationSnapshot>,
}

pub fn view(panel: &Panel) -> Element<'static, Message> {
    let spacing = theme::spacing();
    let mut body = column![
        text(fl!("settings-title")).size(TEXT_TITLE),
        text::body(fl!("categorization-intro", size = DOWNLOAD_SIZE)),
    ]
    .spacing(spacing.space_s)
    .width(Length::Fill);

    match panel.capable {
        Some(true) => {}
        // Offered only once the daemon is known to provide it: a button that
        // cannot work is worse than no button.
        Some(false) => {
            body = body.push(note(fl!("categorization-unsupported")));
            body = body.push(text::caption(fl!("categorization-unsupported-hint")));
            return frame(body);
        }
        None => {
            body = body.push(note(fl!("categorization-reading-status")));
            return frame(body);
        }
    }

    body = body.push(note(status_line(panel)));
    if let Some(error) = panel
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.status.last_error.clone())
    {
        body = body.push(text::caption(fl!("categorization-failed", reason = error)));
    }
    body = body.push(actions_row(panel));
    if let Some(snapshot) = &panel.snapshot
        && let Some(report) = snapshot.report.as_ref()
    {
        body = body.push(report_summary(
            report,
            snapshot.status.last_completed_at.as_deref(),
        ));
    }
    frame(body)
}

/// The one line that says what is happening, or what the state is.
fn status_line(panel: &Panel) -> String {
    let Some(snapshot) = &panel.snapshot else {
        return fl!("categorization-reading-status");
    };
    match snapshot.status.phase {
        CategorizationPhase::Downloading => fl!("categorization-phase-downloading"),
        CategorizationPhase::Loading => fl!("categorization-phase-loading"),
        CategorizationPhase::Scanning => fl!(
            "categorization-phase-scanning",
            completed = snapshot.status.completed,
            total = snapshot.status.total
        ),
        CategorizationPhase::Idle => {
            if snapshot.enabled {
                model_line(&snapshot.status.model)
            } else {
                fl!("categorization-off")
            }
        }
    }
}

fn model_line(model: &ModelStatus) -> String {
    match model {
        ModelStatus::Absent => fl!("categorization-model-absent"),
        ModelStatus::Downloaded { bytes, path } => fl!(
            "categorization-model-downloaded",
            size = human_size(*bytes),
            path = path.display().to_string()
        ),
        ModelStatus::External { path } => fl!(
            "categorization-model-external",
            path = path.display().to_string()
        ),
    }
}

fn actions_row(panel: &Panel) -> Element<'static, Message> {
    let spacing = theme::spacing();
    let mut row = row![].spacing(spacing.space_s).align_y(Alignment::Center);

    for action in actions(panel) {
        row = row.push(action_button(action, panel));
    }
    row.into()
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

fn report_summary(report: &ScanReport, completed_at: Option<&str>) -> Element<'static, Message> {
    let spacing = theme::spacing();
    let recommended: Vec<String> = report
        .categories
        .iter()
        .filter(|category| category.recommended)
        .map(|category| category.name.clone())
        .collect();

    let mut body = column![
        text::body(fl!(
            "categorization-report",
            apps = report.app_count,
            categories = recommended.len(),
            unclassified = report.unclassified.len()
        )),
        text::caption(fl!(
            "categorization-engine",
            engine = report.categorizer.clone()
        )),
    ]
    .spacing(spacing.space_xxs);
    if let Some(when) = completed_at {
        body = body.push(text::caption(fl!(
            "categorization-last-run",
            when = when.to_owned()
        )));
    }
    for name in recommended {
        body = body.push(text::caption(name));
    }
    body.into()
}

fn note(line: String) -> Element<'static, Message> {
    let spacing = theme::spacing();
    container(text::body(line))
        .padding(spacing.space_s)
        .width(Length::Fill)
        .into()
}

fn frame(body: impl Into<Element<'static, Message>>) -> Element<'static, Message> {
    let spacing = theme::spacing();
    scrollable::vertical(
        container(body)
            .padding([spacing.space_l, spacing.space_xl])
            .width(Length::Fill),
    )
    .height(Length::Fill)
    .into()
}
