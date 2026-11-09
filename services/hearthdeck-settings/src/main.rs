//! Hearthdeck's settings window.
//!
//! It ships as its own process beside the library app, and the library app is
//! what opens it: gpui renders through its own window and event loop, so it
//! cannot be mounted inside a libcosmic/iced `Element`.
//!
//! The window is a full surface, not a dialog: it opens fullscreen with no top
//! bar of its own, and closing it puts the user back under the library app
//! exactly where it was.
//!
//! Which means the contract this window keeps:
//!
//! - Escape closes it - unless a text field owns focus, in which case Escape
//!   dismisses the field and the window stays, which is the order
//!   `docs/product-foundations.md` sets for every writable field.
//! - Arrows move a cursor over every row of a page and between pages, Enter
//!   runs or edits the focused row. Nothing on the page needs a pointer, and
//!   the same semantic actions are what the controller transport delivers.
//! - It pairs with the loopback daemon itself, exactly as the library app does,
//!   so nothing secret crosses the process boundary.
//!
//! **Every row writes.** A field, a switch, a schedule, a link: each one reads
//! the daemon's state, writes, reports what the daemon answered, and reverts the
//! control when the daemon refuses. Nothing here is a preview.

mod daemon;
mod input;
mod nav;
mod theme;

use std::ffi::CString;
use std::time::Duration;

use chrono::Timelike as _;
use gpui::{
    Action as _, Animation, AnimationExt as _, AnyElement, AnyWindowHandle, App, AppContext as _,
    Bounds, ClickEvent, Context, Entity, Focusable as _, FontWeight, InteractiveElement as _,
    IntoElement, KeyBinding, ListAlignment, ListState, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, list, point, prelude::FluentBuilder as _, px, relative, size,
};
use gpui_component::alert::Alert;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::marker::{Marker, MarkerAlignment, MarkerContent, MarkerIcon};
use gpui_component::notification::Notification;
use gpui_component::progress::ProgressCircle;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::separator::Separator;
use gpui_component::switch::Switch;
use gpui_component::time_field::{TimeField, TimeFieldState, TimePrecision};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, Theme, WindowExt as _,
    h_flex, v_flex,
};
use nix::sys::statvfs::statvfs;

use daemon::{Categorization, DeviceOffer, LinkStatus, Snapshot, read_snapshot};
use nav::Nav;

gpui::actions!(
    hearthdeck_settings,
    [
        CloseWindow,
        MoveUp,
        MoveDown,
        PreviousPage,
        NextPage,
        Confirm,
        PasteIntoField
    ]
);

/// The TV type scale. Nothing here is below 16px: at three metres a 12px caption
/// is decoration, and the smallest strings on this screen are the footer that
/// explains the controls. The iced page's ratios are the reference
/// (52 / 26 / 20.8 / 18.2 / 15.6).
const TYPE_TITLE: f32 = 48.0;
const TYPE_CARD: f32 = 26.0;
const TYPE_LABEL: f32 = 21.0;
const TYPE_VALUE: f32 = 21.0;
const TYPE_BODY: f32 = 18.0;
const TYPE_CAPTION: f32 = 16.0;
const TYPE_CODE: f32 = 48.0;
/// The schedule field: an hour and a minute, not a URL.
const TIME_FIELD_WIDTH: f32 = 240.0;
/// How long a page change takes to fade the list in. Long enough to read as a
/// transition, short enough that a pad press feels answered instantly.
const PAGE_FADE: std::time::Duration = std::time::Duration::from_millis(180);
/// The space between two cards, and the larger one under the page heading.
const CARD_GAP: u16 = 24;
const HEADER_GAP: u16 = 32;

/// The page column, sized for a television rather than a laptop window.
const SIDEBAR_WIDTH: f32 = 360.0;
/// How wide the reading column may get on a wide display.
const CONTENT_MAX_WIDTH: f32 = 1280.0;
const RING_SIZE: f32 = 64.0;
/// The width a text field takes on the right of its row. The kit's Large input
/// is wider than the Medium one it replaced, so the column grew with it.
const FIELD_WIDTH: f32 = 420.0;
/// A button shorter than its label would read as a chip; this keeps the action
/// column from collapsing to the width of the longest word.
const BUTTON_MIN_WIDTH: f32 = 200.0;
/// The size the window restores to; fullscreen itself is the compositor's call.
const RESTORE_WIDTH: f32 = 1280.0;
const RESTORE_HEIGHT: f32 = 720.0;
/// How many results may wait for the next frame before the oldest is dropped: a
/// burst of background completions must not stack a wall of toasts.
const PENDING_NOTICES: usize = 4;

// ---------------------------------------------------------------------------
// What the page is made of
//
// Two levels only: a page, and the groups of rows inside it. A row is a fact or
// one control, and which control it is decides what Enter does with it - so
// adding a setting is adding a row, not adding a screen.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PageKind {
    Status,
    Connections,
    Library,
    Emulation,
    Backup,
}

impl PageKind {
    const ALL: [Self; 5] = [
        Self::Status,
        Self::Connections,
        Self::Library,
        Self::Emulation,
        Self::Backup,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Connections => "Connections",
            Self::Library => "Library",
            Self::Emulation => "Emulation",
            Self::Backup => "Backup",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Status => "What Hearthdeck sees of the daemon and this machine.",
            Self::Connections => "The accounts Hearthdeck talks to on your behalf.",
            Self::Library => "How the library is gathered and grouped.",
            Self::Emulation => "The cores RetroArch needs, and what is ready to play.",
            Self::Backup => "A nightly copy of saves, databases and metadata, off this machine.",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Status => IconName::Info,
            Self::Connections => IconName::Globe,
            Self::Library => IconName::FolderOpen,
            Self::Emulation => IconName::Cpu,
            Self::Backup => IconName::HardDrive,
        }
    }
}

/// A writable text row. The value itself lives in its [`InputState`], because
/// that is the thing that owns a caret, a selection and an IME session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextField {
    RommUrl,
    BackupRemote,
    BackupRoot,
}

/// A switch row. Categorization writes as soon as it is flipped; backup carries
/// the destination and the schedule with it, so it marks a draft and Save writes
/// the whole decision at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToggleField {
    Categorization,
    Backup,
}

/// What Enter does with a focused row that is not a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowAction {
    Refresh,
    RommLink,
    RommForget,
    StremioLink,
    StremioUnlink,
    RestoreRails,
    ScanLibrary,
    InstallCores,
    SyncBios,
    SaveBackup,
    RunBackup,
}

impl RowAction {
    fn label(self) -> &'static str {
        match self {
            Self::Refresh => "Refresh",
            Self::RommLink => "Link RomM",
            Self::RommForget => "Forget connection",
            Self::StremioLink => "Link account",
            Self::StremioUnlink => "Unlink",
            Self::RestoreRails => "Restore dashboard",
            Self::ScanLibrary => "Scan now",
            Self::InstallCores => "Install missing cores",
            Self::SyncBios => "Sync BIOS",
            Self::SaveBackup => "Save and apply",
            Self::RunBackup => "Run now",
        }
    }

    /// Why an action cannot run right now, in the user's words.
    fn blocked(self) -> &'static str {
        match self {
            Self::ScanLibrary => "Turn categorization on first.",
            _ => "That is already running.",
        }
    }

    /// The action that moves the page forward is the one the eye should land
    /// on. A page of identical grey buttons reads as a list of facts, which is
    /// what made the primary actions invisible on the television.
    fn primary(self) -> bool {
        matches!(
            self,
            Self::RommLink
                | Self::StremioLink
                | Self::ScanLibrary
                | Self::InstallCores
                | Self::SyncBios
                | Self::SaveBackup
                | Self::RunBackup
        )
    }

    /// Removing a stored connection cannot be undone, so it is not drawn like
    /// the action beside it.
    fn destructive(self) -> bool {
        matches!(self, Self::RommForget | Self::StremioUnlink)
    }
}

/// Which service a pending approval belongs to. Both answer with the same
/// offer and the same status, so the window runs one flow for both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinkService {
    RomM,
    Stremio,
}

impl LinkService {
    fn label(self) -> &'static str {
        match self {
            Self::RomM => "RomM",
            Self::Stremio => "Stremio",
        }
    }
}

/// A pending approval: the code to show, how long the daemon says it has, and
/// the generation that identifies the poll loop watching it.
struct PendingLink {
    service: LinkService,
    offer: DeviceOffer,
    seconds_left: Option<u64>,
    generation: u64,
}

/// The right-hand side of a row.
#[derive(Clone)]
enum Control {
    /// A state or a measurement the window only reports.
    Fact(Value),
    Text(TextField),
    Toggle(ToggleField),
    /// The nightly schedule: an hour and minute the kit edits with the dpad.
    Time,
    Action(RowAction),
}

#[derive(Clone)]
enum Value {
    Text(String),
    /// A code to type on another device: big, monospaced, tracked wide.
    Code(String),
    Storage {
        used_percent: f32,
        available: String,
        total: String,
    },
}

#[derive(Clone)]
struct Page {
    kind: PageKind,
    groups: Vec<Group>,
}

impl Page {
    #[cfg(test)]
    fn row_count(&self) -> usize {
        self.groups.iter().map(|group| group.rows.len()).sum()
    }

    /// The flat row indexes the cursor may land on.
    ///
    /// A fact row is drawn but never focused: Enter on it does nothing, and a
    /// cursor that stops on a dead row is worse than one that steps over it.
    fn cursor_rows(&self) -> Vec<usize> {
        let mut flat = 0;
        let mut rows = Vec::new();
        for group in &self.groups {
            for row in &group.rows {
                if !matches!(row.control, Control::Fact(_)) {
                    rows.push(flat);
                }
                flat += 1;
            }
        }
        rows
    }

    /// Where a flat row sits among the cursor rows, or `None` for a fact.
    fn cursor_index_of(&self, flat: usize) -> Option<usize> {
        self.cursor_rows().iter().position(|index| *index == flat)
    }
}

#[derive(Clone)]
struct Group {
    title: &'static str,
    /// A one-word state beside the card's title, when the section has one worth
    /// reading at a glance. The rows still carry the detail.
    status: Option<(&'static str, Tone)>,
    rows: Vec<Row>,
}

/// What a section's status chip says at a glance: working, waiting on the user,
/// or not set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Good,
    Attention,
    Off,
    /// Work in progress: a run, a link waiting for approval.
    Busy,
    /// A failure or an unknown state: never "off, and that is fine".
    Danger,
}

/// What a control's state says at a glance, as a [`Marker`] at the trailing
/// edge of a card heading.
fn tone_marker(group: &'static str, label: &'static str, tone: Tone, theme: &Theme) -> AnyElement {
    let (icon, color) = match tone {
        Tone::Good => (IconName::CircleCheck, theme.success),
        Tone::Attention => (IconName::TriangleAlert, theme.warning),
        Tone::Off => (IconName::Minus, theme.muted_foreground),
        Tone::Busy => (IconName::LoaderCircle, theme.info),
        Tone::Danger => (IconName::CircleX, theme.danger),
    };
    Marker::new()
        .id(SharedString::from(format!("status-{group}-{label}")))
        .role(gpui::Role::Status)
        .w_auto()
        .flex_none()
        .alignment(MarkerAlignment::End)
        .text_size(px(TYPE_BODY))
        .text_color(color)
        .icon(MarkerIcon::new().child(Icon::new(icon)))
        .content(MarkerContent::new().text(label))
        .into_any_element()
}

/// A transient result. Results are notifications, not a strip under the page:
/// a saved setting is over the moment it is read, and drawing it into the page
/// made it look like a fact about the configuration. A condition that *is*
/// state - a missing remote, a dead daemon - stays an [`Alert`] on the page.
struct Notice {
    level: NoticeLevel,
    text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoticeLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl NoticeLevel {
    fn notification(self, text: String) -> Notification {
        match self {
            Self::Info => Notification::info(text),
            Self::Success => Notification::success(text),
            Self::Warning => Notification::warning(text),
            Self::Error => Notification::error(text),
        }
    }
}

#[derive(Clone)]
struct Row {
    label: &'static str,
    description: Option<&'static str>,
    control: Control,
}

/// One line of the scrolling page. A card is drawn by consecutive items - its
/// heading, then its rows - all on the same surface, so splitting a card across
/// items keeps one card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    /// The space between two cards, in px: 32 above the first card, 24 between
    /// the rest, so the header separates from the stack.
    Gap(u16),
    /// A card's heading.
    Header { group: usize },
    /// One row inside a card.
    Row {
        group: usize,
        row: usize,
        /// The row's index within the page, which is what the cursor counts.
        flat: usize,
        last: bool,
    },
}

/// The page as one list, plus the index of the item that draws each row - which
/// is how the cursor's row becomes a scroll target.
fn flat_items(page: &Page) -> (Vec<Item>, Vec<usize>) {
    let mut items = Vec::new();
    let mut rows = Vec::new();
    for (group_ix, group) in page.groups.iter().enumerate() {
        items.push(Item::Gap(if group_ix == 0 { HEADER_GAP } else { CARD_GAP }));
        items.push(Item::Header { group: group_ix });
        for (row_ix, _) in group.rows.iter().enumerate() {
            rows.push(items.len());
            items.push(Item::Row {
                group: group_ix,
                row: row_ix,
                flat: rows.len() - 1,
                last: row_ix + 1 == group.rows.len(),
            });
        }
    }
    items.push(Item::Gap(CARD_GAP));
    (items, rows)
}

/// The text fields, created once with the window so their state survives every
/// frame: a caret that is rebuilt is a caret that loses its place.
struct Inputs {
    romm_url: Entity<InputState>,
    backup_remote: Entity<InputState>,
    backup_root: Entity<InputState>,
    /// The schedule. Its state owns the up/down stepping, so the row needs no
    /// stepper of its own.
    backup_schedule: Entity<TimeFieldState>,
}

impl Inputs {
    fn text(&self, field: TextField) -> &Entity<InputState> {
        match field {
            TextField::RommUrl => &self.romm_url,
            TextField::BackupRemote => &self.backup_remote,
            TextField::BackupRoot => &self.backup_root,
        }
    }
}

/// Which control, if any, owns the caret - the footer says something different for
/// each, because they consume the arrows differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Editing {
    None,
    /// A single-line text field: arrows move the caret, nothing steps a value.
    Text,
    /// The schedule: left/right pick a part, up/down step it.
    Schedule,
}

/// What a row needs to know about the cursor: which page it is on, where it sits
/// in the flat page, whether it is the focused row, and its index among the rows
/// the cursor can reach (`None` for a fact, which can never be focused).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RowFocus {
    page: usize,
    flat: usize,
    cursor: Option<usize>,
    focused: bool,
}

