//! The settings screen.
//!
//! Smart categorization is the one thing here that both costs something and
//! changes something: it downloads most of a gigabyte, calls a local model over
//! the library, and takes over the library's category tabs and the dashboard.
//! So this screen says both of those plainly before anything happens, does it
//! only when asked, and offers the way back.

use std::sync::LazyLock;

use cosmic::Element;
use cosmic::iced::{Alignment, Length};
use cosmic::theme;
use cosmic::theme::Button;
use cosmic::widget::{Id, button, column, container, row, scrollable, text, toggler};

use crate::app::{Message, human_size};
use crate::fl;
use crate::providers::daemon::{
    CategorizationPhase, CategorizationSnapshot, ModelStatus, ScanReport,
};
use crate::style::{TEXT_TITLE, primary_action_button_class};

/// What a first download costs. Named once, so the sentence that warns about it
/// cannot drift away from the number the operator reads in the docs.
const DOWNLOAD_SIZE: &str = "~850 MB";

/// Widget ids for the controls, so the gamepad can find them. Stable strings:
/// the focus is addressed by id across frames.
static TOGGLE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("settings-categorization-toggle"));
static CATEGORIZE_ID: LazyLock<Id> =
    LazyLock::new(|| Id::new("settings-categorization-categorize"));
static CREATE_COLLECTIONS_ID: LazyLock<Id> =
    LazyLock::new(|| Id::new("settings-categorization-create-collections"));
static RESET_ID: LazyLock<Id> = LazyLock::new(|| Id::new("settings-categorization-reset"));
static REMOVE_MODEL_ID: LazyLock<Id> =
    LazyLock::new(|| Id::new("settings-categorization-remove-model"));

/// One control the screen offers.
///
/// Kept as data rather than as a row of widgets built in one place, because the
/// application needs the same list for two other jobs: moving the gamepad's
/// focus through the row, and knowing what a press on a focused control means.
/// One list means the drawn controls and the handled ones cannot drift apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The opt-in itself. Turning it on is what installs and activates the
    /// model; turning it off restores everything the model changed.
    Toggle,
    /// Classify the library with the model. This is the run that calls it.
    Categorize,
    /// Publish the last run's rails as dashboard collections, without calling
    /// the model again.
    CreateCollections,
    /// Give the tabs back to the ones derived from the entries themselves.
    Reset,
    /// Free the downloaded checkpoint.
    RemoveModel,
}

impl Action {
    /// Every action. The renderer and the handler both walk the list returned by
    /// [`actions`], so this is only used to get back from a widget id to the
    /// action it stands for.
    const ALL: [Action; 5] = [
        Action::Toggle,
        Action::Categorize,
        Action::CreateCollections,
        Action::Reset,
        Action::RemoveModel,
    ];

    /// The widget id the control carries, so focus can be addressed to it.
    pub fn id(self) -> &'static Id {
        match self {
            Self::Toggle => &TOGGLE_ID,
            Self::Categorize => &CATEGORIZE_ID,
            Self::CreateCollections => &CREATE_COLLECTIONS_ID,
            Self::Reset => &RESET_ID,
            Self::RemoveModel => &REMOVE_MODEL_ID,
        }
    }

    fn label(self) -> String {
        match self {
            Self::Toggle => fl!("categorization-toggle"),
            Self::Categorize => fl!("categorization-categorize"),
            Self::CreateCollections => fl!("categorization-create-collections"),
            Self::Reset => fl!("categorization-reset"),
            Self::RemoveModel => fl!("categorization-remove-model"),
        }
    }

    /// The message the application handles when this is pressed.
    pub fn message(self) -> Message {
        match self {
            Self::Toggle => Message::ToggleCategorization,
            Self::Categorize => Message::StartCategorization,
            Self::CreateCollections => Message::CreateCategorizationCollections,
            Self::Reset => Message::ResetCategorizationTabs,
            Self::RemoveModel => Message::DeleteCategorizationModel,
        }
    }

    fn class(self) -> Button {
        match self {
            Self::Categorize | Self::CreateCollections => primary_action_button_class(),
            Self::RemoveModel => Button::Destructive,
            Self::Toggle | Self::Reset => Button::Standard,
        }
    }
}

