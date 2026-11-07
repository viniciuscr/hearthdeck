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
//! **Most of the editing UI is still a preview.** Every field is real - it takes
//! focus, it types, a switch flips, a choice cycles - but only one of them
//! reaches the daemon today: the categorization opt-in on the Library page. It
//! is the proof of the write path - read the daemon's state, write, report what
//! the daemon says afterwards, revert the control when it refuses. The rest of
//! the rows attach to the same three steps, one at a time.

mod daemon;
mod nav;

use std::ffi::CString;
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, point, prelude::FluentBuilder as _, px, size,
};
use gpui_component::button::Button;
use gpui_component::group_box::GroupBox;
use gpui_component::input::{Input, InputState};
use gpui_component::progress::ProgressCircle;
use gpui_component::switch::Switch;
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Theme, ThemeMode, h_flex, v_flex,
};
use nix::sys::statvfs::statvfs;

use daemon::{Categorization, Snapshot, StremioLink, StremioOffer, read_snapshot};
use nav::Nav;

gpui::actions!(
    hearthdeck_settings,
    [
        CloseWindow,
        MoveUp,
        MoveDown,
        PreviousPage,
        NextPage,
        Confirm
    ]
);

/// The page column, sized for a television rather than a laptop window.
const SIDEBAR_WIDTH: f32 = 320.0;
/// How wide the reading column may get on a wide display.
const CONTENT_MAX_WIDTH: f32 = 1100.0;
const RING_SIZE: f32 = 48.0;
/// The width a text field takes on the right of its row.
const FIELD_WIDTH: f32 = 380.0;
/// The size the window restores to; fullscreen itself is the compositor's call.
const RESTORE_WIDTH: f32 = 1280.0;
const RESTORE_HEIGHT: f32 = 720.0;

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
    Appearance,
    Emulation,
}

impl PageKind {
    const ALL: [Self; 5] = [
        Self::Status,
        Self::Connections,
        Self::Library,
        Self::Appearance,
        Self::Emulation,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Connections => "Connections",
            Self::Library => "Library",
            Self::Appearance => "Appearance",
            Self::Emulation => "Emulation",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Status => "What Hearthdeck sees of the daemon and this machine.",
            Self::Connections => "The accounts Hearthdeck talks to on your behalf.",
            Self::Library => "How the library is gathered and grouped.",
            Self::Appearance => "How the library app looks and moves.",
            Self::Emulation => "The cores RetroArch needs, and what is ready to play.",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Status => IconName::Info,
            Self::Connections => IconName::Globe,
            Self::Library => IconName::FolderOpen,
            Self::Appearance => IconName::Palette,
            Self::Emulation => IconName::Cpu,
        }
    }
}

/// A writable text row. The value itself lives in its [`InputState`], because
/// that is the thing that owns a caret, a selection and an IME session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextField {
    RommUrl,
    RommUser,
    RommPassword,
    TmdbKey,
}

/// A switch row. The value lives in [`Draft`]: nothing else owns it yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToggleField {
    Categorization,
    ReduceMotion,
}

/// A choice row: Enter cycles the value in place, which is how a television
/// with one button does a dropdown without a popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChoiceField {
    Palette,
    Backdrop,
}

const PALETTES: &[&str] = &["Noir", "Aurora", "Ember", "Indigo"];
const BACKDROPS: &[&str] = &["Solid", "Edge wash", "Quiet grid"];

/// What Enter does with a focused row that is not a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowAction {
    Refresh,
    RommSignIn,
    RommForget,
    StremioLink,
    StremioUnlink,
    SaveTmdbKey,
    RestoreRails,
    ScanLibrary,
    InstallCores,
    SyncBios,
}

impl RowAction {
    fn label(self) -> &'static str {
        match self {
            Self::Refresh => "Refresh",
            Self::RommSignIn => "Sign in",
            Self::RommForget => "Forget connection",
            Self::StremioLink => "Link account",
            Self::StremioUnlink => "Unlink",
            Self::SaveTmdbKey => "Save key",
            Self::RestoreRails => "Restore dashboard",
            Self::ScanLibrary => "Scan now",
            Self::InstallCores => "Install missing cores",
            Self::SyncBios => "Sync BIOS",
        }
    }

    /// Why an action cannot run right now, in the user's words.
    fn blocked(self) -> &'static str {
        match self {
            Self::ScanLibrary => "Turn categorization on first.",
            _ => "That is already running.",
        }
    }

    /// What the row says when it is run. Until the write path lands, every
    /// action except Refresh only reports where it would post.
    fn preview(self) -> &'static str {
        match self {
            Self::Refresh => "Reading the daemon and this machine again.",
            Self::RommSignIn => "RomM sign-in would post these credentials to the daemon.",
            Self::RommForget => "Forgetting the connection would drop the stored RomM token.",
            Self::StremioLink => "Stremio linking would ask the daemon for a code.",
            Self::StremioUnlink => "Unlinking would drop the stored Stremio token.",
            Self::SaveTmdbKey => {
                "TMDB's key is set on the host's admin port, not from this window."
            }
            Self::RestoreRails => "The daemon would rebuild the dashboard's rails.",
            Self::ScanLibrary => "A categorization scan would start on the daemon.",
            Self::InstallCores => "The daemon would install the missing RetroArch cores.",
            Self::SyncBios => "The daemon would sync the BIOS files for the ready systems.",
        }
    }
}