/// Values the first read brings, applied on the next render: a text field cannot
/// be filled without a window, and the read now finishes after the window exists.
struct Prefill {
    romm_url: String,
    backup_remote: String,
    backup_root: String,
    schedule: Option<chrono::NaiveTime>,
}

impl Prefill {
    fn from_snapshot(snapshot: &Snapshot) -> Self {
        let backup = snapshot.backup.as_ref().map(daemon::BackupView::settings);
        Self {
            romm_url: snapshot
                .romm
                .as_ref()
                .map(|romm| romm.base_url.clone())
                .unwrap_or_default(),
            backup_remote: backup
                .as_ref()
                .map(|settings| settings.remote.clone())
                .unwrap_or_default(),
            backup_root: backup
                .as_ref()
                .map(|settings| settings.root.clone())
                .unwrap_or_default(),
            schedule: backup.as_ref().and_then(|settings| {
                chrono::NaiveTime::from_hms_opt(
                    u32::from(settings.schedule.hour),
                    u32::from(settings.schedule.minute),
                    0,
                )
            }),
        }
    }
}

struct DiskUsage {
    total: u64,
    available: u64,
}

impl DiskUsage {
    fn used_percent(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        let used = self.total.saturating_sub(self.available);
        (used as f64 / self.total as f64 * 100.0) as f32
    }
}

fn home_usage() -> Option<DiskUsage> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    let path = CString::new(home).ok()?;
    let vfs = statvfs(path.as_c_str()).ok()?;
    let unit = vfs.fragment_size();
    Some(DiskUsage {
        total: vfs.blocks() * unit,
        available: vfs.blocks_available() * unit,
    })
}

/// "1 core", "4 cores": a count that reads like a sentence rather than a form.
fn plural(count: usize, singular: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {singular}s")
    }
}

/// A timestamp the daemon sends (RFC 3339, UTC) as something a person reads:
/// "7 Nov, 14:43" in local time. An unparseable string is shown as it came.
fn local_time(raw: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%-e %b, %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| raw.to_string())
}

/// Same output as the frontend's `system_status::human_size`.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{:.0} {}", value, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

// ---------------------------------------------------------------------------
// The window
// ---------------------------------------------------------------------------

struct SettingsWindow {
    snapshot: Snapshot,
    disk: Option<DiskUsage>,
    nav: Nav,
    inputs: Inputs,
    /// Set while a refresh is in flight, so the row says so and Enter cannot
    /// stack a second read behind the first.
    refreshing: bool,
    /// The backup switch the user flipped but Save has not written yet.
    backup_pending: Option<bool>,
    /// Set while a started run is being followed, so one watcher at a time.
    watching_backup: bool,
    /// Whether the first successful read has already filled the form.
    prefilled: bool,
    pending_prefill: Option<Prefill>,
    /// The value the window asked the daemon for while that write is in flight.
    /// The switch shows it, so the control answers the press immediately, and
    /// drops back to the daemon's own answer when the write lands or fails.
    categorization_pending: Option<bool>,
    /// The approval waiting on the user, whichever service asked for it. One
    /// field, because RomM and Stremio answer with the same shapes.
    pending_link: Option<PendingLink>,
    /// Bumped whenever a pending approval is replaced, so the older poll loop
    /// stops instead of writing over the newer one.
    link_generation: u64,
    /// The service whose code was asked for and has not arrived yet.
    linking: Option<LinkService>,
    /// The daemon request in flight - a link, an unlink, a maintenance run - so
    /// its row is disabled until the daemon answers.
    busy: Option<RowAction>,
    /// The scrolling page: one item per card heading, card row, or gap. A list,
    /// not a column, because `scroll_to_reveal_item` is what keeps the cursor's
    /// row on screen - the page has to be navigable with no pointer.
    list: ListState,
    /// The page the list was built for, and the row it was last asked to reveal.
    list_page: usize,
    scrolled_to: Option<(usize, usize)>,
    /// Results waiting for the next frame, where a window exists to deliver
    /// them. They are notifications, not page state: a completed action is not
    /// a fact about the configuration.
    notices: Vec<Notice>,
    /// Set by the first Back that would discard something, so the second one is
    /// the answer rather than a surprise.
    close_armed: bool,
}

impl SettingsWindow {
    fn new(snapshot: Snapshot, disk: Option<DiskUsage>, inputs: Inputs) -> Self {
        Self {
            snapshot,
            disk,
            nav: Nav::default(),
            inputs,
            refreshing: false,
            categorization_pending: None,
            backup_pending: None,
            watching_backup: false,
            prefilled: false,
            pending_prefill: None,
            pending_link: None,
            link_generation: 0,
            linking: None,
            busy: None,
            list: ListState::new(0, ListAlignment::Top, px(400.)),
            list_page: 0,
            scrolled_to: None,
            notices: Vec::new(),
            close_armed: false,
        }
    }

