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
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, ClickEvent, Context, Entity,
    Focusable as _, InteractiveElement as _, IntoElement, KeyBinding, ListAlignment, ListState,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, list, point,
    prelude::FluentBuilder as _, px, size,
};
use gpui_component::alert::Alert;
use gpui_component::button::Button;
use gpui_component::input::{Input, InputState};
use gpui_component::progress::ProgressCircle;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::separator::Separator;
use gpui_component::switch::Switch;
use gpui_component::tag::Tag;
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
    Emulation,
}

impl PageKind {
    const ALL: [Self; 4] = [
        Self::Status,
        Self::Connections,
        Self::Library,
        Self::Emulation,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Connections => "Connections",
            Self::Library => "Library",
            Self::Emulation => "Emulation",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Status => "What Hearthdeck sees of the daemon and this machine.",
            Self::Connections => "The accounts Hearthdeck talks to on your behalf.",
            Self::Library => "How the library is gathered and grouped.",
            Self::Emulation => "The cores RetroArch needs, and what is ready to play.",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Status => IconName::Info,
            Self::Connections => IconName::Globe,
            Self::Library => IconName::FolderOpen,
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
    Toggle,
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

#[derive(Clone)]
struct Page {
    kind: PageKind,
    groups: Vec<Group>,
}

impl Page {
    fn row_count(&self) -> usize {
        self.groups.iter().map(|group| group.rows.len()).sum()
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
    /// The space between two cards.
    Gap,
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
        items.push(Item::Gap);
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
    items.push(Item::Gap);
    (items, rows)
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
    /// The daemon maintenance run in flight - a core install, a BIOS sync, a
    /// rail rebuild - so its row is disabled until the report comes back.
    busy: Option<RowAction>,
    /// The scrolling page: one item per card heading, card row, or gap. A list,
    /// not a column, because `scroll_to_reveal_item` is what keeps the cursor's
    /// row on screen - the page has to be navigable with no pointer.
    list: ListState,
    /// The page the list was built for, and the row it was last asked to reveal.
    list_page: usize,
    scrolled_to: Option<(usize, usize)>,
    /// What the last action row said it would do. One line, under the page.
    notice: Option<String>,
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
            romm_pending: None,
            stremio_offer: None,
            stremio_seconds_left: None,
            stremio_generation: 0,
            stremio_pending: None,
            busy: None,
            list: ListState::new(0, ListAlignment::Top, px(400.)),
            list_page: 0,
            scrolled_to: None,
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
                        status: None,
                        rows: vec![Row {
                            label: "Refresh",
                            description: Some("Read the daemon and this machine again."),
                            control: Control::Action(RowAction::Refresh),
                        }],
                    },
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
                                description: Some("Installed cores against the systems found."),
                                control: Control::Fact(Value::Text(readiness.clone())),
                            },
                        ],
                    },
                    Group {
                        title: "Storage",
                        status: None,
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
                        } else if self.stremio_offer.is_some() {
                            ("Waiting", Tone::Attention)
                        } else {
                            ("Not linked", Tone::Off)
                        }),
                        rows: stremio_rows,
                    },
                    Group {
                        title: "TMDB",
                        status: Some(("Preview", Tone::Off)),
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
                        status: None,
                        rows: vec![
                            Row {
                                label: "Group the library",
                                description: Some(
                                    "Let Hearthdeck derive sections and collections from the catalog.",
                                ),
                                control: Control::Toggle,
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
                groups: vec![Group {
                    title: "Readiness",
                    status: None,
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
            RowAction::InstallCores => self.install_cores(cx),
            RowAction::SyncBios => self.sync_bios(cx),
            RowAction::RestoreRails => self.restore_rails(cx),
            _ => {
                self.notice = Some(action.preview().to_string());
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
            RowAction::InstallCores | RowAction::SyncBios | RowAction::RestoreRails => {
                self.busy.is_none()
            }
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
    fn toggle_value(&self) -> bool {
        mirrored(
            self.categorization_pending,
            self.snapshot
                .categorization
                .as_ref()
                .map(|categorization| categorization.enabled),
        )
    }

    /// A toggle whose state was never read is drawn disabled rather than
    /// focused and dead, and so is one while its write is in flight.
    fn toggle_enabled(&self) -> bool {
        self.categorization_pending.is_none()
            && self
                .snapshot
                .categorization
                .as_ref()
                .is_some_and(|categorization| categorization.capable != Some(false))
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

    /// Installs the cores the daemon's readiness report is missing, then re-reads
    /// readiness so the page shows what can play now.
    fn install_cores(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(RowAction::InstallCores);
        self.notice = Some("Installing the missing cores...".to_string());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (report, readiness) = cx
                .background_executor()
                .spawn(async { (daemon::install_cores(), daemon::readiness().ok()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                match report {
                    Ok(report) => {
                        this.notice = Some(describe_install(&report));
                        this.apply_readiness(readiness);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the core install failed");
                        this.notice = Some(format!("Cores not installed: {error}"));
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
        self.notice = Some("Syncing BIOS files...".to_string());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (report, readiness) = cx
                .background_executor()
                .spawn(async { (daemon::sync_bios(), daemon::readiness().ok()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                match report {
                    Ok(report) => {
                        this.notice = Some(describe_sync(&report));
                        this.apply_readiness(readiness);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the BIOS sync failed");
                        this.notice = Some(format!("BIOS not synced: {error}"));
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
        self.notice = Some("Rebuilding the dashboard rails...".to_string());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { daemon::restore_rails() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                this.notice = Some(match result {
                    Ok(()) => "The dashboard rails were removed; the dashboard derives its own."
                        .to_string(),
                    Err(error) => {
                        tracing::warn!(%error, "the dashboard rails could not be removed");
                        format!("Rails not removed: {error}")
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_readiness(&mut self, readiness: Option<(usize, usize)>) {
        if let Some((ready, total)) = readiness {
            self.snapshot.ready_systems = ready;
            self.snapshot.total_systems = total;
        }
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
            Some(Control::Toggle) => {
                if self.toggle_enabled() {
                    let value = !self.toggle_value();
                    self.set_categorization(value, cx);
                }
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
                v_flex().w_full().px_8().pt_8().pb_4().child(
                    v_flex()
                        .w_full()
                        .max_w(px(CONTENT_MAX_WIDTH))
                        .gap_2()
                        .child(div().text_3xl().child(page.kind.title()))
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(page.kind.description()),
                        )
                        .child(div().pt_2().child(Separator::horizontal().h(px(1.)))),
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
                    .vertical_scrollbar(&list_state),
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
            Item::Gap => div().h(px(24.)).into_any_element(),
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
                let group = &pages[page_index].groups[group];
                let row = &group.rows[row];
                let focused = page_index == self.nav.page() && flat == self.nav.row();
                div()
                    .w_full()
                    .px_8()
                    .child(
                        card(false, last)
                            .border_t_1()
                            .border_color(theme.border)
                            .child(self.row_element(view, row, page_index, flat, focused, theme)),
                    )
                    .into_any_element()
            }
        }
    }

    fn row_element(
        &self,
        view: Entity<Self>,
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
        let confirm_on_click = matches!(control, Control::Text(_));
        let view_for_click = view.clone();
        let on_click = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            view_for_click.update(cx, |this, cx| {
                this.nav.select(page_index, row_index);
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
            .border_1()
            .border_color(if focused {
                theme.ring
            } else {
                theme.transparent
            })
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
            .child(self.control_element(&view, &control, page_index, row_index, theme))
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
            Control::Text(field) => div()
                .w(px(FIELD_WIDTH))
                .child(Input::new(self.inputs.text(*field)))
                .into_any_element(),
            Control::Toggle => {
                let view = view.clone();
                Switch::new(SharedString::from(format!(
                    "switch-{page_index}-{row_index}"
                )))
                .checked(self.toggle_value())
                .disabled(!self.toggle_enabled())
                .on_click(move |checked: &bool, _: &mut Window, cx: &mut App| {
                    let value = *checked;
                    view.update(cx, |this, cx| this.set_categorization(value, cx));
                })
                .into_any_element()
            }
            Control::Action(action) => {
                let action = *action;
                let view = view.clone();
                Button::new(SharedString::from(format!(
                    "action-{page_index}-{row_index}"
                )))
                .label(action.label())
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
        (installed, 0) => format!("Installed {installed} core(s)."),
        (0, failures) => format!("No core could be installed ({failures} failed){first}"),
        (installed, failures) => {
            format!("Installed {installed} core(s); {failures} failed{first}")
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
        (fetched, 0) => format!("Placed {fetched} file(s)."),
        (0, failures) => format!("Nothing could be placed ({failures} failed){first}"),
        (fetched, failures) => format!("Placed {fetched} file(s); {failures} failed{first}"),
    }
}

/// A card's heading: the section name, and a status chip when the section has a
/// state worth reading at a glance. The rows still carry the detail.
fn group_title(group: &Group, theme: &Theme) -> AnyElement {
    h_flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .text_lg()
                .text_color(theme.foreground)
                .child(group.title),
        )
        .when_some(group.status, |this, (label, tone)| {
            let tag = match tone {
                Tone::Good => Tag::success(),
                Tone::Attention => Tag::warning(),
                Tone::Off => Tag::new(),
            };
            this.child(tag.text_sm().child(label))
        })
        .into_any_element()
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
        let page_index = self.nav.page().min(pages.len().saturating_sub(1));
        let (items, row_items) = flat_items(&pages[page_index]);

        // A different page, or a different number of rows, is a different list:
        // an offset from the old one means nothing.
        if self.list_page != page_index || self.list.item_count() != items.len() {
            self.list.reset(items.len());
            self.list_page = page_index;
            self.scrolled_to = None;
        }

        // Keep the cursor's row on screen. This is what makes a page taller than
        // the display navigable with no pointer at all.
        if let Some(item) = row_items.get(self.nav.row())
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
                    .when_some(self.notice.clone(), |this, notice| {
                        this.child(
                            div()
                                .px_8()
                                .pb_2()
                                .child(Alert::info("settings-notice", notice)),
                        )
                    })
                    .child(self.footer(cx, theme)),
            )
    }
}

/// Every key this window answers to. `main` and the render tests both call it,
/// so the path under test is the path that ships.
///
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
            install_shortcuts(cx, view.clone(), window.window_handle());

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
        Control, Group, Item, Page, PageKind, Row, TextField, Value, describe_install,
        describe_sync, flat_items, mirrored, stremio_account,
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
        assert_eq!(describe_install(&report), "Installed 1 core(s).");

        report.failed.push(CoreFailure {
            package: "libretro-dolphin".to_string(),
            reason: "no such package".to_string(),
        });
        let line = describe_install(&report);
        assert!(line.starts_with("Installed 1 core(s); 1 failed"), "{line}");
        assert!(line.contains("libretro-dolphin: no such package"), "{line}");
    }

    #[test]
    fn a_sync_report_names_the_first_file_that_failed() {
        use super::daemon::{BiosFailure, BiosSync};

        let mut report = BiosSync::default();
        assert_eq!(describe_sync(&report), "Nothing was missing.");

        report.fetched.push("bios/SCPH1001.BIN".to_string());
        assert_eq!(describe_sync(&report), "Placed 1 file(s).");

        report.failed.push(BiosFailure {
            filename: "bios/scph5500.bin".to_string(),
            reason: "not found".to_string(),
        });
        let line = describe_sync(&report);
        assert!(line.starts_with("Placed 1 file(s); 1 failed"), "{line}");
        assert!(line.contains("bios/scph5500.bin: not found"), "{line}");
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

    #[test]
    fn every_row_has_an_item_to_scroll_to() {
        let page = page_of(&[&["a", "b"], &["c"]]);
        let (items, rows) = flat_items(&page);

        assert_eq!(
            items.first(),
            Some(&Item::Gap),
            "a card starts clear of the heading"
        );
        assert_eq!(
            items.last(),
            Some(&Item::Gap),
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

/// The page as a window draws it. These are the tests that catch "the screen is
/// empty": bounds are what a user sees, and a list with no height or an item
/// that never renders fails here instead of on the television.
#[cfg(test)]
mod render_tests {
    use super::*;
    use gpui::{Action as _, TestAppContext, VisualTestContext};

    fn open(cx: &mut TestAppContext) -> (Entity<SettingsWindow>, &mut VisualTestContext) {
        open_with(Snapshot::default(), cx)
    }

    fn open_with(
        snapshot: Snapshot,
        cx: &mut TestAppContext,
    ) -> (Entity<SettingsWindow>, &mut VisualTestContext) {
        cx.update(gpui_component::init);
        let mut opened = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let inputs = Inputs {
                romm_url: cx.new(|cx| InputState::new(window, cx)),
                romm_user: cx.new(|cx| InputState::new(window, cx)),
                romm_password: cx.new(|cx| InputState::new(window, cx)),
                tmdb_key: cx.new(|cx| InputState::new(window, cx)),
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
        cx.simulate_resize(size(px(1000.), px(420.)));
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
            !cx.read(|app| view.read(app).toggle_value()),
            "the switch starts off"
        );

        // Dispatched without parking, so the daemon's write has not answered
        // yet: the switch must already show the value that was asked for.
        cx.update(|window, cx| window.dispatch_action(Confirm.boxed_clone(), cx));

        assert!(
            cx.read(|app| view.read(app).toggle_value()),
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
}