/// Which RomM request is in flight, so its button is drawn disabled until the
/// daemon answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RommTask {
    SignIn,
    Forget,
}

/// Which Stremio request is in flight. Polling a pending code is not a task:
/// it runs until the code resolves or the offer is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StremioTask {
    Link,
    Unlink,
}

/// The right-hand side of a row.
#[derive(Clone)]
enum Control {
    /// A state or a measurement the window only reports.
    Fact(Value),
    Text(TextField),
    Toggle(ToggleField),
    Choice {
        field: ChoiceField,
        options: &'static [&'static str],
    },
    Action(RowAction),
}

#[derive(Clone)]
enum Value {
    Text(String),
    Storage {
        used_percent: f32,
        available: String,
    },
}

struct Page {
    kind: PageKind,
    groups: Vec<Group>,
}

impl Page {
    fn row_count(&self) -> usize {
        self.groups.iter().map(|group| group.rows.len()).sum()
    }
}

struct Group {
    title: &'static str,
    rows: Vec<Row>,
}

struct Row {
    label: &'static str,
    description: Option<&'static str>,
    control: Control,
}

/// Everything the window can edit that the daemon has not been told about yet.
///
/// Categorization is not here: it is the daemon's, and the window only mirrors
/// it. What lives in the draft is what has no write path yet.
///
/// The defaults are the daemon's, as `docs/appearance-system.md` states them:
/// fresh installations begin on Noir over Solid, with motion allowed.
#[derive(Default)]
struct Draft {
    reduce_motion: bool,
    palette: usize,
    backdrop: usize,
}

impl Draft {
    fn choice(&self, field: ChoiceField) -> usize {
        match field {
            ChoiceField::Palette => self.palette,
            ChoiceField::Backdrop => self.backdrop,
        }
    }

    fn options(field: ChoiceField) -> &'static [&'static str] {
        match field {
            ChoiceField::Palette => PALETTES,
            ChoiceField::Backdrop => BACKDROPS,
        }
    }

    /// The next value, wrapping - so a row with two options behaves like a
    /// switch and a row with four is a ring, with no extra key to learn.
    fn cycle_choice(&mut self, field: ChoiceField) {
        let options = Self::options(field).len();
        if options == 0 {
            return;
        }
        let next = (self.choice(field) + 1) % options;
        match field {
            ChoiceField::Palette => self.palette = next,
            ChoiceField::Backdrop => self.backdrop = next,
        }
    }
}

/// The text fields, created once with the window so their state survives every
/// frame: a caret that is rebuilt is a caret that loses its place.
struct Inputs {
    romm_url: Entity<InputState>,
    romm_user: Entity<InputState>,
    romm_password: Entity<InputState>,
    tmdb_key: Entity<InputState>,
}

impl Inputs {
    fn text(&self, field: TextField) -> &Entity<InputState> {
        match field {
            TextField::RommUrl => &self.romm_url,
            TextField::RommUser => &self.romm_user,
            TextField::RommPassword => &self.romm_password,
            TextField::TmdbKey => &self.tmdb_key,
        }
    }

    fn all(&self) -> [&Entity<InputState>; 4] {
        [
            &self.romm_url,
            &self.romm_user,
            &self.romm_password,
            &self.tmdb_key,
        ]
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
    draft: Draft,
    inputs: Inputs,
    /// Set while a refresh is in flight, so the row says so and Enter cannot
    /// stack a second read behind the first.
    refreshing: bool,
    /// The value the window asked the daemon for while that write is in flight.
    /// The switch shows it, so the control answers the press immediately, and
    /// drops back to the daemon's own answer when the write lands or fails.
    categorization_pending: Option<bool>,
    /// The RomM request in flight, if any: its buttons are drawn disabled until
    /// the daemon answers.
    romm_pending: Option<RommTask>,
    /// The Stremio link code waiting to be approved, and the seconds the daemon
    /// says are left. `None` when no link is pending.
    stremio_offer: Option<StremioOffer>,
    stremio_seconds_left: Option<u64>,
    /// Bumped whenever a pending link is replaced, so the older poll loop stops
    /// instead of writing over the newer one.
    stremio_generation: u64,
    stremio_pending: Option<StremioTask>,
    /// What the last action row said it would do. One line, under the page.
    notice: Option<String>,
}

impl SettingsWindow {
    fn new(snapshot: Snapshot, disk: Option<DiskUsage>, inputs: Inputs) -> Self {
        Self {
            snapshot,
            disk,
            nav: Nav::default(),
            draft: Draft::default(),
            inputs,
            refreshing: false,
            categorization_pending: None,
            romm_pending: None,
            stremio_offer: None,
            stremio_seconds_left: None,
            stremio_generation: 0,
            stremio_pending: None,
            notice: None,
        }
    }