/// The controls on offer for the state `panel` describes, in the order they are
/// drawn.
///
/// The switch is always first and always there: it is the one control that
/// installs and activates the model, and the only way back. The two buttons that
/// make the model do something appear only once it is on — there is nothing for
/// them to act on before that, and a scan with no opt-in is refused.
///
/// Anything that reads or writes the checkpoint is off the list while a scan is
/// running, since only one scan can hold it at a time. The controls stay drawn
/// but unpressable, so the row does not jump about mid-scan.
pub fn actions(panel: &Panel) -> Vec<Action> {
    let Some(snapshot) = &panel.snapshot else {
        return Vec::new();
    };
    let mut actions = vec![Action::Toggle];
    if !snapshot.enabled {
        return actions;
    }
    actions.push(Action::Categorize);
    if snapshot.report.is_some() {
        // Both read the stored report: without one there is nothing to publish,
        // and nothing to undo.
        actions.push(Action::CreateCollections);
        actions.push(Action::Reset);
    }
    if matches!(snapshot.status.model, ModelStatus::Downloaded { .. }) {
        actions.push(Action::RemoveModel);
    }
    actions
}

/// The action a widget id stands for, or `None` when the id belongs to
/// something else. Widget ids are unique across the app, so this is how the
/// confirm handler recognises its own controls.
pub fn action(id: &Id) -> Option<Action> {
    Action::ALL.into_iter().find(|action| action.id() == id)
}

/// Whether pressing `action` would be accepted in the state `panel` describes.
/// The application asks before it moves the focus onto a control, so a scan in
/// progress is skipped rather than confirming into nothing.
pub fn pressable(action: Action, panel: &Panel) -> bool {
    let snapshot = panel.snapshot.as_ref();
    let busy = snapshot.is_some_and(|snapshot| snapshot.status.running);
    let enabled = snapshot.is_some_and(|snapshot| snapshot.enabled);
    match action {
        // The switch reads and writes the opt-in, which a running scan does not
        // hold: the daemon leaves a running scan to finish either way.
        Action::Toggle => snapshot.is_some(),
        Action::Categorize | Action::RemoveModel => enabled && !busy,
        Action::CreateCollections => {
            enabled && !busy && snapshot.is_some_and(|snapshot| snapshot.report.is_some())
        }
        Action::Reset => enabled,
    }
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
        // Offered only once the daemon is known to provide it: a control that
        // cannot work is worse than no control.
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
    // Nothing to draw until the first read answers; the status line says so.
    if panel.snapshot.is_some() {
        body = body.push(toggle_row(panel));
    }
    let buttons: Vec<Action> = actions(panel)
        .into_iter()
        .filter(|action| *action != Action::Toggle)
        .collect();
    if !buttons.is_empty() {
        body = body.push(buttons_row(&buttons, panel));
    }
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

/// The opt-in switch, on its own row above the buttons it gates.
fn toggle_row(panel: &Panel) -> Element<'static, Message> {
    let checked = panel
        .snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot.enabled);
    let control = toggler(checked)
        .id(Action::Toggle.id().clone())
        .label(Action::Toggle.label())
        .spacing(theme::spacing().space_xs);
    let control = if pressable(Action::Toggle, panel) {
        control.on_toggle(|_| Message::ToggleCategorization)
    } else {
        control
    };
    row![control].into()
}

fn buttons_row(actions: &[Action], panel: &Panel) -> Element<'static, Message> {
    let spacing = theme::spacing();
    let mut row = row![].spacing(spacing.space_s).align_y(Alignment::Center);

    for action in actions {
        row = row.push(action_button(*action, panel));
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