    /// Fills the form from the first read. A text field cannot be set without a
    /// window, so this runs at render rather than in the read's completion.
    fn apply_prefill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prefill) = self.pending_prefill.take() else {
            return;
        };
        let romm = self.inputs.romm_url.clone();
        romm.update(cx, |state, cx| {
            state.set_value(prefill.romm_url, window, cx)
        });
        let remote = self.inputs.backup_remote.clone();
        remote.update(cx, |state, cx| {
            state.set_value(prefill.backup_remote, window, cx);
        });
        let root = self.inputs.backup_root.clone();
        root.update(cx, |state, cx| {
            state.set_value(prefill.backup_root, window, cx);
        });
        if let Some(schedule) = prefill.schedule {
            self.inputs
                .backup_schedule
                .update(cx, |state, cx| state.set_time(schedule, window, cx));
        }
        cx.notify();
    }

    /// Everything the window draws, re-derived from the snapshot. Building it
    /// from state rather than caching it is what keeps a refresh honest: there
    /// is no second copy of the page to fall out of step with the read.
    fn pages(&self) -> Vec<Page> {
        // A daemon that never answered is a fact about this window, not about the
        // user's setup. Rendering "Not set up" over services that are fine is
        // worse than an error: it looks like a fact. Every page collapses to one
        // card that says what happened and offers the retry.
        if !self.snapshot.daemon {
            let (label, description, tone) = if self.refreshing {
                (
                    "Reading",
                    "Reading the daemon and this machine.",
                    Tone::Busy,
                )
            } else {
                (
                    "Unavailable",
                    "Could not reach the daemon. Enter to try again.",
                    Tone::Danger,
                )
            };
            return PageKind::ALL
                .iter()
                .map(|kind| Page {
                    kind: *kind,
                    groups: vec![Group {
                        title: "Daemon",
                        status: Some((label, tone)),
                        rows: vec![Row {
                            label: "Backend",
                            description: Some(description),
                            control: Control::Action(RowAction::Refresh),
                        }],
                    }],
                })
                .collect();
        }

        let daemon = if self.snapshot.daemon {
            "Connected".to_string()
        } else {
            "Unavailable".to_string()
        };
        let readiness = if self.snapshot.total_systems == 0 {
            "Nothing reported yet".to_string()
        } else {
            format!(
                "{} of {} systems ready",
                self.snapshot.ready_systems, self.snapshot.total_systems
            )
        };
        let romm = match &self.snapshot.romm {
            Some(romm) if romm.configured => format!("Configured at {}", romm.base_url),
            Some(romm) => format!("Connected, not signed in to {}", romm.base_url),
            None => "Not set up".to_string(),
        };
        let stremio = stremio_account(
            self.snapshot.stremio_linked_at.as_deref(),
            self.pending_link
                .as_ref()
                .is_some_and(|pending| pending.service == LinkService::Stremio),
        );
        let storage = match &self.disk {
            Some(disk) => Value::Storage {
                used_percent: disk.used_percent(),
                available: human_size(disk.available),
                total: human_size(disk.total),
            },
            None => Value::Text("Unavailable".to_string()),
        };

        let mut romm_rows = vec![Row {
            label: "Server address",
            description: Some("Where your RomM library is served."),
            control: Control::Text(TextField::RommUrl),
        }];
        romm_rows.extend(self.link_rows(LinkService::RomM));
        if self.snapshot.romm.is_some() {
            romm_rows.push(Row {
                label: "Saved connection",
                description: None,
                control: Control::Fact(Value::Text(romm)),
            });
        }
        romm_rows.push(if self.snapshot.romm.is_some() {
            Row {
                label: "Forget connection",
                description: Some("Removes the stored token from the daemon."),
                control: Control::Action(RowAction::RommForget),
            }
        } else {
            Row {
                label: "Link RomM",
                description: Some("A code is shown here to approve in RomM."),
                control: Control::Action(RowAction::RommLink),
            }
        });

        let mut stremio_rows = vec![Row {
            label: "Account",
            description: None,
            control: Control::Fact(Value::Text(stremio)),
        }];
        stremio_rows.extend(self.link_rows(LinkService::Stremio));
        stremio_rows.push(if self.snapshot.stremio_linked_at.is_some() {
            Row {
                label: "Unlink account",
                description: Some("Stops reading the Stremio addons and removes their rows."),
                control: Control::Action(RowAction::StremioUnlink),
            }
        } else {
            Row {
                label: "Link account",
                description: Some("A code is shown here to approve elsewhere."),
                control: Control::Action(RowAction::StremioLink),
            }
        });

        // rclone owns the providers: this card reports what it has rather than
        // offering to store a credential of our own.
        let rclone_card = || {
            let view = self.snapshot.backup.as_ref();
            let installed = view.is_some_and(|view| view.rclone_installed);
            let remotes = view.map(|view| view.remotes.clone()).unwrap_or_default();
            let mut rows = vec![Row {
                label: "rclone",
                description: Some("The tool that performs the upload."),
                control: Control::Fact(Value::Text(
                    if installed {
                        "Installed"
                    } else {
                        "Not installed"
                    }
                    .to_string(),
                )),
            }];
            // A missing rclone is the page alert's job; the card only carries
            // the facts and the refresh action.
            if installed && remotes.is_empty() {
                rows.push(Row {
                    label: "No remotes configured",
                    description: Some(
                        "Run `rclone config` on this machine to add one, then refresh this page.",
                    ),
                    control: Control::Fact(Value::Text("None".to_string())),
                });
            } else if installed {
                for remote in remotes {
                    rows.push(Row {
                        label: "Remote",
                        description: None,
                        control: Control::Fact(Value::Text(remote)),
                    });
                }
            }
            rows.push(Row {
                label: "Refresh",
                description: Some("Read rclone again after changing its configuration."),
                control: Control::Action(RowAction::Refresh),
            });
            Group {
                title: "rclone",
                status: Some(if installed {
                    ("Installed", Tone::Good)
                } else {
                    ("Missing", Tone::Attention)
                }),
                rows,
            }
        };

        vec![
            Page {
                kind: PageKind::Status,
                groups: vec![
                    Group {
                        title: "Daemon",
                        status: Some(if self.snapshot.daemon {
                            ("Connected", Tone::Good)
                        } else {
                            ("Unavailable", Tone::Off)
                        }),
                        rows: vec![
                            Row {
                                label: "Backend",
                                description: Some("The loopback service this window reads."),
                                control: Control::Fact(Value::Text(daemon)),
                            },
                            Row {
                                label: "Retro readiness",
                                description: Some(
                                    "Systems whose core and required BIOS are ready.",
                                ),
                                control: Control::Fact(Value::Text(readiness.clone())),
                            },
                        ],
                    },
                    Group {
                        title: "Storage",
                        status: None,
                        rows: vec![Row {
                            label: "Home volume",
                            description: Some("Free space on this machine."),
                            control: Control::Fact(storage),
                        }],
                    },
                    Group {
                        title: "This window",
                        status: None,
                        rows: vec![Row {
                            label: "Refresh",
                            description: Some("Read the daemon and this machine again."),
                            control: Control::Action(RowAction::Refresh),
                        }],
                    },
                ],
            },
            Page {
                kind: PageKind::Connections,
                groups: vec![
                    Group {
                        title: "RomM",
                        status: Some(match &self.snapshot.romm {
                            Some(romm) if romm.configured => ("Signed in", Tone::Good),
                            Some(_) => ("Needs sign-in", Tone::Attention),
                            None => ("Not set up", Tone::Off),
                        }),
                        rows: romm_rows,
                    },
                    Group {
                        title: "Stremio",
                        status: Some(if self.snapshot.stremio_linked_at.is_some() {
                            ("Linked", Tone::Good)
                        } else if self
                            .pending_link
                            .as_ref()
                            .is_some_and(|pending| pending.service == LinkService::Stremio)
                        {
                            ("Waiting for approval", Tone::Busy)
                        } else {
                            ("Not linked", Tone::Off)
                        }),
                        rows: stremio_rows,
                    },
                ],
            },
            Page {
                kind: PageKind::Library,
                groups: vec![
                    Group {
                        title: "Categories",
                        status: None,
                        rows: vec![
                            Row {
                                label: "Group the library",
                                description: Some(
                                    "Let Hearthdeck derive sections and collections from the catalog.",
                                ),
                                control: Control::Toggle(ToggleField::Categorization),
                            },
                            Row {
                                label: "Scan now",
                                description: Some("Runs one categorization pass over the catalog."),
                                control: Control::Action(RowAction::ScanLibrary),
                            },
                        ],
                    },
                    Group {
                        title: "Dashboard",
                        status: None,
                        rows: vec![Row {
                            label: "Restore rails",
                            description: Some(
                                "Removes the rails the last scan published; the dashboard derives its own.",
                            ),
                            control: Control::Action(RowAction::RestoreRails),
                        }],
                    },
                ],
            },
            Page {
                kind: PageKind::Emulation,
                groups: vec![{
                    let mut rows = vec![Row {
                        label: "Systems",
                        description: Some("What RetroArch can start right now."),
                        control: Control::Fact(Value::Text(readiness)),
                    }];
                    // Name what is missing, so "Install missing cores" is a
                    // decision and not a guess. Five is as many as a card can
                    // carry before it stops being scannable.
                    for system in self.snapshot.missing_systems.iter().take(5) {
                        rows.push(Row {
                            label: "Not ready",
                            description: None,
                            control: Control::Fact(Value::Text(format!(
                                "{} — {}",
                                system.name, system.reason
                            ))),
                        });
                    }
                    let more = self.snapshot.missing_systems.len().saturating_sub(5);
                    if more > 0 {
                        rows.push(Row {
                            label: "Not ready",
                            description: None,
                            control: Control::Fact(Value::Text(format!("and {more} more"))),
                        });
                    }
                    rows.push(Row {
                        label: "Missing cores",
                        description: Some("Cores the daemon knows how to fetch."),
                        control: Control::Action(RowAction::InstallCores),
                    });
                    rows.push(Row {
                        label: "BIOS files",
                        description: Some("Extra firmware, for the systems that need it."),
                        control: Control::Action(RowAction::SyncBios),
                    });
                    Group {
                        title: "Readiness",
                        status: None,
                        rows,
                    }
                }],
            },
            Page {
                kind: PageKind::Backup,
                groups: match &self.snapshot.backup {
                    None => vec![
                        rclone_card(),
                        Group {
                            title: "Off-site backup",
                            status: Some(("Not installed", Tone::Off)),
                            rows: vec![Row {
                                label: "Availability",
                                description: Some(
                                    "This Hearthdeck build does not include the backup units.",
                                ),
                                control: Control::Fact(Value::Text("Not installed".to_string())),
                            }],
                        },
                    ],
                    Some(backup) => {
                        let mut rows = vec![
                            Row {
                                label: "Nightly copy",
                                description: Some(
                                    "Runs nightly through the systemd timer. Save carries it with the destination.",
                                ),
                                control: Control::Toggle(ToggleField::Backup),
                            },
                            Row {
                                label: "Remote",
                                description: Some(
                                    "An rclone remote name; its credentials stay in rclone's own config.",
                                ),
                                control: Control::Text(TextField::BackupRemote),
                            },
                            Row {
                                label: "Destination root",
                                description: Some("The folder under that remote."),
                                control: Control::Text(TextField::BackupRoot),
                            },
                            Row {
                                label: "Schedule",
                                description: Some(
                                    "Enter to edit; then up and down set the hour, left and right pick the part.",
                                ),
                                control: Control::Time,
                            },
                        ];
                        rows.push(Row {
                            label: "Save and apply",
                            description: Some(
                                "Writes the destination, the schedule and the timer's state.",
                            ),
                            control: Control::Action(RowAction::SaveBackup),
                        });
                        if let Some(updated_at) = &backup.updated_at {
                            rows.push(Row {
                                label: "Updated",
                                description: None,
                                control: Control::Fact(Value::Text(local_time(updated_at))),
                            });
                        }

                        let status = self.snapshot.backup_status.as_ref();
                        // One row per fact a person can act on. A "None" error row and
                        // a "Never" timestamp are noise, and a result without the time
                        // it finished is half an answer, so they are merged.
                        let last_result = status
                            .and_then(|status| status.last_result.clone())
                            .filter(|result| !result.is_empty());
                        let last_finished = status
                            .and_then(|status| status.last_finished_at.as_deref())
                            .map(local_time);
                        let mut last = vec![
                            Row {
                                label: "Next run",
                                description: None,
                                control: Control::Fact(Value::Text(
                                    status
                                        .and_then(|status| status.next_run.as_deref())
                                        .map(local_time)
                                        .unwrap_or_else(|| "Not scheduled".to_string()),
                                )),
                            },
                            Row {
                                label: "Last run",
                                description: None,
                                control: Control::Fact(Value::Text(
                                    match (last_result, last_finished) {
                                        (Some(result), Some(finished)) => {
                                            format!("{result}, {finished}")
                                        }
                                        (Some(result), None) => result,
                                        (None, Some(finished)) => format!("finished {finished}"),
                                        (None, None) => "Never run".to_string(),
                                    },
                                )),
                            },
                        ];
                        if let Some(error) = status
                            .and_then(|status| status.last_error.clone())
                            .filter(|error| !error.is_empty())
                        {
                            last.push(Row {
                                label: "Last error",
                                description: Some("The runner's own words."),
                                control: Control::Fact(Value::Text(error)),
                            });
                        }
                        last.push(Row {
                            label: "Run now",
                            description: Some("Starts one run immediately; it can take minutes."),
                            control: Control::Action(RowAction::RunBackup),
                        });

                        vec![
                            rclone_card(),
                            Group {
                                title: "Off-site backup",
                                status: Some(if self.backup_pending.is_some() {
                                    ("Unsaved changes", Tone::Attention)
                                } else if backup.enabled {
                                    ("Nightly copy on", Tone::Good)
                                } else {
                                    ("Nightly copy off", Tone::Off)
                                }),
                                rows,
                            },
                            Group {
                                title: "Last run",
                                status: Some(match status {
                                    Some(status) if status.service_active => {
                                        ("Running", Tone::Busy)
                                    }
                                    Some(status)
                                        if status.last_result.as_deref().is_some_and(
                                            |result| !matches!(result, "success" | ""),
                                        ) =>
                                    {
                                        ("Last run failed", Tone::Danger)
                                    }
                                    Some(status) if !status.timer_active => {
                                        ("Timer off", Tone::Off)
                                    }
                                    Some(_) => ("Idle", Tone::Off),
                                    None => ("Unknown", Tone::Danger),
                                }),
                                rows: last,
                            },
                        ]
                    }
                },
            },
        ]
    }

    /// A condition that *is* the page's state, drawn as an [`Alert`] above the
    /// cards. Everything else - a saved setting, a refused write, a finished
    /// run - is a result and goes out as a notification. An alert stays until
    /// the condition does, so it can be read at any point rather than caught as
    /// it slides past.
    fn page_alert(&self, kind: PageKind) -> Option<AnyElement> {
        if !self.snapshot.daemon {
            return Some(
                Alert::error(
                    "daemon-alert",
                    "Could not reach the daemon. Start it, then choose Refresh.",
                )
                .title("Daemon unavailable")
                .large()
                .into_any_element(),
            );
        }
        if kind != PageKind::Backup {
            return None;
        }
        let Some(backup) = self.snapshot.backup.as_ref() else {
            return Some(
                Alert::warning(
                    "backup-missing",
                    "This Hearthdeck build does not ship the backup units.",
                )
                .title("Backup is not installed")
                .large()
                .into_any_element(),
            );
        };
        if !backup.rclone_installed {
            return Some(
                Alert::warning(
                    "rclone-missing",
                    "Install it with `sudo pacman -S --needed rclone`, then refresh this page.",
                )
                .title("rclone is not installed")
                .large()
                .into_any_element(),
            );
        }
        if !backup.remote_available {
            return Some(
                Alert::warning(
                    "remote-missing",
                    format!(
                        "Add `{}` with `rclone config`, then refresh this page.",
                        backup.remote
                    ),
                )
                .title("Remote not found")
                .large()
                .into_any_element(),
            );
        }
        self.backup_pending.is_some().then(|| {
            Alert::info(
                "backup-unsaved",
                "Save and apply writes the switch, the destination and the schedule together.",
            )
            .title("Unsaved backup changes")
            .large()
            .into_any_element()
        })
    }

    /// The focused row's control, so a key handler and the drawn cursor cannot
    /// disagree about what Enter would do.
    fn selection(&self) -> Option<Control> {
        let pages = self.pages();
        let page = pages.get(self.nav.page())?;
        let mut row = *page.cursor_rows().get(self.nav.row())?;
        for group in &page.groups {
            if row < group.rows.len() {
                return Some(group.rows[row].control.clone());
            }
            row -= group.rows.len();
        }
        None
    }

    /// Queues a result for the next frame. A result is a notification: it is
    /// over the moment it is read, so drawing it into the page would give it the
    /// same weight as a setting.
    fn announce(&mut self, level: NoticeLevel, text: impl Into<String>) {
        if self.notices.len() >= PENDING_NOTICES {
            self.notices.remove(0);
        }
        self.notices.push(Notice {
            level,
            text: text.into(),
        });
    }

    /// A completed action.
    fn say(&mut self, text: impl Into<String>) {
        self.announce(NoticeLevel::Success, text);
    }

    /// A refusal, or a failure.
    fn fail(&mut self, text: impl Into<String>) {
        self.announce(NoticeLevel::Error, text);
    }

    /// Something the user should know but that did not fail outright.
    fn warn(&mut self, text: impl Into<String>) {
        self.announce(NoticeLevel::Warning, text);
    }

    /// Something the user asked to happen, stated without an outcome yet.
    fn inform(&mut self, text: impl Into<String>) {
        self.announce(NoticeLevel::Info, text);
    }

    /// Hands every queued result to the kit's notification list.
    ///
    /// It runs at render because that is the only place this window holds a
    /// `Window`: the writes finish on background tasks, which own a `Context`
    /// but cannot push a toast. Rendering never fails on this, and the queue is
    /// bounded, so a burst of completions cannot leave a growing list behind.
    fn deliver_notices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for notice in self.notices.drain(..) {
            window.push_notification(notice.level.notification(notice.text), cx);
        }
    }

    /// Reads the daemon and the disk on a background thread: both are blocking,
    /// and a dead daemon must not freeze the window it is reporting on.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (snapshot, disk) = cx
                .background_executor()
                .spawn(async { (read_snapshot(), home_usage()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.snapshot = snapshot;
                this.disk = disk;
                this.refreshing = false;
                // The first read brings the values the form edits, and a field
                // cannot be filled without a window - so it waits for the render.
                if this.snapshot.daemon && !this.prefilled {
                    this.prefilled = true;
                    this.pending_prefill = Some(Prefill::from_snapshot(&this.snapshot));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn run(&mut self, action: RowAction, cx: &mut Context<Self>) {
        if !self.action_enabled(action) {
            self.warn(action.blocked());
            cx.notify();
            return;
        }
        match action {
            RowAction::Refresh => self.refresh(cx),
            RowAction::ScanLibrary => self.scan_categorization(cx),
            RowAction::RommLink => self.start_link(LinkService::RomM, cx),
            RowAction::RommForget => self.unlink(LinkService::RomM, cx),
            RowAction::StremioLink => self.start_link(LinkService::Stremio, cx),
            RowAction::StremioUnlink => self.unlink(LinkService::Stremio, cx),
            RowAction::SaveBackup => self.save_backup(cx),
            RowAction::RunBackup => self.start_backup(cx),
            RowAction::InstallCores => self.install_cores(cx),
            RowAction::SyncBios => self.sync_bios(cx),
            RowAction::RestoreRails => self.restore_rails(cx),
        }
    }

    fn action_enabled(&self, action: RowAction) -> bool {
        if self.busy.is_some() || self.linking.is_some() {
            return false;
        }
        match action {
            RowAction::Refresh => !self.refreshing,
            RowAction::RunBackup => !self
                .snapshot
                .backup_status
                .as_ref()
                .is_some_and(|status| status.service_active),
            RowAction::ScanLibrary => self.categorization_enabled(),
            RowAction::RommLink | RowAction::StremioLink => self.pending_link.is_none(),
            _ => true,
        }
    }

    /// Whether this row's own request is the one in flight, so its button spins
    /// instead of the page narrating "saving..." somewhere else.
    fn action_loading(&self, action: RowAction) -> bool {
        if self.busy == Some(action) {
            return true;
        }
        match action {
            RowAction::Refresh => self.refreshing,
            RowAction::RommLink => self.linking == Some(LinkService::RomM),
            RowAction::StremioLink => self.linking == Some(LinkService::Stremio),
            _ => false,
        }
    }

    /// The opt-in as the daemon last reported it - never as a click assumed it.
    fn categorization_enabled(&self) -> bool {
        self.snapshot
            .categorization
            .as_ref()
            .is_some_and(|categorization| categorization.enabled)
    }

    /// What the switch shows: the write in flight, else the daemon's answer.
    fn toggle_value(&self, field: ToggleField) -> bool {
        match field {
            ToggleField::Categorization => mirrored(
                self.categorization_pending,
                self.snapshot
                    .categorization
                    .as_ref()
                    .map(|categorization| categorization.enabled),
            ),
            ToggleField::Backup => mirrored(
                self.backup_pending,
                self.snapshot.backup.as_ref().map(|backup| backup.enabled),
            ),
        }
    }

    /// A toggle whose state was never read is drawn disabled rather than
    /// focused and dead, and so is one while its write is in flight.
    /// Why a switch cannot move, when the reason is the daemon's state. A write in
    /// flight is not a missing remote, so this stays `None` while one is pending.
    fn toggle_blocked_reason(&self, field: ToggleField) -> Option<&'static str> {
        match field {
            ToggleField::Backup => {
                let backup = self.snapshot.backup.as_ref()?;
                (!backup.remote_available && !backup.enabled).then_some("Needs a remote")
            }
            ToggleField::Categorization => {
                let categorization = self.snapshot.categorization.as_ref()?;
                (categorization.capable == Some(false)).then_some("Unavailable")
            }
        }
    }

    fn toggle_enabled(&self, field: ToggleField) -> bool {
        match field {
            ToggleField::Categorization => {
                self.categorization_pending.is_none()
                    && self
                        .snapshot
                        .categorization
                        .as_ref()
                        .is_some_and(|categorization| categorization.capable != Some(false))
            }
            // Turning it on needs a remote rclone knows, or the timer would only
            // fail nightly. Turning it off never does: a remote that vanished
            // must not leave a running timer un-disablable.
            ToggleField::Backup => {
                // The backup "pending" is a draft, not a write in flight, so the
                // switch stays live: flipping it back is how a draft is undone.
                self.snapshot
                    .backup
                    .as_ref()
                    .is_some_and(|backup| backup.remote_available || backup.enabled)
            }
        }
    }

    /// Flips the control the user pressed. Categorization writes immediately;
    /// backup only marks the draft, because Save carries the destination and the
    /// schedule with it.
    fn set_toggle(&mut self, field: ToggleField, value: bool, cx: &mut Context<Self>) {
        match field {
            ToggleField::Categorization => self.set_categorization(value, cx),
            ToggleField::Backup => {
                // A draft, not a write. The heading's `Unsaved changes` marker
                // and the page alert say it; a toast would repeat them.
                self.backup_pending = Some(value);
                cx.notify();
            }
        }
    }

    /// Turns the opt-in on or off on the daemon, then settles on what it says.
    fn set_categorization(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.categorization_pending.is_some() {
            return;
        }
        self.categorization_pending = Some(enabled);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { daemon::set_categorization(enabled) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.categorization_pending = None;
                match result {
                    Ok(state) => {
                        // The write's read does not carry health, so capability
                        // stays what the snapshot already knew.
                        let capable = this
                            .snapshot
                            .categorization
                            .as_ref()
                            .and_then(|categorization| categorization.capable);
                        if state.enabled {
                            this.say("Categorization is on.");
                        } else {
                            this.say("Categorization is off.");
                        }
                        this.snapshot.categorization = Some(Categorization { capable, ..state });
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the categorization write failed");
                        this.fail(format!("Categorization unchanged: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn scan_categorization(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { daemon::scan_categorization() })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.say("The daemon accepted the scan."),
                    Err(error) => this.fail(format!("The scan was not started: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The rows a pending approval adds, whichever service asked. One helper,
    /// because the daemon answers both with the same shape.
    fn link_rows(&self, service: LinkService) -> Vec<Row> {
        let Some(pending) = self
            .pending_link
            .as_ref()
            .filter(|pending| pending.service == service)
        else {
            return Vec::new();
        };
        vec![
            Row {
                label: "Code",
                description: Some("Enter this in the app, or open the link below."),
                control: Control::Fact(Value::Code(pending.offer.code.clone())),
            },
            Row {
                label: "Approve at",
                description: None,
                control: Control::Fact(Value::Text(pending.offer.link.clone())),
            },
            Row {
                label: "Waiting",
                description: Some("Until the code is approved or runs out."),
                control: Control::Fact(Value::Text(match pending.seconds_left {
                    Some(seconds) => format!("{seconds}s left"),
                    None => "checking...".to_string(),
                })),
            },
        ]
    }

    /// Asks the daemon for a code, then waits on it. RomM needs the address of
    /// the server to ask; Stremio needs nothing.
    fn start_link(&mut self, service: LinkService, cx: &mut Context<Self>) {
        if self.linking.is_some() || self.pending_link.is_some() {
            return;
        }
        let base_url = match service {
            LinkService::RomM => {
                let url = self.inputs.romm_url.read(cx).value().trim().to_string();
                if url.is_empty() {
                    self.warn("Fill in the RomM server address first.");
                    cx.notify();
                    return;
                }
                Some(url)
            }
            LinkService::Stremio => None,
        };
        self.linking = Some(service);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match base_url {
                        Some(url) => daemon::start_romm_link(url),
                        None => daemon::start_stremio_link(),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.linking = None;
                match result {
                    Ok(offer) => {
                        this.inform(format!(
                            "Approve code {} in {}.",
                            offer.code,
                            service.label()
                        ));
                        this.link_generation = this.link_generation.wrapping_add(1);
                        let generation = this.link_generation;
                        this.pending_link = Some(PendingLink {
                            service,
                            seconds_left: Some(offer.expires_in_seconds),
                            offer,
                            generation,
                        });
                        this.poll_link(service, generation, cx);
                    }
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "the {} link could not be started",
                            service.label()
                        );
                        this.fail(format!("{} link not started: {error}", service.label()));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Polls until the code is approved, refused, expired, or replaced. The loop
    /// stops as soon as its generation is no longer the pending one.
    fn poll_link(&mut self, service: LinkService, generation: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                let status = cx
                    .background_executor()
                    .spawn(async move {
                        match service {
                            LinkService::RomM => daemon::romm_link_status(),
                            LinkService::Stremio => daemon::stremio_link_status(),
                        }
                    })
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        this.apply_link_status(service, generation, status, cx)
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
        })
        .detach();
    }

    /// Applies one poll. Returns whether the loop should keep going.
    fn apply_link_status(
        &mut self,
        service: LinkService,
        generation: u64,
        status: Result<LinkStatus, daemon::Error>,
        cx: &mut Context<Self>,
    ) -> bool {
        let current = self
            .pending_link
            .as_ref()
            .is_some_and(|pending| pending.service == service && pending.generation == generation);
        if !current {
            return false;
        }
        match status {
            Ok(LinkStatus::Waiting { seconds_left }) => {
                if let Some(pending) = self.pending_link.as_mut() {
                    pending.seconds_left = Some(seconds_left);
                }
                cx.notify();
                true
            }
            Ok(LinkStatus::Linked { account }) => {
                self.say(format!("{} linked: {account}.", service.label()));
                match service {
                    LinkService::RomM => {
                        self.snapshot.romm = Some(daemon::Romm {
                            base_url: account,
                            configured: true,
                        });
                    }
                    LinkService::Stremio => self.snapshot.stremio_linked_at = Some(account),
                }
                self.pending_link = None;
                cx.notify();
                false
            }
            Ok(LinkStatus::Expired) => {
                self.warn(format!(
                    "The {} code expired; ask for a new one.",
                    service.label()
                ));
                self.pending_link = None;
                cx.notify();
                false
            }
            Ok(LinkStatus::Denied) => {
                self.fail(format!("{} refused the approval.", service.label()));
                self.pending_link = None;
                cx.notify();
                false
            }
            Ok(LinkStatus::Idle) => {
                self.inform(format!("The {} link was cancelled.", service.label()));
                self.pending_link = None;
                cx.notify();
                false
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    "the {} link status could not be read",
                    service.label()
                );
                self.fail(format!("{}: {error}", service.label()));
                self.pending_link = None;
                cx.notify();
                false
            }
        }
    }

    /// Drops a stored connection. Two services, two endpoints, one flow.
    fn unlink(&mut self, service: LinkService, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(match service {
            LinkService::RomM => RowAction::RommForget,
            LinkService::Stremio => RowAction::StremioUnlink,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match service {
                        LinkService::RomM => daemon::clear_romm(),
                        LinkService::Stremio => daemon::unlink_stremio(),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(()) => {
                        this.say(format!("The {} connection was removed.", service.label()));
                        match service {
                            LinkService::RomM => this.snapshot.romm = None,
                            LinkService::Stremio => this.snapshot.stremio_linked_at = None,
                        }
                        // Nothing is waiting on a code any more, and an older
                        // poll must not bring the account back.
                        this.link_generation = this.link_generation.wrapping_add(1);
                        this.pending_link = None;
                    }
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "the {} connection could not be removed",
                            service.label()
                        );
                        this.fail(format!("Not removed: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Writes the destination, the schedule and the switch in one request: the
    /// daemon validates them together, so a refused write leaves the old
    /// configuration whole.
    fn save_backup(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let time = self.inputs.backup_schedule.read(cx).time();
        let settings = daemon::BackupSettings {
            enabled: self.toggle_value(ToggleField::Backup),
            remote: self.inputs.backup_remote.read(cx).value().to_string(),
            root: self.inputs.backup_root.read(cx).value().to_string(),
            schedule: daemon::BackupSchedule {
                hour: time.hour() as u8,
                minute: time.minute() as u8,
            },
        };
        self.busy = Some(RowAction::SaveBackup);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { daemon::save_backup(&settings) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(view) => {
                        this.snapshot.backup = Some(view);
                        this.backup_pending = None;
                        this.say("The backup settings were saved.");
                        this.close_armed = false;
                        // The schedule decides the next run, and the timer's new
                        // state decides whether there is one: read both back
                        // instead of leaving the card on the old answer.
                        this.refresh_backup(cx);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the backup settings were refused");
                        // Nothing was written, so the draft is not worth keeping:
                        // the switch drops back to what the daemon actually holds.
                        this.backup_pending = None;
                        this.fail(format!("Backup not saved: {error}"));
                        // A PUT can fail *after* writing the files - the timer
                        // could not be moved - so the page reads back what the
                        // daemon actually holds instead of the old values.
                        this.refresh_backup(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Starts one run. The daemon answers accepted and the last-run card fills in
    /// when the service finishes, so this does not wait.
    fn start_backup(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(RowAction::RunBackup);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { daemon::run_backup() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(()) => {
                        this.inform("The backup was started; it can take minutes.");
                        this.watch_backup(cx);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the backup run could not be started");
                        this.fail(format!("Backup not started: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// One re-read of the destination and the last run, after a write.
    fn refresh_backup(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async { daemon::read_backup() })
                .await;
            let _ = this.update(cx, |this, cx| match read {
                Ok((view, status)) => {
                    this.snapshot.backup = Some(view);
                    this.snapshot.backup_status = Some(status);
                    cx.notify();
                }
                Err(error) => {
                    tracing::warn!(%error, "the backup state could not be re-read");
                }
            });
        })
        .detach();
    }

    /// Follows a started run until the service stops, so the last-run card fills
    /// in by itself.
    ///
    /// Read first, then wait: the route starts the unit with `--no-block`, so the
    /// job may not be `activating` yet when the first answer arrives, and a run
    /// that finishes quickly would be missed entirely by a sleep-first loop.
    fn watch_backup(&mut self, cx: &mut Context<Self>) {
        if self.watching_backup {
            return;
        }
        self.watching_backup = true;
        cx.spawn(async move |this, cx| {
            let mut state = WatchState::default();
            let mut polls = 0u32;
            let end = loop {
                let read = cx
                    .background_executor()
                    .spawn(async { daemon::read_backup_status() })
                    .await;
                let step = watch_step(&mut state, &read);
                let updated = this.update(cx, |this, cx| {
                    match &read {
                        Ok(status) => {
                            this.snapshot.backup_status = Some(status.clone());
                        }
                        Err(error) => tracing::debug!(%error, "a backup status poll failed"),
                    }
                    cx.notify();
                });
                if updated.is_err() {
                    // The window is gone; there is nothing left to follow.
                    return;
                }
                if let WatchStep::Stop(end) = step {
                    break Some(end);
                }
                polls += 1;
                if polls >= WATCH_MAX_POLLS {
                    break None;
                }
                cx.background_executor().timer(watch_interval(polls)).await;
            };
            if end == Some(WatchEnd::Unreadable) {
                tracing::warn!("stopped following the backup run: the daemon could not be read");
            }
            let _ = this.update(cx, |this, cx| {
                this.watching_backup = false;
                match end {
                    Some(WatchEnd::Finished) => this.say("The last run finished."),
                    Some(WatchEnd::NeverStarted) => {
                        this.warn("No run was seen within a minute of starting it.")
                    }
                    Some(WatchEnd::Unreadable) => {
                        this.fail("Stopped following the run: the daemon could not be read.")
                    }
                    None => this.warn("Stopped following the run after about six hours."),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn install_cores(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(RowAction::InstallCores);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (report, readiness) = cx
                .background_executor()
                .spawn(async { (daemon::install_cores(), daemon::read_readiness().ok()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                match report {
                    Ok(report) => {
                        let text = describe_install(&report);
                        if report.failed.is_empty() {
                            this.say(text);
                        } else if report.installed.is_empty() {
                            this.fail(text);
                        } else {
                            this.warn(text);
                        }
                        this.apply_readiness(readiness);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the core install failed");
                        this.fail(format!("Cores not installed: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Places whatever BIOS files and source-only cores the daemon is missing,
    /// then re-reads readiness.
    fn sync_bios(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(RowAction::SyncBios);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (report, readiness) = cx
                .background_executor()
                .spawn(async { (daemon::sync_bios(), daemon::read_readiness().ok()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                match report {
                    Ok(report) => {
                        let text = describe_sync(&report);
                        if report.failed.is_empty() {
                            this.say(text);
                        } else if report.fetched.is_empty() {
                            this.fail(text);
                        } else {
                            this.warn(text);
                        }
                        this.apply_readiness(readiness);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the BIOS sync failed");
                        this.fail(format!("BIOS not synced: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Drops the rails the last scan published, so the dashboard falls back to
    /// what Hearthdeck derives itself.
    fn restore_rails(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(RowAction::RestoreRails);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { daemon::restore_rails() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(()) => {
                        this.say("The dashboard rails were removed; the dashboard derives its own.")
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the dashboard rails could not be removed");
                        this.fail(format!("Rails not removed: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Applies a readiness read after an install or a BIOS sync: the counts *and*
    /// the list of what is still missing, or the card would claim a fix that the
    /// names beside it contradict.
    fn apply_readiness(&mut self, readiness: Option<daemon::Readiness>) {
        if let Some(readiness) = readiness {
            self.snapshot.ready_systems = readiness.ready;
            self.snapshot.total_systems = readiness.total;
            self.snapshot.missing_systems = readiness.missing;
        }
    }

    /// Enter, or a click, on whatever the cursor is on.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selection() {
            Some(Control::Text(field)) => {
                let input = self.inputs.text(field).clone();
                input.update(cx, |state, cx| state.focus(window, cx));
            }
            Some(Control::Toggle(field)) => {
                if self.toggle_enabled(field) {
                    let value = !self.toggle_value(field);
                    self.set_toggle(field, value, cx);
                } else {
                    // The row is still a cursor row, so pressing Enter has to say
                    // something rather than feel broken.
                    self.warn(
                        self.toggle_blocked_reason(field)
                            .unwrap_or("That switch cannot move right now."),
                    );
                    cx.notify();
                }
            }
            Some(Control::Time) => {
                let schedule = self.inputs.backup_schedule.clone();
                schedule.update(cx, |state, cx| state.focus(window, cx));
            }
            Some(Control::Action(action)) => self.run(action, cx),
            Some(Control::Fact(_)) | None => {}
        }
    }

    /// Escape: a text field is dismissed first, and only then the window - the
    /// order every writable field in Hearthdeck follows.
    /// Which kind of control owns the caret, if any. The footer's wording depends
    /// on it: a single-line input binds up/down to nothing, while the schedule uses
    /// left/right to pick a part.
    fn editing_kind(&self, window: &Window, cx: &App) -> Editing {
        if self.focused_field(window, cx).is_some() {
            return Editing::Text;
        }
        let Some(focused) = window.focused(cx) else {
            return Editing::None;
        };
        if self.inputs.backup_schedule.read(cx).focus_handle(cx) == focused {
            Editing::Schedule
        } else {
            Editing::None
        }
    }

    /// Whether an editable control owns the caret - a text field or the
    /// schedule. Escape blurs one of these before it closes the window.
    fn editing(&self, window: &Window, cx: &App) -> bool {
        self.editing_kind(window, cx) != Editing::None
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing(window, cx) {
            window.blur(cx);
            return;
        }
        // Back must not destroy work without saying so, and it must not trap the
        // user behind a dialog either: the first press asks, the second answers.
        if !self.close_armed && self.has_unsaved(cx) {
            self.close_armed = true;
            self.warn("Unsaved changes — Back again to close.");
            cx.notify();
            return;
        }
        cx.quit();
    }

    /// Whether closing now would lose something typed or flipped. Compared against
    /// what the daemon's read prefilled, so an untouched form never asks.
    fn has_unsaved(&self, cx: &App) -> bool {
        if self.backup_pending.is_some() {
            return true;
        }
        let prefill = Prefill::from_snapshot(&self.snapshot);
        let schedule = self.inputs.backup_schedule.read(cx).time();
        !self.inputs.romm_url.read(cx).value().is_empty()
            || self.inputs.romm_url.read(cx).value() != prefill.romm_url
            || self.inputs.backup_remote.read(cx).value() != prefill.backup_remote
            || self.inputs.backup_root.read(cx).value() != prefill.backup_root
            || prefill.schedule.is_some_and(|saved| saved != schedule)
    }

    /// Which text row owns the caret, if any.
    fn focused_field(&self, window: &Window, cx: &App) -> Option<TextField> {
        let focused = window.focused(cx)?;
        [
            TextField::RommUrl,
            TextField::BackupRemote,
            TextField::BackupRoot,
        ]
        .into_iter()
        .find(|field| self.inputs.text(*field).read(cx).focus_handle(cx) == focused)
    }

    /// Puts the clipboard into the focused field, and says so when there is
    /// nothing to put.
    ///
    /// The kit binds Ctrl+V itself; this is the fallback for the two other ways
    /// a Linux desktop pastes (`Ctrl+Shift+V`, `Shift+Insert`), and it exists
    /// because a clipboard that cannot be read is otherwise silent - the field
    /// simply stays empty and the user has no way to tell why.
    fn paste_into_focused_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.focused_field(window, cx) else {
            self.warn("Select a field with Enter before pasting.");
            cx.notify();
            return;
        };
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            self.warn("The clipboard is empty, or this session cannot read it.");
            cx.notify();
            return;
        };
        let pasted = text.chars().count();
        let input = self.inputs.text(field).clone();
        input.update(cx, |state, cx| state.set_value(text, window, cx));
        self.say(format!("Pasted {}.", plural(pasted, "character")));
        cx.notify();
    }

    fn sidebar(&self, cx: &Context<Self>, theme: &Theme) -> AnyElement {
        v_flex()
            .id("settings-pages")
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .gap_2()
            .p_6()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(
                div()
                    .pb_4()
                    .text_size(px(TYPE_CARD))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.sidebar_foreground)
                    .child("Settings"),
            )
            .children(PageKind::ALL.iter().enumerate().map(|(index, kind)| {
                let active = index == self.nav.page();
                h_flex()
                    .id(SharedString::from(format!("page-{index}")))
                    .w_full()
                    .items_center()
                    .min_h(px(56.))
                    .gap_3()
                    .px_4()
                    .rounded(theme.radius)
                    .when(active, |this| this.bg(theme.sidebar_accent))
                    .child(Icon::new(kind.icon()).text_color(if active {
                        theme.sidebar_accent_foreground
                    } else {
                        theme.sidebar_foreground
                    }))
                    .child(
                        div()
                            .text_size(px(TYPE_LABEL))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if active {
                                theme.sidebar_accent_foreground
                            } else {
                                theme.sidebar_foreground
                            })
                            .child(kind.title()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.nav.select_page(index);
                        cx.notify();
                    }))
            }))
            .child(div().flex_1())
            // What the window can say about itself: whether it is talking to the
            // daemon. It is not a description of the rows - those write.
            .child(
                div()
                    .text_size(px(TYPE_CAPTION))
                    .text_color(theme.sidebar_foreground)
                    .child(if self.snapshot.daemon {
                        "Connected to the daemon"
                    } else {
                        "Daemon unavailable"
                    }),
            )
            .into_any_element()
    }

    /// The page: a fixed heading, then the cards as one scrolling list. The
    /// heading never moves, so the page a user is on stays legible while the
    /// rows under it scroll.
    fn content(
        &self,
        cx: &Context<Self>,
        theme: &Theme,
        pages: &[Page],
        page_index: usize,
        items: Vec<Item>,
    ) -> AnyElement {
        let page = &pages[page_index];
        let view = cx.entity();
        let list_state = self.list.clone();
        let pages = pages.to_vec();
        let theme = theme.clone();
        v_flex()
            .id("settings-content")
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .child(
                v_flex().w_full().px_8().pt_8().pb_6().child(
                    v_flex()
                        .w_full()
                        .max_w(px(CONTENT_MAX_WIDTH))
                        .mx_auto()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(TYPE_TITLE))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(page.kind.title()),
                        )
                        .child(
                            div()
                                .text_size(px(TYPE_BODY))
                                .line_height(relative(1.4))
                                .text_color(theme.muted_foreground)
                                .child(page.kind.description()),
                        )
                        .child(div().pt_2().child(Separator::horizontal().h(px(1.))))
                        .when_some(self.page_alert(page.kind), |this, alert| {
                            this.child(
                                div()
                                    .debug_selector(|| "page-alert".to_string())
                                    .pt_4()
                                    .child(alert),
                            )
                        }),
                ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .w_full()
                    .debug_selector(|| "settings-list".to_string())
                    .child(
                        // `size_full` is what gives the list a height: on its own
                        // it sizes to nothing, and a list with no height has no
                        // visible items at all.
                        list(list_state.clone(), move |ix, _, cx| {
                            // The list resets in the same frame the items are
                            // built, so the indexes agree; a miss is an empty
                            // line rather than a panic on a television.
                            let Some(item) = items.get(ix).copied() else {
                                return div().into_any_element();
                            };
                            let this = view.read(cx);
                            div()
                                .debug_selector(move || format!("list-item-{ix}"))
                                .w_full()
                                .child(this.item_element(
                                    item,
                                    &pages,
                                    page_index,
                                    view.clone(),
                                    &theme,
                                ))
                                .into_any_element()
                        })
                        .size_full(),
                    )
                    .vertical_scrollbar(&list_state)
                    // A page change paints the new list over the old one in the
                    // same frame, so this fades the new page *in* rather than
                    // cross-fading two layers - including on the first paint.
                    // Only the list fades: the heading and the height never move.
                    // The id carries the page index, so a page change restarts it
                    // and a row-count change does not.
                    .with_animation(
                        SharedString::from(format!("page-fade-{page_index}")),
                        Animation::new(PAGE_FADE),
                        |this, delta| this.opacity(delta),
                    ),
            )
            .into_any_element()
    }

    /// One item of the scrolling page. Every item draws the same card surface,
    /// so a card split across items still reads as one card: the heading rounds
    /// the top, the last row rounds the bottom, and a rule separates the rows.
    fn item_element(
        &self,
        item: Item,
        pages: &[Page],
        page_index: usize,
        view: Entity<Self>,
        theme: &Theme,
    ) -> AnyElement {
        let card = |top: bool, bottom: bool| {
            let mut card = v_flex()
                .w_full()
                .max_w(px(CONTENT_MAX_WIDTH))
                .mx_auto()
                .bg(theme.tokens.group_box);
            if top {
                card = card.rounded_t(theme.radius);
            }
            if bottom {
                card = card.rounded_b(theme.radius);
            }
            card
        };
        match item {
            Item::Gap(height) => div().h(px(f32::from(height))).into_any_element(),
            Item::Header { group } => {
                let group = &pages[page_index].groups[group];
                div()
                    .w_full()
                    .px_8()
                    .child(
                        card(true, false)
                            .px_4()
                            .pt_3()
                            .pb_2()
                            .child(group_title(group, theme)),
                    )
                    .into_any_element()
            }
            Item::Row {
                group,
                row,
                flat,
                last,
            } => {
                let page = &pages[page_index];
                let row_data = &page.groups[group].rows[row];
                let focused = page_index == self.nav.page()
                    && page.cursor_rows().get(self.nav.row()).copied() == Some(flat);
                let cursor = page.cursor_index_of(flat);
                let mut surface = card(false, last).p_2();
                // The separator belongs between rows, not under the cursor: a
                // hairline inside the focus ring reads as a double border.
                if row > 0 && !focused {
                    surface = surface.border_t_1().border_color(theme.border);
                }
                div()
                    .w_full()
                    .px_8()
                    .child(surface.child(self.row_element(
                        view,
                        row_data,
                        RowFocus {
                            page: page_index,
                            flat,
                            cursor,
                            focused,
                        },
                        theme,
                    )))
                    .into_any_element()
            }
        }
    }

    fn row_element(
        &self,
        view: Entity<Self>,
        row: &Row,
        focus: RowFocus,
        theme: &Theme,
    ) -> AnyElement {
        let RowFocus {
            page: page_index,
            flat: row_index,
            cursor,
            focused,
        } = focus;
        let control = row.control.clone();
        // The row's click only carries rows whose control has no click of its
        // own: a switch and a button already do something when pressed, and
        // clicking them must not also run the row and undo it. A Time row does
        // open its field: the schedule is one small control on a large row, so
        // pressing anywhere in the row is how a person starts editing it.
        let confirm_on_click = matches!(control, Control::Text(_) | Control::Time);
        let view_for_click = view.clone();
        let on_click = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            view_for_click.update(cx, |this, cx| {
                // A fact has no cursor position; clicking it must not move the
                // cursor onto a row that Enter cannot act on.
                let Some(cursor) = cursor else {
                    return;
                };
                this.nav.select(page_index, cursor);
                if confirm_on_click {
                    this.confirm(window, cx);
                }
            });
        };
        h_flex()
            .id(SharedString::from(format!("row-{page_index}-{row_index}")))
            .w_full()
            .items_center()
            .justify_between()
            .gap_6()
            .px_4()
            .py_3()
            .rounded(theme.radius)
            // Three cues, not one hue: a surface, a real border and a bar, so
            // the focused row survives a greyscale screenshot at three metres.
            .border_2()
            .border_color(if focused {
                theme.list_active_border
            } else {
                theme.transparent
            })
            .when(focused, |this| this.bg(theme.list_active))
            .child(div().w(px(4.)).h(px(40.)).rounded_full().bg(if focused {
                theme.accent
            } else {
                theme.transparent
            }))
            .child(
                v_flex()
                    .flex_1()
                    // Without this a flex item refuses to shrink below its
                    // text, and a long description pushes the control off the
                    // right edge of the window.
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(TYPE_LABEL))
                            .font_weight(if focused {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::MEDIUM
                            })
                            .child(row.label),
                    )
                    .when_some(row.description, |this, description| {
                        this.child(
                            div()
                                .text_size(px(TYPE_BODY))
                                .line_height(relative(1.4))
                                .text_color(theme.muted_foreground)
                                .child(description),
                        )
                    }),
            )
            .child(
                div()
                    .debug_selector(move || format!("control-{page_index}-{row_index}"))
                    .child(self.control_element(&view, &control, page_index, row_index, theme)),
            )
            // A pointer sets the cursor where it clicked, and does what Enter
            // would have done: nothing on this page is pointer-only.
            .on_click(on_click)
            .into_any_element()
    }

    fn control_element(
        &self,
        view: &Entity<Self>,
        control: &Control,
        page_index: usize,
        row_index: usize,
        theme: &Theme,
    ) -> AnyElement {
        match control {
            Control::Fact(value) => self.value_element(value, theme),
            // Every control is `Large`: the kit's medium step is sized for a
            // pointer on a laptop, which is what made these look miniature
            // beside row text sized for the sofa.
            Control::Text(field) => div()
                .w(px(FIELD_WIDTH))
                .child(Input::new(self.inputs.text(*field)).large())
                .into_any_element(),
            Control::Toggle(field) => {
                let field = *field;
                let view = view.clone();
                let enabled = self.toggle_enabled(field);
                // "Off because you turned it off" and "off because it cannot work"
                // must not be the same pixels - and a write in flight is neither.
                let reason = self.toggle_blocked_reason(field);
                h_flex()
                    .items_center()
                    .gap_2()
                    .when_some(reason, |this, reason| {
                        this.child(
                            Marker::new()
                                .w_auto()
                                .flex_none()
                                .text_size(px(TYPE_BODY))
                                .text_color(theme.warning)
                                .icon(MarkerIcon::new().child(Icon::new(IconName::TriangleAlert)))
                                .content(MarkerContent::new().text(reason)),
                        )
                    })
                    .child(
                        Switch::new(SharedString::from(format!(
                            "switch-{page_index}-{row_index}"
                        )))
                        .checked(self.toggle_value(field))
                        .disabled(!enabled)
                        .on_click(
                            move |checked: &bool, _: &mut Window, cx: &mut App| {
                                let value = *checked;
                                view.update(cx, |this, cx| this.set_toggle(field, value, cx));
                            },
                        ),
                    )
                    .into_any_element()
            }
            Control::Time => div()
                .w(px(TIME_FIELD_WIDTH))
                .child(TimeField::new(&self.inputs.backup_schedule).large())
                .into_any_element(),
            Control::Action(action) => {
                let action = *action;
                let view = view.clone();
                let loading = self.action_loading(action);
                Button::new(SharedString::from(format!(
                    "action-{page_index}-{row_index}"
                )))
                .label(action.label())
                .large()
                .min_w(px(BUTTON_MIN_WIDTH))
                .when(action.primary(), |this| this.primary())
                .when(action.destructive(), |this| this.danger())
                // A press answers at once: the button says it is working while
                // the daemon does, instead of a line appearing under the page.
                .when(loading, |this| {
                    this.icon(IconName::LoaderCircle).loading(true)
                })
                .disabled(!self.action_enabled(action))
                .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    view.update(cx, |this, cx| this.run(action, cx));
                })
                .into_any_element()
            }
        }
    }

    fn value_element(&self, value: &Value, theme: &Theme) -> AnyElement {
        match value {
            Value::Text(text) => div()
                .text_size(px(TYPE_VALUE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.foreground)
                .max_w(px(560.))
                .whitespace_nowrap()
                .truncate()
                .child(text.clone())
                .into_any_element(),
            Value::Code(code) => div()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(TYPE_CODE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.foreground)
                .child(code.clone())
                .into_any_element(),
            Value::Storage {
                used_percent,
                available,
                total,
            } => h_flex()
                .items_center()
                .gap_3()
                .child(
                    ProgressCircle::new("storage")
                        .value(*used_percent)
                        // Thresholds on the number the label shows, or 79.6%
                        // reads "80%" beside a green ring.
                        .color(if used_percent.round() >= 90.0 {
                            theme.danger
                        } else if used_percent.round() >= 80.0 {
                            theme.warning
                        } else {
                            theme.success
                        })
                        .w(px(RING_SIZE))
                        .h(px(RING_SIZE))
                        .child(
                            div()
                                .text_size(px(TYPE_CAPTION))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("{used_percent:.0}%")),
                        ),
                )
                .child(
                    div()
                        .text_size(px(TYPE_VALUE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .max_w(px(560.))
                        .whitespace_nowrap()
                        .truncate()
                        .child(format!("{available} free of {total}")),
                )
                .into_any_element(),
        }
    }

    fn footer(&self, theme: &Theme, editing: Editing) -> AnyElement {
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_6()
            .px_8()
            .py_4()
            .border_t_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .items_center()
                    .gap_5()
                    .text_size(px(TYPE_CAPTION))
                    .text_color(theme.muted_foreground)
                    .when(editing == Editing::Text, |this| {
                        this.child(hint("← →", "Caret", theme))
                            .child(hint("Esc", "Done", theme))
                    })
                    .when(editing == Editing::Schedule, |this| {
                        this.child(hint("← →", "Part", theme))
                            .child(hint("↑ ↓", "Value", theme))
                            .child(hint("Esc", "Done", theme))
                    })
                    .when(editing == Editing::None, |this| {
                        this.child(hint("↑↓", "Move", theme))
                            .child(hint("←→", "Page", theme))
                            .child(hint("A / Enter", "Select or edit", theme))
                            .child(hint("B / Esc", "Back", theme))
                    }),
            )
            .into_any_element()
    }
}

/// How many idle reads a started run gets before the window stops waiting for it
/// to appear: `--no-block` returns before the job is `activating`.
const IDLE_GRACE_READS: u32 = 20;
/// Consecutive unreadable polls before the window stops guessing.
const FAILURE_LIMIT: u32 = 3;
/// Fast polls (three seconds) for the first five minutes, then thirty seconds,
/// which covers the service's own six-hour start timeout without becoming a
/// machine-gun of `systemctl` calls.
const WATCH_FAST_POLLS: u32 = 100;
/// 100 fast polls (5 minutes) plus 710 half-minute ones: six hours, which is the
/// service's own start timeout.
const WATCH_MAX_POLLS: u32 = 810;

/// What one status read means for the watcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WatchStep {
    /// Keep following: the run is working, or has not shown up yet.
    Continue,
    Stop(WatchEnd),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WatchEnd {
    /// The run was seen working and is no longer.
    Finished,
    /// The run never appeared within the grace window.
    NeverStarted,
    /// The daemon could not be read for a few polls in a row.
    Unreadable,
}

#[derive(Default)]
struct WatchState {
    seen_running: bool,
    idle_reads: u32,
    failures: u32,
}

/// The watcher's whole decision, kept pure so it can be tested without a timer.
fn watch_step(
    state: &mut WatchState,
    read: &Result<daemon::BackupStatus, daemon::Error>,
) -> WatchStep {
    match read {
        Ok(status) if status.service_active => {
            state.seen_running = true;
            state.idle_reads = 0;
            state.failures = 0;
            WatchStep::Continue
        }
        // Only a run that was seen working can be said to have finished.
        Ok(_) if state.seen_running => {
            state.failures = 0;
            WatchStep::Stop(WatchEnd::Finished)
        }
        Ok(_) => {
            // A read that worked breaks the run of failures, so three scattered
            // hiccups over six hours do not add up to giving up.
            state.failures = 0;
            state.idle_reads += 1;
            if state.idle_reads >= IDLE_GRACE_READS {
                WatchStep::Stop(WatchEnd::NeverStarted)
            } else {
                WatchStep::Continue
            }
        }
        Err(_) => {
            state.failures += 1;
            if state.failures >= FAILURE_LIMIT {
                WatchStep::Stop(WatchEnd::Unreadable)
            } else {
                WatchStep::Continue
            }
        }
    }
}

fn watch_interval(polls: u32) -> std::time::Duration {
    if polls < WATCH_FAST_POLLS {
        std::time::Duration::from_secs(3)
    } else {
        std::time::Duration::from_secs(30)
    }
}

/// What an install report says in one line, naming the first package that
/// failed so the line is something to act on.
fn describe_install(report: &daemon::CoreInstall) -> String {
    let installed = report.installed.len();
    let failures = report.failed.len();
    let first = report
        .failed
        .first()
        .map(|failure| format!("\n{}: {}", failure.package, failure.reason))
        .unwrap_or_default();
    match (installed, failures) {
        (0, 0) => "Every core was already installed.".to_string(),
        (installed, 0) => format!("Installed {}.", plural(installed, "core")),
        (0, failures) => format!("No core could be installed ({failures} failed){first}"),
        (installed, failures) => {
            format!(
                "Installed {}; {failures} failed{first}",
                plural(installed, "core")
            )
        }
    }
}

/// What a BIOS sync report says in one line, naming the first file that failed.
fn describe_sync(report: &daemon::BiosSync) -> String {
    let fetched = report.fetched.len();
    let failures = report.failed.len();
    let first = report
        .failed
        .first()
        .map(|failure| format!("\n{}: {}", failure.filename, failure.reason))
        .unwrap_or_default();
    match (fetched, failures) {
        (0, 0) => "Nothing was missing.".to_string(),
        (fetched, 0) => format!("Placed {}.", plural(fetched, "file")),
        (0, failures) => format!("Nothing could be placed ({failures} failed){first}"),
        (fetched, failures) => format!(
            "Placed {}; {failures} failed{first}",
            plural(fetched, "file")
        ),
    }
}

/// A card's heading: the section name on the left, its state as a [`Marker`] at
/// the trailing edge. The marker is a label for the card, and the right edge is
/// where a label is read without hunting beside the title's varying width.
fn group_title(group: &Group, theme: &Theme) -> AnyElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(TYPE_CARD))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.foreground)
                .child(group.title),
        )
        .when_some(group.status, |this, (label, tone)| {
            this.child(
                div()
                    .debug_selector(move || format!("card-status-{label}"))
                    .child(tone_marker(group.title, label, tone, theme)),
            )
        })
        .into_any_element()
}

/// What the Stremio account row says: the daemon's account, a pending approval,
/// or nothing linked yet. A linked account outranks a stale offer.
fn stremio_account(linked_at: Option<&str>, waiting: bool) -> String {
    match (linked_at, waiting) {
        (Some(linked_at), _) => format!("Linked since {}", local_time(linked_at)),
        (None, true) => "Waiting for approval".to_string(),
        (None, false) => "Not linked".to_string(),
    }
}

/// What a switch that mirrors the daemon shows: the write in flight, else the
/// daemon's answer, else off. So a press answers at once, and a refused write
/// leaves the switch where the daemon actually is.
fn mirrored(pending: Option<bool>, daemon: Option<bool>) -> bool {
    pending.or(daemon).unwrap_or(false)
}

/// One control hint: the input in a small cap, then what it does. The label names
/// both the pad button and the key, because the window cannot tell which one the
/// user actually has.
fn hint(key: &'static str, meaning: &'static str, theme: &Theme) -> AnyElement {
    h_flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .px_2()
                .py_1()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .text_size(px(TYPE_CAPTION))
                .child(key),
        )
        .child(meaning)
        .into_any_element()
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The first read lands after the window exists, so the form it prefills
        // is filled here, where a window is available.
        self.apply_prefill(window, cx);
        self.deliver_notices(window, cx);
        let theme = cx.theme();
        let pages = self.pages();
        let counts = pages
            .iter()
            .map(|page| page.cursor_rows().len())
            .collect::<Vec<_>>();
        // The content is the source of truth for how many rows exist, so the
        // cursor is pulled back inside it before anything is drawn.
        self.nav.resize(&counts);
        let page_index = self.nav.page().min(pages.len().saturating_sub(1));
        let (items, row_items) = flat_items(&pages[page_index]);

        // A different page, or a different number of rows, is a different list:
        // an offset from the old one means nothing.
        if self.list_page != page_index || self.list.item_count() != items.len() {
            self.list.reset(items.len());
            self.list_page = page_index;
            // A page opens at its top. Scrolling to the cursor here would push the
            // first cards off screen on a page whose only action is at the bottom;
            // the reveal below follows a cursor that actually moves.
            self.scrolled_to = Some((page_index, self.nav.row()));
        }

        // Keep the cursor's row on screen. This is what makes a page taller than
        // the display navigable with no pointer at all.
        let selected = pages[page_index].cursor_rows().get(self.nav.row()).copied();
        if let Some(item) = selected.and_then(|flat| row_items.get(flat))
            && self.scrolled_to != Some((page_index, self.nav.row()))
        {
            self.list.scroll_to_reveal_item(*item);
            self.scrolled_to = Some((page_index, self.nav.row()));
        }

        h_flex()
            .size_full()
            .bg(theme.background)
            .child(self.sidebar(cx, theme))
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .child(self.content(cx, theme, &pages, page_index, items))
                    .child(self.footer(theme, self.editing_kind(window, cx))),
            )
    }
}

/// Every key this window answers to. `main` and the render tests both call it,
/// so the path under test is the path that ships.
///
/// The gpui action a forwarded pad press means. It is the *same* action the
/// keyboard binds to, so the pad and the keys reach one handler, not two.
fn pad_action(remote: input::Remote) -> Box<dyn gpui::Action> {
    use input::Remote;
    match remote {
        Remote::Up => MoveUp.boxed_clone(),
        Remote::Down => MoveDown.boxed_clone(),
        Remote::Left => PreviousPage.boxed_clone(),
        Remote::Right => NextPage.boxed_clone(),
        Remote::Confirm => Confirm.boxed_clone(),
        Remote::Back => CloseWindow.boxed_clone(),
    }
}

/// The keystroke a pad press becomes while a field owns the caret. The fields
/// already bind these keys, so the pad only has to speak them.
fn pad_key(remote: input::Remote) -> Option<&'static str> {
    use input::Remote;
    Some(match remote {
        Remote::Up => "up",
        Remote::Down => "down",
        Remote::Left => "left",
        Remote::Right => "right",
        Remote::Confirm => "enter",
        Remote::Back => return None,
    })
}

/// Reads what the library app forwards and dispatches it as the action that key
/// would have produced.
///
/// While a text field owns the caret the pad gets only Back - the same order the
/// keyboard's Escape follows: dismiss the field, then leave.
fn pump_pad(
    cx: &mut App,
    view: Entity<SettingsWindow>,
    window: AnyWindowHandle,
    received: std::sync::mpsc::Receiver<input::Remote>,
) {
    let received = std::sync::Arc::new(std::sync::Mutex::new(received));
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        loop {
            // A short wait keeps this off the main thread without holding one of
            // the executor's workers for the window's whole life.
            let waiting = std::sync::Arc::clone(&received);
            let remote = cx
                .background_executor()
                .spawn(async move {
                    let Ok(receiver) = waiting.lock() else {
                        return None;
                    };
                    receiver
                        .recv_timeout(std::time::Duration::from_millis(100))
                        .ok()
                })
                .await;
            let Some(remote) = remote else {
                continue;
            };
            let _ = window.update(cx, |_, window, cx| {
                let editing = {
                    let this = view.read(cx);
                    this.editing(window, cx)
                };
                if editing && remote != input::Remote::Back {
                    // A field owns the caret, so the pad speaks to it in the only
                    // language it has: the keys the field already binds. That is
                    // what steps the schedule and moves a caret.
                    if let Some(key) = pad_key(remote)
                        && let Ok(keystroke) = gpui::Keystroke::parse(key)
                    {
                        window.dispatch_keystroke(keystroke, cx);
                    }
                    return;
                }
                window.dispatch_action(pad_action(remote), cx);
            });
        }
    })
    .detach();
}

/// The window handle is captured here rather than asked for with
/// `App::active_window()`: that asks the platform which of *its* windows is
/// active, and a fullscreen kiosk window is not always reported as one. The
/// handle this window was created with always is this window.
fn install_shortcuts(cx: &mut App, view: Entity<SettingsWindow>, window: AnyWindowHandle) {
    cx.bind_keys([
        KeyBinding::new("escape", CloseWindow, None),
        KeyBinding::new("up", MoveUp, None),
        KeyBinding::new("down", MoveDown, None),
        KeyBinding::new("left", PreviousPage, None),
        KeyBinding::new("right", NextPage, None),
        KeyBinding::new("enter", Confirm, None),
        // The kit's input binds Ctrl+V and Cmd+V in its own context. These are
        // the other two ways a Linux desktop pastes, and they land here so a
        // clipboard that cannot be read says so instead of doing nothing.
        KeyBinding::new("ctrl-shift-v", PasteIntoField, None),
        KeyBinding::new("shift-insert", PasteIntoField, None),
    ]);

    // Cursor movement is plain state, so it needs no window; confirming and
    // closing do, because focusing a field is a window operation.
    let movements = view.clone();
    cx.on_action(move |_: &MoveUp, cx: &mut App| {
        movements.update(cx, |this, cx| {
            this.nav.up();
            cx.notify();
        });
    });
    let movements = view.clone();
    cx.on_action(move |_: &MoveDown, cx: &mut App| {
        movements.update(cx, |this, cx| {
            this.nav.down();
            cx.notify();
        });
    });
    let pages = view.clone();
    cx.on_action(move |_: &PreviousPage, cx: &mut App| {
        pages.update(cx, |this, cx| {
            this.nav.previous_page();
            cx.notify();
        });
    });
    let pages = view.clone();
    cx.on_action(move |_: &NextPage, cx: &mut App| {
        pages.update(cx, |this, cx| {
            this.nav.next_page();
            cx.notify();
        });
    });
    let rows = view.clone();
    cx.on_action(move |_: &Confirm, cx: &mut App| {
        // Deferred by one effect flush: an action is dispatched while the
        // window is already leased, and asking for it again inside the listener
        // fails - quietly, which is how Enter did nothing at all.
        let rows = rows.clone();
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                rows.update(cx, |this, cx| this.confirm(window, cx));
            });
        });
    });
    let paste_view = view.clone();
    cx.on_action(move |_: &PasteIntoField, cx: &mut App| {
        let view = paste_view.clone();
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                view.update(cx, |this, cx| this.paste_into_focused_field(window, cx));
            });
        });
    });
    cx.on_action(move |_: &CloseWindow, cx: &mut App| {
        let view = view.clone();
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                view.update(cx, |this, cx| this.close(window, cx));
            });
        });
    });
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // The platform crates are in the filter on purpose: the pieces
                // that can fail silently - a clipboard read, a seat capability -
                // log from gpui, and a window that explains nothing is worse
                // than a noisy journal.
                tracing_subscriber::EnvFilter::new(
                    "hearthdeck_settings=info,gpui=warn,gpui_linux=warn",
                )
            }),
        )
        .init();

    // Open first, read after. Eight sequential daemon reads with a five-second
    // timeout each must never stand between the user's press and a window: the
    // page draws from an empty snapshot with "Reading..." and fills itself in.
    let snapshot = Snapshot::default();
    let disk = home_usage();

    gpui_platform::application().run(move |cx: &mut App| {
        gpui_component::init(cx);
        // The session this is drawn in is dark; the kit defaults to light.
        theme::install(cx);

        let options = WindowOptions {
            // The window is the settings surface, not a dialog over the library:
            // fullscreen is the compositor's call, and the bounds below are only
            // what it restores to.
            window_bounds: Some(WindowBounds::Fullscreen(Bounds::new(
                point(px(0.0), px(0.0)),
                size(px(RESTORE_WIDTH), px(RESTORE_HEIGHT)),
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Hearthdeck - Settings".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        // What `gpui_kit::open_window` did: the application content under a
        // `Root`, which is what gives components their overlay and focus layers.
        let opened = cx.open_window(options, move |window, cx| {
            let inputs = Inputs {
                romm_url: cx
                    .new(|cx| InputState::new(window, cx).placeholder("http://romm.local:8080")),
                backup_remote: cx.new(|cx| InputState::new(window, cx).placeholder("pcloud")),
                backup_root: cx
                    .new(|cx| InputState::new(window, cx).placeholder("hearthdeck-backups")),
                backup_schedule: cx
                    .new(|cx| TimeFieldState::new(window, cx).precision(TimePrecision::Minute)),
            };
            let view = cx.new(|_| SettingsWindow::new(snapshot, disk, inputs));
            let window_handle = window.window_handle();
            install_shortcuts(cx, view.clone(), window_handle);
            // The pad, when the library app forwarded one. A missing socket is
            // not an error: the window is fully usable from a keyboard.
            if let Some(received) = input::listen() {
                pump_pad(cx, view.clone(), window_handle, received);
            }
            // The window is on screen before this runs, so a slow or dead daemon
            // shows "Reading..." instead of a window that never appears.
            view.update(cx, |this, cx| this.refresh(cx));

            cx.new(|cx| gpui_base::Root::new(view, window, cx))
        });
        if let Err(error) = opened {
            tracing::error!(%error, "failed to open the settings window");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        Control, FAILURE_LIMIT, Group, IDLE_GRACE_READS, Item, Page, PageKind, Row, TextField,
        Value, WatchEnd, WatchState, WatchStep, daemon, describe_install, describe_sync,
        flat_items, mirrored, plural, stremio_account, watch_step,
    };

    #[test]
    fn an_install_report_names_the_first_package_that_failed() {
        use super::daemon::{CoreFailure, CoreInstall};

        let mut report = CoreInstall::default();
        assert_eq!(
            describe_install(&report),
            "Every core was already installed."
        );

        report.installed.push("libretro-snes9x".to_string());
        assert_eq!(describe_install(&report), "Installed 1 core.");

        report.failed.push(CoreFailure {
            package: "libretro-dolphin".to_string(),
            reason: "no such package".to_string(),
        });
        let line = describe_install(&report);
        assert!(line.starts_with("Installed 1 core; 1 failed"), "{line}");
        assert!(line.contains("libretro-dolphin: no such package"), "{line}");
    }

    #[test]
    fn a_sync_report_names_the_first_file_that_failed() {
        use super::daemon::{BiosFailure, BiosSync};

        let mut report = BiosSync::default();
        assert_eq!(describe_sync(&report), "Nothing was missing.");

        report.fetched.push("bios/SCPH1001.BIN".to_string());
        assert_eq!(describe_sync(&report), "Placed 1 file.");

        report.failed.push(BiosFailure {
            filename: "bios/scph5500.bin".to_string(),
            reason: "not found".to_string(),
        });
        let line = describe_sync(&report);
        assert!(line.starts_with("Placed 1 file; 1 failed"), "{line}");
        assert!(line.contains("bios/scph5500.bin: not found"), "{line}");
    }

    /// A page with one button per row is a list of facts. The action that moves
    /// the page forward is the primary one, and the destructive action is not
    /// drawn like it.
    #[test]
    fn the_forward_actions_are_primary_and_removal_is_its_own_thing() {
        use super::RowAction;

        assert!(RowAction::SaveBackup.primary());
        assert!(RowAction::RunBackup.primary());
        assert!(RowAction::RommLink.primary());
        assert!(!RowAction::Refresh.primary());
        assert!(!RowAction::RestoreRails.primary());

        assert!(RowAction::RommForget.destructive());
        assert!(RowAction::StremioUnlink.destructive());
        assert!(!RowAction::RommLink.destructive());
        assert!(
            !RowAction::RommForget.primary(),
            "destructive is its own treatment"
        );
    }

    fn plain_row(label: &'static str) -> Row {
        Row {
            label,
            description: None,
            control: Control::Fact(Value::Text(String::new())),
        }
    }

    fn page_of(groups: &[&[&'static str]]) -> Page {
        Page {
            kind: PageKind::Connections,
            groups: groups
                .iter()
                .map(|labels| Group {
                    title: "Service",
                    status: None,
                    rows: labels.iter().map(|label| plain_row(label)).collect(),
                })
                .collect(),
        }
    }

    /// The card is painted from `tokens.group_box`; editing only `colors` leaves
    /// that stale, which is how a surface change becomes a no-op. This test would
    /// have caught it.
    #[gpui::test]
    fn the_palette_reaches_the_tokens_the_card_is_painted_from(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        cx.update(crate::theme::install);

        cx.update(|cx| {
            let theme = gpui_component::Theme::global(cx);
            assert_eq!(
                theme.tokens.group_box.color, theme.colors.group_box,
                "the token the card paints from follows the colour"
            );
            assert_ne!(
                theme.tokens.group_box.color, theme.tokens.background.color,
                "a card must not be the page colour"
            );
        });
    }

    #[test]
    fn the_watcher_waits_for_a_run_that_has_not_started_yet() {
        let mut state = WatchState::default();
        assert_eq!(
            watch_step(&mut state, &Ok(daemon::BackupStatus::default())),
            WatchStep::Continue,
            "`--no-block` returns before the job is activating"
        );
    }

    #[test]
    fn the_watcher_stops_when_a_seen_run_finishes() {
        let mut state = WatchState::default();
        let running = daemon::BackupStatus {
            service_active: true,
            ..Default::default()
        };
        assert_eq!(watch_step(&mut state, &Ok(running)), WatchStep::Continue);
        assert_eq!(
            watch_step(&mut state, &Ok(daemon::BackupStatus::default())),
            WatchStep::Stop(WatchEnd::Finished)
        );
    }

    #[test]
    fn a_run_that_never_appears_is_given_up_on() {
        let mut state = WatchState::default();
        // The last read inside the grace window is the one that gives up, so the
        // loop stops one short of it.
        for _ in 0..IDLE_GRACE_READS - 1 {
            assert_eq!(
                watch_step(&mut state, &Ok(daemon::BackupStatus::default())),
                WatchStep::Continue
            );
        }
        assert_eq!(
            watch_step(&mut state, &Ok(daemon::BackupStatus::default())),
            WatchStep::Stop(WatchEnd::NeverStarted)
        );
    }

    #[test]
    fn unreadable_polls_stop_the_watcher_instead_of_claiming_success() {
        let mut state = WatchState::default();
        let failed = || Err::<daemon::BackupStatus, daemon::Error>("no daemon".into());
        for _ in 0..(FAILURE_LIMIT - 1) {
            assert_eq!(watch_step(&mut state, &failed()), WatchStep::Continue);
        }
        assert_eq!(
            watch_step(&mut state, &failed()),
            WatchStep::Stop(WatchEnd::Unreadable)
        );
    }

    #[test]
    fn every_pad_action_is_the_action_the_keyboard_binds() {
        use super::input::Remote;
        use super::{CloseWindow, Confirm, MoveDown, MoveUp, NextPage, PreviousPage, pad_action};
        use std::any::TypeId;

        // The pad does not get its own handlers: each word becomes the very
        // action `install_shortcuts` binds a key to, so both reach one place.
        for (remote, expected) in [
            (Remote::Up, TypeId::of::<MoveUp>()),
            (Remote::Down, TypeId::of::<MoveDown>()),
            (Remote::Left, TypeId::of::<PreviousPage>()),
            (Remote::Right, TypeId::of::<NextPage>()),
            (Remote::Confirm, TypeId::of::<Confirm>()),
            (Remote::Back, TypeId::of::<CloseWindow>()),
        ] {
            assert_eq!(
                pad_action(remote).as_any().type_id(),
                expected,
                "{remote:?} maps to its own action"
            );
        }
    }

    #[test]
    fn every_row_has_an_item_to_scroll_to() {
        let page = page_of(&[&["a", "b"], &["c"]]);
        let (items, rows) = flat_items(&page);

        assert_eq!(
            items.first(),
            Some(&Item::Gap(32)),
            "the first card starts clear of the heading, and further than the rest"
        );
        assert_eq!(
            items.last(),
            Some(&Item::Gap(24)),
            "the page ends clear of the footer"
        );
        assert!(
            matches!(items[1], Item::Header { group: 0 }),
            "a card opens with its heading: {:?}",
            items[1]
        );
        assert_eq!(rows.len(), 3, "one scroll target per row, not per card");
        for (flat, item) in rows.iter().enumerate() {
            match items[*item] {
                Item::Row { flat: found, .. } => assert_eq!(found, flat),
                other => panic!("row {flat} points at {other:?}"),
            }
        }
    }

    #[test]
    fn the_last_row_of_each_card_closes_it() {
        let page = page_of(&[&["a", "b"], &["c"]]);
        let (items, rows) = flat_items(&page);
        let closes = |flat: usize| matches!(items[rows[flat]], Item::Row { last: true, .. });

        assert!(!closes(0), "a card's first row has square corners");
        assert!(closes(1), "the card's last row rounds the bottom");
        assert!(closes(2));
    }

    #[test]
    fn the_stremio_row_says_which_state_the_account_is_in() {
        // A real RFC 3339 stamp, so this exercises the local-time formatting rather
        // than falling through to the raw string.
        let linked = stremio_account(Some("2026-01-02T03:04:05Z"), false);
        assert!(linked.starts_with("Linked since "), "{linked}");
        assert!(
            !linked.contains('T') && linked.contains(','),
            "a stamp is rendered for a person, not quoted: {linked}"
        );
        assert_eq!(stremio_account(None, true), "Waiting for approval");
        assert_eq!(stremio_account(None, false), "Not linked");
        assert!(
            stremio_account(Some("2026-01-02T03:04:05Z"), true).starts_with("Linked since "),
            "a linked account outranks a stale offer"
        );
    }

    #[test]
    fn a_count_reads_like_a_sentence() {
        assert_eq!(plural(0, "core"), "0 cores");
        assert_eq!(plural(1, "core"), "1 core");
        assert_eq!(plural(2, "file"), "2 files");
    }

    #[test]
    fn a_daemon_timestamp_is_rendered_in_local_time() {
        let formatted = super::local_time("2026-11-08T02:00:00Z");
        assert!(
            formatted.contains(':') && !formatted.contains('T'),
            "a person reads a clock, not RFC 3339: {formatted}"
        );
        assert_eq!(
            super::local_time("not a timestamp"),
            "not a timestamp",
            "an unparseable value is shown as it came"
        );
    }

    #[test]
    fn a_switch_mirroring_the_daemon_shows_the_write_until_it_lands() {
        assert!(
            mirrored(Some(true), Some(false)),
            "the press answers before the daemon does"
        );
        assert!(
            !mirrored(None, Some(false)),
            "a refused write leaves the switch where the daemon is"
        );
        assert!(mirrored(None, Some(true)));
        assert!(
            !mirrored(None, None),
            "a switch whose state was never read is drawn off"
        );
    }

    #[test]
    fn every_text_row_addresses_its_own_field() {
        // Adding a text field without a row, or listing one twice, is a mistake
        // the page cannot catch at runtime - so the vocabulary is checked here.
        let fields = [
            TextField::RommUrl,
            TextField::BackupRemote,
            TextField::BackupRoot,
        ];
        for (index, field) in fields.iter().enumerate() {
            assert!(
                !fields[..index].contains(field),
                "a text field is listed twice"
            );
        }
    }
}

/// The page as a window draws it. These are the tests that catch "the screen is
/// empty": bounds are what a user sees, and a list with no height or an item
/// that never renders fails here instead of on the television.
#[cfg(test)]
mod render_tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    /// A daemon that answered: the pages draw their real rows. Tests that need an
    /// unreachable daemon build that snapshot themselves.
    fn open(cx: &mut TestAppContext) -> (Entity<SettingsWindow>, &mut VisualTestContext) {
        open_with(
            Snapshot {
                daemon: true,
                ..Snapshot::default()
            },
            cx,
        )
    }

    fn open_with(
        snapshot: Snapshot,
        cx: &mut TestAppContext,
    ) -> (Entity<SettingsWindow>, &mut VisualTestContext) {
        cx.update(gpui_component::init);
        cx.update(crate::theme::install);
        let mut opened = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let inputs = Inputs {
                romm_url: cx.new(|cx| InputState::new(window, cx)),
                backup_remote: cx.new(|cx| InputState::new(window, cx)),
                backup_root: cx.new(|cx| InputState::new(window, cx)),
                backup_schedule: cx
                    .new(|cx| TimeFieldState::new(window, cx).precision(TimePrecision::Minute)),
            };
            let view = cx.new(|_| SettingsWindow::new(snapshot, None, inputs));
            install_shortcuts(cx, view.clone(), window.window_handle());
            opened = Some(view.clone());
            gpui_base::Root::new(view, window, cx)
        });
        let view = opened.expect("a settings view");
        draw(cx);
        (view, cx)
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    /// The list item at `index`, by the selector its element carries.
    fn selector(index: usize) -> &'static str {
        Box::leak(format!("list-item-{index}").into_boxed_str())
    }

    /// The Emulation card names what is missing, so the install action is a
    /// decision rather than a blind press.
    #[gpui::test]
    fn the_emulation_page_names_what_is_missing(cx: &mut TestAppContext) {
        let snapshot = Snapshot {
            daemon: true,
            missing_systems: vec![daemon::MissingSystem {
                name: "SNES".to_string(),
                reason: "core bsnes".to_string(),
            }],
            ..Snapshot::default()
        };
        let (view, cx) = open_with(snapshot, cx);

        let page = cx.read(|app| {
            view.read(app)
                .pages()
                .into_iter()
                .find(|page| page.kind == PageKind::Emulation)
                .expect("an emulation page")
        });
        let named = page.groups[0].rows.iter().any(|row| {
            matches!(
                &row.control,
                Control::Fact(Value::Text(text)) if text.contains("SNES — core bsnes")
            )
        });
        assert!(named, "the card says which system and what it needs");
    }

    /// A daemon that never answered must not be rendered as the user's
    /// configuration: no "Not set up", no switch drawn off, one card that says
    /// what happened and offers the retry.
    #[gpui::test]
    fn an_unreachable_daemon_is_one_card_not_a_false_configuration(cx: &mut TestAppContext) {
        let (view, cx) = open_with(Snapshot::default(), cx);
        cx.simulate_resize(size(px(1280.), px(720.)));
        draw(cx);

        let pages = cx.read(|app| view.read(app).pages());
        for page in &pages {
            assert_eq!(
                page.row_count(),
                1,
                "{} must not claim the user's setup",
                page.kind.title()
            );
            let row = &page.groups[0].rows[0];
            assert!(matches!(row.control, Control::Action(RowAction::Refresh)));
            assert!(
                row.description
                    .unwrap_or_default()
                    .contains("Could not reach"),
                "the card says what happened: {:?}",
                row.description
            );
        }
    }

    /// Turning the timer off is always allowed: a remote that vanished from
    /// rclone must not leave a running timer that cannot be stopped.
    #[gpui::test]
    fn a_remote_that_vanished_still_lets_the_switch_off(cx: &mut TestAppContext) {
        let (view, cx) = open_with(
            Snapshot {
                daemon: true,
                backup: Some(crate::daemon::BackupView {
                    enabled: true,
                    remote: "pcloud".to_string(),
                    root: "hearthdeck-backups".to_string(),
                    schedule: crate::daemon::BackupSchedule {
                        hour: 4,
                        minute: 30,
                    },
                    remote_available: false,
                    rclone_installed: true,
                    remotes: vec!["pcloud".to_string()],
                    updated_at: None,
                }),
                ..Snapshot::default()
            },
            cx,
        );
        cx.simulate_resize(size(px(1280.), px(720.)));
        draw(cx);

        assert!(
            cx.read(|app| view.read(app).toggle_enabled(ToggleField::Backup)),
            "an enabled timer can always be turned off"
        );
    }

    /// With the units installed but the remote absent from rclone, the page says
    /// so and the switch stays dead: enabling a timer that cannot run would only
    /// fail nightly.
    #[gpui::test]
    fn the_backup_page_explains_a_missing_remote(cx: &mut TestAppContext) {
        let (view, cx) = open_with(
            Snapshot {
                daemon: true,
                backup: Some(crate::daemon::BackupView {
                    enabled: false,
                    remote: "pcloud".to_string(),
                    root: "hearthdeck-backups".to_string(),
                    schedule: crate::daemon::BackupSchedule {
                        hour: 4,
                        minute: 30,
                    },
                    remote_available: false,
                    rclone_installed: true,
                    remotes: vec!["pcloud".to_string()],
                    updated_at: None,
                }),
                ..Snapshot::default()
            },
            cx,
        );
        cx.simulate_resize(size(px(1280.), px(720.)));
        draw(cx);
        // The backup page is the last one: one step left from Status wraps to it.
        cx.simulate_keystrokes("left");
        draw(cx);

        let (rows, switch) = cx.read(|app| {
            let this = view.read(app);
            let page = this
                .pages()
                .into_iter()
                .find(|page| page.kind == PageKind::Backup)
                .expect("a backup page");
            (page.row_count(), this.toggle_enabled(ToggleField::Backup))
        });
        assert!(rows >= 5, "the page lists its rows: {rows}");
        assert!(
            !switch,
            "the switch is dead while rclone does not know the remote"
        );
    }

    #[gpui::test]
    fn the_page_draws_its_rows(cx: &mut TestAppContext) {
        let (_, cx) = open(cx);
        cx.simulate_resize(size(px(1280.), px(720.)));
        draw(cx);

        let list = cx
            .debug_bounds("settings-list")
            .expect("the list has bounds");
        assert!(list.size.height > px(0.), "the list has height: {list:?}");

        // A page starts with a gap, then a card heading, then its first row.
        let row = cx.debug_bounds(selector(2)).expect("the first row draws");
        assert!(row.size.height > px(0.), "the row has height: {row:?}");
        assert!(row.size.width > px(0.), "the row has width: {row:?}");
    }

    #[gpui::test]
    fn the_cursor_scrolls_a_row_below_the_fold_into_view(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        // Short, like a television drawing at a scale factor of two.
        cx.simulate_resize(size(px(1000.), px(600.)));
        draw(cx);
        cx.simulate_keystrokes("right");
        draw(cx);

        let last = cx.read(|app| {
            let this = view.read(app);
            let pages = this.pages();
            let page = this.nav.page().min(pages.len().saturating_sub(1));
            let (_, rows) = flat_items(&pages[page]);
            *rows.last().expect("the page has rows")
        });
        let list = cx
            .debug_bounds("settings-list")
            .expect("the list has bounds");
        let before = cx.debug_bounds(selector(last));
        assert!(
            before.is_none_or(|bounds| bounds.origin.y >= list.origin.y + list.size.height),
            "the last row starts below the fold: {before:?}"
        );

        for _ in 0..12 {
            cx.simulate_keystrokes("down");
        }
        draw(cx);

        let after = cx
            .debug_bounds(selector(last))
            .expect("the last row is laid out");
        assert!(
            after.origin.y >= list.origin.y - px(1.)
                && after.origin.y + after.size.height <= list.origin.y + list.size.height + px(1.),
            "the cursor scrolled the row into the list: row {after:?} in list {list:?}"
        );
    }

    #[gpui::test]
    fn enter_flips_the_focused_toggle(cx: &mut TestAppContext) {
        use super::daemon::Categorization;

        // The switch is the daemon's, so the snapshot has to say it was read.
        let snapshot = Snapshot {
            daemon: true,
            categorization: Some(Categorization {
                enabled: false,
                capable: Some(true),
            }),
            ..Snapshot::default()
        };
        let (view, cx) = open_with(snapshot, cx);
        cx.simulate_resize(size(px(1280.), px(720.)));
        draw(cx);

        // `Library` is the third page; its first row is the categorisation switch.
        cx.simulate_keystrokes("right");
        cx.simulate_keystrokes("right");
        draw(cx);
        assert!(
            !cx.read(|app| view.read(app).toggle_value(ToggleField::Categorization)),
            "the switch starts off"
        );

        // Dispatched without parking, so the daemon's write has not answered
        // yet: the switch must already show the value that was asked for.
        cx.update(|window, cx| window.dispatch_action(Confirm.boxed_clone(), cx));

        assert!(
            cx.read(|app| view.read(app).toggle_value(ToggleField::Categorization)),
            "Enter flips the row the cursor is on"
        );
    }

    #[gpui::test]
    fn enter_focuses_a_text_row_so_typing_lands_in_it(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        cx.simulate_resize(size(px(1280.), px(720.)));
        draw(cx);

        // `Connections` is the second page; its first row is the RomM address.
        cx.simulate_keystrokes("right");
        cx.simulate_keystrokes("enter");
        draw(cx);
        cx.simulate_input("romm.local");

        let typed = cx.read(|app| view.read(app).inputs.romm_url.read(app).value().to_string());
        assert_eq!(typed, "romm.local", "typing lands in the focused field");
    }

    #[gpui::test]
    fn a_pasted_token_lands_in_the_focused_field(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        cx.simulate_resize(size(px(1280.), px(720.)));
        draw(cx);

        // `Connections` → the RomM address, the first text row on the page.
        cx.simulate_keystrokes("right");
        cx.simulate_keystrokes("enter");
        draw(cx);

        cx.write_to_clipboard(gpui::ClipboardItem::new_string("pcloud".to_string()));
        cx.simulate_keystrokes("ctrl-v");

        let pasted = cx.read(|app| view.read(app).inputs.romm_url.read(app).value().to_string());
        assert_eq!(pasted, "pcloud", "Ctrl+V lands in the focused field");

        // The other two Linux paste muscle memories reach the same field, and
        // they replace what is there rather than silently doing nothing.
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("romm.local".to_string()));
        cx.simulate_keystrokes("ctrl-shift-v");
        let replaced = cx.read(|app| view.read(app).inputs.romm_url.read(app).value().to_string());
        assert_eq!(replaced, "romm.local", "Ctrl+Shift+V pastes too");
    }

    #[gpui::test]
    fn a_paste_with_nothing_on_the_clipboard_says_so(cx: &mut TestAppContext) {
        use gpui_component::WindowExt as _;

        let (_, cx) = open(cx);
        cx.simulate_resize(size(px(1280.), px(720.)));
        draw(cx);
        cx.simulate_keystrokes("right");
        cx.simulate_keystrokes("enter");
        draw(cx);

        let before = cx.update(|window, cx| window.notifications(cx).len());
        cx.simulate_keystrokes("ctrl-shift-v");
        draw(cx);
        let after = cx.update(|window, cx| window.notifications(cx).len());
        assert_eq!(
            after,
            before + 1,
            "an unreadable clipboard is reported, not silent"
        );
    }

    fn backup_snapshot(remote_available: bool) -> Snapshot {
        Snapshot {
            daemon: true,
            backup: Some(crate::daemon::BackupView {
                enabled: false,
                remote: "pcloud".to_string(),
                root: "hearthdeck-backups".to_string(),
                schedule: crate::daemon::BackupSchedule {
                    hour: 4,
                    minute: 30,
                },
                remote_available,
                rclone_installed: true,
                remotes: vec!["pcloud".to_string()],
                updated_at: None,
            }),
            ..Snapshot::default()
        }
    }

    /// The first row whose control matches: its flat index (where the item and
    /// the drawn row are) and its cursor index (what a click or arrow addresses).
    fn locate_row(page: &Page, wanted: impl Fn(&Control) -> bool) -> Option<(usize, usize)> {
        let mut flat = 0;
        let mut cursor = 0;
        for group in &page.groups {
            for row in &group.rows {
                if wanted(&row.control) {
                    return Some((flat, cursor));
                }
                if !matches!(row.control, Control::Fact(_)) {
                    cursor += 1;
                }
                flat += 1;
            }
        }
        None
    }

    /// A result is a notification. Drawing it under the page gave a saved
    /// setting the same weight as a setting that is actually on.
    #[gpui::test]
    fn a_result_is_delivered_as_a_notification(cx: &mut TestAppContext) {
        use gpui_component::WindowExt as _;

        let (view, cx) = open(cx);
        cx.simulate_resize(size(px(1280.), px(720.)));
        draw(cx);
        let before = cx.update(|window, cx| window.notifications(cx).len());

        view.update(cx, |this, cx| {
            this.say("The backup settings were saved.");
            cx.notify();
        });
        draw(cx);

        let after = cx.update(|window, cx| window.notifications(cx).len());
        assert_eq!(
            after,
            before + 1,
            "the result reached the kit's notification list"
        );
        assert!(
            cx.read(|app| view.read(app).notices.is_empty()),
            "the queue is drained once the window has delivered it"
        );
    }

    /// The status label is a `Marker` at the trailing edge of the heading, not a
    /// small chip glued to the title's varying width.
    #[gpui::test]
    fn the_card_status_is_a_marker_at_the_trailing_edge(cx: &mut TestAppContext) {
        let (_, cx) = open(cx);
        cx.simulate_resize(size(px(1280.), px(720.)));
        draw(cx);

        let header = cx
            .debug_bounds(selector(1))
            .expect("the first card heading draws");
        let marker = cx
            .debug_bounds("card-status-Connected")
            .expect("the status marker draws");
        assert!(
            marker.origin.x > header.origin.x + header.size.width / 2.,
            "the marker is at the trailing edge: marker {marker:?} in header {header:?}"
        );
        assert!(
            marker.size.height > px(0.),
            "the marker has a readable height: {marker:?}"
        );
    }

    /// A missing remote is a condition, so it is an `Alert` above the cards -
    /// not a fact row that scrolls away and reads like a setting.
    #[gpui::test]
    fn a_missing_remote_is_an_alert_above_the_cards(cx: &mut TestAppContext) {
        let (view, cx) = open_with(backup_snapshot(false), cx);
        cx.simulate_resize(size(px(1280.), px(720.)));
        draw(cx);
        cx.simulate_keystrokes("left");
        draw(cx);

        assert!(
            cx.debug_bounds("page-alert").is_some(),
            "the backup page draws the alert"
        );
        let claimed = cx.read(|app| {
            let this = view.read(app);
            this.pages()
                .into_iter()
                .find(|page| page.kind == PageKind::Backup)
                .expect("a backup page")
                .groups
                .iter()
                .flat_map(|group| &group.rows)
                .any(|row| row.label == "Remote not found")
        });
        assert!(!claimed, "the condition is not also drawn as a row");
    }

    /// Enter on the schedule focuses it; so does a click anywhere on its row.
    /// The row is wide and the field is one small control, so a press on the
    /// label has to start editing or the field reads as broken.
    #[gpui::test]
    fn the_schedule_opens_from_its_row_and_takes_digits(cx: &mut TestAppContext) {
        let (view, cx) = open_with(backup_snapshot(true), cx);
        cx.simulate_resize(size(px(1280.), px(720.)));
        draw(cx);

        let page_index = PageKind::ALL
            .iter()
            .position(|kind| *kind == PageKind::Backup)
            .expect("a backup page");
        let (flat, cursor) = cx.read(|app| {
            let this = view.read(app);
            let pages = this.pages();
            let page = &pages[page_index];
            locate_row(page, |control| matches!(control, Control::Time))
                .expect("the schedule is a cursor row")
        });
        // Open the page at its top first: the list resets on a page change and
        // stays there by design, and only a cursor that then moves is revealed.
        view.update(cx, |this, _| this.nav.select(page_index, 0));
        draw(cx);
        for _ in 0..cursor {
            cx.simulate_keystrokes("down");
            draw(cx);
        }

        let (_, items) = cx.read(|app| {
            let this = view.read(app);
            let pages = this.pages();
            flat_items(&pages[page_index])
        });
        let item = items[flat];
        let bounds = cx
            .debug_bounds(selector(item))
            .expect("the schedule row draws");
        cx.simulate_click(
            point(
                // Inside the row's own padding, on the label side: the card
                // surface starts after the list item's horizontal inset.
                bounds.origin.x + px(120.),
                bounds.origin.y + bounds.size.height / 2.,
            ),
            gpui::Modifiers::default(),
        );
        draw(cx);

        let focused = cx.update(|window, cx| {
            view.read(cx)
                .inputs
                .backup_schedule
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        });
        assert!(focused, "the press on the row focused the schedule");

        cx.simulate_input("0930");
        let time = cx.read(|app| view.read(app).inputs.backup_schedule.read(app).time());
        assert_eq!(
            time,
            chrono::NaiveTime::from_hms_opt(9, 30, 0).expect("a valid time"),
            "digits typed after opening the row land in the schedule"
        );
    }

    /// The controls are sized for the sofa: a field is taller than the row text
    /// it sits beside, not smaller than it.
    #[gpui::test]
    fn the_inputs_and_buttons_are_sized_for_a_television(cx: &mut TestAppContext) {
        let (_, cx) = open(cx);
        cx.simulate_resize(size(px(1280.), px(720.)));
        draw(cx);

        // Connections: its first row is the RomM address field, and the row
        // under it is the primary Link RomM button.
        cx.simulate_keystrokes("right");
        draw(cx);
        let field = cx
            .debug_bounds("control-1-0")
            .expect("the RomM address field draws");
        assert!(
            field.size.height >= px(48.),
            "a text field is at least a large control: {field:?}"
        );

        let button = cx
            .debug_bounds("control-1-1")
            .expect("the link button draws");
        assert!(
            button.size.height >= px(40.),
            "a button is at least a large control: {button:?}"
        );
    }
}