    /// Everything the window draws, re-derived from the snapshot. Building it
    /// from state rather than caching it is what keeps a refresh honest: there
    /// is no second copy of the page to fall out of step with the read.
    fn pages(&self) -> Vec<Page> {
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
            self.stremio_offer.is_some(),
        );
        let storage = match &self.disk {
            Some(disk) => Value::Storage {
                used_percent: disk.used_percent(),
                available: human_size(disk.available),
            },
            None => Value::Text("unavailable".to_string()),
        };

        let mut romm_rows = vec![
            Row {
                label: "Server address",
                description: Some("Where your RomM library is served."),
                control: Control::Text(TextField::RommUrl),
            },
            Row {
                label: "Username",
                description: Some("The RomM account Hearthdeck signs in as."),
                control: Control::Text(TextField::RommUser),
            },
            Row {
                label: "Password",
                description: Some("Used once for the sign-in; the daemon keeps only the token."),
                control: Control::Text(TextField::RommPassword),
            },
            Row {
                label: "Sign in",
                description: Some("Exchanges these credentials for a RomM token."),
                control: Control::Action(RowAction::RommSignIn),
            },
        ];
        // Only a connection that exists can be forgotten, and the row's presence
        // is what tells the user which state they are in.
        if self.snapshot.romm.is_some() {
            romm_rows.push(Row {
                label: "Forget connection",
                description: Some("Removes the stored token from the daemon."),
                control: Control::Action(RowAction::RommForget),
            });
        }
        romm_rows.push(Row {
            label: "Saved connection",
            description: None,
            control: Control::Fact(Value::Text(romm)),
        });

        let mut stremio_rows = vec![Row {
            label: "Account",
            description: None,
            control: Control::Fact(Value::Text(stremio)),
        }];
        // A pending code is a state, not a dialog: the rows say what to type and
        // where, and disappear when the code resolves.
        if let Some(offer) = &self.stremio_offer {
            stremio_rows.push(Row {
                label: "Code",
                description: Some("Enter this in the Stremio app, or open the link below."),
                control: Control::Fact(Value::Text(offer.code.clone())),
            });
            stremio_rows.push(Row {
                label: "Approve at",
                description: None,
                control: Control::Fact(Value::Text(offer.link.clone())),
            });
            stremio_rows.push(Row {
                label: "Waiting",
                description: Some("Until the code is approved or runs out."),
                control: Control::Fact(Value::Text(match self.stremio_seconds_left {
                    Some(seconds) => format!("{seconds}s left"),
                    None => "checking...".to_string(),
                })),
            });
        }
        if self.snapshot.stremio_linked_at.is_some() {
            stremio_rows.push(Row {
                label: "Unlink account",
                description: Some("Stops reading the Stremio addons and removes their rows."),
                control: Control::Action(RowAction::StremioUnlink),
            });
        } else if self.stremio_offer.is_none() {
            stremio_rows.push(Row {
                label: "Link account",
                description: Some("A code is shown here to approve elsewhere."),
                control: Control::Action(RowAction::StremioLink),
            });
        }

        vec![
            Page {
                kind: PageKind::Status,
                groups: vec![
                    Group {
                        title: "This window",
                        rows: vec![Row {
                            label: "Refresh",
                            description: Some("Read the daemon and this machine again."),
                            control: Control::Action(RowAction::Refresh),
                        }],
                    },
                    Group {
                        title: "Daemon",
                        rows: vec![
                            Row {
                                label: "Backend",
                                description: Some("The loopback service this window reads."),
                                control: Control::Fact(Value::Text(daemon)),
                            },
                            Row {
                                label: "Retro readiness",
                                description: Some("Installed cores against the systems found."),
                                control: Control::Fact(Value::Text(readiness.clone())),
                            },
                        ],
                    },
                    Group {
                        title: "Storage",
                        rows: vec![Row {
                            label: "Free space",
                            description: Some("On this machine's home volume."),
                            control: Control::Fact(storage),
                        }],
                    },
                ],
            },
            Page {
                kind: PageKind::Connections,
                groups: vec![
                    Group {
                        title: "RomM",
                        rows: romm_rows,
                    },
                    Group {
                        title: "Stremio",
                        rows: stremio_rows,
                    },
                    Group {
                        title: "TMDB",
                        rows: vec![
                            Row {
                                label: "API key",
                                description: Some("Used for artwork and release dates."),
                                control: Control::Text(TextField::TmdbKey),
                            },
                            Row {
                                label: "Save key",
                                description: None,
                                control: Control::Action(RowAction::SaveTmdbKey),
                            },
                        ],
                    },
                ],
            },
            Page {
                kind: PageKind::Library,
                groups: vec![
                    Group {
                        title: "Categories",
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
                        rows: vec![Row {
                            label: "Restore rails",
                            description: Some(
                                "Puts the dashboard's rails back as Hearthdeck derives them.",
                            ),
                            control: Control::Action(RowAction::RestoreRails),
                        }],
                    },
                ],
            },
            Page {
                kind: PageKind::Appearance,
                groups: vec![
                    Group {
                        title: "Palette",
                        rows: vec![Row {
                            label: "Color identity",
                            description: Some(
                                "Curated dark families; the library app applies them.",
                            ),
                            control: Control::Choice {
                                field: ChoiceField::Palette,
                                options: PALETTES,
                            },
                        }],
                    },
                    Group {
                        title: "Backdrop",
                        rows: vec![Row {
                            label: "Route treatment",
                            description: Some("Static by design: no video, no shaders."),
                            control: Control::Choice {
                                field: ChoiceField::Backdrop,
                                options: BACKDROPS,
                            },
                        }],
                    },
                    Group {
                        title: "Motion",
                        rows: vec![Row {
                            label: "Reduce motion",
                            description: Some(
                                "Shortens transitions instead of playing them in full.",
                            ),
                            control: Control::Toggle(ToggleField::ReduceMotion),
                        }],
                    },
                ],
            },
            Page {
                kind: PageKind::Emulation,
                groups: vec![Group {
                    title: "Readiness",
                    rows: vec![
                        Row {
                            label: "Systems",
                            description: Some("What RetroArch can start right now."),
                            control: Control::Fact(Value::Text(readiness)),
                        },
                        Row {
                            label: "Missing cores",
                            description: Some("Cores the daemon knows how to fetch."),
                            control: Control::Action(RowAction::InstallCores),
                        },
                        Row {
                            label: "BIOS files",
                            description: Some("Extra firmware, for the systems that need it."),
                            control: Control::Action(RowAction::SyncBios),
                        },
                    ],
                }],
            },
        ]
    }

    /// The focused row's control, so a key handler and the drawn cursor cannot
    /// disagree about what Enter would do.
    fn selection(&self) -> Option<Control> {
        let pages = self.pages();
        let page = pages.get(self.nav.page())?;
        let mut row = self.nav.row();
        for group in &page.groups {
            if row < group.rows.len() {
                return Some(group.rows[row].control.clone());
            }
            row -= group.rows.len();
        }
        None
    }

    /// Reads the daemon and the disk on a background thread: both are blocking,
    /// and a dead daemon must not freeze the window it is reporting on.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        self.notice = Some(RowAction::Refresh.preview().to_string());
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
                cx.notify();
            });
        })
        .detach();
    }

    fn run(&mut self, action: RowAction, cx: &mut Context<Self>) {
        if !self.action_enabled(action) {
            self.notice = Some(action.blocked().to_string());
            cx.notify();
            return;
        }
        match action {
            RowAction::Refresh => self.refresh(cx),
            RowAction::ScanLibrary => self.scan_categorization(cx),
            RowAction::RommSignIn => self.sign_in_romm(cx),
            RowAction::RommForget => self.forget_romm(cx),
            RowAction::StremioLink => self.start_stremio_link(cx),
            RowAction::StremioUnlink => self.unlink_stremio(cx),
            other => {
                self.notice = Some(other.preview().to_string());
                cx.notify();
            }
        }
    }

    fn action_enabled(&self, action: RowAction) -> bool {
        match action {
            RowAction::ScanLibrary => self.categorization_enabled(),
            RowAction::RommSignIn | RowAction::RommForget => self.romm_pending.is_none(),
            RowAction::StremioLink => {
                self.stremio_pending.is_none() && self.stremio_offer.is_none()
            }
            RowAction::StremioUnlink => self.stremio_pending.is_none(),
            _ => true,
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
            ToggleField::ReduceMotion => self.draft.reduce_motion,
        }
    }

    /// A toggle with no daemon behind it is still offered; one whose state was
    /// never read is drawn disabled rather than focused and dead.
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
            ToggleField::ReduceMotion => true,
        }
    }

    fn set_toggle(&mut self, field: ToggleField, value: bool, cx: &mut Context<Self>) {
        match field {
            ToggleField::Categorization => self.set_categorization(value, cx),
            ToggleField::ReduceMotion => {
                self.draft.reduce_motion = value;
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
        self.notice = Some(
            if enabled {
                "Turning categorization on..."
            } else {
                "Turning categorization off..."
            }
            .to_string(),
        );
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
                        this.notice = Some(
                            if state.enabled {
                                "Categorization is on."
                            } else {
                                "Categorization is off."
                            }
                            .to_string(),
                        );
                        this.snapshot.categorization = Some(Categorization { capable, ..state });
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the categorization write failed");
                        this.notice = Some(format!("Categorization unchanged: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn scan_categorization(&mut self, cx: &mut Context<Self>) {
        self.notice = Some("Asking the daemon to scan...".to_string());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { daemon::scan_categorization() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.notice = Some(match result {
                    Ok(()) => "The daemon accepted the scan.".to_string(),
                    Err(error) => format!("The scan was not started: {error}"),
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// The RomM form's three values, trimmed. The password is not trimmed: a
    /// leading or trailing space may be part of it.
    fn romm_form(&self, cx: &App) -> (String, String, String) {
        (
            self.inputs.romm_url.read(cx).value().trim().to_string(),
            self.inputs.romm_user.read(cx).value().trim().to_string(),
            self.inputs.romm_password.read(cx).value().to_string(),
        )
    }

    /// Signs in once and keeps only what the daemon saves: the window never
    /// stores the credentials, and empties the password field when they worked.
    fn sign_in_romm(&mut self, cx: &mut Context<Self>) {
        if self.romm_pending.is_some() {
            return;
        }
        let (base_url, username, password) = self.romm_form(cx);
        if base_url.is_empty() || username.is_empty() || password.is_empty() {
            self.notice = Some("Fill in the address, the username and the password.".to_string());
            cx.notify();
            return;
        }
        self.romm_pending = Some(RommTask::SignIn);
        self.notice = Some("Signing in to RomM...".to_string());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { daemon::sign_in_romm(base_url, username, password) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.romm_pending = None;
                match result {
                    Ok(connection) => {
                        this.notice = Some(format!("Signed in to {}.", connection.base_url));
                        this.snapshot.romm = Some(connection);
                        clear_field(cx, &this.inputs.romm_password);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the RomM sign-in failed");
                        this.notice = Some(format!("Sign-in failed: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn forget_romm(&mut self, cx: &mut Context<Self>) {
        if self.romm_pending.is_some() {
            return;
        }
        self.romm_pending = Some(RommTask::Forget);
        self.notice = Some("Removing the RomM connection...".to_string());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { daemon::clear_romm() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.romm_pending = None;
                match result {
                    Ok(()) => {
                        this.notice = Some("The RomM connection was removed.".to_string());
                        this.snapshot.romm = None;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the RomM connection could not be removed");
                        this.notice = Some(format!("Not removed: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn unlink_stremio(&mut self, cx: &mut Context<Self>) {
        if self.stremio_pending.is_some() {
            return;
        }
        self.stremio_pending = Some(StremioTask::Unlink);
        self.notice = Some("Removing the Stremio account...".to_string());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { daemon::unlink_stremio() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.stremio_pending = None;
                match result {
                    Ok(()) => {
                        this.notice = Some("The Stremio account was unlinked.".to_string());
                        this.snapshot.stremio_linked_at = None;
                        // Nothing is waiting on a code any more, and an older
                        // poll must not bring the account back.
                        this.stremio_generation = this.stremio_generation.wrapping_add(1);
                        this.stremio_offer = None;
                        this.stremio_seconds_left = None;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the Stremio account could not be unlinked");
                        this.notice = Some(format!("Not unlinked: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks the daemon for a link code, then waits on it.
    fn start_stremio_link(&mut self, cx: &mut Context<Self>) {
        if self.stremio_pending.is_some() {
            return;
        }
        self.stremio_pending = Some(StremioTask::Link);
        self.notice = Some("Asking the daemon for a Stremio code...".to_string());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { daemon::start_stremio_link() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.stremio_pending = None;
                match result {
                    Ok(offer) => {
                        this.notice = Some(format!("Approve code {} in Stremio.", offer.code));
                        this.stremio_seconds_left = Some(offer.expires_in_seconds);
                        this.stremio_offer = Some(offer);
                        this.stremio_generation = this.stremio_generation.wrapping_add(1);
                        this.poll_stremio_link(cx);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the Stremio link could not be started");
                        this.notice = Some(format!("Stremio link not started: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Polls until the code is approved, expires, or the offer is replaced. The
    /// loop stops as soon as its generation is no longer the current one.
    fn poll_stremio_link(&mut self, cx: &mut Context<Self>) {
        let generation = self.stremio_generation;
        cx.spawn(async move |this, cx| {
            loop {
                let status = cx
                    .background_executor()
                    .spawn(async { daemon::stremio_link_status() })
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        this.apply_link_status(generation, status, cx)
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
        generation: u64,
        status: Result<StremioLink, daemon::Error>,
        cx: &mut Context<Self>,
    ) -> bool {
        if generation != self.stremio_generation || self.stremio_offer.is_none() {
            return false;
        }
        match status {
            Ok(StremioLink::Waiting { seconds_left }) => {
                self.stremio_seconds_left = Some(seconds_left);
                cx.notify();
                true
            }
            Ok(StremioLink::Linked { connection }) => {
                self.notice = Some("Stremio account linked.".to_string());
                self.snapshot.stremio_linked_at = Some(connection.linked_at);
                self.stremio_offer = None;
                self.stremio_seconds_left = None;
                cx.notify();
                false
            }
            Ok(StremioLink::Expired) => {
                self.notice = Some("The Stremio code expired; ask for a new one.".to_string());
                self.stremio_offer = None;
                self.stremio_seconds_left = None;
                cx.notify();
                false
            }
            Ok(StremioLink::Idle) => {
                self.notice = Some("The Stremio link was cancelled.".to_string());
                self.stremio_offer = None;
                self.stremio_seconds_left = None;
                cx.notify();
                false
            }
            Err(error) => {
                tracing::warn!(%error, "the Stremio link status could not be read");
                self.notice = Some(format!("Stremio: {error}"));
                self.stremio_offer = None;
                self.stremio_seconds_left = None;
                cx.notify();
                false
            }
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
                }
            }
            Some(Control::Choice { field, .. }) => {
                self.draft.cycle_choice(field);
                cx.notify();
            }
            Some(Control::Action(action)) => self.run(action, cx),
            Some(Control::Fact(_)) | None => {}
        }
    }

    /// Escape: a text field is dismissed first, and only then the window - the
    /// order every writable field in Hearthdeck follows.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.text_field_focused(window, cx) {
            window.blur(cx);
            return;
        }
        cx.quit();
    }

    fn text_field_focused(&self, window: &Window, cx: &App) -> bool {
        let Some(focused) = window.focused(cx) else {
            return false;
        };
        self.inputs
            .all()
            .iter()
            .any(|input| input.read(cx).focus_handle(cx) == focused)
    }

    fn sidebar(&self, cx: &Context<Self>, theme: &Theme) -> AnyElement {
        v_flex()
            .id("settings-pages")
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .gap_1()
            .p_6()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(div().pb_4().text_xl().child("Settings"))
            .children(PageKind::ALL.iter().enumerate().map(|(index, kind)| {
                let active = index == self.nav.page();
                h_flex()
                    .id(SharedString::from(format!("page-{index}")))
                    .w_full()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .rounded(theme.radius)
                    .when(active, |this| this.bg(theme.sidebar_accent))
                    .child(Icon::new(kind.icon()).text_color(if active {
                        theme.foreground
                    } else {
                        theme.muted_foreground
                    }))
                    .child(
                        div()
                            .text_lg()
                            .text_color(if active {
                                theme.foreground
                            } else {
                                theme.muted_foreground
                            })
                            .child(kind.title()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.nav.select_page(index);
                        cx.notify();
                    }))
            }))
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Preview: edits stay in this window for now."),
            )
            .into_any_element()
    }

    fn content(&self, cx: &Context<Self>, theme: &Theme, pages: &[Page]) -> AnyElement {
        let page_index = self.nav.page().min(pages.len().saturating_sub(1));
        let page = &pages[page_index];
        let mut row_index = 0;
        let groups = page
            .groups
            .iter()
            .map(|group| {
                let rows = group
                    .rows
                    .iter()
                    .map(|row| {
                        let index = row_index;
                        let focused = page_index == self.nav.page() && index == self.nav.row();
                        row_index += 1;
                        self.row_element(cx, row, page_index, index, focused, theme)
                    })
                    .collect::<Vec<_>>();
                GroupBox::new()
                    .id(SharedString::from(group.title))
                    .title(div().text_lg().child(group.title))
                    .gap_1()
                    .children(rows)
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        v_flex()
            .id("settings-content")
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(CONTENT_MAX_WIDTH))
                    .flex_1()
                    .gap_6()
                    .p_8()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(div().text_3xl().child(page.kind.title()))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(page.kind.description()),
                            ),
                    )
                    .children(groups)
                    .when_some(self.notice.clone(), |this, notice| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(notice),
                        )
                    }),
            )
            .into_any_element()
    }

    fn row_element(
        &self,
        cx: &Context<Self>,
        row: &Row,
        page_index: usize,
        row_index: usize,
        focused: bool,
        theme: &Theme,
    ) -> AnyElement {
        let control = row.control.clone();
        // The row's click only carries rows whose control has no click of its
        // own: a switch and a button already do something when pressed, and
        // clicking them must not also run the row and undo it.
        let confirm_on_click = matches!(control, Control::Text(_) | Control::Choice { .. });
        h_flex()
            .id(SharedString::from(format!("row-{page_index}-{row_index}")))
            .w_full()
            .items_center()
            .justify_between()
            .gap_6()
            .px_4()
            .py_3()
            .rounded(theme.radius)
            .border_1()
            .border_color(if focused { theme.ring } else { theme.border })
            .when(focused, |this| this.bg(theme.list_active))
            .child(
                v_flex()
                    .flex_1()
                    .gap_1()
                    .child(div().text_lg().child(row.label))
                    .when_some(row.description, |this, description| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(description),
                        )
                    }),
            )
            .child(self.control_element(cx, &control, page_index, row_index, theme))
            // A pointer sets the cursor where it clicked, and does what Enter
            // would have done: nothing on this page is pointer-only.
            .on_click(cx.listener(move |this, _, window, cx| {
                this.nav.select(page_index, row_index);
                if confirm_on_click {
                    this.confirm(window, cx);
                }
            }))
            .into_any_element()
    }

    fn control_element(
        &self,
        cx: &Context<Self>,
        control: &Control,
        page_index: usize,
        row_index: usize,
        theme: &Theme,
    ) -> AnyElement {
        match control {
            Control::Fact(value) => self.value_element(value, theme),
            Control::Text(field) => div()
                .w(px(FIELD_WIDTH))
                .child(Input::new(self.inputs.text(*field)))
                .into_any_element(),
            Control::Toggle(field) => {
                let field = *field;
                Switch::new(SharedString::from(format!(
                    "switch-{page_index}-{row_index}"
                )))
                .checked(self.toggle_value(field))
                .disabled(!self.toggle_enabled(field))
                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                    this.set_toggle(field, *checked, cx);
                }))
                .into_any_element()
            }
            Control::Choice { field, options } => {
                let selected = self
                    .draft
                    .choice(*field)
                    .min(options.len().saturating_sub(1));
                let value = options.get(selected).copied().unwrap_or("--");
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded(theme.radius)
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.secondary)
                            .text_lg()
                            .child(value),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Enter to change"),
                    )
                    .into_any_element()
            }
            Control::Action(action) => {
                let action = *action;
                Button::new(SharedString::from(format!(
                    "action-{page_index}-{row_index}"
                )))
                .label(action.label())
                .disabled(!self.action_enabled(action))
                .on_click(cx.listener(move |this, _, _, cx| this.run(action, cx)))
                .into_any_element()
            }
        }
    }

    fn value_element(&self, value: &Value, theme: &Theme) -> AnyElement {
        match value {
            Value::Text(text) => div()
                .text_lg()
                .text_color(theme.foreground)
                .child(text.clone())
                .into_any_element(),
            Value::Storage {
                used_percent,
                available,
            } => h_flex()
                .items_center()
                .gap_3()
                .child(
                    ProgressCircle::new("storage")
                        .value(*used_percent)
                        .color(theme.progress_bar)
                        .w(px(RING_SIZE))
                        .h(px(RING_SIZE))
                        .child(div().text_xs().child(format!("{used_percent:.0}%"))),
                )
                .child(
                    div()
                        .text_lg()
                        .text_color(theme.foreground)
                        .child(available.clone()),
                )
                .into_any_element(),
        }
    }

    fn footer(&self, cx: &Context<Self>, theme: &Theme) -> AnyElement {
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
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(hint("Up/Down", "Move", theme))
                    .child(hint("Left/Right", "Page", theme))
                    .child(hint("Enter", "Select or edit", theme))
                    .child(hint("Escape", "Close", theme)),
            )
            .child(
                Button::new("close")
                    .label("Close")
                    .on_click(cx.listener(|_, _, _, cx| cx.quit())),
            )
            .into_any_element()
    }
}

/// Empties a text field. A write's completion has no window of its own, so the
/// window is taken from the app when it has one.
fn clear_field(cx: &mut Context<SettingsWindow>, field: &Entity<InputState>) {
    let Some(window) = cx.active_window() else {
        return;
    };
    let field = field.clone();
    let _ = window.update(cx, |_, window, cx| {
        field.update(cx, |state, cx| state.set_value("", window, cx));
    });
}

/// What the Stremio account row says: the daemon's account, a pending approval,
/// or nothing linked yet. A linked account outranks a stale offer.
fn stremio_account(linked_at: Option<&str>, waiting: bool) -> String {
    match (linked_at, waiting) {
        (Some(linked_at), _) => format!("Linked since {linked_at}"),
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

/// One keyboard hint: the key in a small cap, then what it does.
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
                .text_xs()
                .child(key),
        )
        .child(meaning)
        .into_any_element()
}

impl Render for SettingsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let pages = self.pages();
        let counts = pages.iter().map(Page::row_count).collect::<Vec<_>>();
        // The content is the source of truth for how many rows exist, so the
        // cursor is pulled back inside it before anything is drawn.
        self.nav.resize(&counts);

        h_flex()
            .size_full()
            .bg(theme.background)
            .child(self.sidebar(cx, theme))
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .child(self.content(cx, theme, &pages))
                    .child(self.footer(cx, theme)),
            )
    }
}

/// Runs `update` against the active window. App-level action handlers get an
/// `App`, and focusing a field - or blurring one - needs the window that owns
/// it.
fn with_active_window(cx: &mut App, update: impl FnOnce(&mut Window, &mut App)) {
    let Some(window) = cx.active_window() else {
        return;
    };
    let _ = window.update(cx, |_, window, cx| update(window, cx));
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("hearthdeck_settings=info")),
        )
        .init();

    let snapshot = read_snapshot();
    let disk = home_usage();
    // A saved connection prefills the form, so signing in again starts from the
    // address the daemon already holds.
    let saved_romm = snapshot
        .romm
        .as_ref()
        .map(|romm| romm.base_url.clone())
        .unwrap_or_default();

    gpui_platform::application().run(move |cx: &mut App| {
        gpui_component::init(cx);
        // The session this is drawn in is dark; the kit defaults to light.
        Theme::change(ThemeMode::Dark, None, cx);
        cx.bind_keys([
            KeyBinding::new("escape", CloseWindow, None),
            KeyBinding::new("up", MoveUp, None),
            KeyBinding::new("down", MoveDown, None),
            KeyBinding::new("left", PreviousPage, None),
            KeyBinding::new("right", NextPage, None),
            KeyBinding::new("enter", Confirm, None),
        ]);

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
                romm_url: cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("http://romm.local:8080")
                        .default_value(saved_romm.clone())
                }),
                romm_user: cx.new(|cx| InputState::new(window, cx).placeholder("Username")),
                romm_password: cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("Password")
                        .masked(true)
                }),
                tmdb_key: cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("TMDB API key")
                        .masked(true)
                }),
            };
            let view = cx.new(|_| SettingsWindow::new(snapshot, disk, inputs));

            // Cursor movement is plain state, so it needs no window; confirming
            // and closing do, because focusing a field is a window operation.
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
                with_active_window(cx, |window, cx| {
                    rows.update(cx, |this, cx| this.confirm(window, cx));
                });
            });
            let window_view = view.clone();
            cx.on_action(move |_: &CloseWindow, cx: &mut App| {
                with_active_window(cx, |window, cx| {
                    window_view.update(cx, |this, cx| this.close(window, cx));
                });
            });

            cx.new(|cx| gpui_base::Root::new(view, window, cx))
        });
        if let Err(error) = opened {
            tracing::error!(%error, "failed to open the settings window");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{BACKDROPS, ChoiceField, Draft, PALETTES, TextField, mirrored, stremio_account};

    #[test]
    fn the_stremio_row_says_which_state_the_account_is_in() {
        assert_eq!(
            stremio_account(Some("2026-01-02"), false),
            "Linked since 2026-01-02"
        );
        assert_eq!(stremio_account(None, true), "Waiting for approval");
        assert_eq!(stremio_account(None, false), "Not linked");
        assert_eq!(
            stremio_account(Some("2026-01-02"), true),
            "Linked since 2026-01-02",
            "a linked account outranks a stale offer"
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
    fn a_fresh_draft_is_the_documented_default() {
        let draft = Draft::default();

        assert!(
            !draft.reduce_motion,
            "motion is allowed until asked otherwise"
        );
        assert_eq!(draft.choice(ChoiceField::Palette), 0, "Noir");
        assert_eq!(draft.choice(ChoiceField::Backdrop), 0, "Solid");
    }

    #[test]
    fn a_choice_wraps_after_its_last_option() {
        let mut draft = Draft::default();
        assert_eq!(draft.choice(ChoiceField::Palette), 0);

        for _ in 0..PALETTES.len() {
            draft.cycle_choice(ChoiceField::Palette);
        }
        assert_eq!(
            draft.choice(ChoiceField::Palette),
            0,
            "cycling every option returns to the first"
        );
    }

    #[test]
    fn choices_cycle_one_at_a_time() {
        let mut draft = Draft::default();

        draft.cycle_choice(ChoiceField::Backdrop);

        assert_eq!(draft.choice(ChoiceField::Backdrop), 1);
        assert_eq!(
            draft.choice(ChoiceField::Palette),
            0,
            "the other choice keeps its value"
        );
    }

    #[test]
    fn the_choices_offer_the_options_the_appearance_doc_lists() {
        assert_eq!(PALETTES.len(), 4);
        assert_eq!(BACKDROPS.len(), 3);
        assert_eq!(Draft::options(ChoiceField::Palette), PALETTES);
        assert_eq!(Draft::options(ChoiceField::Backdrop), BACKDROPS);
    }

    #[test]
    fn every_text_row_addresses_its_own_field() {
        // Adding a text field without a row, or listing one twice, is a mistake
        // the page cannot catch at runtime - so the vocabulary is checked here.
        let fields = [
            TextField::RommUrl,
            TextField::RommUser,
            TextField::RommPassword,
            TextField::TmdbKey,
        ];
        for (index, field) in fields.iter().enumerate() {
            assert!(
                !fields[..index].contains(field),
                "a text field is listed twice"
            );
        }
    }
}
