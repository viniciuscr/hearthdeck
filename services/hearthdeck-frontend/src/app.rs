use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use clap::Parser;
use cosmic::iced::window;
use cosmic::surface::action::{LiveSettings, simple_popup};
use cosmic::widget::menu::menu_column::MenuColumn;
use cosmic::widget::reorderable_flex_row;
use cosmic::{
    Application as CosmicApplication, Element,
    app::{Core, CosmicFlags, Settings, Task},
    cctk::sctk::{
        self,
        shell::wlr_layer::{Anchor, KeyboardInteractivity},
    },
    cosmic_config::{Config, ConfigGet, CosmicConfigEntry},
    cosmic_theme::Spacing,
    dbus_activation,
    desktop::{DesktopEntryData, fde::IconSource, fde::PathSource, load_desktop_file},
    iced::{
        self, Alignment, ContentFit, Length, Limits, Subscription,
        event::listen_with,
        executor,
        id::Id,
        stream,
        widget::{
            column, container, row,
            scrollable::{AbsoluteOffset, RelativeOffset},
        },
    },
    iced::{
        core::{
            Padding, Rectangle, Vector,
            alignment::{Horizontal, Vertical},
            keyboard::{Key, key::Named},
            widget::operation::{
                self, Operation, Outcome,
                focusable::{Focusable, find_focused, focus},
            },
            window::Event as WindowEvent,
            window::Id as SurfaceId,
        },
        platform_specific::shell::wayland::commands::{
            self,
            layer_surface::{destroy_layer_surface, get_layer_surface},
            popup::destroy_popup,
        },
        runtime::{
            self as iced_runtime,
            dnd::end_dnd,
            platform_specific::wayland::{
                layer_surface::SctkLayerSurfaceSettings,
                popup::{SctkPopupSettings, SctkPositioner},
            },
        },
    },
    keyboard_nav,
    theme::{self, Button, TextInput},
    widget::{
        self,
        autosize::autosize,
        button::{self},
        divider,
        dnd_destination::dnd_destination_for_data,
        icon, list,
        list::list_column,
        scrollable, space, svg, text, text_input,
        toaster::{self, Toast, ToastId, Toasts},
        tooltip,
    },
};
use cosmic_app_list_config::AppListConfig;
use hearthdeck_protocol::InputProfile;
use itertools::Itertools;
use log::{error, warn};
use serde::{Deserialize, Serialize};

use crate::app_group::{AppGroup, AppLibraryConfig, Section};
use crate::fl;
use crate::input_ownership::{
    Event as InputEvent, InputOwnership, LaunchTarget, managed_launch_target,
};
use crate::launch_state::{Effect as LaunchEffect, Event as LaunchEvent, LaunchState};
use crate::providers::daemon::{
    RetroDetails, RetroGameDetails, RetroRomVersion, retro_rom_versions, retro_version_label,
};
use crate::style::{
    DASHBOARD_GAME_ASPECT, DASHBOARD_RAIL_TILES, DETAILS_ACTION_WIDTH, DETAILS_FACT_LABEL_WIDTH,
    DETAILS_HERO_RASTER, DETAILS_PICKER_WIDTH, DETAILS_PLAY_WIDTH, DETAILS_SHOT_ASPECT,
    DETAILS_SHOT_RASTER, DETAILS_SHOT_SIZE, DIALOG_ACTION_WIDTH, DIALOG_WIDTH, DIVIDER_WIDTH,
    EDIT_NAME_INPUT_WIDTH, GRID_COLUMNS, ICON_BODY, ICON_LARGE, ICON_SEARCH, ICON_SMALL,
    ICON_TILE_ACTION, MENU_MAX_HEIGHT, MENU_MAX_WIDTH, PAGE_TRANSITION_DURATION, SEARCH_WIDTH,
    SIDEBAR_ACCENT_BAR_WIDTH, TAB_TRANSITION_DURATION, TEXT_BODY, TEXT_CAPTION, TEXT_HEADER,
    TEXT_LARGE, TEXT_TITLE, WINDOW_HEIGHT, WINDOW_WIDTH, accent_bar, action_bar, artwork_fit,
    backdrop_scrim, backdrop_wash, card_surface, content_horizontal_padding,
    dashboard_console_tile_size, dashboard_nav_button_class, dashboard_tile_size,
    details_action_bar_padding, details_action_height, details_hero_width,
    details_toggle_button_class, filter_button_height, grid_gap, grid_top_padding, hero_card,
    launch_overlay, modal_scrim, primary_action_button_class, root_background, search_icon_padding,
    section_button_class, sidebar_accent_bar_height, sidebar_divider, sidebar_header_height,
    sidebar_item_height, sidebar_width, tab_button_class, tab_height, tab_underline_height,
    tab_width, tile_height, tile_width, title_action_height,
};
use crate::subscriptions::gamepad::{GamepadEvent, gamepad_events};
use crate::system_status::SystemStatus;
use crate::toplevel::{WindowHint, fullscreen_when_it_appears};
use crate::widgets::application::{AppletString, ApplicationButton};
use crate::widgets::rail::{rail, rail_item};
use crate::widgets::transition::{PageTransition, TabTransition};

// popovers should show options, but also the desktop info options
// should be a way to add apps to groups
// should be a way to remove apps from groups

static SEARCH_ID: LazyLock<Id> = LazyLock::new(|| Id::new("search"));
static FILTER_ID: LazyLock<Id> = LazyLock::new(|| Id::new("filter"));
static DASHBOARD_HOME_ID: LazyLock<Id> = LazyLock::new(|| Id::new("dashboard-home"));
static DASHBOARD_LIBRARY_ID: LazyLock<Id> = LazyLock::new(|| Id::new("dashboard-library"));
static DASHBOARD_SEARCH_ID: LazyLock<Id> = LazyLock::new(|| Id::new("dashboard-search"));

/// Rail key for the dashboard console list, which is not a shelved catalog
/// section, so it needs its own stable scrollable id.
const CONSOLES_RAIL_KEY: &str = "consoles";

static APP_ICON: LazyLock<icon::Handle> = LazyLock::new(|| {
    icon::from_svg_bytes(include_bytes!(
        "../data/icons/org.hearthdeck.HearthDeck.svg"
    ))
});

/// Single fixed scrollable id shared by all sections.
static SCROLLABLE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("section-scrollable"));

/// Horizontal scrollable that keeps the section tab strip on one line.
static TAB_STRIP_SCROLLABLE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("section-tabs-strip"));

/// Widget id of the group tab that selects the custom group with `key`.
fn group_tab_id(key: u64) -> Id {
    Id::new(format!("group-tab-{key}"))
}

static EDIT_GROUP_ID: LazyLock<Id> = LazyLock::new(|| Id::new("edit_group"));
static NEW_GROUP_ID: LazyLock<Id> = LazyLock::new(|| Id::new("new_group"));
static SUBMIT_DELETE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("cancel_delete"));

static CREATE_NEW: LazyLock<String> = LazyLock::new(|| fl!("create-new"));
static ADD_GROUP: LazyLock<String> = LazyLock::new(|| fl!("add-group"));
static SEARCH_PLACEHOLDER: LazyLock<String> = LazyLock::new(|| fl!("search-placeholder"));
static NEW_GROUP_PLACEHOLDER: LazyLock<String> = LazyLock::new(|| fl!("new-group-placeholder"));
static SAVE: LazyLock<String> = LazyLock::new(|| fl!("save"));
static CANCEL: LazyLock<String> = LazyLock::new(|| fl!("cancel"));
static RUN: LazyLock<String> = LazyLock::new(|| fl!("run"));
const LAUNCH_OVERLAY_DELAY: std::time::Duration = std::time::Duration::from_millis(1200);
const SESSION_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
const SYSTEM_STATUS_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
const DASHBOARD_HEALTH_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
const RECENT_ACTIVITY_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
const ROMM_PAGE_SIZE: u32 = 48;
const ROMM_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
static REMOVE: LazyLock<String> = LazyLock::new(|| fl!("remove"));
static FLATPAK: LazyLock<String> = LazyLock::new(|| fl!("flatpak"));
static LOCAL: LazyLock<String> = LazyLock::new(|| fl!("local"));
static NIX: LazyLock<String> = LazyLock::new(|| fl!("nix"));
static SNAP: LazyLock<String> = LazyLock::new(|| fl!("snap"));
static SYSTEM: LazyLock<String> = LazyLock::new(|| fl!("system"));

static NEW_GROUP_WINDOW_ID: LazyLock<SurfaceId> = LazyLock::new(SurfaceId::unique);
static NEW_GROUP_AUTOSIZE_ID: LazyLock<cosmic::widget::Id> =
    LazyLock::new(cosmic::widget::Id::unique);
static DELETE_GROUP_WINDOW_ID: LazyLock<SurfaceId> = LazyLock::new(SurfaceId::unique);
static DELETE_GROUP_AUTOSIZE_ID: LazyLock<cosmic::widget::Id> =
    LazyLock::new(cosmic::widget::Id::unique);
pub(crate) static MENU_ID: LazyLock<SurfaceId> = LazyLock::new(SurfaceId::unique);
pub(crate) static MENU_AUTOSIZE_ID: LazyLock<cosmic::widget::Id> =
    LazyLock::new(cosmic::widget::Id::unique);

/// Watch channel for provider records. The ProviderService writes to the
/// sender; the iced subscription creates a receiver and streams records as
/// `Message::ProviderRecords`. Uses `watch` instead of `broadcast` because
/// broadcast drops messages when no receivers exist, and the initial
/// discovery may complete before the subscription starts.
///
/// A guard receiver is kept alive in the static to ensure `send()` never
/// fails due to having zero receivers. Without it, the initial discovery
/// may complete and send records before the iced subscription has called
/// `subscribe()`, and `watch::Sender::send()` would drop the value.
struct ProviderRecordsChannel {
    tx: tokio::sync::watch::Sender<Vec<crate::providers::GameRecord>>,
    _guard: tokio::sync::watch::Receiver<Vec<crate::providers::GameRecord>>,
}

static PROVIDER_RECORDS: LazyLock<ProviderRecordsChannel> = LazyLock::new(|| {
    let (tx, guard) = tokio::sync::watch::channel(Vec::new());
    ProviderRecordsChannel { tx, _guard: guard }
});

fn provider_records_subscription() -> Subscription<Message> {
    Subscription::run_with((), |_| {
        stream::channel(
            4,
            |mut output: cosmic::iced::futures::channel::mpsc::Sender<Message>| async move {
                use cosmic::iced::futures::SinkExt;
                let mut rx = PROVIDER_RECORDS.tx.subscribe();
                // A freshly-subscribed receiver starts at the current version,
                // so `changed()` would block until a NEW value arrives —
                // missing the initial discovery records entirely.  Read the
                // current value first to deliver any records already present.
                let records = rx.borrow_and_update().clone();
                if !records.is_empty() {
                    let _ = output.send(Message::ProviderRecords(records)).await;
                }
                loop {
                    if rx.changed().await.is_err() {
                        break;
                    }
                    let records = rx.borrow_and_update().clone();
                    if records.is_empty() {
                        continue;
                    }
                    let _ = output.send(Message::ProviderRecords(records)).await;
                }
            },
        )
    })
}

/// Display name of the current user (the GECOS field from `/etc/passwd`),
/// falling back to the login name.
fn current_user_name() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    for line in passwd.lines() {
        let mut fields = line.split(':');
        if fields.next() == Some(&user) {
            if let Some(gecos) = fields.nth(3) {
                let name = gecos.split(',').next().unwrap_or_default().trim();
                if !name.is_empty() {
                    return name.to_string();
                }
            }
            break;
        }
    }
    if user.is_empty() {
        "User".to_string()
    } else {
        user
    }
}

/// Available bytes on the filesystem containing the given path.
fn available_disk_bytes(path: &str) -> u64 {
    let Ok(cstr) = std::ffi::CString::new(path) else {
        return 0;
    };
    nix::sys::statvfs::statvfs(cstr.as_c_str())
        .map(|vfs| vfs.blocks_available() * vfs.fragment_size())
        .unwrap_or(0)
}

/// Formats a byte count for humans, e.g. "128.4 GB".
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

#[derive(Parser, Debug, Serialize, Deserialize, Clone)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Args {
    #[clap(subcommand)]
    pub subcommand: Option<ApplicationsTasks>,
}

impl CosmicFlags for Args {
    type SubCommand = ApplicationsTasks;
    type Args = Vec<String>;

    fn action(&self) -> Option<&ApplicationsTasks> {
        self.subcommand.as_ref()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, clap::Subcommand)]
pub enum ApplicationsTasks {
    #[clap(about = "Start app-library with an input")]
    Input { input: Option<String> },
    #[clap(about = "Close app-library if open")]
    Close,
    #[clap(about = "Run a standalone instance (not single-instance)")]
    Run,
}

impl Display for ApplicationsTasks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", serde_json::ser::to_string(self).unwrap())
    }
}

impl FromStr for ApplicationsTasks {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::de::from_str(s)
    }
}

pub fn run() -> cosmic::iced::Result {
    let args = Args::parse();
    let settings = Settings::default()
        .antialiasing(true)
        .client_decorations(true)
        .debug(false)
        .default_text_size(16.0)
        .scale_factor(1.0)
        .size(iced::Size::new(WINDOW_WIDTH, WINDOW_HEIGHT))
        .resizable(None)
        .exit_on_close(true);

    // Use standalone run if requested, otherwise use single-instance
    if matches!(args.subcommand, Some(ApplicationsTasks::Run)) {
        cosmic::app::run::<HearthDeck>(settings, args)
    } else {
        cosmic::app::run_single_instance::<HearthDeck>(settings, args)
    }
}

pub struct AppSource(PathSource);

impl AppSource {
    pub fn as_icon(&self) -> Option<widget::icon::Handle> {
        let name = match &self.0 {
            PathSource::Local | PathSource::LocalDesktop => "app-source-local-symbolic",
            PathSource::System | PathSource::SystemLocal => "app-source-system-symbolic",
            PathSource::LocalFlatpak | PathSource::SystemFlatpak => "app-source-flatpak",
            PathSource::SystemSnap => "app-source-snap",
            PathSource::Nix | PathSource::LocalNix => "app-source-nix",
            PathSource::Other(_) => return None,
        };
        let handle = crate::icon_cache::icon_cache_handle(name, 16);
        Some(handle)
    }
}

impl<'a> From<&'a Path> for AppSource {
    fn from(path: &'a Path) -> Self {
        AppSource(PathSource::guess_from(path))
    }
}

impl Display for AppSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:.7}",
            match &self.0 {
                PathSource::Local | PathSource::LocalDesktop => LOCAL.as_str(),
                PathSource::SystemFlatpak | PathSource::LocalFlatpak => FLATPAK.as_str(),
                PathSource::SystemSnap => SNAP.as_str(),
                PathSource::Nix | PathSource::LocalNix => NIX.as_str(),
                PathSource::System | PathSource::SystemLocal => SYSTEM.as_str(),
                PathSource::Other(s) => s.as_str(),
            }
        )
    }
}

struct HearthDeck {
    page: Page,
    search_value: String,
    entry_path_input: Vec<Arc<DesktopEntryData>>,
    all_entries: Vec<Arc<DesktopEntryData>>,
    recent_entries: Vec<Arc<DesktopEntryData>>,
    menu: Option<usize>,
    /// Highlighted entry within the open context menu, navigated by the gamepad.
    menu_selection: usize,
    helper: Option<Config>,
    config: AppLibraryConfig,
    cur_section: Section,
    cur_group: Option<usize>,
    #[allow(dead_code)]
    locale: Option<String>,
    edit_name: Option<String>,
    new_group: Option<String>,
    dnd_icon: Option<usize>,
    offer_group: Option<Option<usize>>,
    waiting_for_filtered: bool,
    scroll_offset: f32,
    viewport_height: f32,
    /// Horizontal scroll offset and visible width of each dashboard rail,
    /// reported by the rail's `on_scroll`. Lets focus navigation scroll only
    /// when the selected card would leave the visible window.
    dashboard_rail_scroll: HashMap<&'static str, (f32, f32)>,
    window_width: f32,
    core: Core,
    group_to_delete: Option<usize>,
    duplicates: HashMap<PathBuf, (AppSource, Option<widget::icon::Handle>)>,
    app_list_config: AppListConfig,
    focused_id: Option<widget::Id>,
    entry_ids: Vec<widget::Id>,
    entry_icon_handles: Vec<widget::icon::Handle>,
    group_keys: Vec<u64>,
    next_group_key: u64,
    gamepad_focus_first: bool,
    /// Kept alive to hold the background provider discovery tasks.
    #[allow(dead_code)]
    provider_service: Option<crate::providers::service::ProviderService>,
    /// Client for the HearthDeck daemon, if available.
    daemon_client: Option<crate::providers::daemon::DaemonClient>,
    launch_state: LaunchState,
    input_ownership: InputOwnership,
    system_status: SystemStatus,
    /// Transient user-facing alerts, rendered with libcosmic's toaster so the
    /// dashboard stays a browsing surface instead of an alert log.
    toasts: Toasts<Message>,
    /// Reachability of the Hearthdeck background service, tracked so an alert
    /// fires only when its state flips rather than on every health poll.
    /// `None` until the first result arrives.
    backend_available: Option<bool>,
    /// Providers the daemon last reported as degraded. Compared against each
    /// new health result so the alert fires only when the set changes.
    degraded_providers: Vec<String>,
    dashboard_health_generation: u64,
    romm_request_generation: u64,
    /// Cached local path to each RomM platform's console artwork, used by the
    /// dashboard's console rail.
    romm_platform_icons: HashMap<i64, String>,
    /// Authoritative game count per RomM platform id, straight from the
    /// consoles endpoint. The Console Games grid lazy-loads pages, so the
    /// number of loaded entries grows as the user scrolls; this is the stable
    /// total the header should show instead of `entry_path_input.len()`.
    romm_platform_counts: HashMap<i64, u64>,
    /// Alternative versions (regions, revisions, discs) per RomM entry id,
    /// captured while the record still carries its metadata. The grid stores
    /// entries as `DesktopEntryData`, which has no metadata field, so the
    /// version picker reads from here instead.
    romm_versions: HashMap<String, Vec<crate::providers::daemon::RetroRomVersion>>,
    /// Grouped game total RomM reported for the last loaded scope. Overrides
    /// the per-platform `rom_count` in the Console Games header, which counts
    /// files rather than collapsed games. Cleared on a fresh scope reload.
    romm_grouped_total: Option<u64>,
    /// Next RomM page offset for the active console scope. `None` means the
    /// current scope has no further pages to fetch.
    romm_next_offset: Option<u32>,
    /// Whether a RomM page request is in flight, so scrolling and the periodic
    /// platform refresh cannot queue duplicate pages.
    romm_loading_page: bool,
    /// The console-game details screen, when one is open. `None` on every other
    /// page; which page is showing lives in [`HearthDeck::page`].
    details: Option<Details>,
    virtual_keyboard: VirtualKeyboard,
    /// In-flight Dashboard<->Library fade-through transition, if any. Bounded
    /// by [`PAGE_TRANSITION_DURATION`] and cleared by
    /// [`HearthDeck::expire_page_animation`].
    page_animation: Option<PageAnimation>,
    /// In-flight tab slide, if any. Bounded by [`TAB_TRANSITION_DURATION`] and
    /// cleared by [`HearthDeck::advance_tab_animation`].
    tab_animation: Option<TabAnimation>,
    /// Timestamp of the current frame, refreshed by the `window::frames()`
    /// subscription while a transition is in flight (the application's `update`
    /// does not receive it the way raw iced's does).
    now: Instant,
}

impl Default for HearthDeck {
    fn default() -> Self {
        Self {
            page: Page::Dashboard,
            search_value: Default::default(),
            entry_path_input: Default::default(),
            all_entries: Default::default(),
            recent_entries: Default::default(),
            menu: Default::default(),
            menu_selection: Default::default(),
            helper: Default::default(),
            config: Default::default(),
            cur_section: Section::PcGames,
            cur_group: Default::default(),
            locale: Default::default(),
            edit_name: Default::default(),
            new_group: Default::default(),
            dnd_icon: Default::default(),
            offer_group: Default::default(),
            waiting_for_filtered: Default::default(),
            scroll_offset: Default::default(),
            viewport_height: Default::default(),
            dashboard_rail_scroll: Default::default(),
            window_width: WINDOW_WIDTH,
            core: Default::default(),
            group_to_delete: Default::default(),
            duplicates: Default::default(),
            app_list_config: Default::default(),
            focused_id: Default::default(),
            entry_ids: Default::default(),
            entry_icon_handles: Default::default(),
            group_keys: Default::default(),
            next_group_key: Default::default(),
            gamepad_focus_first: Default::default(),
            provider_service: None,
            daemon_client: None,
            launch_state: LaunchState::default(),
            input_ownership: InputOwnership::default(),
            system_status: SystemStatus::default(),
            toasts: Toasts::new(Message::DismissToast),
            backend_available: None,
            degraded_providers: Vec::new(),
            dashboard_health_generation: 0,
            romm_request_generation: 0,
            romm_platform_icons: HashMap::new(),
            romm_platform_counts: HashMap::new(),
            romm_versions: HashMap::new(),
            romm_grouped_total: None,
            romm_next_offset: None,
            romm_loading_page: false,
            details: None,
            virtual_keyboard: VirtualKeyboard::default(),
            page_animation: None,
            tab_animation: None,
            now: Instant::now(),
        }
    }
}

impl HearthDeck {
    /// Update entry IDs and their icon handles.
    fn update_entry_metadata(&mut self) {
        self.rebuild_entry_ids();

        self.entry_icon_handles = self
            .entry_path_input
            .iter()
            .map(|e| {
                crate::icon_cache::entry_icon_handle(&e.icon, tile_width(self.window_width) as u32)
            })
            .collect();
    }

    /// Rebuild the stable named entry IDs (see `update_entry_metadata`).
    fn rebuild_entry_ids(&mut self) {
        // Use stable named ids derived from the entry path. iced's widget tree
        // diff preserves the id of a widget occupying the same position across
        // rebuilds (and thereby breaks `focus(id)` when the entry set changes,
        // e.g. on a group switch). Named ids are matched by name instead: when
        // a new entry appears, its tree node is recreated with the fresh id,
        // so focus operations can find it.
        self.entry_ids = self
            .entry_path_input
            .iter()
            .map(|e| {
                widget::Id::from(
                    e.path
                        .as_deref()
                        .map(|p| format!("app-entry-{}", p.to_string_lossy()))
                        .unwrap_or_else(|| format!("app-entry-{}", e.id)),
                )
            })
            .collect();
    }

    /// A task that scrolls the single-line tab strip so the currently
    /// selected group tab is visible. When no custom group is selected the
    /// strip returns to the leading "all apps" tab.
    fn reveal_tab_strip(&self) -> Option<Task<Message>> {
        if self.page != Page::Library {
            return None;
        }
        match self.cur_group {
            None => Some(iced::widget::scrollable::scroll_to(
                TAB_STRIP_SCROLLABLE_ID.clone(),
                AbsoluteOffset {
                    x: Some(0.0),
                    y: None,
                },
            )),
            Some(index) => {
                let key = self.group_keys.get(index).copied()?;
                Some(
                    iced_runtime::task::widget(FindTabStripReveal {
                        scrollable_id: TAB_STRIP_SCROLLABLE_ID.clone(),
                        tab_id: group_tab_id(key),
                        viewport: None,
                        content_width: None,
                        translation_x: None,
                        tab: None,
                    })
                    .map(|offset| cosmic::Action::App(Message::ScrollTabStrip(offset))),
                )
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Dashboard,
    Library,
    /// One console game's details. Not part of the primary navigation: it is
    /// pushed on top of the Library by opening a game, and Back returns there.
    Details,
}

/// State for the Dashboard<->Library fade-through transition. The pages
/// themselves are rebuilt from `self`, so only the origin and start time need
/// to be remembered.
#[derive(Clone, Copy, Debug)]
struct PageAnimation {
    from: Page,
    started_at: Instant,
}

/// State for a tab (group) change: the target is applied under the transition's
/// cover, once the outgoing grid has slid off-stage.
#[derive(Clone, Copy, Debug)]
struct TabAnimation {
    target: Option<usize>,
    started_at: Instant,
    /// `+1.0` when the incoming tab sits to the right of the outgoing one.
    direction: f32,
    /// Whether the pending `target` has been applied yet.
    swapped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DashboardShelf {
    Recent,
    Favorites,
    Library,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DashboardHealthError {
    /// The daemon could not be reached at all (not running, wrong address, or a
    /// rejected pairing token).
    Unreachable,
    /// The daemon answered, but with an error status or an API error message.
    Service(String),
    /// The daemon answered with a body we could not parse, which usually means
    /// the frontend and daemon versions disagree.
    Protocol(String),
}

/// Splits the daemon's error types into the three cases a user can act on,
/// keeping the raw `reqwest` detail out of the alert text.
fn dashboard_health_error(error: crate::providers::daemon::DaemonError) -> DashboardHealthError {
    use crate::providers::daemon::DaemonError;
    match error {
        DaemonError::Api { message, .. } => DashboardHealthError::Service(message),
        DaemonError::UnexpectedStatus(status) => DashboardHealthError::Service(status.to_string()),
        DaemonError::Deserialization(error) => DashboardHealthError::Protocol(error.to_string()),
        DaemonError::Connection(_)
        | DaemonError::InvalidBaseUrl(_)
        | DaemonError::InvalidToken(_)
        | DaemonError::PairingRequiresLoopback
        | DaemonError::Unavailable => DashboardHealthError::Unreachable,
    }
}

/// Provider ids the daemon reports as degraded: discovery failed, but the
/// source is still registered rather than removed.
fn degraded_providers(health: &crate::providers::daemon::HealthResponse) -> Vec<String> {
    health
        .providers
        .iter()
        .filter(|provider| provider.status == "degraded")
        .map(|provider| provider.id.clone())
        .collect()
}

impl DashboardShelf {
    fn key(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Favorites => "favorites",
            Self::Library => "library",
        }
    }

    fn title(self) -> String {
        match self {
            Self::Recent => fl!("recently-played"),
            Self::Favorites => fl!("favorites"),
            Self::Library => "Games & apps".to_owned(),
        }
    }

    fn empty_message(self) -> String {
        match self {
            Self::Recent => fl!("no-recently-played"),
            Self::Favorites => fl!("no-favorites"),
            Self::Library => fl!("library-empty"),
        }
    }

    fn widget_id(self, entry_id: &str) -> widget::Id {
        widget::Id::from(format!("dashboard-{}-{entry_id}", self.key()))
    }
}

/// Which control the details screen's controller cursor is on.
///
/// The screen is a small, fixed set of rows rather than a free grid: the action
/// line along the bottom, and the screenshot strip above it. Movement is derived
/// from this value, not from iced's focus state, so Confirm always acts on the
/// highlighted control even if the renderer has not reported focus yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DetailsFocus {
    #[default]
    Play,
    Favorite,
    /// Opens the disc picker. Only present for a game RomM collapsed out of
    /// several files.
    Disc,
    PlayLater,
    /// Leaves the screen. Sits in the action line so that line holds every
    /// control the screen has, instead of a lone back button elsewhere.
    Back,
    /// One of the game's screenshots, which becomes the backdrop image when
    /// picked.
    Screenshot(usize),
    /// One row of the disc picker, whose rows replace the page's while it is
    /// open.
    DiscChoice(usize),
}

/// The details screen's action line, left to right. Disc selection only exists
/// for a game with more than one file.
fn details_actions(has_discs: bool) -> &'static [DetailsFocus] {
    const MULTI_FILE: &[DetailsFocus] = &[
        DetailsFocus::Back,
        DetailsFocus::Play,
        DetailsFocus::Favorite,
        DetailsFocus::Disc,
        DetailsFocus::PlayLater,
    ];
    const SINGLE_FILE: &[DetailsFocus] = &[
        DetailsFocus::Back,
        DetailsFocus::Play,
        DetailsFocus::Favorite,
        DetailsFocus::PlayLater,
    ];
    if has_discs { MULTI_FILE } else { SINGLE_FILE }
}

/// Widget id for a details-screen focus target. Stable for the page's lifetime,
/// which is what lets both `focus()` (the accent ring) and [`DetailsFocus`]
/// (the action Confirm takes) name the same control.
fn details_focus_id(focus: DetailsFocus) -> Id {
    match focus {
        DetailsFocus::Play => Id::new("details-play"),
        DetailsFocus::Favorite => Id::new("details-favorite"),
        DetailsFocus::Disc => Id::new("details-disc"),
        DetailsFocus::PlayLater => Id::new("details-play-later"),
        DetailsFocus::Back => Id::new("details-back"),
        DetailsFocus::Screenshot(index) => Id::new(format!("details-screenshot-{index}")),
        DetailsFocus::DiscChoice(index) => Id::new(format!("details-disc-{index}")),
    }
}

/// Which RomM per-user flag a write answer settles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DetailsStateKind {
    Favorite,
    Backlog,
}

/// State of the console-game details screen.
///
/// Seeded from the grid tile that opened it — the title, cover and version list
/// are already in memory — so the screen never opens blank, then filled in by
/// the daemon's per-game read. `info` staying `None` is a normal, temporary
/// state: the seed alone is a complete screen for the fields it carries.
struct Details {
    /// RomM id of the file being shown: the game's representative, or the disc
    /// the user picked from the picker. Everything the buttons act on — launch,
    /// favorite, play later — is scoped to this id.
    rom_id: i64,
    title: String,
    /// Cover handle recycled from the grid tile, shown until (or instead of) a
    /// large cover arriving from the daemon.
    cover: Option<icon::Handle>,
    /// Large cover for the hero, once it has been downloaded.
    hero: Option<icon::Handle>,
    /// Screenshot thumbnails, in RomM's order.
    screenshots: Vec<icon::Handle>,
    /// Screenshot the user picked to show as the backdrop, if any.
    hero_shot: Option<usize>,
    /// Every version of the game, the one Play targets first. Only populated
    /// for games RomM collapsed from several files; empty means single-file.
    versions: Vec<RetroRomVersion>,
    /// Whether the game is in the user's RomM favorites. `None` means the screen
    /// does not know yet (nothing has been read or written for this file), which
    /// the buttons treat as "off" and a press turns into "add".
    favorite: Option<bool>,
    /// RomM's per-user "play later" flag, same unknown-state handling.
    backlogged: Option<bool>,
    /// A favorite or play-later write is in flight: stops a second press from
    /// racing the first.
    state_pending: bool,
    /// The disc picker overlay is open.
    disc_picker: bool,
    /// The daemon's detail read. `None` while it is still in flight.
    info: Option<RetroGameDetails>,
    /// Failure text from that read, shown in place of the detail rows.
    error: Option<String>,
    /// The cursor.
    focus: DetailsFocus,
    /// Grid index the screen was opened from, so Back can put the cursor back
    /// on the tile the user came from.
    origin_index: usize,
}

impl Details {
    /// Poster art for the card: the large cover, otherwise the grid tile's own
    /// cover. Always the cover, so the card stays the game's identity whatever
    /// the backdrop is previewing.
    fn cover_art(&self) -> Option<&icon::Handle> {
        self.hero.as_ref().or(self.cover.as_ref())
    }

    /// Art for the full-bleed backdrop: the screenshot being previewed, or the
    /// game's first. Screenshots are 16:9 and survive being scaled to the
    /// window; `None` when the game has none, which leaves the page a flat
    /// surface rather than a stretched, blurry poster.
    fn backdrop_art(&self) -> Option<&icon::Handle> {
        match self.hero_shot {
            Some(index) => self.screenshots.get(index),
            None => self.screenshots.first(),
        }
    }

    /// The version Play launches, when the game has more than one file. `None`
    /// for single-file games, where the label would only repeat the title.
    fn current_version_label(&self) -> Option<&str> {
        if self.versions.len() < 2 {
            return None;
        }
        self.versions
            .iter()
            .find(|version| version.id == self.rom_id)
            .map(|version| version.title.as_str())
    }

    /// Short labels shown under the title: release year, platform, community
    /// score, genres, player count and age rating. Empty fields are skipped so
    /// a sparse game does not leave blank chips.
    fn chips(&self) -> Vec<String> {
        let Some(info) = self.info.as_ref() else {
            return Vec::new();
        };
        let mut chips = Vec::new();
        if let Some(year) = info.release_year {
            chips.push(year.to_string());
        }
        if let Some(platform) = info.platform_name.as_deref() {
            chips.push(platform.to_owned());
        }
        if let Some(rating) = info.average_rating {
            chips.push(fl!("community-rating", value = format!("{rating:.0}")));
        }
        chips.extend(info.genres.iter().cloned());
        if let Some(players) = info.player_count.as_deref() {
            chips.push(fl!("players-value", count = players));
        }
        chips.extend(info.age_ratings.iter().cloned());
        // Which file Play launches, when the game has more than one: the disc
        // picker sets it, and without it named here the choice would be
        // invisible on the screen itself.
        if let Some(label) = self.current_version_label() {
            chips.push(label.to_owned());
        }
        chips
    }

    /// The labelled fact rows below the actions: who made it, how it plays,
    /// when it shipped, and the file itself. Anything RomM did not send is
    /// omitted rather than shown empty.
    fn facts(&self) -> Vec<(String, String)> {
        let Some(info) = self.info.as_ref() else {
            return Vec::new();
        };
        let mut facts: Vec<(String, String)> = Vec::new();
        let mut push = |label: String, value: String| {
            if !value.is_empty() {
                facts.push((label, value));
            }
        };
        // RomM merges studios and publishers into one list, so the label does
        // not claim to be the developer specifically.
        push(fl!("companies"), info.companies.join(", "));
        push(fl!("play-modes"), info.game_modes.join(", "));
        push(
            fl!("players"),
            info.player_count.clone().unwrap_or_default(),
        );
        push(
            fl!("released"),
            info.release_date.clone().unwrap_or_default(),
        );
        // RomM's own region metadata is often empty where the region tag parsed
        // from the file name is not, so fall back to the tags.
        let regions = if info.regions.is_empty() {
            &info.tags
        } else {
            &info.regions
        };
        push(fl!("regions"), regions.join(", "));
        push(fl!("languages"), info.languages.join(", "));
        push(
            fl!("file-size"),
            info.file_size_bytes.map(human_size).unwrap_or_default(),
        );
        facts
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum GroupRowKey {
    AllApps,
    Custom(u64),
    NewGroup,
}

#[derive(Clone, Debug)]
enum Message {
    ProviderRecords(Vec<crate::providers::GameRecord>),
    UpdateFocused(Option<widget::Id>),
    InputChanged(String),
    KeyboardNav(keyboard_nav::Action),
    PrevRow,
    NextRow,
    PrevCol,
    NextCol,
    GamepadEvent(GamepadEvent),
    FocusGridFirst,
    OpenDashboard,
    OpenLibrary,
    /// Open the Library on a console's tab. The payload is the console's
    /// index within the Console Games tab strip.
    OpenConsole(usize),
    OpenSearch,
    /// Open the details screen for one entry of the current grid. Only RomM
    /// console games have one, so the caller checks the entry kind first.
    OpenDetails(usize),
    /// Leave the details screen and return to the grid it was opened from.
    CloseDetails,
    /// Show one of the game's screenshots as the details screen's hero image.
    DetailsSelectScreenshot(usize),
    /// Add or remove the current game from the user's RomM favorites.
    DetailsToggleFavorite,
    /// Set or clear RomM's "play later" flag for the current game.
    DetailsToggleBacklog,
    /// Open or close the disc picker.
    DetailsToggleDiscPicker,
    /// Point the details screen at another file of the same game, by RomM id.
    DetailsSelectDisc(i64),
    /// Answer to a favorite write. Carries the state to fall back to, so a
    /// failed write does not leave the button claiming it worked.
    DetailsFavorite {
        rom_id: i64,
        previous: Option<bool>,
        result: Result<bool, String>,
    },
    /// Answer to a play-later write, as above.
    DetailsBacklog {
        rom_id: i64,
        previous: Option<bool>,
        result: Result<bool, String>,
    },
    Close,
    ActivateApp(usize),
    ActivateDashboardApp(String),
    ActivateFirstApp,
    ConfirmFocused(widget::Id),
    SelectSection(Section),
    SelectGroup(Option<usize>),
    ToggleFilterMenu,
    ReorderGroup(Vec<GroupRowKey>),
    Delete(usize),
    ConfirmDelete,
    CancelDelete,
    StartEditName(String),
    EditName(String),
    SubmitName,
    StartNewGroup,
    NewGroup(String),
    SubmitNewGroup,
    CancelNewGroup,
    FilterApps(
        String,
        Vec<Arc<DesktopEntryData>>,
        Vec<widget::icon::Handle>,
    ),
    OpenContextMenu(Rectangle, usize),
    CloseContextMenu,
    /// Confirm the highlighted entry of an open context menu (keyboard Enter).
    MenuConfirm,
    SelectAction(MenuAction),
    StartDrag(usize),
    FinishDrag(bool),
    CancelDrag,
    StartDndOffer(Option<usize>),
    FinishDndOffer(Option<usize>, Option<DesktopEntryData>),
    LeaveDndOffer(Option<usize>),
    ScrollYOffset(f32, f32),
    ViewportHeight(f32),
    /// A dashboard rail's live scroll viewport, keyed by the rail's stable
    /// key. `offset` is the horizontal scroll position and `viewport` the
    /// visible width, both needed to keep the selected card in view without
    /// pinning it to an edge.
    DashboardRailScrolled {
        key: &'static str,
        offset: f32,
        viewport: f32,
    },
    /// Absolute horizontal scroll offset that keeps the selected tab visible
    /// inside the single-line tab strip.
    ScrollTabStrip(f32),
    PinToAppTray(usize),
    UnPinFromAppTray(usize),
    ToggleFavorite(usize),
    AppListConfig(AppListConfig),
    Opened,
    WindowFocusChanged(bool),
    WindowResized(f32),
    DaemonLaunchResult(Result<(), String>),
    /// Launch a specific RomM version (another disc/region/revision) chosen
    /// from the context menu, by the daemon's RomM rom ID.
    ActivateRomVariant {
        rom_id: i64,
        title: String,
    },
    ActiveSessionResult(Result<bool, String>),
    RommPlatforms(Result<Vec<crate::providers::daemon::RetroConsole>, String>),
    RecentEntries(Result<Vec<crate::providers::GameRecord>, String>),
    DashboardHealth {
        generation: u64,
        result: Result<crate::providers::daemon::HealthResponse, DashboardHealthError>,
    },
    /// The user pressed Retry on an alert: dismiss it and re-run the failed
    /// operation (health check, or a provider rescan when sources degraded).
    RetryDashboardAlert(ToastId),
    /// The user closed an alert, or its display duration elapsed.
    DismissToast(ToastId),
    DashboardRescanResult(Result<(), String>),
    SystemStatus(SystemStatus),
    RommGames {
        generation: u64,
        platform_id: Option<i64>,
        result: Result<crate::providers::daemon::RetroRecordPage, String>,
    },
    /// One game's detail read, answering the details screen that asked for it.
    /// Boxed because the payload dwarfs every other message variant.
    DetailsLoaded {
        rom_id: i64,
        result: Result<Box<RetroDetails>, String>,
    },
    DismissLaunch,
    /// Redraw tick for an in-flight page transition, carrying the frame time.
    Animate(Instant),
    VirtualKeyboardToggled {
        target: bool,
        result: Result<(), String>,
    },
}

#[derive(Default)]
struct VirtualKeyboard {
    visible: bool,
    requested_visible: bool,
    pending_target: Option<bool>,
}

impl VirtualKeyboard {
    fn request(&mut self, visible: bool) -> Option<bool> {
        self.requested_visible = visible;
        self.next_toggle()
    }

    fn complete(&mut self, target: bool, succeeded: bool) -> Option<bool> {
        if self.pending_target != Some(target) {
            return None;
        }
        self.pending_target = None;
        if !succeeded {
            return None;
        }
        self.visible = target;
        self.next_toggle()
    }

    fn did_dismiss_externally(&mut self) {
        self.visible = false;
        self.requested_visible = false;
        self.pending_target = None;
    }

    fn next_toggle(&mut self) -> Option<bool> {
        if self.pending_target.is_some() || self.visible == self.requested_visible {
            return None;
        }
        self.pending_target = Some(self.requested_visible);
        self.pending_target
    }
}

#[derive(Clone, Debug)]
enum MenuAction {
    ToggleControllerCompatibility,
    Remove,
}

pub fn menu_button<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
) -> cosmic::widget::Button<'a, Message> {
    cosmic::widget::button::custom(content)
        .class(Button::MenuItem)
        .padding(menu_control_padding())
        .width(Length::Fill)
}

pub fn menu_control_padding() -> Padding {
    let theme = cosmic::theme::active();
    let cosmic = theme.cosmic();
    [cosmic.space_xxs(), cosmic.space_m()].into()
}

impl HearthDeck {
    fn request_virtual_keyboard(&mut self, visible: bool) -> Task<Message> {
        let Some(target) = self.virtual_keyboard.request(visible) else {
            return Task::none();
        };
        Self::toggle_virtual_keyboard(target)
    }

    fn toggle_virtual_keyboard(target: bool) -> Task<Message> {
        Task::perform(
            async move {
                let result = tokio::process::Command::new("gamepad-osk")
                    .arg("--toggle")
                    .status()
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|status| {
                        status
                            .success()
                            .then_some(())
                            .ok_or_else(|| format!("gamepad-osk exited with {status}"))
                    });
                (target, result)
            },
            |(target, result)| {
                cosmic::Action::App(Message::VirtualKeyboardToggled { target, result })
            },
        )
    }

    fn focus_text_input(&mut self, id: widget::Id) -> Task<Message> {
        self.focused_id = Some(id.clone());
        Task::batch([text_input::focus(id), self.request_virtual_keyboard(true)])
    }

    fn load_romm_platforms(&self, delay: std::time::Duration) -> Task<Message> {
        let Some(client) = self.daemon_client.clone() else {
            return Task::none();
        };
        Task::perform(
            async move {
                tokio::time::sleep(delay).await;
                client
                    .list_retro_consoles()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::RommPlatforms(result)),
        )
    }

    fn current_romm_platform_id(&self) -> Option<Option<i64>> {
        selected_romm_platform_id(self.cur_section, self.cur_group, &self.config)
    }

    fn load_romm_page(&mut self, offset: u32) -> Task<Message> {
        let Some(platform_id) = self.current_romm_platform_id() else {
            return Task::none();
        };
        let Some(client) = self.daemon_client.clone() else {
            return Task::none();
        };
        // Only one page request may be in flight. Reloading from the start
        // (offset 0) is always allowed: it bumps the generation so any older
        // in-flight page is discarded when it lands.
        if offset > 0 && self.romm_loading_page {
            return Task::none();
        }
        if offset == 0 {
            self.romm_request_generation += 1;
            self.all_entries
                .retain(|entry| !entry.id.starts_with("romm:"));
            self.romm_versions.clear();
            self.romm_grouped_total = None;
            self.romm_next_offset = None;
            self.load_apps();
        }
        self.romm_loading_page = true;
        let generation = self.romm_request_generation;
        Task::perform(
            async move {
                client
                    .list_retro_records(platform_id, ROMM_PAGE_SIZE, offset)
                    .await
                    .map_err(|error| error.to_string())
            },
            move |result| {
                cosmic::Action::App(Message::RommGames {
                    generation,
                    platform_id,
                    result,
                })
            },
        )
    }

    fn poll_active_session(&self, delay: std::time::Duration) -> Task<Message> {
        let Some(client) = self.daemon_client.clone() else {
            return Task::none();
        };
        Task::perform(
            async move {
                tokio::time::sleep(delay).await;
                client
                    .active_session()
                    .await
                    .map(|session| session.is_some())
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::ActiveSessionResult(result)),
        )
    }

    fn load_system_status(delay: std::time::Duration) -> Task<Message> {
        Task::perform(
            async move {
                tokio::time::sleep(delay).await;
                SystemStatus::load().await
            },
            |status| cosmic::Action::App(Message::SystemStatus(status)),
        )
    }

    fn load_dashboard_health(&self, delay: std::time::Duration) -> Task<Message> {
        let Some(client) = self.daemon_client.clone() else {
            return Task::none();
        };
        let generation = self.dashboard_health_generation;
        Task::perform(
            async move {
                tokio::time::sleep(delay).await;
                client.health().await.map_err(|error| {
                    log::debug!("dashboard health is not available: {error}");
                    dashboard_health_error(error)
                })
            },
            move |result| cosmic::Action::App(Message::DashboardHealth { generation, result }),
        )
    }

    /// Shows a transient alert through libcosmic's toaster. When `retry` is set
    /// the alert carries a Retry action wired to [`Message::RetryDashboardAlert`].
    fn push_alert(&mut self, message: String, retry: bool) -> Task<Message> {
        let mut toast = Toast::new(message).duration(std::time::Duration::from_secs(20));
        if retry {
            toast = toast.action(fl!("retry"), Message::RetryDashboardAlert);
        }
        self.toasts.push(toast).map(cosmic::Action::App)
    }

    fn retry_dashboard_alert(&mut self, id: ToastId) -> Task<Message> {
        self.toasts.remove(id);
        let Some(client) = self.daemon_client.clone() else {
            return Task::none();
        };
        let rescan = !self.degraded_providers.is_empty();
        // Forget the memoized state so a running Retry that doesn't fix the
        // problem alerts again instead of silently doing nothing.
        self.backend_available = None;
        self.degraded_providers.clear();
        if !rescan {
            self.dashboard_health_generation += 1;
            return self.load_dashboard_health(std::time::Duration::ZERO);
        }
        Task::perform(
            async move {
                client
                    .rescan_library()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::DashboardRescanResult(result)),
        )
    }

    fn load_recent_activity(&self, delay: std::time::Duration) -> Task<Message> {
        let Some(client) = self.daemon_client.clone() else {
            return Task::none();
        };
        Task::perform(
            async move {
                tokio::time::sleep(delay).await;
                client
                    .recent_records(DASHBOARD_RAIL_TILES as u32)
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::RecentEntries(result)),
        )
    }

    fn sync_category_groups(&mut self) {
        let selected_group = self
            .cur_group
            .and_then(|index| self.config.sections.get(self.cur_section).get(index))
            .cloned();
        if !self.config.sync_category_groups(&self.all_entries) {
            return;
        }

        self.cur_group = selected_group.and_then(|selected| {
            self.config
                .sections
                .get(self.cur_section)
                .iter()
                .position(|group| group == &selected)
        });
        self.group_keys = (0..self.config.sections.get(self.cur_section).len())
            .map(|_| {
                let key = self.next_group_key;
                self.next_group_key += 1;
                key
            })
            .collect();
    }

    fn current_group(&self) -> &AppGroup {
        match self.cur_group {
            None => AppLibraryConfig::home(),
            Some(i) => &self.config.sections.get(self.cur_section)[i],
        }
    }

    /// The game total the Console Games header should show for the current
    /// scope, from RomM's own console metadata rather than the number of pages
    /// lazy-loaded so far. A selected console uses its `rom_count`; the "all
    /// consoles" tab sums every platform. Returns `None` when the console
    /// totals are not known yet (before the first consoles response), so the
    /// caller can fall back to the loaded count.
    ///
    /// The grouped total from the list endpoint wins when known: RomM's
    /// per-platform `rom_count` counts every file, so it overstates a library
    /// whose regions/revisions/discs are collapsed into single tiles.
    fn console_total_count(&self) -> Option<usize> {
        if let Some(total) = self.romm_grouped_total {
            return Some(total as usize);
        }
        match self.cur_group {
            Some(index) => self
                .config
                .sections
                .console_games
                .get(index)
                .and_then(AppGroup::romm_platform_id)
                .and_then(|id| self.romm_platform_counts.get(&id))
                .map(|count| *count as usize),
            None => (!self.romm_platform_counts.is_empty()).then(|| {
                self.romm_platform_counts
                    .values()
                    .map(|count| *count as usize)
                    .sum()
            }),
        }
    }

    pub fn load_apps(&mut self) {
        // The daemon owns discovery; this only projects its catalog records
        // into the currently selected section and group.

        self.entry_path_input = self.config.filtered(
            self.cur_section,
            self.cur_group,
            &self.search_value,
            &self.all_entries,
        );

        // collect duplicates
        self.duplicates.clear();
        self.duplicates = self
            .all_entries
            .iter()
            .enumerate()
            .fold(
                (std::mem::take(&mut self.duplicates), 0, "", ""),
                |(mut dups, cur_count, cur_name, cur_id): (HashMap<_, _>, usize, &str, &str),
                 (i, e)| {
                    if cur_name.to_lowercase().trim() == e.name.to_lowercase().trim()
                        || e.id == cur_id
                    {
                        if cur_count == 1 {
                            // insert previous entry
                            if let Some(path) = self.all_entries[i - 1].path.as_ref() {
                                let source = AppSource::from(path.as_ref());
                                let icon_handle = source.as_icon();
                                dups.insert(path.clone(), (source, icon_handle));
                            }
                        }
                        if let Some(path) = e.path.as_ref() {
                            let source = AppSource::from(path.as_ref());
                            let icon_handle = source.as_icon();
                            dups.insert(path.clone(), (source, icon_handle));
                        }
                        (dups, cur_count + 1, cur_name, cur_id)
                    } else {
                        (dups, 1, e.name.as_str(), e.id.as_str())
                    }
                },
            )
            .0;
        self.update_entry_metadata();
    }

    fn filter_apps(&mut self) -> Task<Message> {
        let config = self.config.clone();
        let all_entries = self.all_entries.clone();
        let cur_section = self.cur_section;
        let cur_group = self.cur_group;
        let input = self.search_value.clone();
        let prerender = tile_width(self.window_width) as u32;
        if !self.waiting_for_filtered {
            self.waiting_for_filtered = true;
            iced::Task::perform(
                async move {
                    let mut apps = config.filtered(cur_section, cur_group, &input, &all_entries);
                    apps.sort_by(|a, b| a.name.cmp(&b.name));
                    let icon_handles = apps
                        .iter()
                        .map(|e| crate::icon_cache::entry_icon_handle(&e.icon, prerender))
                        .collect::<Vec<_>>();
                    (input, apps, icon_handles)
                },
                |(input, apps, icon_handles)| Message::FilterApps(input, apps, icon_handles),
            )
            .map(cosmic::Action::App)
        } else {
            iced::Task::none()
        }
    }

    /// Switches the visible page, starting a fade-through transition when the
    /// page actually changes. Re-selecting the current page snaps instead of
    /// fading to the same place.
    fn switch_page(&mut self, to: Page) {
        if self.page == to {
            self.page_animation = None;
            return;
        }
        let from = self.page;
        self.page = to;
        // Take the frame clock once so the first drawn frame is at progress 0.
        self.now = Instant::now();
        self.page_animation = Some(PageAnimation {
            from,
            started_at: self.now,
        });
    }

    /// Eased `0.0..=1.0` progress of `animation` at the current frame.
    fn transition_progress(&self, animation: &PageAnimation) -> f32 {
        let elapsed = self.now.saturating_duration_since(animation.started_at);
        let raw = (elapsed.as_secs_f32() / PAGE_TRANSITION_DURATION.as_secs_f32()).clamp(0.0, 1.0);
        cosmic::anim::smootherstep(raw)
    }

    /// Drops a transition that has run its course. Called on every message so a
    /// transition can never outlive [`PAGE_TRANSITION_DURATION`], even if a redraw
    /// tick is missed.
    ///
    /// This is also what retires the details screen. Leaving it keeps `details`
    /// alive for the length of the fade, because the outgoing page is still
    /// drawn under the transition: clearing it when Back is pressed would blank
    /// the screen instead of fading it out.
    fn expire_page_animation(&mut self) {
        let Some(animation) = self.page_animation else {
            return;
        };
        if Instant::now().saturating_duration_since(animation.started_at) < PAGE_TRANSITION_DURATION
        {
            return;
        }
        self.page_animation = None;
        if self.page != Page::Details {
            self.details = None;
        }
    }

    /// Applies a tab (group) selection: reset the view state, reload the grid and
    /// move focus. Called on a tick, underneath the slide's opaque cover.
    fn apply_group(&mut self, group: Option<usize>) -> Task<Message> {
        self.edit_name = None;
        self.search_value.clear();
        self.cur_group = group;
        self.scroll_offset = 0.0;
        let load = if self.cur_section == Section::ConsoleGames {
            self.load_romm_page(0)
        } else {
            self.filter_apps()
        };
        let mut cmds = vec![
            load,
            iced::widget::scrollable::scroll_to(
                SCROLLABLE_ID.clone(),
                AbsoluteOffset {
                    x: Some(0.0),
                    y: Some(0.0),
                },
            ),
        ];
        if group.is_none() && !self.gamepad_focus_first {
            cmds.push(self.focus_text_input(SEARCH_ID.clone()));
        } else {
            cmds.push(self.request_virtual_keyboard(false));
        }
        if let Some(task) = self.reveal_tab_strip() {
            cmds.push(task);
        }
        iced::Task::batch(cmds)
    }

    /// Starts the directional slide for a tab change. The grid itself is only
    /// swapped once the cover is opaque (see [`HearthDeck::advance_tab_animation`]),
    /// so the outgoing content slides off before the incoming one slides on.
    fn begin_tab_animation(&mut self, target: Option<usize>) {
        let from = tab_position(self.cur_group);
        let to = tab_position(target);
        self.now = Instant::now();
        self.tab_animation = Some(TabAnimation {
            target,
            started_at: self.now,
            direction: if to >= from { 1.0 } else { -1.0 },
            swapped: false,
        });
    }

    /// Eased `0.0..=1.0` progress of the tab slide, if one is in flight.
    fn tab_progress(&self) -> Option<f32> {
        let animation = self.tab_animation?;
        let elapsed = self.now.saturating_duration_since(animation.started_at);
        let raw = (elapsed.as_secs_f32() / TAB_TRANSITION_DURATION.as_secs_f32()).clamp(0.0, 1.0);
        Some(cosmic::anim::smootherstep(raw))
    }

    /// Drives the tab slide: apply the pending tab under cover at the midpoint,
    /// then drop the animation once it has run its course.
    fn advance_tab_animation(&mut self) -> Task<Message> {
        let Some(animation) = self.tab_animation else {
            return Task::none();
        };
        let Some(progress) = self.tab_progress() else {
            return Task::none();
        };

        if progress >= 1.0 {
            self.tab_animation = None;
            return Task::none();
        }

        if progress >= 0.5 && !animation.swapped {
            if let Some(animation) = self.tab_animation.as_mut() {
                animation.swapped = true;
            }
            return self.apply_group(animation.target);
        }

        Task::none()
    }

    pub fn close(&mut self) -> Task<Message> {
        // cancel existing dnd if it exists then try again...
        if self.dnd_icon.take().is_some() {
            return Task::batch(vec![
                end_dnd(),
                Task::perform(async {}, |_| cosmic::Action::App(Message::Close)),
            ]);
        }
        self.focused_id = None;
        self.entry_ids.clear();
        self.entry_icon_handles.clear();
        self.new_group = None;
        self.search_value.clear();
        self.edit_name = None;
        self.cur_group = None;
        self.menu = None;
        self.details = None;
        self.group_to_delete = None;
        self.scroll_offset = 0.0;
        let keyboard = self.request_virtual_keyboard(false);

        iced::Task::batch(vec![
            keyboard,
            destroy_popup(*MENU_ID),
            destroy_layer_surface(*NEW_GROUP_WINDOW_ID),
            destroy_layer_surface(*DELETE_GROUP_WINDOW_ID),
            window::close(window::Id::RESERVED),
        ])
    }

    fn activate_app(&mut self, i: usize) -> Task<<Self as cosmic::Application>::Message> {
        let entry = self.entry_path_input.get(i).cloned();
        self.activate_entry(entry)
    }

    fn activate_dashboard_app(&mut self, id: &str) -> Task<<Self as cosmic::Application>::Message> {
        let entry = self
            .all_entries
            .iter()
            .chain(&self.recent_entries)
            .find(|entry| entry.id == id)
            .cloned();
        self.activate_entry(entry)
    }

    fn dashboard_shelves(&self) -> Vec<(DashboardShelf, Vec<&Arc<DesktopEntryData>>)> {
        // "Recently Played" is a games shelf: launch activity for plain
        // applications is filtered out here, matching how the catalog already
        // splits games from apps everywhere else.
        let recent = self
            .recent_entries
            .iter()
            .filter(|entry| {
                entry
                    .categories
                    .iter()
                    .any(|category| category.eq_ignore_ascii_case("game"))
            })
            .take(DASHBOARD_RAIL_TILES)
            .collect::<Vec<_>>();
        let favorites = self
            .config
            .favorite_entries(&self.all_entries)
            .into_iter()
            .take(DASHBOARD_RAIL_TILES)
            .collect::<Vec<_>>();
        let show_library = recent.is_empty() && favorites.is_empty();
        let mut shelves = vec![
            (DashboardShelf::Recent, recent),
            (DashboardShelf::Favorites, favorites),
        ];
        if show_library {
            shelves.push((
                DashboardShelf::Library,
                self.all_entries.iter().take(DASHBOARD_RAIL_TILES).collect(),
            ));
        }
        shelves
    }

    fn activate_entry(
        &mut self,
        entry: Option<Arc<DesktopEntryData>>,
    ) -> Task<<Self as cosmic::Application>::Message> {
        let Some(de) = entry else {
            if self.input_ownership.frontend_has_control() {
                self.edit_name = None;
            }
            return Task::none();
        };
        let Some(target) = managed_launch_target(&de.id) else {
            error!("refusing unmanaged application launch: {}", de.id);
            return Task::none();
        };
        self.launch_managed(target, de.id.clone(), de.name.clone())
    }

    /// Launches a RomM version picked from the context menu. Behaves like
    /// activating a tile, but targets the chosen sibling rom ID directly,
    /// so a multi-disc game's other discs are reachable.
    fn activate_rom_variant(&mut self, rom_id: i64, title: String) -> Task<Message> {
        self.launch_managed(LaunchTarget::Romm(rom_id), format!("romm:{rom_id}"), title)
    }

    /// Shared launch tail for tile activation and RomM version picks: guards
    /// input ownership, drives the launch overlay, and calls the matching
    /// daemon launch route for the target kind.
    fn launch_managed(
        &mut self,
        target: LaunchTarget,
        app_id: String,
        title: String,
    ) -> Task<Message> {
        // A frontend-owned surface (the context menu) still owns the launch
        // even if it took keyboard focus and drove `input_ownership` off
        // `Frontend`; the menu can only be open while the frontend is the
        // active surface.
        if !self.input_ownership.frontend_has_control() && !self.frontend_surface_open() {
            return Task::none();
        }
        self.edit_name = None;
        let input_profile = if self.config.desktop_input_enabled(&app_id) {
            InputProfile::Desktop
        } else {
            InputProfile::Native
        };

        let Some(client) = self.daemon_client.clone() else {
            return Task::none();
        };
        // Built before the launch so the compositor watcher is already listening
        // when the window appears. A launch that fails leaves the hint to expire
        // on its own; nothing matches, so nothing happens.
        let window_hint = self.launch_window_hint(&app_id, &title);
        if self.launch_state.update(LaunchEvent::Start(title)) != LaunchEffect::Launch {
            return Task::none();
        }
        self.input_ownership.update(InputEvent::LaunchStarted);
        fullscreen_when_it_appears(window_hint);
        Task::perform(
            async move {
                match target {
                    LaunchTarget::Catalog(id) => client.launch_app(&id, input_profile).await,
                    LaunchTarget::Romm(id) => client.launch_retro_rom(id, input_profile).await,
                }
            },
            move |result| match result {
                Ok(_) => cosmic::Action::App(Message::DaemonLaunchResult(Ok(()))),
                Err(error) => {
                    cosmic::Action::App(Message::DaemonLaunchResult(Err(error.to_string())))
                }
            },
        )
    }

    /// What the window this launch is about to produce should look like, for the
    /// compositor watcher that takes it fullscreen.
    ///
    /// The entry's declared window class matters here: a Wayland app reports it
    /// as its app id, and it is often the only name that differs from the
    /// desktop file's own.
    fn launch_window_hint(&self, app_id: &str, title: &str) -> WindowHint {
        let wm_class = self
            .all_entries
            .iter()
            .find(|entry| entry.id == app_id)
            .and_then(|entry| entry.wm_class.clone());
        WindowHint::for_entry(app_id, title, wm_class.as_deref())
    }

    /// Consoles offered on the dashboard, derived from the live RomM platform
    /// list rather than any fixed set. Each entry keeps its index within the
    /// Console Games tab strip so a selection can open the matching tab.
    /// Custom (user-created) groups are skipped: they are not consoles.
    fn dashboard_consoles(&self) -> Vec<(usize, &AppGroup)> {
        self.config
            .sections
            .console_games
            .iter()
            .enumerate()
            .filter(|(_, group)| group.romm_platform_id().is_some())
            .collect()
    }

    fn dashboard_console_id_for_widget(&self, id: &widget::Id) -> Option<usize> {
        self.dashboard_consoles()
            .into_iter()
            .find_map(|(index, _)| (dashboard_console_id(index) == *id).then_some(index))
    }

    /// Opens the Library on the Console Games section with `index`'s tab
    /// selected. Used when the dashboard console rail is activated.
    fn open_console(&mut self, index: usize) -> Task<Message> {
        if self.config.sections.console_games.get(index).is_none() {
            return Task::none();
        }
        self.edit_name = None;
        self.search_value.clear();
        self.cur_section = Section::ConsoleGames;
        self.cur_group = Some(index);
        self.scroll_offset = 0.0;
        // The dashboard is what the user was just on, so jump straight to the
        // tab without replaying a tab slide.
        self.tab_animation = None;
        self.group_keys = (0..self.config.sections.console_games.len() as u64).collect();
        self.switch_page(Page::Library);
        self.focused_id = None;

        let mut cmds = vec![
            self.load_romm_page(0),
            iced::widget::scrollable::scroll_to(
                SCROLLABLE_ID.clone(),
                AbsoluteOffset {
                    x: Some(0.0),
                    y: Some(0.0),
                },
            ),
        ];
        if let Some(task) = self.reveal_tab_strip() {
            cmds.push(task);
        }
        cmds.push(self.focus_grid_index(0));
        iced::Task::batch(cmds)
    }

    /// Opens the details screen for grid entry `index`, seeded from the entry
    /// that already exists in memory so the header paints immediately, and
    /// starts the daemon's detail read for everything else.
    fn open_details(&mut self, index: usize) -> Task<Message> {
        let Some(entry) = self.entry_path_input.get(index) else {
            return Task::none();
        };
        let Some(rom_id) = entry
            .id
            .strip_prefix("romm:")
            .and_then(|id| id.parse::<i64>().ok())
        else {
            return Task::none();
        };
        let title = entry.name.clone();
        // The tile's cover is decoded at tile size already, so the screen has
        // artwork before any download finishes.
        let cover = self.entry_icon_handles.get(index).cloned();
        // The version list is built when the RomM page loads, so the details
        // screen reads it rather than re-deriving it from the detail payload.
        let versions = self
            .romm_versions
            .get(&entry.id)
            .cloned()
            .unwrap_or_default();

        self.details = Some(Details {
            rom_id,
            title,
            cover,
            hero: None,
            screenshots: Vec::new(),
            hero_shot: None,
            versions,
            favorite: None,
            backlogged: None,
            state_pending: false,
            disc_picker: false,
            info: None,
            error: None,
            focus: DetailsFocus::Play,
            origin_index: index,
        });
        self.edit_name = None;
        self.switch_page(Page::Details);
        self.focused_id = None;

        let mut tasks = vec![self.focus_details(DetailsFocus::Play)];
        if let Some(client) = self.daemon_client.clone() {
            tasks.push(Task::perform(
                async move {
                    client
                        .retro_rom_details(rom_id)
                        .await
                        .map(Box::new)
                        .map_err(|error| error.to_string())
                },
                move |result| cosmic::Action::App(Message::DetailsLoaded { rom_id, result }),
            ));
        }
        iced::Task::batch(tasks)
    }

    /// Leaves the details screen for the grid it was opened from, putting the
    /// cursor back on the tile the user came from. The screen's state is kept
    /// until its page transition finishes (see [`Self::expire_page_animation`]).
    fn close_details(&mut self) -> Task<Message> {
        let Some(origin) = self.details.as_ref().map(|details| details.origin_index) else {
            return Task::none();
        };
        self.switch_page(Page::Library);
        self.focused_id = None;
        let mut tasks = vec![self.focus_grid_index(origin)];
        if let Some(task) = self.reveal_tab_strip() {
            tasks.push(task);
        }
        iced::Task::batch(tasks)
    }

    /// Adds or removes the current game from RomM's favorites.
    ///
    /// Applied locally first so the button answers the press, then settled by
    /// the write's answer; a failure puts the previous state back and says so,
    /// rather than leaving the button claiming something RomM never stored.
    fn toggle_details_favorite(&mut self) -> Task<Message> {
        let Some(details) = self.details.as_mut() else {
            return Task::none();
        };
        if details.state_pending {
            return Task::none();
        }
        let Some(client) = self.daemon_client.clone() else {
            return Task::none();
        };
        let rom_id = details.rom_id;
        let previous = details.favorite;
        let favorite = !previous.unwrap_or(false);
        details.favorite = Some(favorite);
        details.state_pending = true;
        Task::perform(
            async move {
                client
                    .set_retro_favorite(rom_id, favorite)
                    .await
                    .map_err(|error| error.to_string())
            },
            move |result| {
                cosmic::Action::App(Message::DetailsFavorite {
                    rom_id,
                    previous,
                    result,
                })
            },
        )
    }

    /// Sets or clears RomM's "play later" flag for the current game, with the
    /// same optimistic-then-settle handling as favorites.
    fn toggle_details_backlog(&mut self) -> Task<Message> {
        let Some(details) = self.details.as_mut() else {
            return Task::none();
        };
        if details.state_pending {
            return Task::none();
        }
        let Some(client) = self.daemon_client.clone() else {
            return Task::none();
        };
        let rom_id = details.rom_id;
        let previous = details.backlogged;
        let backlogged = !previous.unwrap_or(false);
        details.backlogged = Some(backlogged);
        details.state_pending = true;
        Task::perform(
            async move {
                client
                    .set_retro_backlogged(rom_id, backlogged)
                    .await
                    .map_err(|error| error.to_string())
            },
            move |result| {
                cosmic::Action::App(Message::DetailsBacklog {
                    rom_id,
                    previous,
                    result,
                })
            },
        )
    }

    /// Settles an optimistic favorite or play-later write: the daemon's answer
    /// becomes the shown state, or the state it replaced when the write failed,
    /// plus an alert saying why.
    fn settle_details_state(
        &mut self,
        rom_id: i64,
        previous: Option<bool>,
        result: Result<bool, String>,
        kind: DetailsStateKind,
    ) -> Task<Message> {
        // An answer for a game the user has moved past is dropped: it no longer
        // describes what is on screen.
        let Some(details) = self
            .details
            .as_mut()
            .filter(|details| details.rom_id == rom_id)
        else {
            return Task::none();
        };
        details.state_pending = false;
        let slot = match kind {
            DetailsStateKind::Favorite => &mut details.favorite,
            DetailsStateKind::Backlog => &mut details.backlogged,
        };
        match result {
            Ok(state) => {
                *slot = Some(state);
                Task::none()
            }
            Err(error) => {
                *slot = previous;
                let message = fl!("retro-state-failed", reason = error);
                self.push_alert(message, false)
            }
        }
    }

    /// Opens the disc picker, or closes it when it is already open.
    fn toggle_details_disc_picker(&mut self) -> Task<Message> {
        let Some(details) = self.details.as_ref() else {
            return Task::none();
        };
        if details.versions.len() < 2 {
            return Task::none();
        }
        let opening = !details.disc_picker;
        let cursor = if opening {
            // Start on the disc Play already launches.
            details
                .versions
                .iter()
                .position(|version| version.id == details.rom_id)
                .unwrap_or(0)
        } else {
            0
        };
        if let Some(details) = self.details.as_mut() {
            details.disc_picker = opening;
        }
        if opening {
            return self.focus_details(DetailsFocus::DiscChoice(cursor));
        }
        self.focus_details(DetailsFocus::Disc)
    }

    /// Points the screen at another file of the same game. Play, favorite and
    /// play later then apply to that file, which is how RomM models them: one
    /// `rom_user` row and one collection membership per file.
    ///
    /// The file's own user state is unknown until it is read, so it is cleared
    /// rather than inherited from the file the user came from.
    fn select_details_disc(&mut self, rom_id: i64) -> Task<Message> {
        let Some(details) = self.details.as_mut() else {
            return Task::none();
        };
        details.disc_picker = false;
        if details.rom_id != rom_id {
            details.rom_id = rom_id;
            details.favorite = None;
            details.backlogged = None;
        }
        self.focus_details(DetailsFocus::Play)
    }

    /// Moves the details screen's cursor and gives the matching widget the real
    /// keyboard focus, which is what draws the shared accent ring. The cursor
    /// itself is [`Details`]'s `focus`, so Confirm never depends on iced having
    /// reported that focus back.
    fn focus_details(&mut self, target: DetailsFocus) -> Task<Message> {
        let Some(details) = self.details.as_mut() else {
            return Task::none();
        };
        details.focus = target;
        let id = details_focus_id(target);
        iced_runtime::task::widget(focus(id.clone()))
            .map(|id| cosmic::Action::App(Message::UpdateFocused(Some(id))))
    }

    /// The details screen's focus rows, top to bottom: the screenshot strip,
    /// then the action line. A row that would be empty is left out, so D-pad
    /// movement never lands on nothing.
    ///
    /// The open disc picker replaces the rows entirely: it covers the page, so
    /// an open picker must not leave the cursor on a control behind it.
    fn details_rows(&self) -> Vec<Vec<DetailsFocus>> {
        let Some(details) = self.details.as_ref() else {
            return Vec::new();
        };
        if details.disc_picker {
            return vec![
                (0..details.versions.len())
                    .map(DetailsFocus::DiscChoice)
                    .collect(),
            ];
        }
        let mut rows = Vec::new();
        if !details.screenshots.is_empty() {
            rows.push(
                (0..details.screenshots.len())
                    .map(DetailsFocus::Screenshot)
                    .collect(),
            );
        }
        rows.push(details_actions(details.versions.len() > 1).to_vec());
        rows
    }

    /// The details cursor, corrected when the row it pointed at is gone: Play
    /// if it is still there, otherwise the first control that exists. Without
    /// this the D-pad would have nothing left to move onto.
    fn details_cursor(&self) -> Option<DetailsFocus> {
        let details = self.details.as_ref()?;
        let rows = self.details_rows();
        if rows.iter().any(|row| row.contains(&details.focus)) {
            return Some(details.focus);
        }
        if rows.iter().any(|row| row.contains(&DetailsFocus::Play)) {
            return Some(DetailsFocus::Play);
        }
        rows.first()?.first().copied()
    }

    fn details_horizontal_target(&self, delta: i32) -> Option<DetailsFocus> {
        let focus = self.details_cursor()?;
        let rows = self.details_rows();
        let row = rows.iter().find(|row| row.contains(&focus))?;
        let current = row.iter().position(|candidate| candidate == &focus)? as i32;
        let next = (current + delta).clamp(0, row.len() as i32 - 1) as usize;
        row.get(next).copied()
    }

    fn details_vertical_target(&self, delta: i32) -> Option<DetailsFocus> {
        let focus = self.details_cursor()?;
        let rows = self.details_rows();
        let row_index = rows.iter().position(|row| row.contains(&focus))?;
        let column = rows[row_index]
            .iter()
            .position(|candidate| candidate == &focus)?;
        let target = row_index as i32 + delta;
        if target < 0 || target >= rows.len() as i32 {
            return None;
        }
        let row = &rows[target as usize];
        row.get(column.min(row.len() - 1)).copied()
    }

    /// Confirm on the details screen: act on whatever the cursor is on.
    fn activate_details(&mut self) -> Task<Message> {
        let Some(focus) = self.details_cursor() else {
            return Task::none();
        };
        match focus {
            DetailsFocus::Back => self.update(Message::CloseDetails),
            DetailsFocus::Favorite => self.update(Message::DetailsToggleFavorite),
            DetailsFocus::PlayLater => self.update(Message::DetailsToggleBacklog),
            DetailsFocus::Disc => self.update(Message::DetailsToggleDiscPicker),
            DetailsFocus::Play => {
                let Some((rom_id, title)) = self
                    .details
                    .as_ref()
                    .map(|details| (details.rom_id, details.title.clone()))
                else {
                    return Task::none();
                };
                self.update(Message::ActivateRomVariant { rom_id, title })
            }
            DetailsFocus::Screenshot(index) => {
                if let Some(details) = self.details.as_mut() {
                    details.hero_shot = Some(index);
                }
                Task::none()
            }
            DetailsFocus::DiscChoice(index) => {
                let Some(rom_id) = self
                    .details
                    .as_ref()
                    .and_then(|details| details.versions.get(index))
                    .map(|version| version.id)
                else {
                    return Task::none();
                };
                self.update(Message::DetailsSelectDisc(rom_id))
            }
        }
    }

    /// Dashboard console rail as an element, or `None` when there are no
    /// consoles to show. Kept as a method so [`Self::view_dashboard`] can place
    /// it between shelves without duplicating the card/rail wiring.
    fn console_rail<'a>(&'a self, tile_size: f32) -> Option<Element<'a, Message>> {
        let consoles = self.dashboard_consoles();
        (!consoles.is_empty()).then(|| {
            let cards: Vec<Element<'a, Message>> = consoles
                .into_iter()
                .map(|(index, group)| {
                    // Bundled console art is square-ish pixel art, so contain
                    // it rather than crop; fall back to a generic glyph when a
                    // platform has no artwork or it could not be cached.
                    let handle = group
                        .romm_platform_id()
                        .and_then(|id| self.romm_platform_icons.get(&id))
                        .map(|path| {
                            crate::icon_cache::entry_icon_handle(
                                &IconSource::Path(PathBuf::from(path)),
                                tile_size as u32,
                            )
                        })
                        .unwrap_or_else(|| {
                            crate::icon_cache::icon_cache_handle(
                                "input-gaming-symbolic",
                                ICON_LARGE,
                            )
                        });
                    rail_item(dashboard_console_id(index), group.name(), handle, tile_size)
                        .fit(ContentFit::Contain)
                        .on_press(Message::OpenConsole(index))
                        .into()
                })
                .collect();
            rail(
                dashboard_rail_id(CONSOLES_RAIL_KEY),
                fl!("consoles"),
                tile_size,
            )
            .items(cards)
            .on_scroll(|viewport| Message::DashboardRailScrolled {
                key: CONSOLES_RAIL_KEY,
                offset: viewport.absolute_offset().x,
                viewport: viewport.bounds().width,
            })
            .into()
        })
    }

    fn dashboard_entry_ids(&self) -> Vec<widget::Id> {
        self.dashboard_entry_rows().into_iter().flatten().collect()
    }

    /// Rails shown on the dashboard, in visual order, each paired with its key
    /// and the exact card ids it renders. Focus navigation and scroll-into-view
    /// both derive from this single definition of the layout, so the console
    /// rail and the shelves cannot drift apart.
    fn dashboard_rails(&self) -> Vec<(&'static str, Vec<widget::Id>)> {
        let mut rails: Vec<(&'static str, Vec<widget::Id>)> = Vec::new();
        for (shelf, entries) in self.dashboard_shelves() {
            if !entries.is_empty() {
                rails.push((
                    shelf.key(),
                    entries
                        .into_iter()
                        .map(|entry| shelf.widget_id(&entry.id))
                        .collect(),
                ));
            }
            // The console rail sits immediately below Recently Played, so its
            // focus order must match that visual position.
            if shelf == DashboardShelf::Recent {
                let consoles: Vec<widget::Id> = self
                    .dashboard_consoles()
                    .into_iter()
                    .map(|(index, _)| dashboard_console_id(index))
                    .collect();
                if !consoles.is_empty() {
                    rails.push((CONSOLES_RAIL_KEY, consoles));
                }
            }
        }
        rails
    }

    fn dashboard_entry_rows(&self) -> Vec<Vec<widget::Id>> {
        self.dashboard_rails()
            .into_iter()
            .map(|(_, ids)| ids)
            .collect()
    }

    /// Scrolls the rail holding `id` just enough to keep the focused card in
    /// view. Cards are uniform, so a card occupies `index * (card + gap)`.
    ///
    /// The rail only moves when the focused card would leave the visible
    /// window: moving right past the trailing edge pins the card there, and
    /// moving left past the leading edge pins it at the start. Inside the
    /// window the selection walks card by card and the strip stays put, so
    /// controller navigation moves a cursor instead of dragging the shelf.
    fn reveal_dashboard_rail(&self, id: &widget::Id) -> Option<Task<Message>> {
        let spacing = theme::spacing();
        let card = dashboard_tile_size(self.window_width, spacing.space_l, spacing.space_l);
        let gap = f32::from(spacing.space_l);
        for (key, ids) in self.dashboard_rails() {
            let Some(index) = ids.iter().position(|candidate| candidate == id) else {
                continue;
            };
            let card = if key == CONSOLES_RAIL_KEY {
                dashboard_console_tile_size(self.window_width, spacing.space_l, spacing.space_l)
            } else {
                // The same width the rail actually renders its cards at, or the
                // scroll target would be computed for a wider card than exists.
                card * DASHBOARD_GAME_ASPECT
            };
            let (offset, viewport) = self
                .dashboard_rail_scroll
                .get(key)
                .copied()
                .unwrap_or((0.0, 0.0));
            // The scrollable only reports its viewport after it handles an
            // event, so before that (and on the first frame) fall back to the
            // space the rail actually gets, so the first move right can
            // scroll.
            let viewport = if viewport > 0.0 {
                viewport
            } else {
                (self.window_width - 2.0 * f32::from(spacing.space_l)).max(card)
            };
            let target = rail_scroll_target(index, ids.len(), offset, viewport, card, gap)?;
            return Some(iced::widget::scrollable::scroll_to(
                dashboard_rail_id(key),
                AbsoluteOffset {
                    x: Some(target),
                    y: None,
                },
            ));
        }
        None
    }

    fn dashboard_entry_id_for_widget(&self, id: &widget::Id) -> Option<String> {
        self.dashboard_shelves()
            .into_iter()
            .flat_map(|(shelf, entries)| {
                entries
                    .into_iter()
                    .map(move |entry| (shelf.widget_id(&entry.id), entry.id.clone()))
            })
            .find_map(|(widget_id, entry_id)| (widget_id == *id).then_some(entry_id))
    }

    fn focus_dashboard_id(&mut self, id: widget::Id) -> Task<Message> {
        self.focused_id = Some(id.clone());
        let mut tasks = vec![
            iced_runtime::task::widget(focus(id.clone()))
                .map(|id| cosmic::Action::App(Message::UpdateFocused(Some(id)))),
            self.request_virtual_keyboard(false),
        ];
        if let Some(reveal) = self.reveal_dashboard_rail(&id) {
            tasks.push(reveal);
        }
        Task::batch(tasks)
    }

    fn dashboard_horizontal_target(&self, delta: i32) -> Option<widget::Id> {
        let nav_ids = [
            DASHBOARD_HOME_ID.clone(),
            DASHBOARD_LIBRARY_ID.clone(),
            DASHBOARD_SEARCH_ID.clone(),
        ];
        let focused = self.focused_id.as_ref();
        let entry_rows = self.dashboard_entry_rows();
        let ids = focused
            .and_then(|focused| entry_rows.iter().find(|row| row.contains(focused)))
            .map(Vec::as_slice)
            .or_else(|| {
                focused
                    .is_some_and(|focused| nav_ids.contains(focused))
                    .then_some(nav_ids.as_slice())
            })?;
        let current = focused.and_then(|focused| ids.iter().position(|id| id == focused));
        let next = (current? as i32 + delta).clamp(0, ids.len() as i32 - 1) as usize;
        ids.get(next).cloned()
    }

    fn dashboard_vertical_target(&self, delta: i32) -> Option<widget::Id> {
        let rows = self.dashboard_entry_rows();
        let focused = self.focused_id.as_ref();
        let nav_ids = [
            DASHBOARD_HOME_ID.clone(),
            DASHBOARD_LIBRARY_ID.clone(),
            DASHBOARD_SEARCH_ID.clone(),
        ];
        if focused.is_some_and(|id| nav_ids.contains(id)) {
            return if delta > 0 {
                rows.first()?.first().cloned()
            } else {
                None
            };
        }
        for (row_index, row) in rows.iter().enumerate() {
            let Some(column) =
                focused.and_then(|id| row.iter().position(|candidate| candidate == id))
            else {
                continue;
            };
            if delta < 0 && row_index == 0 {
                return Some(DASHBOARD_HOME_ID.clone());
            }
            let target_row = row_index as i32 + delta;
            if target_row < 0 || target_row >= rows.len() as i32 {
                return None;
            }
            let target = &rows[target_row as usize];
            return target
                .get(column.min(target.len().saturating_sub(1)))
                .cloned();
        }
        if delta > 0 {
            rows.first()?.first().cloned()
        } else {
            Some(DASHBOARD_HOME_ID.clone())
        }
    }

    /// The index of the currently focused app in the grid, if any.
    fn focused_grid_index(&self) -> Option<usize> {
        self.focused_id
            .as_ref()
            .and_then(|focused| focused_entry_index(focused, &self.entry_ids))
    }

    /// True if focus is inside a text input, where gamepad movement should
    /// be left to the input itself.
    fn focused_is_text_input(&self) -> bool {
        self.focused_id
            .as_ref()
            .is_some_and(|id| id == &*SEARCH_ID || id == &*EDIT_GROUP_ID || id == &*NEW_GROUP_ID)
    }

    /// Height of one row of the application grid, in logical pixels.
    fn grid_row_height(&self) -> f32 {
        tile_height(self.window_width, self.cur_section != Section::Applications)
            + grid_gap(self.window_width)
    }

    /// The number of grid rows currently visible in the scrollable viewport.
    fn visible_row_count(&self) -> f32 {
        let viewport = if self.viewport_height > 0.0 {
            self.viewport_height
        } else {
            // Fallback before the first layout reports the real size: the
            // window is 690px tall with a header and a tab row on top.
            690.0 - 160.0
        };
        (viewport / self.grid_row_height()).max(1.0)
    }

    /// Returns a task that queries the grid scrollable's viewport height,
    /// unless we already know it.
    fn query_viewport_task(&self) -> Task<Message> {
        if self.viewport_height > 0.0 {
            return Task::none();
        }
        iced_runtime::task::widget(FindViewport {
            target: SCROLLABLE_ID.clone(),
            height: None,
        })
        .map(|height| cosmic::Action::App(Message::ViewportHeight(height)))
    }

    /// Scrolls the grid scrollable to the given relative offset (0..=1).
    fn snap_to(&self, id: widget::Id, y: f32) -> Task<Message> {
        iced::widget::scrollable::snap_to(
            id,
            RelativeOffset {
                x: None,
                y: Some(y),
            },
        )
    }

    /// The target relative scroll offset (0..=1) that keeps grid row `row`
    /// fully visible, or `None` if it is already inside the viewport and no
    /// scrolling is needed. The viewport only moves when the focused row
    /// would otherwise leave the visible area: it is then pinned to the
    /// nearest edge, instead of scrolling proportionally with every move.
    fn scroll_offset_for_row(&self, row: usize) -> Option<f32> {
        let total_rows = self.entry_path_input.len().div_ceil(GRID_COLUMNS);
        if total_rows <= 1 {
            return Some(0.0);
        }
        let visible = self.visible_row_count();
        if visible >= total_rows as f32 {
            return Some(0.0);
        }
        let row = row as f32;
        let top = (self.scroll_offset / self.grid_row_height()).max(0.0);
        let bottom = top + visible;
        if row >= top && row + 1.0 <= bottom {
            // The row is fully visible: leave the viewport alone.
            return None;
        }
        let target_top = if row + 1.0 > bottom {
            // The row is leaving through the bottom: pin it to the bottom.
            (row + 1.0 - visible).max(0.0)
        } else {
            // The row left through the top: pin it to the top.
            row
        };
        let out = (target_top / (total_rows as f32 - visible)).clamp(0.0, 1.0);
        Some(out)
    }

    /// Focuses the app at the given grid index, scrolling it into view only if
    /// it would otherwise leave the visible viewport.
    fn focus_grid_index(&mut self, i: usize) -> Task<Message> {
        let Some(focused) = self.entry_ids.get(i).cloned() else {
            return Task::none();
        };
        self.focused_id = Some(focused.clone());
        let mut tasks = vec![
            iced_runtime::task::widget(focus(focused))
                .map(|id| cosmic::Action::App(Message::UpdateFocused(Some(id)))),
            self.request_virtual_keyboard(false),
        ];
        if let Some(y) = self.scroll_offset_for_row(i / GRID_COLUMNS) {
            tasks.push(self.snap_to(SCROLLABLE_ID.clone(), y));
        }
        tasks.push(self.query_viewport_task());
        Task::batch(tasks)
    }

    /// Handle gamepad directional movement. Movement is ignored while a modal
    /// dialog is open, but a text input never traps the controller: moving
    /// down or right from an input jumps into the grid.
    fn gamepad_move(&mut self, msg: Message) -> Task<Message> {
        if self.launch_state.is_visible() {
            return Task::none();
        }
        if self.new_group.is_some() || self.group_to_delete.is_some() {
            return Task::none();
        }
        if self.menu.is_some() {
            // While the context menu is open the D-pad moves its highlight
            // instead of the grid selection behind it.
            let delta = match msg {
                Message::PrevRow | Message::PrevCol => -1,
                Message::NextRow | Message::NextCol => 1,
                _ => 0,
            };
            self.move_menu_selection(delta);
            return Task::none();
        }
        if self.focused_is_text_input() {
            return match msg {
                Message::NextRow | Message::NextCol => {
                    self.virtual_keyboard.did_dismiss_externally();
                    self.focus_grid_index(0)
                }
                _ => Task::none(),
            };
        }
        self.update(msg)
    }

    /// Handle the gamepad confirm (A) button.
    fn gamepad_confirm(&mut self) -> Task<Message> {
        if self.launch_state.error().is_some() {
            return self.update(Message::DismissLaunch);
        }
        if self.launch_state.is_visible() {
            return Task::none();
        }
        if self.page == Page::Details {
            // The details screen acts on its own cursor rather than on iced's
            // focus, so Confirm still works on the first frame after opening.
            return self.activate_details();
        }
        if self.menu.is_some() {
            // A context menu is open: confirm its highlighted entry.
            return self.activate_menu_entry(self.menu_selection);
        }
        iced_runtime::task::widget(find_focused())
            .map(|id| cosmic::Action::App(Message::ConfirmFocused(id)))
    }

    /// Handle the gamepad back (B) button.
    fn gamepad_back(&mut self) -> Task<Message> {
        if self.launch_state.error().is_some() {
            return self.update(Message::DismissLaunch);
        }
        if self.launch_state.is_visible() {
            return Task::none();
        }
        // Back is the global "leave this surface" contract. The disc picker is
        // a surface of its own: it closes first, then the screen itself.
        if self.page == Page::Details {
            if self
                .details
                .as_ref()
                .is_some_and(|details| details.disc_picker)
            {
                return self.update(Message::DetailsToggleDiscPicker);
            }
            return self.update(Message::CloseDetails);
        }
        if self.menu.is_some() {
            return self.update(Message::CloseContextMenu);
        }
        if self.new_group.is_some() {
            return self.update(Message::CancelNewGroup);
        }
        if self.group_to_delete.is_some() {
            return self.update(Message::CancelDelete);
        }
        if self.edit_name.is_some() {
            return self.update(Message::SubmitName);
        }
        self.update(Message::Close)
    }

    /// Handle the gamepad context menu (X) button for the focused entry.
    ///
    /// On a RomM console game this opens the details screen instead of the
    /// context menu: the menu's actions for a game (Run, and the other
    /// versions) all live on that screen, so X stays a single "what can I do
    /// with this" button. Every other entry keeps the menu, whose actions
    /// (pin, favorite, controller compatibility) have no details screen yet.
    fn gamepad_context_menu(&mut self) -> Task<Message> {
        if self.page == Page::Dashboard {
            return Task::none();
        }
        if self.menu.is_some() {
            return self.update(Message::CloseContextMenu);
        }
        let Some(i) = self.focused_grid_index() else {
            return Task::none();
        };
        if self
            .entry_path_input
            .get(i)
            .is_some_and(|entry| entry.id.starts_with("romm:"))
        {
            return self.update(Message::OpenDetails(i));
        }
        let Some(target) = self.entry_ids.get(i).cloned() else {
            return Task::none();
        };
        iced_runtime::task::widget(FindBounds {
            target,
            bounds: None,
        })
        .map(move |rect| cosmic::Action::App(Message::OpenContextMenu(rect, i)))
    }

    /// Whether a frontend-owned transient surface is open: the context menu
    /// popup or one of the modal dialogs. These live in separate Wayland
    /// surfaces, so the main window can look unfocused while they are up, and
    /// app-state changes (the menu highlight) do not otherwise repaint them.
    fn frontend_surface_open(&self) -> bool {
        self.menu.is_some() || self.new_group.is_some() || self.group_to_delete.is_some()
    }

    /// Number of entries the context menu shows for the current selection.
    /// Mirrors the order built in `view_window` for `MENU_ID`.
    fn menu_entry_count(&self) -> usize {
        // Run, pin, favorite, controller compatibility, plus remove when the
        // app belongs to a group.
        let fixed = 4 + usize::from(self.cur_group.is_some());
        // A RomM game's other versions are appended after the fixed actions.
        fixed + self.menu_rom_versions().len()
    }

    /// RomM versions offered for the entry the context menu is open on. Empty
    /// for non-RomM entries and single-file games.
    fn menu_rom_versions(&self) -> Vec<crate::providers::daemon::RetroRomVersion> {
        self.menu
            .and_then(|i| self.entry_path_input.get(i))
            .and_then(|entry| self.romm_versions.get(&entry.id))
            .cloned()
            .unwrap_or_default()
    }

    /// Move the context-menu highlight by `delta`, wrapping at both ends.
    fn move_menu_selection(&mut self, delta: i32) {
        let count = self.menu_entry_count();
        if count == 0 {
            return;
        }
        let current = self.menu_selection.min(count - 1) as i32;
        self.menu_selection = (current + delta).rem_euclid(count as i32) as usize;
    }

    /// Activate the context-menu entry at `index`, the gamepad confirm action.
    fn activate_menu_entry(&mut self, index: usize) -> Task<Message> {
        let Some(i) = self.menu else {
            return Task::none();
        };
        let fixed = 4 + usize::from(self.cur_group.is_some());
        if index >= fixed {
            // The tail of the menu lists this game's versions.
            let version = self.menu_rom_versions().into_iter().nth(index - fixed);
            let Some(version) = version else {
                return Task::none();
            };
            // Launch before clearing `menu`: an open frontend surface is what
            // authorizes the launch if a popup focus change moved ownership.
            let launch = self.update(Message::ActivateRomVariant {
                rom_id: version.id,
                title: version.title,
            });
            self.menu = None;
            return Task::batch(vec![commands::popup::destroy_popup(*MENU_ID), launch]);
        }
        let pinned = self
            .entry_path_input
            .get(i)
            .is_some_and(|entry| self.app_list_config.favorites.contains(&entry.id));
        match index {
            1 if pinned => self.update(Message::UnPinFromAppTray(i)),
            1 => self.update(Message::PinToAppTray(i)),
            2 => self.update(Message::ToggleFavorite(i)),
            3 => self.update(Message::SelectAction(
                MenuAction::ToggleControllerCompatibility,
            )),
            4 if self.cur_group.is_some() => self.update(Message::SelectAction(MenuAction::Remove)),
            // Run, and any out-of-range index, launches the app.
            _ => {
                // As above: authorize the launch while the menu is still open.
                let launch = self.update(Message::ActivateApp(i));
                self.menu = None;
                Task::batch(vec![commands::popup::destroy_popup(*MENU_ID), launch])
            }
        }
    }

    /// Handle switching to a neighbouring section with the shoulder buttons.
    ///
    /// The cycle consists of the three fixed sidebar tabs in order, so
    /// circling wraps around both ends and never skips a section.
    fn gamepad_switch_section(&mut self, delta: i32) -> Task<Message> {
        if self.page != Page::Library {
            return Task::none();
        }
        let total = Section::ALL.len() as i32;
        let current = self.cur_section.index() as i32;
        let next = (current + delta).rem_euclid(total) as usize;
        let section = Section::ALL[next];
        self.gamepad_focus_first = true;
        self.update(Message::SelectSection(section))
    }

    /// Handle switching to a neighbouring group tab with the trigger buttons.
    ///
    /// Cycles through the tabs of the current section, wrapping at both ends.
    /// The "all apps" state (no tab selected) is treated as preceding the
    /// first tab, so the next tab after it is the first one.
    fn gamepad_switch_tab(&mut self, delta: i32) -> Task<Message> {
        if self.page != Page::Library {
            return Task::none();
        }
        let groups = self.config.sections.get(self.cur_section);
        let len = groups.len() as i32;
        if len <= 0 {
            return Task::none();
        }
        // Positions: 0 = "All" tab (cur_group = None), 1..=len = group tabs.
        let total = len + 1;
        let current = self.cur_group.map_or(0, |i| (i as i32) + 1);
        let next = (current + delta).rem_euclid(total);
        let next_group = if next == 0 {
            None
        } else {
            Some((next - 1) as usize)
        };
        self.gamepad_focus_first = true;
        self.update(Message::SelectGroup(next_group))
    }
}

/// An operation that finds the layout bounds of the focusable widget with
/// the given ID, so the gamepad can open a context menu for it.
struct FindBounds {
    target: Id,
    bounds: Option<Rectangle>,
}

impl Operation<Rectangle> for FindBounds {
    fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, _state: &mut dyn Focusable) {
        if id.is_some_and(|id| id == &self.target) {
            self.bounds = Some(bounds);
        }
    }

    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Rectangle>)) {
        if self.bounds.is_none() {
            operate(self);
        }
    }

    fn finish(&self) -> Outcome<Rectangle> {
        self.bounds.map_or(Outcome::None, Outcome::Some)
    }
}

/// An operation that reports the layout height of the grid scrollable's
/// viewport, so navigation can keep the focused tile visible.
struct FindViewport {
    target: Id,
    height: Option<f32>,
}

impl Operation<f32> for FindViewport {
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        _translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        if self.height.is_none() && id.is_some_and(|id| id == &self.target) {
            self.height = Some(bounds.height);
        }
    }

    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<f32>)) {
        if self.height.is_none() {
            operate(self);
        }
    }

    fn finish(&self) -> Outcome<f32> {
        self.height.map_or(Outcome::None, Outcome::Some)
    }
}

/// An operation that measures how far a group tab sits outside the visible
/// part of the single-line tab strip, and returns the absolute scroll offset
/// that would center it. Layout coordinates from the tree are the strip's
/// natural (unscrolled) positions, so the current scroll offset must be
/// subtracted before comparing against the viewport.
struct FindTabStripReveal {
    scrollable_id: Id,
    tab_id: Id,
    viewport: Option<Rectangle>,
    content_width: Option<f32>,
    translation_x: Option<f32>,
    tab: Option<Rectangle>,
}

impl Operation<f32> for FindTabStripReveal {
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        if id.is_some_and(|id| id == &self.scrollable_id) {
            self.viewport = Some(bounds);
            self.content_width = Some(content_bounds.width);
            self.translation_x = Some(translation.x);
        }
    }

    fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, _state: &mut dyn Focusable) {
        if id.is_some_and(|id| id == &self.tab_id) {
            self.tab = Some(bounds);
        }
    }

    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<f32>)) {
        if self.tab.is_none() {
            operate(self);
        }
    }

    fn finish(&self) -> Outcome<f32> {
        let Some(viewport) = self.viewport else {
            return Outcome::None;
        };
        let Some(content_width) = self.content_width else {
            return Outcome::None;
        };
        let Some(scroll_x) = self.translation_x else {
            return Outcome::None;
        };
        let Some(tab) = self.tab else {
            return Outcome::None;
        };
        let visible_left = tab.x - scroll_x;
        if visible_left >= viewport.x && visible_left + tab.width <= viewport.x + viewport.width {
            // Already fully visible: keep the strip where the user put it.
            return Outcome::None;
        }
        let max = (content_width - viewport.width).max(0.0);
        let target =
            (tab.x + tab.width * 0.5 - (viewport.x + viewport.width * 0.5)).clamp(0.0, max);
        Outcome::Some(target)
    }
}

impl cosmic::Application for HearthDeck {
    type Message = Message;
    type Executor = executor::Default;
    type Flags = Args;
    const APP_ID: &'static str = "org.hearthdeck.HearthDeck";

    fn core(&self) -> &Core {
        &self.core
    }

    fn update(&mut self, message: Message) -> Task<Self::Message> {
        // A transition must never outlive its duration, whatever message
        // happens to arrive next.
        self.expire_page_animation();
        match message {
            Message::ProviderRecords(records) => {
                let new_entries: Vec<_> = records
                    .into_iter()
                    .map(|r| std::sync::Arc::new(r.into_desktop_entry()))
                    .collect();
                // Merge provider entries into existing list instead of
                // replacing.  Replacing all_entries causes every widget ID
                // to change, triggering an iced tree-diff panic
                // ("Downcast on stateless state") because the scrollable
                // Column's children are reconstructed from scratch.
                for entry in new_entries {
                    if !self.all_entries.iter().any(|e| e.id == entry.id) {
                        self.all_entries.push(entry);
                    }
                }
                self.all_entries.sort_by(|a, b| a.name.cmp(&b.name));
                self.sync_category_groups();
                if let Some(helper) = AppLibraryConfig::helper() {
                    let _ = self.config.write_entry(&helper);
                }
                self.load_apps();
                return Task::none();
            }
            Message::RommPlatforms(result) => {
                let consoles = match result {
                    Ok(consoles) => consoles,
                    Err(error) => {
                        tracing::debug!(%error, "RomM platforms are not available yet");
                        return self.load_romm_platforms(ROMM_REFRESH_INTERVAL);
                    }
                };
                self.romm_platform_icons = consoles
                    .iter()
                    .filter_map(|console| console.icon.clone().map(|icon| (console.id, icon)))
                    .collect();
                self.romm_platform_counts = consoles
                    .iter()
                    .map(|console| (console.id, console.rom_count))
                    .collect();
                let selected_group = self
                    .cur_group
                    .and_then(|index| self.config.sections.console_games.get(index))
                    .cloned();
                let platforms = consoles
                    .iter()
                    .filter(|console| console.rom_count > 0)
                    .map(|console| (console.id, console.label.clone()))
                    .collect::<Vec<_>>();
                let changed = self.config.sync_console_groups(&platforms);
                let refresh = self.load_romm_platforms(ROMM_REFRESH_INTERVAL);
                if self.cur_section == Section::ConsoleGames {
                    if changed {
                        self.cur_group = selected_group.and_then(|selected| {
                            self.config
                                .sections
                                .console_games
                                .iter()
                                .position(|group| group == &selected)
                        });
                        self.group_keys =
                            (0..self.config.sections.console_games.len() as u64).collect();
                    }
                    // Fetch the first page only when nothing is loaded for this
                    // scope and no request is already in flight. Further pages
                    // load on demand as the grid is scrolled.
                    let needs_initial_page = !self.romm_loading_page
                        && self.romm_next_offset.is_none()
                        && !self
                            .all_entries
                            .iter()
                            .any(|entry| entry.id.starts_with("romm:"));
                    if changed || needs_initial_page {
                        return Task::batch([self.load_romm_page(0), refresh]);
                    }
                }
                return refresh;
            }
            Message::RommGames {
                generation,
                platform_id,
                result,
            } => {
                if !romm_page_is_current(
                    generation,
                    self.romm_request_generation,
                    platform_id,
                    self.current_romm_platform_id(),
                ) {
                    return Task::none();
                }
                let page = match result {
                    Ok(page) => page,
                    Err(error) => {
                        self.romm_loading_page = false;
                        tracing::warn!(%error, "failed to load RomM games");
                        return Task::none();
                    }
                };
                let item_count = page.items.len() as u32;
                self.romm_grouped_total = Some(page.total);
                for record in page.items {
                    // Store the full version list, representative first, so the
                    // context menu can label every disc (including the one
                    // "Run" targets) instead of leaving it implicit.
                    let siblings = retro_rom_versions(&record.metadata);
                    if !siblings.is_empty() {
                        let mut versions = Vec::with_capacity(siblings.len() + 1);
                        if let Some(current_id) = record
                            .id
                            .strip_prefix("romm:")
                            .and_then(|id| id.parse::<i64>().ok())
                        {
                            versions.push(RetroRomVersion {
                                id: current_id,
                                title: retro_version_label(&record.metadata)
                                    .unwrap_or_else(|| record.name.clone()),
                                is_main_sibling: true,
                            });
                        }
                        versions.extend(siblings);
                        // Two files can share a name; append the id so no two
                        // menu entries read the same.
                        let mut seen = std::collections::HashSet::new();
                        for version in &mut versions {
                            if !seen.insert(version.title.clone()) {
                                version.title = format!("{} (#{})", version.title, version.id);
                            }
                        }
                        self.romm_versions.insert(record.id.clone(), versions);
                    }
                    let entry = Arc::new(record.into_desktop_entry());
                    if !self
                        .all_entries
                        .iter()
                        .any(|existing| existing.id == entry.id)
                    {
                        self.all_entries.push(entry);
                    }
                }
                self.romm_loading_page = false;
                self.romm_next_offset = next_romm_offset(page.offset, item_count, page.total);
                // Deliberately no re-sort. RomM already returns name order, and
                // sorting the combined catalog here moved already-rendered
                // tiles each time a later page landed (visible reflow). Pages
                // are appended in order; more load on scroll.
                self.load_apps();
                // While a search is active, the local filter must see the whole
                // scope, so keep pulling pages until it is exhausted. Plain
                // browsing stays on-demand.
                if !self.search_value.is_empty()
                    && let Some(offset) = self.romm_next_offset
                {
                    return self.load_romm_page(offset);
                }
                // Load just enough further pages to fill the viewport.
                if let Some(offset) = self.romm_next_offset {
                    let loaded_rows = self.entry_path_input.len().div_ceil(GRID_COLUMNS) as f32;
                    if loaded_rows < self.visible_row_count() + 1.0 {
                        return self.load_romm_page(offset);
                    }
                }
            }
            Message::UpdateFocused(id) => {
                self.focused_id = id;
                let text_input_focused = self.focused_is_text_input();
                let keyboard = self.request_virtual_keyboard(text_input_focused);
                let Some(i) = self
                    .focused_id
                    .as_ref()
                    .and_then(|focused| self.entry_ids.iter().position(|i| i == focused))
                else {
                    return keyboard;
                };
                let mut tasks = vec![keyboard, self.query_viewport_task()];
                if let Some(y) = self.scroll_offset_for_row(i / GRID_COLUMNS) {
                    tasks.push(self.snap_to(SCROLLABLE_ID.clone(), y));
                }
                return Task::batch(tasks);
            }
            Message::KeyboardNav(message) => match message {
                keyboard_nav::Action::FocusNext => {
                    return iced::Task::batch(vec![
                        iced::widget::operation::focus_next()
                            .map(|id| cosmic::Action::App(Message::UpdateFocused(id))),
                        iced_runtime::task::widget(find_focused())
                            .map(|id| cosmic::Action::App(Message::UpdateFocused(Some(id)))),
                    ]);
                }
                keyboard_nav::Action::FocusPrevious => {
                    return iced::Task::batch(vec![
                        iced::widget::operation::focus_previous()
                            .map(|id| cosmic::Action::App(Message::UpdateFocused(id))),
                        iced_runtime::task::widget(find_focused())
                            .map(|id| cosmic::Action::App(Message::UpdateFocused(Some(id)))),
                    ]);
                }
                keyboard_nav::Action::Escape => return self.on_escape(),
                keyboard_nav::Action::Search => return self.update(Message::OpenSearch),

                keyboard_nav::Action::Fullscreen => {}
            },

            Message::PrevRow => {
                if self.menu.is_some() {
                    // Keyboard arrows drive the menu highlight too, so a
                    // controller that presents as a keyboard can navigate it.
                    return self.gamepad_move(Message::PrevRow);
                }
                if self.page == Page::Dashboard {
                    let Some(id) = self.dashboard_vertical_target(-1) else {
                        return Task::none();
                    };
                    return self.focus_dashboard_id(id);
                }
                if self.page == Page::Details {
                    let Some(focus) = self.details_vertical_target(-1) else {
                        return Task::none();
                    };
                    return self.focus_details(focus);
                }
                let mut i = self
                    .focused_id
                    .as_ref()
                    .and_then(|focused| self.entry_ids.iter().position(|i| i == focused))
                    .unwrap_or(self.entry_ids.len().saturating_add(GRID_COLUMNS - 1));
                if i == 0 {
                    self.focused_id = None;

                    return iced::Task::batch(vec![
                        iced::widget::operation::focus_previous()
                            .map(|id| cosmic::Action::App(Message::UpdateFocused(id))),
                        iced_runtime::task::widget(find_focused())
                            .map(|id| cosmic::Action::App(Message::UpdateFocused(Some(id)))),
                        self.query_viewport_task(),
                    ]);
                }
                i = i.saturating_sub(GRID_COLUMNS);
                let Some(focused) = self.entry_ids.get(i).cloned() else {
                    return Task::none();
                };
                self.focused_id = Some(focused.clone());
                let mut tasks = vec![
                    iced_runtime::task::widget(focus(focused))
                        .map(|id| cosmic::Action::App(Message::UpdateFocused(Some(id)))),
                ];
                if let Some(y) = self.scroll_offset_for_row(i / GRID_COLUMNS) {
                    tasks.push(self.snap_to(SCROLLABLE_ID.clone(), y));
                }
                tasks.push(self.query_viewport_task());
                return Task::batch(tasks);
            }
            Message::NextRow => {
                if self.menu.is_some() {
                    return self.gamepad_move(Message::NextRow);
                }
                if self.page == Page::Dashboard {
                    let Some(id) = self.dashboard_vertical_target(1) else {
                        return Task::none();
                    };
                    return self.focus_dashboard_id(id);
                }
                if self.page == Page::Details {
                    let Some(focus) = self.details_vertical_target(1) else {
                        return Task::none();
                    };
                    return self.focus_details(focus);
                }
                let mut i: i32 = self
                    .focused_id
                    .as_ref()
                    .and_then(|focused| self.entry_ids.iter().position(|i| i == focused))
                    .map(|i| i as i32)
                    .unwrap_or(-(GRID_COLUMNS as i32));
                if i == self.entry_ids.len() as i32 - 1 {
                    self.focused_id = None;
                    return iced::Task::batch(vec![
                        iced::widget::operation::focus_next()
                            .map(|id| cosmic::Action::App(Message::UpdateFocused(id))),
                        iced_runtime::task::widget(find_focused())
                            .map(|id| cosmic::Action::App(Message::UpdateFocused(Some(id)))),
                        self.query_viewport_task(),
                    ]);
                }
                i += GRID_COLUMNS as i32;
                i = i.min(self.entry_ids.len() as i32 - 1);
                let Some(focused) = self.entry_ids.get(i as usize).cloned() else {
                    return Task::none();
                };
                self.focused_id = Some(focused.clone());
                let mut tasks = vec![
                    iced_runtime::task::widget(focus(focused))
                        .map(|id| cosmic::Action::App(Message::UpdateFocused(Some(id)))),
                ];
                if let Some(y) = self.scroll_offset_for_row(i as usize / GRID_COLUMNS) {
                    tasks.push(self.snap_to(SCROLLABLE_ID.clone(), y));
                }
                tasks.push(self.query_viewport_task());
                return Task::batch(tasks);
            }
            Message::PrevCol => {
                if self.menu.is_some() {
                    return self.gamepad_move(Message::PrevCol);
                }
                if self.page == Page::Dashboard {
                    let Some(id) = self.dashboard_horizontal_target(-1) else {
                        return Task::none();
                    };
                    return self.focus_dashboard_id(id);
                }
                if self.page == Page::Details {
                    let Some(focus) = self.details_horizontal_target(-1) else {
                        return Task::none();
                    };
                    return self.focus_details(focus);
                }
                let Some(i) = self.focused_grid_index() else {
                    return self.focus_grid_index(0);
                };
                if i == 0 {
                    return Task::none();
                }
                return self.focus_grid_index(i - 1);
            }
            Message::NextCol => {
                if self.menu.is_some() {
                    return self.gamepad_move(Message::NextCol);
                }
                if self.page == Page::Dashboard {
                    let Some(id) = self.dashboard_horizontal_target(1) else {
                        return Task::none();
                    };
                    return self.focus_dashboard_id(id);
                }
                if self.page == Page::Details {
                    let Some(focus) = self.details_horizontal_target(1) else {
                        return Task::none();
                    };
                    return self.focus_details(focus);
                }
                let i = self.focused_grid_index().unwrap_or(0);
                let Some(last) = self.entry_ids.len().checked_sub(1) else {
                    return Task::none();
                };
                if i >= last {
                    return Task::none();
                }
                return self.focus_grid_index(i + 1);
            }
            Message::GamepadEvent(event) => {
                // The frontend owns gamepad input while one of its own surfaces
                // is open, even if that surface drove input ownership away from
                // `Frontend` (e.g. a popup that took keyboard focus).
                if !self.input_ownership.frontend_has_control() && !self.frontend_surface_open() {
                    return Task::none();
                }
                return match event {
                    GamepadEvent::MoveUp => self.gamepad_move(Message::PrevRow),
                    GamepadEvent::MoveDown => self.gamepad_move(Message::NextRow),
                    GamepadEvent::MoveLeft => self.gamepad_move(Message::PrevCol),
                    GamepadEvent::MoveRight => self.gamepad_move(Message::NextCol),
                    GamepadEvent::Confirm => self.gamepad_confirm(),
                    GamepadEvent::Back => self.gamepad_back(),
                    GamepadEvent::Search => self.update(Message::OpenSearch),
                    GamepadEvent::ContextMenu => self.gamepad_context_menu(),
                    GamepadEvent::PrevGroup => self.gamepad_switch_section(-1),
                    GamepadEvent::NextGroup => self.gamepad_switch_section(1),
                    GamepadEvent::PrevTab => self.gamepad_switch_tab(-1),
                    GamepadEvent::NextTab => self.gamepad_switch_tab(1),
                };
            }
            Message::FocusGridFirst => {
                return self.focus_grid_index(0);
            }
            Message::OpenDashboard => {
                self.switch_page(Page::Dashboard);
                self.focused_id = None;
                let id = self
                    .dashboard_entry_ids()
                    .first()
                    .cloned()
                    .unwrap_or_else(|| DASHBOARD_HOME_ID.clone());
                return self.focus_dashboard_id(id);
            }
            Message::OpenLibrary => {
                self.switch_page(Page::Library);
                self.focused_id = None;
                let mut tasks = vec![self.focus_grid_index(0)];
                if let Some(task) = self.reveal_tab_strip() {
                    tasks.push(task);
                }
                return iced::Task::batch(tasks);
            }
            Message::OpenConsole(index) => {
                return self.open_console(index);
            }
            Message::OpenDetails(index) => {
                return self.open_details(index);
            }
            Message::CloseDetails => {
                return self.close_details();
            }
            Message::DetailsToggleFavorite => {
                return self.toggle_details_favorite();
            }
            Message::DetailsToggleBacklog => {
                return self.toggle_details_backlog();
            }
            Message::DetailsToggleDiscPicker => {
                return self.toggle_details_disc_picker();
            }
            Message::DetailsSelectDisc(rom_id) => {
                return self.select_details_disc(rom_id);
            }
            Message::DetailsFavorite {
                rom_id,
                previous,
                result,
            } => {
                return self.settle_details_state(
                    rom_id,
                    previous,
                    result,
                    DetailsStateKind::Favorite,
                );
            }
            Message::DetailsBacklog {
                rom_id,
                previous,
                result,
            } => {
                return self.settle_details_state(
                    rom_id,
                    previous,
                    result,
                    DetailsStateKind::Backlog,
                );
            }
            Message::DetailsSelectScreenshot(index) => {
                if let Some(details) = self.details.as_mut() {
                    details.hero_shot = Some(index);
                }
                return Task::none();
            }
            Message::DetailsLoaded { rom_id, result } => {
                // A slow answer for a game the user has already left must not
                // paint onto whatever screen is showing now.
                let Some(details) = self
                    .details
                    .as_mut()
                    .filter(|details| details.rom_id == rom_id)
                else {
                    return Task::none();
                };
                match result {
                    Ok(loaded) => {
                        // Screenshot art is decoded here, once, rather than on
                        // every frame of the strip.
                        details.hero = loaded
                            .hero
                            .as_deref()
                            .map(|path| details_handle(path, DETAILS_HERO_RASTER));
                        details.screenshots = loaded
                            .screenshots
                            .iter()
                            .map(|path| details_handle(path, DETAILS_SHOT_RASTER))
                            .collect();
                        // RomM's per-user state for this file, now known.
                        details.favorite = Some(loaded.info.favorite);
                        details.backlogged = Some(loaded.info.backlogged);
                        details.info = Some(loaded.info);
                    }
                    Err(error) => {
                        tracing::warn!(%error, rom_id, "failed to load RomM game details");
                        details.error = Some(error);
                    }
                }
                return Task::none();
            }
            Message::OpenSearch => {
                self.switch_page(Page::Library);
                return self.focus_text_input(SEARCH_ID.clone());
            }
            Message::InputChanged(value) => {
                self.search_value = value;
                let filter = self.filter_apps();
                // Console search filters the loaded RomM entries locally, so an
                // active query has to pull the remaining pages.
                if self.cur_section == Section::ConsoleGames
                    && !self.search_value.is_empty()
                    && !self.romm_loading_page
                    && let Some(offset) = self.romm_next_offset
                {
                    return Task::batch([filter, self.load_romm_page(offset)]);
                }
                return filter;
            }
            Message::Close => {
                if self.launch_state.is_visible() {
                    if self.launch_state.error().is_some() {
                        self.launch_state.update(LaunchEvent::Dismiss);
                    }
                    return Task::none();
                }
                if self.page == Page::Details {
                    // Escape closes the disc picker before it leaves the screen,
                    // matching the gamepad's Back.
                    if self
                        .details
                        .as_ref()
                        .is_some_and(|details| details.disc_picker)
                    {
                        return self.update(Message::DetailsToggleDiscPicker);
                    }
                    return self.update(Message::CloseDetails);
                }
                if self.page == Page::Library {
                    return self.update(Message::OpenDashboard);
                }
                return Task::none();
            }
            Message::ActivateApp(i) => {
                return self.activate_app(i);
            }
            Message::ActivateDashboardApp(id) => {
                return self.activate_dashboard_app(&id);
            }
            Message::ActivateRomVariant { rom_id, title } => {
                return self.activate_rom_variant(rom_id, title);
            }
            Message::ActivateFirstApp => {
                let keyboard = self.request_virtual_keyboard(false);
                return Task::batch([keyboard, self.activate_app(0)]);
            }
            Message::ConfirmFocused(focused) => {
                self.focused_id = Some(focused.clone());
                if focused == *DASHBOARD_HOME_ID {
                    return self.update(Message::OpenDashboard);
                }
                if focused == *DASHBOARD_LIBRARY_ID {
                    return self.update(Message::OpenLibrary);
                }
                if focused == *DASHBOARD_SEARCH_ID {
                    return self.update(Message::OpenSearch);
                }
                if self.page == Page::Dashboard {
                    if let Some(index) = self.dashboard_console_id_for_widget(&focused) {
                        return self.open_console(index);
                    }
                    let Some(entry_id) = self.dashboard_entry_id_for_widget(&focused) else {
                        return Task::none();
                    };
                    return self.activate_dashboard_app(&entry_id);
                }
                if focused == *NEW_GROUP_ID {
                    return self.update(Message::SubmitNewGroup);
                }
                if focused == *SUBMIT_DELETE_ID {
                    return self.update(Message::ConfirmDelete);
                }
                if focused == *EDIT_GROUP_ID {
                    return self.update(Message::SubmitName);
                }
                let Some(i) = focused_entry_index(&focused, &self.entry_ids) else {
                    return Task::none();
                };
                return self.activate_app(i);
            }
            Message::SelectSection(section) => {
                if section == self.cur_section {
                    return Task::none();
                }
                self.edit_name = None;
                self.search_value.clear();
                self.cur_section = section;
                self.cur_group = None;
                self.scroll_offset = 0.0;
                self.group_keys = (0..self.config.sections.get(section).len() as u64).collect();
                let load = if section == Section::ConsoleGames {
                    self.load_romm_page(0)
                } else {
                    self.filter_apps()
                };
                let mut cmds = vec![
                    load,
                    iced::widget::scrollable::scroll_to(
                        SCROLLABLE_ID.clone(),
                        AbsoluteOffset {
                            x: Some(0.0),
                            y: Some(0.0),
                        },
                    ),
                ];
                if self.gamepad_focus_first {
                    cmds.push(self.request_virtual_keyboard(false));
                } else {
                    cmds.push(self.focus_text_input(SEARCH_ID.clone()));
                }
                if let Some(task) = self.reveal_tab_strip() {
                    cmds.push(task);
                }
                return iced::Task::batch(cmds);
            }
            Message::SelectGroup(group) => {
                if group == self.cur_group {
                    // Re-selecting the active tab: keep the current behaviour of
                    // refreshing, but skip the slide.
                    return self.apply_group(group);
                }
                self.begin_tab_animation(group);
                return Task::none();
            }
            // TODO: wire the filter popover. The button and total-item count
            // are already in place; this message currently does nothing.
            Message::ToggleFilterMenu => {}
            Message::ReorderGroup(new_order) => {
                let prev_selected_key =
                    self.cur_group.and_then(|i| self.group_keys.get(i).copied());

                let reorder_keys: Vec<u64> = new_order
                    .into_iter()
                    .filter_map(|key| match key {
                        GroupRowKey::Custom(k) => Some(k),
                        GroupRowKey::AllApps | GroupRowKey::NewGroup => None,
                    })
                    .collect();

                if reorder_keys.len() != self.config.sections.get(self.cur_section).len() {
                    return Task::none();
                }

                let key_to_index: HashMap<u64, usize> = self
                    .group_keys
                    .iter()
                    .enumerate()
                    .map(|(i, &k)| (k, i))
                    .collect();

                let reordered: Vec<crate::app_group::AppGroup> = reorder_keys
                    .iter()
                    .filter_map(|k| {
                        key_to_index.get(k).and_then(|&i| {
                            self.config.sections.get(self.cur_section).get(i).cloned()
                        })
                    })
                    .collect();

                if reordered.len() != self.config.sections.get(self.cur_section).len() {
                    return Task::none();
                }

                *self.config.sections.get_mut(self.cur_section) = reordered;
                self.group_keys = reorder_keys.clone();

                if let Some(key) = prev_selected_key {
                    self.cur_group = reorder_keys.iter().position(|&k| k == key);
                }

                if let Some(helper) = self.helper.as_ref()
                    && let Err(err) = self.config.write_entry(helper)
                {
                    error!("{:?}", err);
                }
                // Reordering changes the strip geometry, so keep the selected
                // tab in view after the flex row snaps to its new order.
                return match self.reveal_tab_strip() {
                    Some(task) => task,
                    None => Task::none(),
                };
            }
            Message::Delete(group) => {
                self.group_to_delete = Some(group);
                return Task::batch(vec![
                    get_layer_surface(SctkLayerSurfaceSettings {
                        id: *DELETE_GROUP_WINDOW_ID,
                        keyboard_interactivity: KeyboardInteractivity::Exclusive,
                        anchor: Anchor::empty(),
                        namespace: "dialog".into(),
                        size: None,
                        ..Default::default()
                    }),
                    button::focus(SUBMIT_DELETE_ID.clone()),
                ]);
            }
            Message::EditName(name) => {
                self.edit_name = Some(name);
            }
            Message::SubmitName => {
                if let Some(name) = self.edit_name.take()
                    && let Some(i) = self.cur_group
                {
                    self.config.set_name(self.cur_section, i, name);
                }
                if let Some(helper) = self.helper.as_ref()
                    && let Err(err) = self.config.write_entry(helper)
                {
                    error!("{:?}", err);
                }
                return self.request_virtual_keyboard(false);
            }
            Message::StartEditName(name) => {
                self.edit_name = Some(name);
                return self.focus_text_input(EDIT_GROUP_ID.clone());
            }
            Message::StartNewGroup => {
                if self.new_group.is_some() {
                    return Task::none();
                }
                self.new_group = Some(String::new());
                return Task::batch(vec![
                    get_layer_surface(SctkLayerSurfaceSettings {
                        id: *NEW_GROUP_WINDOW_ID,
                        keyboard_interactivity: KeyboardInteractivity::Exclusive,
                        anchor: Anchor::empty(),
                        namespace: "dialog".into(),
                        size: None,
                        ..Default::default()
                    }),
                    self.focus_text_input(NEW_GROUP_ID.clone()),
                ]);
            }
            Message::NewGroup(group_name) => {
                self.new_group = Some(group_name);
            }
            Message::SubmitNewGroup => {
                if let Some(group_name) = self.new_group.take() {
                    self.config.add(self.cur_section, group_name);
                    self.group_keys.push(self.next_group_key);
                    self.next_group_key += 1;
                }
                if let Some(helper) = self.helper.as_ref()
                    && let Err(err) = self.config.write_entry(helper)
                {
                    error!("{:?}", err);
                }
                return Task::batch([
                    destroy_layer_surface(*NEW_GROUP_WINDOW_ID),
                    self.request_virtual_keyboard(false),
                ]);
            }
            Message::CancelNewGroup => {
                self.new_group = None;
                return Task::batch([
                    destroy_layer_surface(*NEW_GROUP_WINDOW_ID),
                    self.request_virtual_keyboard(false),
                ]);
            }
            Message::VirtualKeyboardToggled { target, result } => {
                let succeeded = result.is_ok();
                if let Err(error) = result {
                    warn!("could not toggle virtual keyboard: {error}");
                }
                let Some(next_target) = self.virtual_keyboard.complete(target, succeeded) else {
                    return Task::none();
                };
                return Self::toggle_virtual_keyboard(next_target);
            }
            Message::OpenContextMenu(rect, i) => {
                if self.menu.take().is_some() {
                    return destroy_popup(*MENU_ID);
                } else {
                    self.menu = Some(i);
                    self.menu_selection = 0;
                    let offset = self.scroll_offset as i32;
                    return cosmic::surface::surface_task(simple_popup(
                        LiveSettings::default,
                        move || {
                            SctkPopupSettings {
                        parent: SurfaceId::RESERVED,
                        id: *MENU_ID,
                        positioner: SctkPositioner {
                            size: None,
                            size_limits: Limits::NONE.min_width(1.0).min_height(1.0).max_width(MENU_MAX_WIDTH).max_height(MENU_MAX_HEIGHT),
                            anchor_rect: Rectangle {
                                x: rect.x as i32,
                                y: rect.y as i32 - offset,
                                width: rect.width as i32,
                                height: rect.height as i32,
                            },
                            anchor:
                                sctk::reexports::protocols::xdg::shell::client::xdg_positioner::Anchor::Right,
                            gravity: sctk::reexports::protocols::xdg::shell::client::xdg_positioner::Gravity::Right,
                            reactive: true,
                            ..Default::default()
                        },
                        grab: false,
                        parent_size: None,
                        close_with_children: true,
                        input_zone: None,
                    }
                        },
                        None::<Box<fn() -> cosmic::Element<'static, cosmic::Action<Message>>>>,
                    ));
                }
            }
            Message::CloseContextMenu => {
                self.menu = None;
                return commands::popup::destroy_popup(*MENU_ID);
            }
            Message::MenuConfirm => {
                if self.menu.is_some() {
                    return self.activate_menu_entry(self.menu_selection);
                }
            }
            Message::SelectAction(action) => {
                let mut tasks = vec![commands::popup::destroy_popup(*MENU_ID)];
                if let Some(info) = self.menu.take().and_then(|i| self.entry_path_input.get(i)) {
                    match action {
                        MenuAction::ToggleControllerCompatibility => {
                            self.config.toggle_desktop_input(&info.id);
                            if let Some(helper) = self.helper.as_ref()
                                && let Err(err) = self.config.write_entry(helper)
                            {
                                error!("{:?}", err);
                            }
                        }
                        MenuAction::Remove => {
                            self.config
                                .remove_entry(self.cur_section, self.cur_group, &info.id);
                            if let Some(helper) = self.helper.as_ref()
                                && let Err(err) = self.config.write_entry(helper)
                            {
                                error!("{:?}", err);
                            }
                            tasks.push(self.filter_apps());
                        }
                    }
                }
                return Task::batch(tasks);
            }
            Message::StartDrag(i) => {
                self.dnd_icon = Some(i);
            }
            Message::FinishDrag(copy) => {
                if !copy
                    && let Some(info) = self
                        .dnd_icon
                        .take()
                        .and_then(|i| self.entry_path_input.get(i))
                {
                    self.config
                        .remove_entry(self.cur_section, self.cur_group, &info.id);
                    if let Some(helper) = self.helper.as_ref()
                        && let Err(err) = self.config.write_entry(helper)
                    {
                        error!("{:?}", err);
                    }
                    return self.filter_apps();
                }
            }
            Message::CancelDrag => {
                self.dnd_icon = None;
            }
            Message::StartDndOffer(group) => {
                self.offer_group = Some(group);
            }
            Message::FinishDndOffer(group, entry) => {
                self.offer_group = None;
                let Some(entry) = entry else {
                    return Task::none();
                };
                self.config.add_entry(self.cur_section, group, &entry.id);
                if let Some(helper) = self.helper.as_ref()
                    && let Err(err) = self.config.write_entry(helper)
                {
                    error!("{:?}", err);
                }
            }
            Message::LeaveDndOffer(group) => {
                self.offer_group = self.offer_group.filter(|g| *g != group);
            }
            Message::ScrollYOffset(y, viewport_height) => {
                self.scroll_offset = y;
                self.viewport_height = viewport_height;
                // Infinite scroll for the RomM console grid: load the next page
                // once the viewport nears the end of what is loaded.
                if self.cur_section == Section::ConsoleGames
                    && !self.romm_loading_page
                    && let Some(offset) = self.romm_next_offset
                {
                    let total_rows = self.entry_path_input.len().div_ceil(GRID_COLUMNS) as f32;
                    let content_height = total_rows * self.grid_row_height();
                    if y + viewport_height >= content_height - self.grid_row_height() {
                        return self.load_romm_page(offset);
                    }
                }
            }
            Message::ViewportHeight(height) => {
                self.viewport_height = height;
            }
            Message::DashboardRailScrolled {
                key,
                offset,
                viewport,
            } => {
                self.dashboard_rail_scroll.insert(key, (offset, viewport));
            }
            Message::ScrollTabStrip(offset) => {
                return iced::widget::scrollable::scroll_to(
                    TAB_STRIP_SCROLLABLE_ID.clone(),
                    AbsoluteOffset {
                        x: Some(offset),
                        y: None,
                    },
                );
            }
            Message::ConfirmDelete => {
                let mut cmds = vec![destroy_layer_surface(*DELETE_GROUP_WINDOW_ID)];
                if let Some(group) = self.group_to_delete.take() {
                    self.config.remove(self.cur_section, group);
                    if group < self.group_keys.len() {
                        self.group_keys.remove(group);
                    }
                    if let Some(helper) = self.helper.as_ref()
                        && let Err(err) = self.config.write_entry(helper)
                    {
                        error!("{:?}", err);
                    }
                    self.cur_group = None;
                    cmds.push(self.filter_apps());
                }
                return Task::batch(cmds);
            }
            Message::CancelDelete => {
                self.group_to_delete = None;
                return destroy_layer_surface(*DELETE_GROUP_WINDOW_ID);
            }
            Message::FilterApps(input, filtered_apps, icon_handles) => {
                self.entry_path_input = filtered_apps;
                self.entry_icon_handles = icon_handles;
                self.rebuild_entry_ids();

                self.waiting_for_filtered = false;
                if self.search_value != input {
                    return self.filter_apps();
                }
                if std::mem::take(&mut self.gamepad_focus_first) {
                    return Task::perform(async {}, |_| {
                        cosmic::Action::App(Message::FocusGridFirst)
                    });
                }
            }
            Message::PinToAppTray(usize) => {
                let pinned_id = self.entry_path_input.get(usize).map(|e| e.id.clone());
                if let Some((pinned_id, app_list_helper)) = pinned_id
                    .zip(Config::new(cosmic_app_list_config::APP_ID, AppListConfig::VERSION).ok())
                {
                    self.app_list_config.add_pinned(pinned_id, &app_list_helper);
                }
                self.menu = None;
                return commands::popup::destroy_popup(*MENU_ID);
            }
            Message::UnPinFromAppTray(usize) => {
                let pinned_id = self.entry_path_input.get(usize).map(|e| e.id.clone());
                if let Some((pinned_id, app_list_helper)) = pinned_id
                    .zip(Config::new(cosmic_app_list_config::APP_ID, AppListConfig::VERSION).ok())
                {
                    self.app_list_config
                        .remove_pinned(&pinned_id, &app_list_helper);
                }
                self.menu = None;
                return commands::popup::destroy_popup(*MENU_ID);
            }
            Message::ToggleFavorite(usize) => {
                if let Some(id) = self
                    .entry_path_input
                    .get(usize)
                    .map(|entry| entry.id.clone())
                {
                    self.config.toggle_favorite(&id);
                    if let Some(helper) = self.helper.as_ref()
                        && let Err(error) = self.config.write_entry(helper)
                    {
                        error!("failed to save favorite: {error:?}");
                    }
                }
                self.menu = None;
                return commands::popup::destroy_popup(*MENU_ID);
            }
            Message::AppListConfig(config) => {
                self.app_list_config = config;
            }
            Message::Opened => {
                return window::set_mode(SurfaceId::RESERVED, window::Mode::Fullscreen);
            }
            Message::WindowFocusChanged(focused) => {
                // A frontend-owned transient surface (context menu, dialog)
                // takes keyboard focus when it opens, which arrives here as the
                // *main* window losing focus. That is not the frontend yielding
                // input: the popup is part of it. Treating it as unfocused
                // flipped ownership to `Unfocused`, and the gamepad gate then
                // dropped every event, leaving the context menu unnavigable by
                // controller even though it is still on screen.
                if !focused && self.frontend_surface_open() {
                    return Task::none();
                }
                self.input_ownership.update(if focused {
                    InputEvent::FrontendFocused
                } else {
                    InputEvent::FrontendUnfocused
                });
            }
            Message::WindowResized(width) => {
                self.window_width = width;
            }
            Message::DaemonLaunchResult(result) => {
                let refresh_recent = result.is_ok();
                let effect = match result {
                    Ok(()) => {
                        self.input_ownership.update(InputEvent::LaunchAccepted);
                        self.launch_state.update(LaunchEvent::Accepted)
                    }
                    Err(error) => {
                        error!("daemon launch failed: {error}");
                        self.input_ownership.update(InputEvent::LaunchFailed);
                        self.launch_state.update(LaunchEvent::Failed(error))
                    }
                };
                let mut tasks = Vec::new();
                if refresh_recent {
                    tasks.push(self.load_recent_activity(std::time::Duration::ZERO));
                }
                if effect == LaunchEffect::DelayDismiss {
                    tasks.push(Task::perform(
                        tokio::time::sleep(LAUNCH_OVERLAY_DELAY),
                        |_| cosmic::Action::App(Message::DismissLaunch),
                    ));
                }
                return Task::batch(tasks);
            }
            Message::ActiveSessionResult(result) => {
                match result {
                    Ok(active) => self
                        .input_ownership
                        .update(InputEvent::SessionObserved(active)),
                    Err(error) => {
                        log::warn!("active session check failed: {error}");
                        self.input_ownership.update(InputEvent::SessionCheckFailed);
                    }
                }
                return self.poll_active_session(SESSION_POLL_INTERVAL);
            }
            Message::SystemStatus(status) => {
                self.system_status = status;
                return Self::load_system_status(SYSTEM_STATUS_POLL_INTERVAL);
            }
            Message::DashboardHealth { generation, result } => {
                if generation != self.dashboard_health_generation {
                    return Task::none();
                }
                let mut alerts = Task::none();
                match result {
                    Ok(health) => {
                        if self.backend_available == Some(false) {
                            alerts = self.push_alert(fl!("backend-restored"), false);
                        }
                        self.backend_available = Some(true);
                        let degraded = degraded_providers(&health);
                        if degraded != self.degraded_providers {
                            if !degraded.is_empty() {
                                let message = if degraded.len() == 1 {
                                    fl!("library-source-degraded", source = degraded[0].clone())
                                } else {
                                    fl!("library-sources-degraded", count = degraded.len())
                                };
                                alerts = Task::batch([alerts, self.push_alert(message, true)]);
                            }
                            self.degraded_providers = degraded;
                        }
                    }
                    Err(error) => {
                        // Alert on the transition into failure, not on every poll.
                        if self.backend_available != Some(false) {
                            let message = match &error {
                                DashboardHealthError::Unreachable => fl!("backend-unavailable"),
                                DashboardHealthError::Service(reason) => {
                                    fl!("backend-error", reason = reason.clone())
                                }
                                DashboardHealthError::Protocol(reason) => {
                                    fl!("backend-protocol-mismatch", reason = reason.clone())
                                }
                            };
                            alerts = self.push_alert(message, true);
                        }
                        self.backend_available = Some(false);
                        self.degraded_providers.clear();
                    }
                }
                let poll = self.load_dashboard_health(DASHBOARD_HEALTH_POLL_INTERVAL);
                return Task::batch([alerts, poll]);
            }
            Message::RetryDashboardAlert(id) => {
                return self.retry_dashboard_alert(id);
            }
            Message::DismissToast(id) => {
                self.toasts.remove(id);
            }
            Message::DashboardRescanResult(result) => {
                let mut alerts = Task::none();
                if let Err(error) = result {
                    log::warn!("dashboard library refresh failed: {error}");
                    alerts = self.push_alert(fl!("library-refresh-failed", reason = error), true);
                }
                self.dashboard_health_generation += 1;
                let health = self.load_dashboard_health(std::time::Duration::from_secs(1));
                return Task::batch([alerts, health]);
            }
            Message::RecentEntries(result) => match result {
                Ok(records) => {
                    self.recent_entries = records
                        .into_iter()
                        .map(|record| Arc::new(record.into_desktop_entry()))
                        .collect();
                    if self.page == Page::Dashboard {
                        let entry_ids = self.dashboard_entry_ids();
                        let nav_ids = [
                            DASHBOARD_HOME_ID.clone(),
                            DASHBOARD_LIBRARY_ID.clone(),
                            DASHBOARD_SEARCH_ID.clone(),
                        ];
                        let focus_is_valid = self.focused_id.as_ref().is_some_and(|focused| {
                            nav_ids.contains(focused) || entry_ids.contains(focused)
                        });
                        if !focus_is_valid {
                            let id = entry_ids
                                .first()
                                .cloned()
                                .unwrap_or_else(|| DASHBOARD_HOME_ID.clone());
                            return self.focus_dashboard_id(id);
                        }
                    }
                }
                Err(error) => {
                    log::debug!("recent activity is not available yet: {error}");
                    return self.load_recent_activity(RECENT_ACTIVITY_RETRY_INTERVAL);
                }
            },
            Message::DismissLaunch => {
                self.launch_state.update(LaunchEvent::Dismiss);
            }
            Message::Animate(now) => {
                self.now = now;
                self.expire_page_animation();
                return self.advance_tab_animation();
            }
        }
        Task::none()
    }

    fn dbus_activation(&mut self, msg: dbus_activation::Message) -> Task<Self::Message> {
        match msg.msg {
            dbus_activation::Details::Activate => Task::none(),
            dbus_activation::Details::ActivateAction { action, .. } => {
                let Ok(cmd) = ApplicationsTasks::from_str(&action) else {
                    return Task::none();
                };
                match cmd {
                    ApplicationsTasks::Input { input } => {
                        if let Some(input) = input {
                            self.search_value = input;
                            return self.filter_apps();
                        }
                        Task::none()
                    }
                    ApplicationsTasks::Close => self.close(),
                    // Run is handled at startup, not via D-Bus
                    ApplicationsTasks::Run => Task::none(),
                }
            }
            _ => Task::none(),
        }
    }

    fn view<'a>(&'a self) -> Element<'a, Message> {
        let content: Element<'a, Message> = if self.launch_state.is_visible() {
            self.view_launch_overlay()
        } else if let Some(animation) = &self.page_animation {
            PageTransition::new(
                self.page_element(animation.from),
                self.page_element(self.page),
                self.transition_progress(animation),
            )
            .into()
        } else {
            self.page_element(self.page)
        };

        // Alerts float above whatever page is showing, including the launch
        // overlay, so a daemon outage is visible wherever the user is.
        toaster::toaster(&self.toasts, content)
    }

    fn view_window<'a>(&'a self, id: SurfaceId) -> Element<'a, Message> {
        let Spacing {
            space_xxs, space_s, ..
        } = theme::spacing();

        if id == *MENU_ID {
            let Some((menu, i)) = self
                .menu
                .as_ref()
                .and_then(|i| self.entry_path_input.get(*i).map(|e| (e, i)))
            else {
                return container(space::horizontal())
                    .width(Length::Fixed(1.0))
                    .height(Length::Fixed(1.0))
                    .into();
            };

            let mut list_column = Vec::new();

            list_column.push(
                menu_button(text::body(RUN.clone()).size(TEXT_BODY))
                    .on_press(Message::ActivateApp(*i))
                    .selected(self.menu_selection == 0)
                    .into(),
            );

            // add to pinned
            let svg_accent = Rc::new(|theme: &cosmic::Theme| {
                let color = theme.cosmic().accent_color().into();
                svg::Style { color: Some(color) }
            });
            let is_pinned = self.app_list_config.favorites.iter().any(|p| p == &menu.id);
            let pin_to_app_tray = menu_button(
                if is_pinned {
                    row![
                        icon::icon(
                            icon::from_name("checkbox-checked-symbolic")
                                .size(ICON_SMALL)
                                .into()
                        )
                        .class(cosmic::theme::Svg::Custom(svg_accent.clone())),
                        text::body(fl!("pin-to-app-tray")).size(TEXT_BODY)
                    ]
                } else {
                    row![
                        space::horizontal().width(ICON_SMALL),
                        text::body(fl!("pin-to-app-tray")).size(TEXT_BODY)
                    ]
                }
                .spacing(space_xxs),
            )
            .on_press(if is_pinned {
                Message::UnPinFromAppTray(*i)
            } else {
                Message::PinToAppTray(*i)
            })
            .selected(self.menu_selection == 1);
            list_column.push(divider::horizontal::light().into());
            list_column.push(pin_to_app_tray.into());

            let is_favorite = self.config.is_favorite(&menu.id);
            list_column.push(divider::horizontal::light().into());
            list_column.push(
                menu_button(
                    row![
                        if is_favorite {
                            icon::icon(
                                icon::from_name("checkbox-checked-symbolic")
                                    .size(ICON_SMALL)
                                    .into(),
                            )
                            .class(cosmic::theme::Svg::Custom(svg_accent.clone()))
                        } else {
                            icon::icon(icon::from_name("checkbox-symbolic").size(ICON_SMALL).into())
                        },
                        text::body(fl!("favorite")).size(TEXT_BODY)
                    ]
                    .spacing(space_xxs),
                )
                .on_press(Message::ToggleFavorite(*i))
                .selected(self.menu_selection == 2)
                .into(),
            );

            let compatibility_enabled = self.config.desktop_input_enabled(&menu.id);
            list_column.push(divider::horizontal::light().into());
            list_column.push(
                menu_button(
                    row![
                        if compatibility_enabled {
                            icon::icon(
                                icon::from_name("checkbox-checked-symbolic")
                                    .size(ICON_SMALL)
                                    .into(),
                            )
                            .class(cosmic::theme::Svg::Custom(svg_accent.clone()))
                        } else {
                            icon::icon(icon::from_name("checkbox-symbolic").size(ICON_SMALL).into())
                        },
                        text::body(fl!("controller-compatibility")).size(TEXT_BODY)
                    ]
                    .spacing(space_xxs),
                )
                .on_press(Message::SelectAction(
                    MenuAction::ToggleControllerCompatibility,
                ))
                .selected(self.menu_selection == 3)
                .into(),
            );

            if self.cur_group.is_some() {
                list_column.push(divider::horizontal::light().into());
                list_column.push(
                    menu_button(text::body(REMOVE.clone()).size(TEXT_BODY))
                        .on_press(Message::SelectAction(MenuAction::Remove))
                        .selected(self.menu_selection == 4)
                        .into(),
                );
            }

            // A collapsed multi-disc/multi-region game offers every file here,
            // the representative first. Each is labelled with its own file name,
            // so discs and regions are distinguishable; the one "Run" targets
            // is checked.
            let versions = self.menu_rom_versions();
            if !versions.is_empty() {
                let fixed = 4 + usize::from(self.cur_group.is_some());
                let current_id = menu
                    .id
                    .strip_prefix("romm:")
                    .and_then(|id| id.parse::<i64>().ok());
                list_column.push(divider::horizontal::light().into());
                for (offset, version) in versions.into_iter().enumerate() {
                    let version_icon = if current_id == Some(version.id) {
                        "checkbox-checked-symbolic"
                    } else {
                        "media-optical-symbolic"
                    };
                    let index = fixed + offset;
                    list_column.push(
                        menu_button(
                            row![
                                icon::icon(icon::from_name(version_icon).size(ICON_SMALL).into())
                                    .class(cosmic::theme::Svg::Custom(svg_accent.clone())),
                                text::body(version.title.clone()).size(TEXT_BODY),
                            ]
                            .spacing(space_xxs),
                        )
                        .on_press(Message::ActivateRomVariant {
                            rom_id: version.id,
                            title: version.title,
                        })
                        .selected(self.menu_selection == index)
                        .into(),
                    );
                }
            }

            return autosize(
                container(scrollable(MenuColumn::with_children(list_column))).padding(1),
                MENU_AUTOSIZE_ID.clone(),
            )
            .max_height(MENU_MAX_HEIGHT)
            .max_width(MENU_MAX_WIDTH)
            .into();
        }
        if id == *NEW_GROUP_WINDOW_ID {
            let Some(group_name) = self.new_group.as_ref() else {
                return container(space::horizontal())
                    .width(Length::Fixed(1.0))
                    .height(Length::Fixed(1.0))
                    .into();
            };
            let dialog = widget::dialog::dialog()
                .title(CREATE_NEW.as_str())
                .control(
                    text_input("", group_name)
                        .label(&*NEW_GROUP_PLACEHOLDER)
                        .on_input(Message::NewGroup)
                        .on_submit(|_| Message::SubmitNewGroup)
                        .width(Length::Fixed(DIALOG_WIDTH))
                        .size(TEXT_BODY)
                        .id(NEW_GROUP_ID.clone()),
                )
                .primary_action(
                    button::custom(
                        text::body(SAVE.as_str())
                            .size(TEXT_BODY)
                            .center()
                            .width(Length::Fill),
                    )
                    .class(Button::Suggested)
                    .on_press(Message::SubmitNewGroup)
                    .padding([space_xxs, space_s])
                    .width(DIALOG_ACTION_WIDTH),
                )
                .secondary_action(
                    button::custom(
                        text::body(CANCEL.as_str())
                            .size(TEXT_BODY)
                            .center()
                            .width(Length::Fill),
                    )
                    .on_press(Message::CancelNewGroup)
                    .padding([space_xxs, space_s])
                    .width(DIALOG_ACTION_WIDTH),
                )
                .width(Length::Fixed(DIALOG_WIDTH));

            return autosize(dialog, NEW_GROUP_AUTOSIZE_ID.clone()).into();
        }
        if id == *DELETE_GROUP_WINDOW_ID {
            let dialog = widget::dialog::dialog()
                .icon(icon::from_name("edit-delete-symbolic").size(ICON_LARGE))
                .title(fl!("delete-folder"))
                .body(fl!("delete-folder", "msg"))
                .primary_action(
                    button::custom(
                        text::body(fl!("delete"))
                            .size(TEXT_BODY)
                            .center()
                            .width(Length::Fill),
                    )
                    .id(SUBMIT_DELETE_ID.clone())
                    .class(Button::Destructive)
                    .on_press(Message::ConfirmDelete)
                    .padding([space_xxs, space_s])
                    .width(DIALOG_ACTION_WIDTH),
                )
                .secondary_action(
                    button::custom(
                        text::body(CANCEL.to_string())
                            .size(TEXT_BODY)
                            .center()
                            .width(Length::Fill),
                    )
                    .on_press(Message::CancelDelete)
                    .padding([space_xxs, space_s])
                    .width(DIALOG_ACTION_WIDTH),
                )
                .width(Length::Fixed(DIALOG_WIDTH));

            return autosize(dialog, DELETE_GROUP_AUTOSIZE_ID.clone()).into();
        }

        container(space::horizontal())
            .width(Length::Fixed(1.0))
            .height(Length::Fixed(1.0))
            .into()
    }

    fn system_theme_update(
        &mut self,
        _keys: &[&'static str],
        _new_theme: &cosmic::cosmic_theme::Theme,
    ) -> Task<Message> {
        // The theme also carries COSMIC's density (spacing) and roundness, and
        // the entry icons are rasterized at the tile size those produce. Rebuild
        // them so a live density change stays crisp instead of a scaled blur;
        // the icon cache is keyed by (icon, size), so a color-only change is
        // just a cache hit.
        self.update_entry_metadata();
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            listen_with(|e, status, id| match e {
                cosmic::iced::Event::Window(WindowEvent::Opened { .. })
                    if is_primary_window(id) =>
                {
                    Some(Message::Opened)
                }
                cosmic::iced::Event::Window(WindowEvent::Focused) if id == SurfaceId::RESERVED => {
                    Some(Message::WindowFocusChanged(true))
                }
                cosmic::iced::Event::Window(WindowEvent::Unfocused)
                    if id == SurfaceId::RESERVED =>
                {
                    Some(Message::WindowFocusChanged(false))
                }
                // Only the main window drives the layout. Popups (context menu)
                // and dialogs are separate surfaces, and letting their size
                // overwrite `window_width` reflows the whole grid.
                cosmic::iced::Event::Window(WindowEvent::Resized(size))
                    if is_primary_window(id) =>
                {
                    Some(Message::WindowResized(size.width))
                }
                cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyReleased {
                    key: Key::Named(Named::Escape),
                    modifiers: _mods,
                    ..
                }) => Some(Message::Close),
                cosmic::iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_))
                    if id == SurfaceId::RESERVED =>
                {
                    Some(Message::CloseContextMenu)
                }
                cosmic::iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key,
                    text: _,
                    modifiers,
                    ..
                }) => match key {
                    Key::Character(c) if modifiers.control() && (c == "p" || c == "k") => {
                        Some(Message::PrevRow)
                    }
                    Key::Character(c) if modifiers.control() && (c == "n" || c == "j") => {
                        Some(Message::NextRow)
                    }
                    Key::Character(c) if modifiers.control() && (c == "f" || c == "l") => {
                        Some(Message::KeyboardNav(keyboard_nav::Action::FocusNext))
                    }
                    Key::Character(c) if modifiers.control() && (c == "b" || c == "h") => {
                        Some(Message::KeyboardNav(keyboard_nav::Action::FocusPrevious))
                    }
                    Key::Character(c) if modifiers.control() && (c == "z") => {
                        Some(Message::GamepadEvent(GamepadEvent::PrevGroup))
                    }
                    Key::Character(c) if modifiers.control() && (c == "x") => {
                        Some(Message::GamepadEvent(GamepadEvent::NextGroup))
                    }
                    Key::Named(Named::Enter) if matches!(status, iced::event::Status::Ignored) => {
                        Some(Message::MenuConfirm)
                    }
                    Key::Named(Named::ArrowUp)
                        if matches!(status, iced::event::Status::Ignored) =>
                    {
                        Some(Message::PrevRow)
                    }
                    Key::Named(Named::ArrowDown)
                        if matches!(status, iced::event::Status::Ignored) =>
                    {
                        Some(Message::NextRow)
                    }
                    Key::Named(Named::ArrowLeft)
                        if matches!(status, iced::event::Status::Ignored) =>
                    {
                        Some(Message::KeyboardNav(keyboard_nav::Action::FocusPrevious))
                    }
                    Key::Named(Named::ArrowRight)
                        if matches!(status, iced::event::Status::Ignored) =>
                    {
                        Some(Message::KeyboardNav(keyboard_nav::Action::FocusNext))
                    }
                    _ => None,
                },
                _ => None,
            }),
            keyboard_nav::subscription().map(Message::KeyboardNav),
            self.core
                .watch_config::<cosmic_app_list_config::AppListConfig>(
                    cosmic_app_list_config::APP_ID,
                )
                .map(|config| Message::AppListConfig(config.config)),
        ];

        let frontend_surface_open = self.frontend_surface_open();
        if self.input_ownership.frontend_has_control() || frontend_surface_open {
            subs.push(gamepad_events().map(Message::GamepadEvent));
        }

        subs.push(provider_records_subscription());

        // Only subscribe to the frame clock while a transition is in flight, so
        // an idle window schedules no redraws at all. This is iced's documented
        // way to drive application animations: see `iced::window::frames`.
        if self.page_animation.is_some() || self.tab_animation.is_some() {
            subs.push(window::frames().map(|(_window, at)| Message::Animate(at)));
        }

        Subscription::batch(subs)
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(mut core: Core, _flags: Args) -> (Self, iced::Task<cosmic::Action<Self::Message>>) {
        core.set_keyboard_nav(false);
        core.set_app_type(cosmic::core::AppType::Window);
        core.window.use_template = false;

        let daemon_client =
            crate::providers::daemon::DaemonClient::new(crate::providers::daemon::DaemonConfig {
                base_url: std::env::var("HEARTHDECK_BACKEND_URL")
                    .unwrap_or_else(|_| "http://127.0.0.1:38400".to_string()),
                token: std::env::var("HEARTHDECK_PAIRING_TOKEN").unwrap_or_default(),
            });
        let (provider_service, mut provider_rx) =
            crate::providers::service::ProviderService::start(vec![std::sync::Arc::new(
                crate::providers::daemon::DaemonProvider::with_client(daemon_client.clone()),
            )]);

        let helper = AppLibraryConfig::helper();

        let config: AppLibraryConfig = helper
            .as_ref()
            .map(|helper| {
                let mut config =
                    AppLibraryConfig::get_entry(helper).unwrap_or_else(|(errors, config)| {
                        for err in errors {
                            error!("{:?}", err);
                        }
                        config
                    });

                // Migration from the v1 schema (single `groups` key) to the v2
                // `sections` schema. The v1 groups were the PC games groups.
                if config.sections.pc_games.is_empty()
                    && config.sections.console_games.is_empty()
                    && let Ok(legacy) = Config::new(Self::APP_ID, 1)
                    && let Ok(groups) = ConfigGet::get::<Vec<AppGroup>>(&legacy, "groups")
                    && !groups.is_empty()
                {
                    config.sections.pc_games = groups;
                }

                config
            })
            .unwrap_or_default();
        let group_count = config.sections.get(Section::PcGames).len() as u64;
        let group_keys: Vec<u64> = (0..group_count).collect();
        let mut self_ = Self {
            locale: std::env::var("LANG")
                .ok()
                .and_then(|l| l.split(".").next().map(str::to_string)),
            config,
            core,
            helper,
            group_keys,
            next_group_key: group_count,
            provider_service: Some(provider_service),
            daemon_client: Some(daemon_client),
            ..Default::default()
        };

        // Bridge provider records into the watch channel for the subscription.
        // We also use a oneshot channel to synchronously receive the first
        // batch of records in init() so the first frame already contains all
        // entries. Without this, iced delivers ProviderRecords after the first
        // frame, causing a tree-diff panic ("Downcast on stateless state")
        // when the scrollable Column's children change.
        let (initial_tx, initial_rx) = tokio::sync::oneshot::channel();
        let mut initial_tx = Some(initial_tx);
        let tx = PROVIDER_RECORDS.tx.clone();
        tokio::spawn(async move {
            while let Some(records) = provider_rx.recv().await {
                if let Some(tx) = initial_tx.take() {
                    let _ = tx.send(records.clone());
                }
                let _ = tx.send(records);
            }
        });

        // Block until the first batch of provider records arrives.
        // ProviderService runs discovery in a tokio task that completes
        // almost instantly when config dirs are missing.
        let initial_records = tokio::runtime::Handle::current()
            .block_on(initial_rx)
            .unwrap_or_default();

        let new_entries: Vec<_> = initial_records
            .into_iter()
            .map(|r| std::sync::Arc::new(r.into_desktop_entry()))
            .collect();
        for entry in new_entries {
            if !self_.all_entries.iter().any(|e| e.id == entry.id) {
                self_.all_entries.push(entry);
            }
        }
        self_.all_entries.sort_by(|a, b| a.name.cmp(&b.name));
        self_.sync_category_groups();

        self_.load_apps();

        // libcosmic's `system_preference()` falls back to a *fixed* `Theme::dark()`
        // when the theme-mode config cannot be read yet. The minimal COSMIC test
        // session runs no settings daemon, so that file may not exist when this
        // app starts, and libcosmic only applies later theme/mode changes while
        // the active theme is a `System` one. A fixed fallback therefore leaves
        // the app stuck on its startup colors until it is restarted. Force a
        // system-tracked theme: keep the preference when it is already `System`,
        // otherwise choose by the brightness that was applied at startup.
        let preference = cosmic::theme::system_preference();
        let theme = if matches!(
            &preference.theme_type,
            cosmic::theme::ThemeType::System { .. }
        ) {
            preference
        } else if preference.theme_type.is_dark() {
            cosmic::theme::system_dark()
        } else {
            cosmic::theme::system_light()
        };

        let fullscreen = window::set_mode(SurfaceId::RESERVED, window::Mode::Fullscreen);
        let dashboard_focus = self_
            .dashboard_entry_ids()
            .first()
            .cloned()
            .unwrap_or_else(|| DASHBOARD_HOME_ID.clone());
        let focus_dashboard = iced_runtime::task::widget(focus(dashboard_focus))
            .map(|id| cosmic::Action::App(Message::UpdateFocused(Some(id))));
        let poll_active_session = self_.poll_active_session(std::time::Duration::ZERO);
        let load_romm_platforms = self_.load_romm_platforms(std::time::Duration::ZERO);
        let load_system_status = Self::load_system_status(std::time::Duration::ZERO);
        let load_recent_activity = self_.load_recent_activity(std::time::Duration::ZERO);
        let load_dashboard_health = self_.load_dashboard_health(std::time::Duration::ZERO);
        (
            self_,
            Task::batch([
                cosmic::command::set_theme::<Message>(theme),
                fullscreen,
                focus_dashboard,
                poll_active_session,
                load_romm_platforms,
                load_system_status,
                load_recent_activity,
                load_dashboard_health,
            ]),
        )
    }
}

impl HearthDeck {
    fn page_element<'a>(&'a self, page: Page) -> Element<'a, Message> {
        match page {
            Page::Dashboard => self.view_dashboard(),
            Page::Library => self.view_main_content(),
            Page::Details => self.view_details(),
        }
    }

    fn view_dashboard<'a>(&'a self) -> Element<'a, Message> {
        let Spacing {
            space_xs,
            space_s,
            space_m,
            space_l,
            space_xl,
            space_xxl,
            ..
        } = theme::spacing();
        let tile_size = dashboard_tile_size(self.window_width, space_l, space_l);
        let nav_button_size = f32::from(space_xl);
        let user_name = current_user_name();

        let nav_button = |id: widget::Id,
                          icon_name: &'static str,
                          label: &'static str,
                          selected: bool,
                          message: Message|
         -> Element<'_, Message> {
            tooltip(
                button::custom(icon::icon(icon::from_name(icon_name).into()).size(ICON_BODY))
                    .id(id)
                    .width(Length::Fixed(nav_button_size))
                    .height(Length::Fixed(nav_button_size))
                    .class(dashboard_nav_button_class(selected))
                    .on_press(message),
                text::body(label).size(TEXT_BODY),
                tooltip::Position::Bottom,
            )
            .into()
        };

        let status_icon = |icon_name: &'static str, label: &'static str| -> Element<'_, Message> {
            tooltip(
                icon::icon(icon::from_name(icon_name).into()).size(ICON_BODY),
                text::body(label).size(TEXT_BODY),
                tooltip::Position::Bottom,
            )
            .into()
        };
        let wifi = self
            .system_status
            .wifi
            .as_ref()
            .map(|status| status_icon(status.icon_name(), status.label()))
            .unwrap_or_else(|| status_icon("network-wireless-error-symbolic", "Wi-Fi unavailable"));
        let bluetooth = self
            .system_status
            .bluetooth
            .as_ref()
            .map(|status| status_icon(status.icon_name(), status.label()))
            .unwrap_or_else(|| status_icon("bluetooth-disabled-symbolic", "Bluetooth unavailable"));
        let system_status = row![
            text::body(crate::system_status::current_time()).size(TEXT_HEADER),
            wifi,
            bluetooth,
        ]
        .spacing(space_s)
        .align_y(Alignment::Center);

        let profile = row![
            icon::icon(icon::from_name("avatar-default-symbolic").into()).size(ICON_LARGE),
            column![
                text::body(user_name).size(TEXT_HEADER),
                text::caption("Hearthdeck").size(TEXT_CAPTION),
            ]
            .spacing(space_xs),
        ]
        .spacing(space_s)
        .align_y(Alignment::Center)
        .width(Length::FillPortion(1));

        let navigation = row![
            nav_button(
                DASHBOARD_HOME_ID.clone(),
                "go-home-symbolic",
                "Home",
                true,
                Message::OpenDashboard,
            ),
            nav_button(
                DASHBOARD_LIBRARY_ID.clone(),
                "view-grid-symbolic",
                "Library",
                false,
                Message::OpenLibrary,
            ),
            nav_button(
                DASHBOARD_SEARCH_ID.clone(),
                "system-search-symbolic",
                "Search",
                false,
                Message::OpenSearch,
            ),
        ]
        .spacing(space_xs)
        .align_y(Alignment::Center);

        let top_bar = row![
            profile,
            navigation,
            container(system_status)
                .width(Length::FillPortion(1))
                .align_x(Horizontal::Right),
        ]
        .spacing(space_l)
        .align_y(Alignment::Center)
        .height(Length::Fixed(f32::from(space_xxl)));

        let console_tile_size = dashboard_console_tile_size(self.window_width, space_l, space_l);
        let content = column![top_bar].spacing(space_m);
        let mut lower = column![].spacing(space_l);
        // Shelves render in order; the console rail is inserted right after
        // Recently Played so games lead the dashboard and consoles sit below.
        for (shelf, entries) in self.dashboard_shelves() {
            let cards: Vec<Element<'_, Message>> = entries
                .into_iter()
                .map(|entry| {
                    rail_item(
                        shelf.widget_id(&entry.id),
                        &entry.name,
                        crate::icon_cache::entry_icon_handle(&entry.icon, tile_size as u32),
                        tile_size,
                    )
                    // Portrait, so a poster is cropped on the sides rather than
                    // losing its title art to a square crop.
                    .aspect(DASHBOARD_GAME_ASPECT)
                    .on_press(Message::ActivateDashboardApp(entry.id.clone()))
                    .into()
                })
                .collect();
            let empty: Element<'_, Message> = container(
                row![
                    icon::icon(APP_ICON.clone()).size(ICON_BODY),
                    text::body(shelf.empty_message()).size(TEXT_BODY),
                ]
                .spacing(space_s)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fixed(tile_size))
            .align_x(Horizontal::Left)
            .align_y(Alignment::Center)
            .into();
            lower = lower.push(
                rail(dashboard_rail_id(shelf.key()), shelf.title(), tile_size)
                    .items(cards)
                    .empty(empty)
                    .on_scroll(move |viewport| Message::DashboardRailScrolled {
                        key: shelf.key(),
                        offset: viewport.absolute_offset().x,
                        viewport: viewport.bounds().width,
                    }),
            );
            if shelf == DashboardShelf::Recent
                && let Some(consoles) = self.console_rail(console_tile_size)
            {
                lower = lower.push(consoles);
            }
        }
        let content = content
            .push(space::vertical().height(Length::Fill))
            .push(lower)
            .push(space::vertical().height(space_xxl))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(space_l);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .class(theme::Container::Custom(Box::new(root_background)))
            .into()
    }

    /// A console game's details screen: its art, what it is, and the actions
    /// that used to live only in the context menu (Play, and every version).
    ///
    /// A fixed hero column beside a text column, with the screenshot strip
    /// along the bottom. The page itself does not scroll — every focus row is
    /// inside the window, and only the summary scrolls locally — so the
    /// controller cursor can never move to something unseen.
    fn view_details<'a>(&'a self) -> Element<'a, Message> {
        let Spacing {
            space_xxs,
            space_xs,
            space_s,
            space_m,
            space_l,
            space_xl,
            ..
        } = theme::spacing();
        let Some(details) = self.details.as_ref() else {
            return container(space::horizontal()).into();
        };
        // The cursor is read through `details_cursor`, not off `details.focus`,
        // so a row that disappeared (screenshots that failed to download) moves
        // the highlight back to Play instead of highlighting nothing.
        let cursor = self.details_cursor().unwrap_or_default();

        // ---- Backdrop: the screenshot being previewed, full-bleed. Screenshots
        // are 16:9 and survive being scaled to the window; a portrait cover
        // stretched here only ever looked blurry, so with no screenshots the
        // page stays a flat surface and the cover card carries the art. ----
        let backdrop: Element<'_, Message> = match details.backdrop_art() {
            Some(handle) => artwork_fit(handle, ContentFit::Cover, Length::Fill, Length::Fill),
            None => container(space::horizontal())
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        };
        let wash: Element<'_, Message> = container(space::horizontal())
            .width(Length::Fill)
            .height(Length::Fill)
            .class(theme::Container::Custom(Box::new(backdrop_wash)))
            .into();
        let scrim: Element<'_, Message> = container(space::horizontal())
            .width(Length::Fill)
            .height(Length::Fill)
            .class(theme::Container::Custom(Box::new(backdrop_scrim)))
            .into();

        let hero: Element<'_, Message> = match details.cover_art() {
            Some(handle) => artwork_fit(handle, ContentFit::Contain, Length::Fill, Length::Fill),
            None => container(
                icon::icon(icon::from_name("applications-games-symbolic").into()).size(ICON_LARGE),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into(),
        };
        let art_card = container(hero)
            .width(Length::Fixed(details_hero_width(self.window_width)))
            .height(Length::Fill)
            .padding(space_s)
            .class(theme::Container::Custom(Box::new(hero_card)));

        // ---- Action line: every interaction the screen has, in one strip. ----
        // One builder for all of them, so heights, text sizes, icon sizes and
        // padding cannot drift apart; Play differs only by width and by using the
        // accent fill.
        let action_button = |focus: DetailsFocus,
                             icon_name: &'static str,
                             label: String,
                             width: f32,
                             class: Button,
                             message: Message|
         -> Element<'_, Message> {
            button::custom(
                container(
                    row![
                        icon::icon(icon::from_name(icon_name).size(ICON_BODY).into()),
                        text::body(label).size(TEXT_LARGE),
                    ]
                    .spacing(space_s)
                    .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center),
            )
            .id(details_focus_id(focus))
            .width(Length::Fixed(width))
            .height(Length::Fixed(details_action_height()))
            .padding(0)
            .class(class)
            .on_press(message)
            .into()
        };
        let favorite_icon = if details.favorite.unwrap_or(false) {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        };

        let show_disc = details.versions.len() > 1;
        let mut action_line = row![
            action_button(
                DetailsFocus::Back,
                "go-previous-symbolic",
                fl!("back"),
                DETAILS_ACTION_WIDTH,
                details_toggle_button_class(false),
                Message::CloseDetails,
            ),
            action_button(
                DetailsFocus::Play,
                "media-playback-start-symbolic",
                fl!("play"),
                DETAILS_PLAY_WIDTH,
                primary_action_button_class(),
                Message::ActivateRomVariant {
                    rom_id: details.rom_id,
                    title: details.title.clone(),
                },
            ),
            action_button(
                DetailsFocus::Favorite,
                favorite_icon,
                fl!("favorite"),
                DETAILS_ACTION_WIDTH,
                details_toggle_button_class(details.favorite.unwrap_or(false)),
                Message::DetailsToggleFavorite,
            ),
        ]
        .spacing(space_m)
        .align_y(Alignment::Center);
        if show_disc {
            action_line = action_line.push(action_button(
                DetailsFocus::Disc,
                "media-optical-symbolic",
                fl!("disc"),
                DETAILS_ACTION_WIDTH,
                details_toggle_button_class(details.disc_picker),
                Message::DetailsToggleDiscPicker,
            ));
        }
        action_line = action_line.push(action_button(
            DetailsFocus::PlayLater,
            "alarm-symbolic",
            fl!("play-later"),
            DETAILS_ACTION_WIDTH,
            details_toggle_button_class(details.backlogged.unwrap_or(false)),
            Message::DetailsToggleBacklog,
        ));
        let action_strip = container(action_line)
            .width(Length::Fill)
            .padding(details_action_bar_padding())
            .align_x(Horizontal::Center)
            .class(theme::Container::Custom(Box::new(action_bar)));

        let chips: Element<'_, Message> = row(details
            .chips()
            .into_iter()
            .map(|label| {
                container(text(label).size(TEXT_CAPTION))
                    .padding([space_xxs, space_s])
                    .class(theme::Container::Custom(Box::new(card_surface)))
                    .into()
            })
            .collect::<Vec<Element<'_, Message>>>())
        .spacing(space_xs)
        .align_y(Alignment::Center)
        .into();

        // The disc picker: a panel over the page, using the native list look so
        // the highlighted disc reads the same way as any other COSMIC list.
        let disc_picker: Element<'_, Message> = {
            let mut rows = list_column::<Message>();
            for (index, version) in details.versions.iter().enumerate() {
                let is_current = version.id == details.rom_id;
                let icon_name = if is_current {
                    "checkbox-checked-symbolic"
                } else {
                    "media-optical-symbolic"
                };
                rows = rows.add(
                    list::button(
                        row![
                            icon::icon(icon::from_name(icon_name).size(ICON_SMALL).into()),
                            text::body(version.title.clone()).size(TEXT_BODY),
                        ]
                        .spacing(space_s)
                        .align_y(Alignment::Center),
                    )
                    .on_press(Message::DetailsSelectDisc(version.id))
                    .selected(cursor == DetailsFocus::DiscChoice(index)),
                );
            }

            container(
                container(
                    column![
                        text::title3(fl!("disc-selection")).size(TEXT_HEADER),
                        autosize(
                            container(scrollable(rows.into_element())).padding(1),
                            Id::new("details-disc-autosize"),
                        )
                        .max_height(MENU_MAX_HEIGHT),
                    ]
                    .spacing(space_m),
                )
                .width(Length::Fixed(DETAILS_PICKER_WIDTH))
                .padding(space_m)
                .class(theme::Container::Custom(Box::new(card_surface))),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .class(theme::Container::Custom(Box::new(modal_scrim)))
            .into()
        };

        let fact_rows: Vec<Element<'_, Message>> = details
            .facts()
            .into_iter()
            .map(|(label, value)| {
                row![
                    container(text::caption(label).size(TEXT_CAPTION))
                        .width(Length::Fixed(DETAILS_FACT_LABEL_WIDTH)),
                    text::body(value).size(TEXT_BODY).width(Length::Fill),
                ]
                .spacing(space_m)
                .align_y(Alignment::Start)
                .into()
            })
            .collect();

        let summary: Element<'_, Message> =
            match details.info.as_ref().and_then(|info| info.summary.clone()) {
                Some(summary) => column![
                    text::caption(fl!("about")).size(TEXT_CAPTION),
                    scrollable(text::body(summary).size(TEXT_BODY).width(Length::Fill))
                        .height(Length::Fill)
                        .scrollbar_width(0)
                        .scroller_width(0),
                ]
                .spacing(space_xxs)
                .height(Length::Fill)
                .into(),
                None => space::vertical().height(Length::Fill).into(),
            };

        let mut right = column![
            text(details.title.clone())
                .size(TEXT_TITLE)
                .width(Length::Fill)
        ]
        .spacing(space_s)
        .width(Length::Fill)
        .height(Length::Fill);
        right = right.push(chips);
        if let Some(error) = details.error.as_deref() {
            right = right.push(
                row![
                    icon::icon(icon::from_name("dialog-warning-symbolic").into()).size(ICON_SMALL),
                    text::caption(fl!("details-unavailable", reason = error.to_owned()))
                        .size(TEXT_CAPTION),
                ]
                .spacing(space_xs)
                .align_y(Alignment::Center),
            );
        }
        right = right.push(column(fact_rows).spacing(space_xs));
        right = right.push(summary);

        let shots: Element<'_, Message> = if details.screenshots.is_empty() {
            space::vertical().height(Length::Fixed(0.0)).into()
        } else {
            let cards: Vec<Element<'_, Message>> = details
                .screenshots
                .iter()
                .enumerate()
                .map(|(index, handle)| {
                    // Bound to a local first: the `fl!` macro cannot infer the
                    // type of a bare `index + 1` expression argument.
                    let number = index + 1;
                    rail_item(
                        details_focus_id(DetailsFocus::Screenshot(index)),
                        fl!("screenshot", index = number),
                        handle.clone(),
                        DETAILS_SHOT_SIZE,
                    )
                    .aspect(DETAILS_SHOT_ASPECT)
                    .on_press(Message::DetailsSelectScreenshot(index))
                    .into()
                })
                .collect();
            rail(
                Id::new("details-screenshots"),
                fl!("screenshots"),
                DETAILS_SHOT_SIZE,
            )
            .items(cards)
            .into()
        };

        // Scrolls on its own: a long summary or a long fact list must never
        // push the action line off the bottom of the screen.
        let info = scrollable(right)
            .height(Length::Fill)
            .scrollbar_width(0)
            .scroller_width(0);
        let body = row![art_card, info]
            .spacing(space_xl)
            .height(Length::Fill)
            .width(Length::Fill);
        let page = column![body, shots, action_strip,]
            .spacing(space_m)
            .padding(space_l)
            .height(Length::Fill)
            .width(Length::Fill);

        let mut layers: Vec<Element<'_, Message>> = vec![backdrop, wash, scrim, page.into()];
        if details.disc_picker {
            layers.push(disc_picker);
        }

        container(cosmic::iced::widget::stack(layers))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_launch_overlay<'a>(&'a self) -> Element<'a, Message> {
        let spacing = theme::spacing();
        let title = self.launch_state.title().unwrap_or_default();
        let content: Element<'_, Message> = if let Some(error) = self.launch_state.error() {
            column![
                icon::icon(icon::from_name("dialog-error-symbolic").into()).size(ICON_LARGE),
                text::title2("Launch failed"),
                text::body(title),
                text::caption(error),
                button::custom(text::body("Dismiss"))
                    .class(Button::Suggested)
                    .on_press(Message::DismissLaunch)
                    .padding([spacing.space_xs, spacing.space_l]),
            ]
            .spacing(spacing.space_s)
            .align_x(Alignment::Center)
            .into()
        } else {
            column![
                icon::icon(APP_ICON.clone()).size(ICON_LARGE),
                text::title2(format!("Launching {title}...")),
            ]
            .spacing(spacing.space_m)
            .align_x(Alignment::Center)
            .into()
        };

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .class(theme::Container::Custom(Box::new(launch_overlay)))
            .into()
    }

    fn view_main_content<'a>(&'a self) -> Element<'a, Message> {
        let Spacing {
            space_none,
            space_xxs,
            space_xs,
            space_s,
            space_m,
            space_l,
            space_xxl,
            ..
        } = theme::spacing();

        let cur_section = self.cur_section;
        let cur_group = self.current_group();

        // The Console Games grid lazy-loads pages, so its "N items" label must
        // not track the loaded-page count (which climbs as the user scrolls).
        // Use RomM's own per-console total unless a local search is narrowing
        // the visible set, where the loaded count *is* the result count.
        let item_count = if cur_section == Section::ConsoleGames && self.search_value.is_empty() {
            self.console_total_count()
                .unwrap_or(self.entry_path_input.len())
        } else {
            self.entry_path_input.len()
        };

        let user_name = current_user_name();
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        let disk_free = human_size(available_disk_bytes(&home));

        // ===== Sidebar: fixed section navigation =====
        let build_section_button = |section: crate::app_group::Section| {
            let is_active = self.cur_section == section;
            let inner = container(
                row![
                    icon::icon(icon::from_name(section.icon_name()).into()).size(ICON_BODY),
                    text(section.name()).size(TEXT_LARGE),
                ]
                .spacing(space_m)
                .align_y(Alignment::Center),
            )
            .align_y(Vertical::Center)
            .width(Length::Fill)
            .padding([space_none, space_l]);

            let content = if is_active {
                row![
                    inner,
                    container(space::horizontal().width(Length::Fixed(SIDEBAR_ACCENT_BAR_WIDTH)))
                        .width(Length::Fixed(SIDEBAR_ACCENT_BAR_WIDTH))
                        .height(Length::Fixed(sidebar_accent_bar_height()))
                        .class(theme::Container::Custom(Box::new(accent_bar))),
                ]
                .align_y(Alignment::Center)
            } else {
                row![inner]
            };

            button::custom(
                container(content)
                    .align_y(Vertical::Center)
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .height(Length::Fixed(sidebar_item_height()))
            .width(Length::Fill)
            .class(section_button_class(is_active))
            .on_press(Message::SelectSection(section))
        };

        let sidebar_header = container(
            row![
                icon::icon(APP_ICON.clone()).size(ICON_LARGE),
                container(text(user_name).size(TEXT_HEADER))
                    .align_y(Vertical::Center)
                    .width(Length::Fill),
            ]
            .spacing(space_m)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(sidebar_header_height()))
        .align_y(Vertical::Center)
        .padding([0, space_l]);

        let storage_info = container(
            row![
                icon::icon(icon::from_name("drive-harddisk-solidstate-symbolic").into())
                    .size(ICON_BODY),
                column![
                    text::caption(fl!("storage-available")).size(TEXT_CAPTION),
                    text::body(disk_free).size(TEXT_BODY),
                ]
                .spacing(space_xxs),
            ]
            .spacing(space_xs)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([space_m, space_l, space_l, space_l]);

        let sidebar = container(
            column![
                sidebar_header,
                build_section_button(crate::app_group::Section::PcGames),
                build_section_button(crate::app_group::Section::ConsoleGames),
                build_section_button(crate::app_group::Section::Streaming),
                build_section_button(crate::app_group::Section::Applications),
                space::vertical().height(Length::Fill),
                storage_info,
            ]
            .spacing(space_xs),
        )
        .width(Length::Fixed(sidebar_width(self.window_width)))
        .height(Length::Fill)
        // Inset the navigation items from the window edge and the divider so the
        // selected chip reads as a rounded pill instead of a full-bleed bar.
        .padding([0, space_xs, space_m, space_xs]);

        // ===== Top bar: title + search =====
        let title_element: Element<'_, Message> = if let Some(edit_name) = self.edit_name.as_ref() {
            container(
                text_input(cur_group.name(), edit_name)
                    .on_input(Message::EditName)
                    .on_paste(Message::EditName)
                    .on_clear(Message::EditName(String::new()))
                    .on_submit(|_| Message::SubmitName)
                    .id(EDIT_GROUP_ID.clone())
                    .width(Length::Fixed(EDIT_NAME_INPUT_WIDTH))
                    .size(TEXT_HEADER),
            )
            .align_y(Vertical::Center)
            .into()
        } else {
            let label = if self.cur_group.is_some() {
                cur_group.name()
            } else {
                cur_section.name()
            };
            // A console tab shows its own logo beside the name, so the header
            // says which platform the grid belongs to without reading the tab.
            let logo = cur_group
                .romm_platform_id()
                .and_then(|platform_id| self.romm_platform_icons.get(&platform_id))
                .map(|path| {
                    crate::icon_cache::entry_icon_handle(
                        &IconSource::Path(PathBuf::from(path)),
                        u32::from(ICON_LARGE),
                    )
                });
            let heading: Element<'_, Message> = match logo {
                Some(handle) => row![
                    icon::icon(handle).size(ICON_LARGE),
                    text(label).size(TEXT_TITLE),
                ]
                .spacing(space_s)
                .align_y(Alignment::Center)
                .into(),
                None => row![text(label).size(TEXT_TITLE)].into(),
            };
            container(heading).align_y(Vertical::Center).into()
        };

        let title_actions: Element<'_, Message> = if self.cur_group.is_some() {
            row![
                tooltip(
                    container({
                        let mut b = button::custom(
                            icon::icon(icon::from_name("edit-symbolic").into())
                                .width(Length::Fixed(ICON_TILE_ACTION))
                                .height(Length::Fixed(ICON_TILE_ACTION)),
                        )
                        .padding(space_xs)
                        .class(Button::Icon);
                        if self.edit_name.is_none() {
                            b = b.on_press(Message::StartEditName(cur_group.name()));
                        }
                        b
                    })
                    .height(Length::Fixed(title_action_height()))
                    .align_y(Vertical::Center),
                    text(fl!("rename")).size(TEXT_HEADER),
                    tooltip::Position::Bottom
                ),
                tooltip(
                    container(
                        button::custom(
                            icon::icon(icon::from_name("edit-delete-symbolic").into())
                                .width(Length::Fixed(ICON_TILE_ACTION))
                                .height(Length::Fixed(ICON_TILE_ACTION)),
                        )
                        .padding(space_xs)
                        .class(Button::Icon)
                        .on_press_maybe(self.cur_group.map(Message::Delete))
                    )
                    .height(Length::Fixed(title_action_height()))
                    .align_y(Vertical::Center),
                    text(fl!("delete")).size(TEXT_HEADER),
                    tooltip::Position::Bottom
                )
            ]
            .spacing(space_xxs)
            .into()
        } else {
            row![].spacing(0).into()
        };

        let title_row = row![
            title_element,
            title_actions,
            space::horizontal().width(Length::FillPortion(1)),
            container(
                text_input(SEARCH_PLACEHOLDER.as_str(), self.search_value.as_str())
                    .on_input(Message::InputChanged)
                    .on_paste(Message::InputChanged)
                    .on_submit(|_| Message::ActivateFirstApp)
                    .style(TextInput::Search)
                    .width(Length::Fixed(SEARCH_WIDTH))
                    .size(TEXT_HEADER)
                    .padding([space_s, space_m])
                    .leading_icon(
                        container(
                            icon::icon(icon::from_name("system-search-symbolic").into())
                                .size(ICON_SEARCH)
                        )
                        .padding(search_icon_padding())
                        .into(),
                    )
                    .id(SEARCH_ID.clone())
            )
            .align_y(Vertical::Center),
        ]
        .align_y(Alignment::Center)
        .spacing(space_s);

        // ===== Filter button + item count (right side of the tab row) =====
        let filter_btn = button::custom(
            container(
                row![
                    icon::icon(icon::from_name("view-filter-symbolic").into()).size(ICON_BODY),
                    text::body(fl!("filter")).size(TEXT_BODY),
                ]
                .spacing(space_xs)
                .align_y(Alignment::Center),
            )
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .padding([space_none, space_m]),
        )
        .height(Length::Fixed(filter_button_height()))
        .width(Length::Shrink)
        .class(section_button_class(false))
        .id(FILTER_ID.clone())
        .on_press(Message::ToggleFilterMenu);

        // ===== Sub-tab strip =====
        // A tab is a text button with a 4px underline that shows the accent
        // color on the active tab. Both custom group tabs and the locked
        // "all apps" tab share this single builder.
        let build_tab =
            |is_active: bool, label: String, on_press: Option<Message>, id: Option<widget::Id>| {
                let width = tab_width(&label);
                let mut tab_btn = button::custom(
                    container(text::body(label).size(TEXT_BODY))
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center)
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .padding([space_none, space_m]),
                )
                .width(Length::Shrink)
                .height(Length::Fill)
                .class(tab_button_class(is_active))
                .on_press_maybe(on_press);
                if let Some(id) = id {
                    tab_btn = tab_btn.id(id);
                }

                let underline = if is_active {
                    container(space::horizontal().width(Length::Fixed(1.0)))
                        .width(Length::Fill)
                        .height(Length::Fixed(tab_underline_height()))
                        .class(theme::Container::Custom(Box::new(accent_bar)))
                } else {
                    container(space::horizontal())
                        .width(Length::Fill)
                        .height(Length::Fixed(tab_underline_height()))
                };

                // Cap the tab column so the accent underline can span the label
                // without the Fill widths expanding it (and wrapping) the row.
                column![tab_btn, underline]
                    .width(Length::Shrink)
                    .height(Length::Fixed(tab_height()))
                    .max_width(width)
                    .align_x(Alignment::Center)
            };

        let build_group_tab = |i: usize, key: u64, group: &crate::app_group::AppGroup| {
            dnd_destination_for_data::<AppletString, Message>(
                build_tab(
                    self.cur_group == Some(i),
                    group.name(),
                    self.menu.is_none().then_some(Message::SelectGroup(Some(i))),
                    Some(group_tab_id(key)),
                ),
                move |data, _| {
                    Message::FinishDndOffer(
                        Some(i),
                        data.and_then(|data| load_desktop_file(&[], data.0)),
                    )
                },
            )
            .drag_id(i as u64 + 1)
            .on_enter(move |_, _, _| Message::StartDndOffer(Some(i)))
            .on_leave(move || Message::LeaveDndOffer(Some(i)))
        };

        let all_apps_tab = build_tab(
            self.cur_group.is_none(),
            self.cur_section.all_name(),
            self.menu.is_none().then_some(Message::SelectGroup(None)),
            None,
        );

        let add_tab_btn = button::custom(
            container(
                row![
                    icon::icon(icon::from_name("list-add-symbolic").into()).size(ICON_BODY),
                    text::body(ADD_GROUP.as_str()).size(TEXT_BODY)
                ]
                .spacing(space_xs)
                .align_y(Alignment::Center),
            )
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding([space_none, space_m]),
        )
        .height(Length::Fixed(tab_height()))
        .width(Length::Shrink)
        .class(theme::Button::IconVertical)
        .on_press(Message::StartNewGroup);

        // Keep the group tabs on a single line inside a horizontal scrollable:
        // `reorderable_flex_row` wraps onto extra rows when the strip is
        // narrower than its tabs, eating header space. Inside a scrollable it
        // measures with an unbounded width, so every tab stays on one row and
        // the strip scrolls when stores/platforms/groups overflow. The
        // scrollbar is hidden but scrolling still works, so overflow never
        // steals header space.
        let tab_strip = widget::scrollable::horizontal(
            self.config
                .sections
                .get(cur_section)
                .iter()
                .enumerate()
                .fold(
                    reorderable_flex_row::<GroupRowKey, Message>(Message::ReorderGroup)
                        .spacing(space_m)
                        .width(Length::Fill)
                        // Slot animations reposition chips over ~180ms after every
                        // section change, which both glitches the layout (chips
                        // drawn on top of each other) and forces a full redraw
                        // every frame while animating. Snap instead.
                        .animation_duration(std::time::Duration::ZERO)
                        .push_locked(GroupRowKey::AllApps, all_apps_tab),
                    |row, (i, group)| {
                        let key = self.group_keys.get(i).copied().unwrap_or(i as u64);
                        row.push(GroupRowKey::Custom(key), build_group_tab(i, key, group))
                    },
                )
                .push_locked(GroupRowKey::NewGroup, add_tab_btn),
        )
        .id(TAB_STRIP_SCROLLABLE_ID.clone())
        .width(Length::Fill)
        .scrollbar_width(0)
        .scroller_width(0);

        let tab_row = row![
            tab_strip,
            filter_btn,
            container(text::body(fl!("count-items", count = item_count)).size(TEXT_BODY),)
                .align_y(Vertical::Center),
        ]
        .spacing(space_m)
        .align_y(Alignment::Center)
        .width(Length::Fill);

        // ===== Application grid =====
        let app_grid_list: Vec<_> = self
            .entry_path_input
            .iter()
            .zip(self.entry_ids.iter())
            .zip(self.entry_icon_handles.iter())
            .enumerate()
            .map(|(i, ((entry, id), icon_handle))| {
                let dup = entry
                    .path
                    .as_ref()
                    .and_then(|path| self.duplicates.get(path));
                let selected = self.menu.is_some_and(|m| m == i);

                let b = ApplicationButton::new(
                    id.clone(),
                    &entry.name,
                    icon_handle.clone(),
                    &entry.path,
                    tile_width(self.window_width),
                    tile_height(self.window_width, self.cur_section != Section::Applications),
                    self.romm_versions
                        .get(&entry.id)
                        .map_or(1, |versions| versions.len()),
                    move |rect| Message::OpenContextMenu(rect, i),
                    if self.menu.is_none() {
                        Some(Message::ActivateApp(i))
                    } else if selected {
                        Some(Message::CloseContextMenu)
                    } else {
                        None
                    },
                    // TODO add icon and text if duplicated
                    dup,
                    selected,
                    self.menu.is_none().then_some(Message::StartDrag(i)),
                    self.menu.is_none().then_some(Message::FinishDrag(false)),
                    self.menu.is_none().then_some(Message::CancelDrag),
                );

                b.into()
            })
            .chunks(GRID_COLUMNS)
            .into_iter()
            .map(|row_chunk| {
                let mut new_row = row_chunk.collect_vec();
                let missing = GRID_COLUMNS - new_row.len();
                if missing > 0 {
                    new_row.push(
                        iced::widget::space::horizontal()
                            .width(Length::FillPortion(missing as u16))
                            .into(),
                    );
                }
                row(new_row).spacing(grid_gap(self.window_width)).into()
            })
            .collect();

        let app_grid = container(
            scrollable(
                column(app_grid_list)
                    .width(Length::Fill)
                    .spacing(grid_gap(self.window_width))
                    // padding on top needed to avoid focus highlight clipping
                    .padding([grid_top_padding(), 0, space_xxl, 0]),
            )
            .on_scroll(|viewport| {
                let offset = viewport.absolute_offset();
                Message::ScrollYOffset(offset.y, viewport.bounds().height)
            })
            .id(SCROLLABLE_ID.clone())
            .scrollbar_width(0)
            .scroller_width(0)
            .height(Length::Fill),
        )
        .height(Length::Fill);

        // A tab change slides the whole grid area; the swap happens under cover.
        let app_scrollable: Element<'_, Message> = match self.tab_progress() {
            Some(progress) => {
                let direction = self
                    .tab_animation
                    .map_or(1.0, |animation| animation.direction);
                TabTransition::new(app_grid.into(), progress, direction).into()
            }
            None => app_grid.into(),
        };

        let sidebar_divider = container(space::horizontal())
            .width(Length::Fixed(DIVIDER_WIDTH))
            .height(Length::Fill)
            .class(theme::Container::Custom(Box::new(sidebar_divider)));

        let content = row![
            sidebar,
            sidebar_divider,
            column![
                container(title_row).padding([space_l, 0, 0, 0]),
                // Keep the tab strip visually attached to its section title:
                // a full 64px gap pushed the grid too far down the window.
                space::vertical().height(space_m),
                container(tab_row).padding([0, 0, 0, 0]),
                app_scrollable,
            ]
            .width(Length::Fill)
            .padding([0, content_horizontal_padding()]),
        ]
        .height(Length::Fill);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .class(theme::Container::Custom(Box::new(root_background)))
            .into()
    }
}

fn focused_entry_index(focused: &widget::Id, entry_ids: &[widget::Id]) -> Option<usize> {
    entry_ids.iter().position(|id| id == focused)
}

/// Resolves a RomM artwork path the daemon has already cached on disk into a
/// rasterized icon handle. Artwork reaches the details screen through that
/// cache, so building a handle is a local read rather than a download.
fn details_handle(path: &str, size: u32) -> icon::Handle {
    crate::icon_cache::entry_icon_handle(&IconSource::Path(PathBuf::from(path)), size)
}

/// Widget id of the dashboard tile for the console at `index` within the
/// Console Games tab strip.
fn dashboard_console_id(index: usize) -> widget::Id {
    widget::Id::from(format!("dashboard-console-{index}"))
}

/// Scrollable id of the dashboard rail with `key`.
fn dashboard_rail_id(key: &str) -> widget::Id {
    widget::Id::from(format!("dashboard-rail-{key}"))
}

/// Absolute horizontal scroll target that keeps card `index` of a `count`-card
/// rail inside the visible window, or `None` when it is already fully visible.
/// `offset` is the current scroll position, `viewport` the visible width, and
/// `card`/`gap` the card size and the space between cards.
///
/// A card leaving through the trailing edge is pinned to that edge; one
/// leaving through the leading edge is pinned to the start. Anything already
/// inside the window returns `None`, which is what lets the selection walk
/// across the row before the strip starts moving.
fn rail_scroll_target(
    index: usize,
    count: usize,
    offset: f32,
    viewport: f32,
    card: f32,
    gap: f32,
) -> Option<f32> {
    let step = card + gap;
    let item_start = index as f32 * step;
    let item_end = item_start + card;
    let target = if item_start < offset {
        item_start
    } else if item_end > offset + viewport {
        item_end - viewport
    } else {
        return None;
    };
    let content = count as f32 * step - gap;
    Some(target.clamp(0.0, (content - viewport).max(0.0)))
}

/// Position of a tab within the strip: the "all apps" tab is 0, custom groups
/// follow in order. Used to pick the slide direction.
fn tab_position(group: Option<usize>) -> i32 {
    group.map_or(0, |index| index as i32 + 1)
}

fn selected_romm_platform_id(
    section: Section,
    group: Option<usize>,
    config: &AppLibraryConfig,
) -> Option<Option<i64>> {
    if section != Section::ConsoleGames {
        return None;
    }
    match group {
        None => Some(None),
        Some(index) => config
            .sections
            .console_games
            .get(index)
            .and_then(AppGroup::romm_platform_id)
            .map(Some),
    }
}

fn romm_page_is_current(
    response_generation: u64,
    current_generation: u64,
    response_platform: Option<i64>,
    current_platform: Option<Option<i64>>,
) -> bool {
    response_generation == current_generation && current_platform == Some(response_platform)
}

fn next_romm_offset(offset: u32, item_count: u32, total: u64) -> Option<u32> {
    let next = offset.saturating_add(item_count);
    (item_count > 0 && u64::from(next) < total).then_some(next)
}

fn is_primary_window(id: SurfaceId) -> bool {
    id == SurfaceId::RESERVED
}

#[cfg(test)]
mod tests {
    use super::{
        DashboardShelf, HearthDeck, Page, TAB_TRANSITION_DURATION, VirtualKeyboard,
        degraded_providers, focused_entry_index, is_primary_window, next_romm_offset,
        rail_scroll_target, romm_page_is_current, selected_romm_platform_id,
    };
    use crate::app_group::{AppGroup, AppLibraryConfig, FilterType, Section};
    use crate::providers::GameRecord;
    use crate::providers::daemon::{
        HealthResponse, HostCapabilities, ProviderHealthInfo, RetroConsole, RetroGameDetails,
        RetroRecordPage, RetroRomVersion,
    };
    use cosmic::{
        desktop::{DesktopEntryData, fde::IconSource},
        iced::widget,
        widget::icon,
    };
    use std::sync::Arc;

    fn entry(id: &str) -> Arc<DesktopEntryData> {
        Arc::new(DesktopEntryData {
            id: id.into(),
            name: id.into(),
            wm_class: None,
            exec: None,
            icon: IconSource::Name(String::new()),
            path: None,
            categories: Vec::new(),
            desktop_actions: Vec::new(),
            mime_types: Vec::new(),
            prefers_dgpu: false,
            terminal: false,
        })
    }

    fn game_entry(id: &str) -> Arc<DesktopEntryData> {
        let mut data = entry(id).as_ref().clone();
        data.categories = vec!["Game".into()];
        Arc::new(data)
    }

    #[test]
    fn gamepad_navigates_the_open_context_menu() {
        use crate::input_ownership::Event as InputEvent;

        let mut app = HearthDeck {
            entry_path_input: vec![entry("one"), entry("two")],
            ..Default::default()
        };
        app.input_ownership
            .update(InputEvent::SessionObserved(false));
        app.update_entry_metadata();
        app.menu = Some(0);
        app.menu_selection = 0;

        // No group selected and no versions: Run, pin, favorite, controller.
        assert_eq!(app.menu_entry_count(), 4);

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::GamepadEvent(super::GamepadEvent::MoveDown),
        );
        assert_eq!(app.menu_selection, 1);

        // Wraps at the end.
        for _ in 0..3 {
            let _ = <HearthDeck as cosmic::Application>::update(
                &mut app,
                super::Message::GamepadEvent(super::GamepadEvent::MoveDown),
            );
        }
        assert_eq!(app.menu_selection, 0);

        // Keyboard arrows drive the same highlight.
        let _ = <HearthDeck as cosmic::Application>::update(&mut app, super::Message::NextRow);
        assert_eq!(app.menu_selection, 1);

        // Back is a global contract and closes the menu.
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::GamepadEvent(super::GamepadEvent::Back),
        );
        assert!(app.menu.is_none());
    }

    #[test]
    fn gamepad_navigates_the_menu_even_when_a_popup_took_focus() {
        use crate::input_ownership::Event as InputEvent;

        let mut app = HearthDeck {
            entry_path_input: vec![entry("one")],
            ..Default::default()
        };
        app.input_ownership
            .update(InputEvent::SessionObserved(false));
        // A popup taking keyboard focus can leave ownership `Unfocused`; the
        // open menu must still keep the gamepad subscription alive and accept
        // events, or it is dead to the controller.
        app.input_ownership.update(InputEvent::FrontendUnfocused);
        assert!(!app.input_ownership.frontend_has_control());
        app.update_entry_metadata();
        app.menu = Some(0);
        app.menu_selection = 0;

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::GamepadEvent(super::GamepadEvent::MoveDown),
        );
        assert_eq!(app.menu_selection, 1);
    }

    #[test]
    fn gamepad_reaches_the_romm_version_entries() {
        use crate::input_ownership::Event as InputEvent;
        use crate::providers::daemon::RetroRomVersion;

        let mut app = HearthDeck {
            entry_path_input: vec![entry("romm:1")],
            ..Default::default()
        };
        app.input_ownership
            .update(InputEvent::SessionObserved(false));
        app.update_entry_metadata();
        app.romm_versions.insert(
            "romm:1".to_string(),
            vec![
                RetroRomVersion {
                    id: 1,
                    title: "Disc 1".to_string(),
                    is_main_sibling: true,
                },
                RetroRomVersion {
                    id: 2,
                    title: "Disc 2".to_string(),
                    is_main_sibling: false,
                },
            ],
        );
        app.menu = Some(0);
        app.menu_selection = 0;

        // Run, pin, favorite, controller, then the two versions.
        assert_eq!(app.menu_entry_count(), 6);

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::GamepadEvent(super::GamepadEvent::MoveDown),
        );
        assert_eq!(app.menu_selection, 1);
        assert_eq!(app.menu_rom_versions().len(), 2);
    }

    #[test]
    fn virtual_keyboard_ignores_duplicate_visibility_requests() {
        let mut keyboard = VirtualKeyboard::default();

        assert_eq!(keyboard.request(true), Some(true));
        assert_eq!(keyboard.request(true), None);
        assert_eq!(keyboard.complete(true, true), None);
        assert_eq!(keyboard.request(true), None);
    }

    #[test]
    fn virtual_keyboard_queues_hide_while_show_is_pending() {
        let mut keyboard = VirtualKeyboard::default();

        assert_eq!(keyboard.request(true), Some(true));
        assert_eq!(keyboard.request(false), None);
        assert_eq!(keyboard.complete(true, true), Some(false));
        assert_eq!(keyboard.complete(false, true), None);
    }

    #[test]
    fn virtual_keyboard_external_dismiss_does_not_toggle_again() {
        let mut keyboard = VirtualKeyboard::default();

        assert_eq!(keyboard.request(true), Some(true));
        assert_eq!(keyboard.complete(true, true), None);
        keyboard.did_dismiss_externally();
        assert_eq!(keyboard.request(false), None);
    }

    #[test]
    fn virtual_keyboard_ignores_completion_after_external_dismiss() {
        let mut keyboard = VirtualKeyboard::default();

        assert_eq!(keyboard.request(true), Some(true));
        keyboard.did_dismiss_externally();
        assert_eq!(keyboard.complete(true, true), None);
        assert!(!keyboard.visible);
        assert_eq!(keyboard.request(false), None);
    }

    #[test]
    fn only_the_primary_window_is_forced_fullscreen() {
        assert!(is_primary_window(cosmic::iced::window::Id::RESERVED));
        assert!(!is_primary_window(cosmic::iced::window::Id::unique()));
    }

    #[test]
    fn dashboard_is_the_startup_page() {
        assert_eq!(HearthDeck::default().page, Page::Dashboard);
    }

    #[test]
    fn dashboard_routes_to_library_and_back() {
        let mut app = HearthDeck::default();

        let _ = <HearthDeck as cosmic::Application>::update(&mut app, super::Message::OpenLibrary);
        assert_eq!(app.page, Page::Library);

        let _ = <HearthDeck as cosmic::Application>::update(&mut app, super::Message::Close);
        assert_eq!(app.page, Page::Dashboard);
    }

    #[test]
    fn dashboard_ignores_back() {
        let mut app = HearthDeck::default();

        let _ = <HearthDeck as cosmic::Application>::update(&mut app, super::Message::Close);

        assert_eq!(app.page, Page::Dashboard);
    }

    #[test]
    fn dashboard_search_opens_the_library() {
        let mut app = HearthDeck::default();

        let _ = <HearthDeck as cosmic::Application>::update(&mut app, super::Message::OpenSearch);

        assert_eq!(app.page, Page::Library);
        assert_eq!(app.focused_id, Some(super::SEARCH_ID.clone()));
    }

    #[test]
    fn a_page_change_transitions_from_the_previous_page() {
        let mut app = HearthDeck::default();

        let _ = <HearthDeck as cosmic::Application>::update(&mut app, super::Message::OpenLibrary);
        let forward = app.page_animation.expect("forward transition");
        assert_eq!(forward.from, Page::Dashboard);

        let _ = <HearthDeck as cosmic::Application>::update(&mut app, super::Message::Close);
        let back = app.page_animation.expect("back transition");
        assert_eq!(back.from, Page::Library);
    }

    #[test]
    fn reselecting_the_current_page_does_not_transition() {
        let mut app = HearthDeck::default();

        let _ =
            <HearthDeck as cosmic::Application>::update(&mut app, super::Message::OpenDashboard);
        assert!(app.page_animation.is_none());

        let _ = <HearthDeck as cosmic::Application>::update(&mut app, super::Message::OpenSearch);
        let _ = <HearthDeck as cosmic::Application>::update(&mut app, super::Message::OpenLibrary);

        assert_eq!(app.page, Page::Library);
        assert!(app.page_animation.is_none());
    }

    #[test]
    fn finishing_the_transition_drops_it() {
        let mut app = HearthDeck::default();

        let _ = <HearthDeck as cosmic::Application>::update(&mut app, super::Message::OpenLibrary);
        assert!(app.page_animation.is_some());

        // Age the transition past its duration, then deliver a redraw tick.
        age_page_transition(&mut app);
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::Animate(std::time::Instant::now()),
        );

        assert!(app.page_animation.is_none());
        assert_eq!(app.page, Page::Library);
    }

    /// Ages the in-flight page transition past its duration, so the next
    /// message expires it instead of advancing it.
    fn age_page_transition(app: &mut HearthDeck) {
        let animation = app
            .page_animation
            .as_mut()
            .expect("page transition in flight");
        animation.started_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
    }

    /// Ages the in-flight tab slide by `elapsed` so a tick lands at a known
    /// point in the transition.
    fn age_tab_slide(app: &mut HearthDeck, elapsed: std::time::Duration) {
        let animation = app.tab_animation.as_mut().expect("tab slide in flight");
        animation.started_at = std::time::Instant::now() - elapsed;
    }

    #[test]
    fn switching_tabs_starts_a_directional_slide() {
        let mut app = HearthDeck::default();

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::SelectGroup(Some(1)),
        );

        let animation = app.tab_animation.expect("tab slide");
        assert_eq!(animation.target, Some(1));
        assert_eq!(animation.direction, 1.0);
        // The grid is not swapped until the cover is opaque.
        assert_eq!(app.cur_group, None);
    }

    #[test]
    fn tab_direction_follows_strip_order() {
        let mut app = HearthDeck {
            cur_group: Some(2),
            ..Default::default()
        };

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::SelectGroup(None),
        );

        assert_eq!(app.tab_animation.expect("tab slide").direction, -1.0);
    }

    #[test]
    fn reselecting_the_current_tab_does_not_slide() {
        let mut app = HearthDeck::default();

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::SelectGroup(None),
        );

        assert!(app.tab_animation.is_none());
    }

    #[test]
    fn the_tab_slide_swaps_under_cover_then_clears() {
        let mut app = HearthDeck::default();
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::SelectGroup(Some(2)),
        );

        // Midway: the pending tab is applied, still covered.
        age_tab_slide(&mut app, TAB_TRANSITION_DURATION / 2);
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::Animate(std::time::Instant::now()),
        );
        assert_eq!(app.cur_group, Some(2));
        assert!(app.tab_animation.is_some_and(|animation| animation.swapped));

        // Past the end: the slide is dropped.
        age_tab_slide(&mut app, TAB_TRANSITION_DURATION);
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::Animate(std::time::Instant::now()),
        );
        assert!(app.tab_animation.is_none());
    }

    #[test]
    fn dashboard_controller_moves_across_top_navigation() {
        let mut app = HearthDeck {
            focused_id: Some(super::DASHBOARD_HOME_ID.clone()),
            ..Default::default()
        };

        assert_eq!(
            app.dashboard_horizontal_target(1),
            Some(super::DASHBOARD_LIBRARY_ID.clone())
        );
        app.focused_id = Some(super::DASHBOARD_SEARCH_ID.clone());
        assert_eq!(
            app.dashboard_horizontal_target(-1),
            Some(super::DASHBOARD_LIBRARY_ID.clone())
        );
    }

    #[test]
    fn dashboard_favorites_follow_saved_ids_not_catalog_order() {
        let config = AppLibraryConfig {
            favorite_ids: vec!["second".into(), "missing".into(), "first".into()],
            ..Default::default()
        };
        let app = HearthDeck {
            all_entries: vec![entry("first"), entry("second"), entry("other")],
            config,
            ..Default::default()
        };
        let shelves = app.dashboard_shelves();

        assert_eq!(shelves[1].0, DashboardShelf::Favorites);
        assert_eq!(
            shelves[1]
                .1
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["second", "first"]
        );
    }

    #[test]
    fn empty_dashboard_keeps_recent_and_favorites_visible() {
        let app = HearthDeck {
            all_entries: vec![entry("game")],
            ..Default::default()
        };
        let shelves = app.dashboard_shelves();

        assert_eq!(shelves.len(), 3);
        assert_eq!(shelves[0].0, DashboardShelf::Recent);
        assert!(shelves[0].1.is_empty());
        assert_eq!(shelves[1].0, DashboardShelf::Favorites);
        assert!(shelves[1].1.is_empty());
        assert_eq!(shelves[2].0, DashboardShelf::Library);
        assert_eq!(app.dashboard_entry_rows().len(), 1);
    }

    #[test]
    fn dashboard_navigation_moves_between_recent_and_favorite_shelves() {
        let config = AppLibraryConfig {
            favorite_ids: vec!["favorite".into()],
            ..Default::default()
        };
        let mut app = HearthDeck {
            all_entries: vec![entry("favorite")],
            recent_entries: vec![game_entry("recent-a"), game_entry("recent-b")],
            config,
            ..Default::default()
        };
        let rows = app.dashboard_entry_rows();
        app.focused_id = Some(rows[0][1].clone());

        assert_eq!(app.dashboard_vertical_target(1), Some(rows[1][0].clone()));
        assert_eq!(
            app.dashboard_entry_id_for_widget(&rows[0][0]),
            Some("recent-a".into())
        );
    }

    #[test]
    fn dashboard_lists_consoles_from_the_live_platforms() {
        let mut config = AppLibraryConfig::default();
        config.sync_console_groups(&[(7, "SNES".to_string())]);
        // A user-created folder in the console section is not a console.
        config.sections.console_games.push(AppGroup {
            name: "My folder".into(),
            icon: String::new(),
            filter: FilterType::AppIds(vec!["x".into()]),
        });

        let app = HearthDeck {
            config,
            ..Default::default()
        };

        assert_eq!(
            app.dashboard_consoles()
                .into_iter()
                .map(|(index, group)| (index, group.name()))
                .collect::<Vec<_>>(),
            vec![(0, "SNES".to_string())]
        );
        // The console rail leads the dashboard's focusable rows.
        assert_eq!(app.dashboard_entry_rows()[0].len(), 1);
    }

    #[test]
    fn opening_a_dashboard_console_selects_its_tab() {
        let mut config = AppLibraryConfig::default();
        config.sync_console_groups(&[(7, "SNES".to_string()), (9, "Mega Drive".to_string())]);
        let mut app = HearthDeck {
            config,
            ..Default::default()
        };

        let _ =
            <HearthDeck as cosmic::Application>::update(&mut app, super::Message::OpenConsole(1));

        assert_eq!(app.page, Page::Library);
        assert_eq!(app.cur_section, Section::ConsoleGames);
        assert_eq!(app.cur_group, Some(1));
    }

    #[test]
    fn confirming_a_dashboard_console_opens_its_tab() {
        let mut config = AppLibraryConfig::default();
        config.sync_console_groups(&[(7, "SNES".to_string())]);
        let mut app = HearthDeck {
            config,
            ..Default::default()
        };

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::ConfirmFocused(super::dashboard_console_id(0)),
        );

        assert_eq!(app.page, Page::Library);
        assert_eq!(app.cur_section, Section::ConsoleGames);
        assert_eq!(app.cur_group, Some(0));
    }

    #[test]
    fn dashboard_health_only_reports_degraded_providers() {
        let health = |providers| HealthResponse {
            version: "test".into(),
            lan_enabled: false,
            transport: "http".into(),
            providers,
            capabilities: HostCapabilities {
                launch: true,
                application_sessions: true,
                install_requests: false,
                retro_launch: true,
            },
        };
        let provider = |id: &str, status: &str| ProviderHealthInfo {
            id: id.into(),
            status: status.into(),
            record_count: None,
            last_attempt_at: None,
        };

        assert!(degraded_providers(&health(vec![provider("heroic", "ready")])).is_empty());
        assert_eq!(
            degraded_providers(&health(vec![
                provider("heroic", "degraded"),
                provider("desktop-apps", "ready"),
            ])),
            vec!["heroic".to_string()]
        );
    }

    #[test]
    fn dashboard_vertical_navigation_walks_from_nav_into_the_first_row() {
        let mut app = HearthDeck {
            all_entries: vec![entry("game")],
            focused_id: Some(super::DASHBOARD_HOME_ID.clone()),
            ..Default::default()
        };
        let first_entry = app.dashboard_entry_rows()[0][0].clone();

        assert_eq!(app.dashboard_vertical_target(1), Some(first_entry.clone()));
        app.focused_id = Some(first_entry);
        assert_eq!(
            app.dashboard_vertical_target(-1),
            Some(super::DASHBOARD_HOME_ID.clone())
        );
    }

    #[test]
    fn stale_dashboard_focus_does_not_resolve_after_catalog_changes() {
        let mut app = HearthDeck {
            all_entries: vec![entry("old"), entry("current")],
            ..Default::default()
        };
        let stale = app.dashboard_entry_ids()[0].clone();

        app.all_entries = vec![entry("current")];

        assert_eq!(
            focused_entry_index(&stale, &app.dashboard_entry_ids()),
            None
        );
    }

    #[test]
    fn cosmic_session_does_not_start_or_configure_a_panel() {
        let session = include_str!("../../../packaging/arch/cosmic-test-session");
        let package = include_str!("../../../packaging/arch/PKGBUILD");

        assert!(!session.contains(".config/cosmic"));
        assert!(!session.contains("cosmic-panel"));
        assert!(!package.contains("cosmic-panel"));
        assert!(!package.contains("hearthdeck-applet-user"));
    }

    #[test]
    fn non_grid_focus_cannot_activate_a_stale_grid_entry() {
        let app = widget::Id::new("app-entry-example");
        let filter = widget::Id::new("filter");
        let entries = [app.clone()];

        assert_eq!(focused_entry_index(&app, &entries), Some(0));
        assert_eq!(focused_entry_index(&filter, &entries), None);
    }

    #[test]
    fn romm_pages_load_only_for_the_selected_console_scope() {
        let mut config = AppLibraryConfig::default();
        config.sync_console_groups(&[(7, "SNES".to_string())]);

        assert_eq!(
            selected_romm_platform_id(Section::ConsoleGames, None, &config),
            Some(None)
        );
        assert_eq!(
            selected_romm_platform_id(Section::ConsoleGames, Some(0), &config),
            Some(Some(7))
        );
        assert_eq!(
            selected_romm_platform_id(Section::PcGames, Some(0), &config),
            None
        );
    }

    #[test]
    fn romm_paging_rejects_stale_results_and_stops_at_total() {
        assert!(romm_page_is_current(3, 3, Some(7), Some(Some(7))));
        assert!(!romm_page_is_current(2, 3, Some(7), Some(Some(7))));
        assert!(!romm_page_is_current(3, 3, Some(7), Some(Some(9))));

        assert_eq!(next_romm_offset(0, 48, 100), Some(48));
        assert_eq!(next_romm_offset(96, 4, 100), None);
        assert_eq!(next_romm_offset(0, 0, 100), None);
    }

    fn romm_record(id: i64, name: &str, platform_id: i64) -> GameRecord {
        GameRecord {
            id: format!("romm:{id}"),
            name: name.to_string(),
            exec: Some(id.to_string()),
            icon: None,
            path: None,
            categories: vec!["Game".into(), format!("hearthdeck-console:{platform_id}")],
            terminal: false,
            prefers_dgpu: false,
            source: "RomM".into(),
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn romm_pages_append_in_order_instead_of_resorting() {
        let mut config = AppLibraryConfig::default();
        config.sync_console_groups(&[(7, "SNES".to_string())]);
        let mut app = HearthDeck {
            config,
            cur_section: Section::ConsoleGames,
            cur_group: Some(0),
            ..Default::default()
        };

        // Page 0 then page 1, in the daemon's name order. A name sort over the
        // combined catalog would reorder page 1 before page 0 ("Alpha" vs
        // "Zeta"); the grid must instead keep arrival order.
        for (offset, items) in [
            (0_u32, vec![romm_record(1, "Zeta", 7)]),
            (48, vec![romm_record(2, "Alpha", 7)]),
        ] {
            let _ = <HearthDeck as cosmic::Application>::update(
                &mut app,
                super::Message::RommGames {
                    generation: 0,
                    platform_id: Some(7),
                    result: Ok(RetroRecordPage {
                        items,
                        total: 2,
                        offset,
                    }),
                },
            );
        }

        let names: Vec<&str> = app
            .entry_path_input
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["Zeta", "Alpha"]);
        // The last page ends pagination.
        assert_eq!(app.romm_next_offset, None);
        assert!(!app.romm_loading_page);
    }

    #[test]
    fn dashboard_rails_are_the_single_layout_definition() {
        let mut config = AppLibraryConfig::default();
        config.sync_console_groups(&[(7, "SNES".to_string())]);
        let app = HearthDeck {
            config,
            all_entries: vec![entry("game")],
            recent_entries: vec![game_entry("recent")],
            ..Default::default()
        };

        let rails = app.dashboard_rails();
        // Recently Played leads, then the console rail, then the remaining
        // shelves. Focus navigation is derived from the same definition.
        assert_eq!(rails[0].0, DashboardShelf::Recent.key());
        assert_eq!(rails[1].0, super::CONSOLES_RAIL_KEY);
        assert_eq!(
            app.dashboard_entry_rows(),
            rails.into_iter().map(|(_, ids)| ids).collect::<Vec<_>>()
        );
    }

    #[test]
    fn dashboard_recently_played_only_shows_games() {
        let app = HearthDeck {
            recent_entries: vec![entry("app"), game_entry("game")],
            ..Default::default()
        };

        let shelves = app.dashboard_shelves();
        assert_eq!(shelves[0].0, DashboardShelf::Recent);
        assert_eq!(
            shelves[0]
                .1
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["game"]
        );
    }

    #[test]
    fn romm_platform_logos_feed_the_dashboard_rail() {
        let mut app = HearthDeck::default();
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::RommPlatforms(Ok(vec![
                RetroConsole {
                    id: 7,
                    label: "SNES".into(),
                    rom_count: 3,
                    icon: Some("/tmp/snes.png".into()),
                },
                RetroConsole {
                    id: 9,
                    label: "Mega Drive".into(),
                    rom_count: 0,
                    icon: Some("/tmp/mega-drive.png".into()),
                },
            ])),
        );

        assert_eq!(
            app.romm_platform_icons.get(&7).map(String::as_str),
            Some("/tmp/snes.png")
        );
        // A platform with no games gets no tab, but its logo is still cached.
        assert_eq!(app.config.sections.console_games.len(), 1);
        assert!(app.romm_platform_icons.contains_key(&9));
        // The authoritative per-console totals are kept separately from the
        // lazy-loaded grid entries.
        assert_eq!(app.romm_platform_counts.get(&7), Some(&3));
        assert_eq!(app.console_total_count(), Some(3));
    }

    #[test]
    fn console_header_uses_romm_totals_not_the_loaded_pages() {
        let mut config = AppLibraryConfig::default();
        config.sync_console_groups(&[(7, "SNES".to_string())]);
        let mut app = HearthDeck {
            config,
            cur_section: Section::ConsoleGames,
            cur_group: Some(0),
            ..Default::default()
        };
        // Only two entries are lazy-loaded so far, but RomM reports 240 games
        // on the console: the header must show the console total, not 2.
        app.romm_platform_counts.insert(7, 240);
        app.entry_path_input = vec![entry("one"), entry("two")];

        assert_eq!(app.console_total_count(), Some(240));
    }

    #[test]
    fn grouped_list_total_overrides_the_file_count_header() {
        let mut config = AppLibraryConfig::default();
        config.sync_console_groups(&[(7, "SNES".to_string())]);
        let mut app = HearthDeck {
            config,
            cur_section: Section::ConsoleGames,
            cur_group: Some(0),
            ..Default::default()
        };
        // RomM counts every file (240), but grouping collapses regions and
        // discs into fewer tiles; the header must show the grouped total once
        // the list endpoint has reported it.
        app.romm_platform_counts.insert(7, 240);
        app.romm_grouped_total = Some(198);

        assert_eq!(app.console_total_count(), Some(198));
    }

    #[test]
    fn rail_scroll_leaves_a_visible_selection_alone() {
        // 10 cards of 100px with 10px gaps inside a 1000px window: cards 0..=8
        // are fully visible, so moving among them must not move the rail.
        assert_eq!(rail_scroll_target(0, 10, 0.0, 1000.0, 100.0, 10.0), None);
        assert_eq!(rail_scroll_target(3, 10, 0.0, 1000.0, 100.0, 10.0), None);
        assert_eq!(rail_scroll_target(8, 10, 0.0, 1000.0, 100.0, 10.0), None);
    }

    #[test]
    fn rail_scroll_pins_only_once_the_selection_crosses_an_edge() {
        // Card 9 is the first that leaves through the trailing edge.
        assert_eq!(
            rail_scroll_target(9, 10, 0.0, 1000.0, 100.0, 10.0),
            Some(90.0)
        );
        // A card leaving through the leading edge pins to the start, not to
        // the trailing edge, so moving left keeps it just inside the window.
        assert_eq!(
            rail_scroll_target(0, 10, 220.0, 1000.0, 100.0, 10.0),
            Some(0.0)
        );
    }

    #[test]
    fn rail_scroll_clamps_to_the_content_end() {
        // Narrow window: the last card cannot be pinned left of the maximum
        // scroll, or the rail would overscroll past its content.
        assert_eq!(
            rail_scroll_target(9, 10, 0.0, 300.0, 100.0, 10.0),
            Some(790.0)
        );
    }

    /// A 1x1 transparent handle, enough for a row to exist without decoding a
    /// real image.
    fn stub_handle() -> icon::Handle {
        icon::from_raster_pixels(1, 1, vec![0, 0, 0, 0])
    }

    /// An app showing the details screen for `rom_id`, seeded the way
    /// `open_details` seeds it, with `versions` versions and `screenshots`
    /// screenshot thumbnails.
    fn details_app(versions: usize, screenshots: usize) -> HearthDeck {
        use crate::input_ownership::Event as InputEvent;

        let mut app = HearthDeck {
            details: Some(super::Details {
                rom_id: 4014,
                title: "Zoop".into(),
                cover: None,
                hero: None,
                screenshots: vec![stub_handle(); screenshots],
                hero_shot: None,
                versions: (0..versions)
                    .map(|index| RetroRomVersion {
                        id: 4014 + index as i64,
                        title: format!("Disc {}", index + 1),
                        is_main_sibling: index == 0,
                    })
                    .collect(),
                info: None,
                error: None,
                favorite: None,
                backlogged: None,
                state_pending: false,
                disc_picker: false,
                focus: super::DetailsFocus::Play,
                origin_index: 0,
            }),
            ..Default::default()
        };
        app.switch_page(super::Page::Details);
        // The frontend owns input while no session is running, which is what
        // authorizes a launch from the details screen.
        app.input_ownership
            .update(InputEvent::SessionObserved(false));
        app
    }

    #[test]
    fn the_context_menu_button_opens_details_only_for_console_games() {
        use crate::input_ownership::Event as InputEvent;

        let mut app = HearthDeck {
            entry_path_input: vec![entry("org.example.App"), entry("romm:42")],
            ..Default::default()
        };
        app.update_entry_metadata();
        app.switch_page(super::Page::Library);
        // The frontend owns gamepad input while no session is running.
        app.input_ownership
            .update(InputEvent::SessionObserved(false));

        // An application has no details screen yet, so X keeps opening the
        // context menu (its only route to Run/pin/favorite).
        app.focused_id = app.entry_ids.first().cloned();
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::GamepadEvent(super::GamepadEvent::ContextMenu),
        );
        assert!(app.details.is_none());
        assert_eq!(app.page, super::Page::Library);

        // A RomM game gets the details screen instead, seeded from the tile.
        app.focused_id = app.entry_ids.get(1).cloned();
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::GamepadEvent(super::GamepadEvent::ContextMenu),
        );
        let details = app.details.as_ref().expect("details screen");
        assert_eq!(app.page, super::Page::Details);
        assert_eq!(details.rom_id, 42);
        assert_eq!(details.title, "romm:42");
        assert_eq!(details.origin_index, 1);
        // The details screen replaces the menu for games, it does not stack
        // on top of it.
        assert!(app.menu.is_none());
    }

    #[test]
    fn details_back_returns_to_the_tile_that_opened_it() {
        use crate::input_ownership::Event as InputEvent;

        let mut app = HearthDeck {
            entry_path_input: vec![entry("romm:7"), entry("romm:8")],
            ..Default::default()
        };
        app.update_entry_metadata();
        app.switch_page(super::Page::Library);
        app.input_ownership
            .update(InputEvent::SessionObserved(false));
        app.focused_id = app.entry_ids.get(1).cloned();
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::GamepadEvent(super::GamepadEvent::ContextMenu),
        );
        assert_eq!(app.page, super::Page::Details);

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::GamepadEvent(super::GamepadEvent::Back),
        );
        assert_eq!(app.page, super::Page::Library);
        assert_eq!(app.focused_id, app.entry_ids.get(1).cloned());
        // The screen stays alive under the fade, so the outgoing page can still
        // be drawn while the transition runs...
        assert!(app.details.is_some());
        // ...and is dropped once it finishes.
        age_page_transition(&mut app);
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::Animate(std::time::Instant::now()),
        );
        assert!(app.details.is_none());
    }

    #[test]
    fn details_navigation_walks_rows_without_leaving_them() {
        let mut app = details_app(2, 2);

        let move_once = |app: &mut HearthDeck, message: super::Message| {
            let _ = <HearthDeck as cosmic::Application>::update(app, message);
        };

        // Rows are the screenshot strip, then the action line
        // [Back, Play, Favorite, Disc, Play later].
        assert_eq!(app.details_cursor(), Some(super::DetailsFocus::Play));
        move_once(&mut app, super::Message::NextCol);
        assert_eq!(app.details_cursor(), Some(super::DetailsFocus::Favorite));
        move_once(&mut app, super::Message::PrevCol);
        assert_eq!(app.details_cursor(), Some(super::DetailsFocus::Play));
        move_once(&mut app, super::Message::PrevCol);
        assert_eq!(app.details_cursor(), Some(super::DetailsFocus::Back));
        // Clamped at the row's start: no wrapping onto Play later.
        move_once(&mut app, super::Message::PrevCol);
        assert_eq!(app.details_cursor(), Some(super::DetailsFocus::Back));
        // The action line is the last row: nothing below it.
        move_once(&mut app, super::Message::NextRow);
        assert_eq!(app.details_cursor(), Some(super::DetailsFocus::Back));
        move_once(&mut app, super::Message::PrevRow);
        assert_eq!(
            app.details_cursor(),
            Some(super::DetailsFocus::Screenshot(0))
        );
        move_once(&mut app, super::Message::NextCol);
        assert_eq!(
            app.details_cursor(),
            Some(super::DetailsFocus::Screenshot(1))
        );
        // Down from the strip lands on the action line, keeping its column.
        move_once(&mut app, super::Message::NextRow);
        assert_eq!(app.details_cursor(), Some(super::DetailsFocus::Play));
    }

    #[test]
    fn details_action_line_drops_disc_selection_for_single_file_games() {
        let app = details_app(0, 0);

        assert_eq!(
            app.details_rows(),
            vec![vec![
                super::DetailsFocus::Back,
                super::DetailsFocus::Play,
                super::DetailsFocus::Favorite,
                super::DetailsFocus::PlayLater,
            ]]
        );
    }

    #[test]
    fn details_cursor_recovers_when_its_row_disappears() {
        let mut app = details_app(0, 1);
        app.details.as_mut().unwrap().focus = super::DetailsFocus::Screenshot(0);
        // A later load that found no screenshots leaves the cursor on a row
        // that no longer exists: the next move must recover, not do nothing.
        app.details.as_mut().unwrap().screenshots.clear();

        assert_eq!(app.details_cursor(), Some(super::DetailsFocus::Play));
        let _ = <HearthDeck as cosmic::Application>::update(&mut app, super::Message::NextCol);
        assert_eq!(app.details_cursor(), Some(super::DetailsFocus::Favorite));
    }

    #[test]
    fn an_open_disc_picker_replaces_the_pages_rows() {
        let mut app = details_app(2, 1);

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::DetailsToggleDiscPicker,
        );

        // The picker covers the page, so it owns the rows: the cursor starts on
        // the disc Play already launches.
        let details = app.details.as_ref().unwrap();
        assert!(details.disc_picker);
        assert_eq!(app.details_rows().len(), 1);
        assert_eq!(
            app.details_cursor(),
            Some(super::DetailsFocus::DiscChoice(0))
        );

        // Back closes the picker before it leaves the screen.
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::GamepadEvent(super::GamepadEvent::Back),
        );
        assert!(!app.details.as_ref().unwrap().disc_picker);
        assert_eq!(app.page, super::Page::Details);
    }

    #[test]
    fn choosing_a_disc_points_the_screen_at_it_and_drops_stale_state() {
        let mut app = details_app(2, 0);
        app.details.as_mut().unwrap().favorite = Some(true);
        app.details.as_mut().unwrap().disc_picker = true;

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::DetailsSelectDisc(4015),
        );

        let details = app.details.as_ref().unwrap();
        assert_eq!(details.rom_id, 4015);
        assert!(!details.disc_picker);
        // The other file's RomM state has not been read, so the buttons must not
        // inherit the previous file's.
        assert_eq!(details.favorite, None);
        assert_eq!(details.backlogged, None);
        assert_eq!(app.details_cursor(), Some(super::DetailsFocus::Play));
    }

    #[test]
    fn confirming_a_details_action_runs_that_action_only() {
        use crate::providers::daemon::{DaemonClient, DaemonConfig};

        let mut app = details_app(0, 1);
        app.daemon_client = Some(DaemonClient::new(DaemonConfig {
            base_url: "http://127.0.0.1:1".into(),
            token: "test".into(),
        }));

        // A screenshot only swaps the backdrop art; it is not a launch target.
        app.details.as_mut().unwrap().focus = super::DetailsFocus::Screenshot(0);
        let _ = app.activate_details();
        assert_eq!(app.details.as_ref().unwrap().hero_shot, Some(0));
        assert!(!app.launch_state.is_visible());

        // Play launches the file the screen is pointing at.
        app.details.as_mut().unwrap().focus = super::DetailsFocus::Play;
        let _ = app.activate_details();
        assert_eq!(app.launch_state.title(), Some("Zoop"));
    }

    #[test]
    fn a_favorite_press_shows_first_and_settles_on_the_answer() {
        use crate::providers::daemon::{DaemonClient, DaemonConfig};

        let mut app = details_app(0, 0);
        app.daemon_client = Some(DaemonClient::new(DaemonConfig {
            base_url: "http://127.0.0.1:1".into(),
            token: "test".into(),
        }));

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::DetailsToggleFavorite,
        );
        // Applied immediately, and a second press cannot race the first.
        let details = app.details.as_ref().unwrap();
        assert_eq!(details.favorite, Some(true));
        assert!(details.state_pending);
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::DetailsToggleFavorite,
        );
        assert_eq!(app.details.as_ref().unwrap().favorite, Some(true));

        // RomM refusing the write puts the button back where it was.
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::DetailsFavorite {
                rom_id: 4014,
                previous: None,
                result: Err("forbidden".into()),
            },
        );
        let details = app.details.as_ref().unwrap();
        assert_eq!(details.favorite, None);
        assert!(!details.state_pending);
    }

    #[test]
    fn a_play_later_press_toggles_and_ignores_an_answer_for_another_game() {
        use crate::providers::daemon::{DaemonClient, DaemonConfig};

        let mut app = details_app(0, 0);
        app.daemon_client = Some(DaemonClient::new(DaemonConfig {
            base_url: "http://127.0.0.1:1".into(),
            token: "test".into(),
        }));

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::DetailsToggleBacklog,
        );
        assert_eq!(app.details.as_ref().unwrap().backlogged, Some(true));

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::DetailsBacklog {
                rom_id: 999,
                previous: Some(false),
                result: Ok(true),
            },
        );
        let details = app.details.as_ref().unwrap();
        // The screen still points at 4014, so a write about 999 settles nothing.
        assert_eq!(details.backlogged, Some(true));
        assert!(details.state_pending);
    }

    /// A detail payload with every field populated, so a view test exercises
    /// every row and label rather than the early returns for a sparse game.
    fn stub_details() -> RetroGameDetails {
        RetroGameDetails {
            platform_name: Some("Super Nintendo Entertainment System".into()),
            summary: Some("A puzzle game".into()),
            cover_path: Some("/assets/romm/resources/cover.png".into()),
            cover_large_path: None,
            screenshot_paths: vec!["/assets/romm/resources/shot.jpg".into()],
            genres: vec!["Puzzle".into()],
            player_count: Some("1".into()),
            release_year: Some(1995),
            release_date: Some("1995-02-01".into()),
            regions: Vec::new(),
            languages: vec!["English".into()],
            tags: vec!["NA".into()],
            companies: vec!["PanelComp".into()],
            game_modes: vec!["Single player".into()],
            age_ratings: vec!["E".into()],
            average_rating: Some(62.47),
            file_size_bytes: Some(524_288),
            favorite: true,
            backlogged: false,
        }
    }

    #[test]
    fn the_details_view_builds_with_a_fully_populated_game() {
        let mut app = details_app(2, 2);
        app.details.as_mut().unwrap().info = Some(stub_details());

        // Building the view evaluates every localised label the screen uses, so
        // a key missing from the fluent catalogue fails here instead of at
        // runtime. The disc picker is a second layer with labels of its own.
        let _ = app.view_details();
        app.details.as_mut().unwrap().disc_picker = true;
        let _ = app.view_details();
        app.details.as_mut().unwrap().disc_picker = false;

        let details = app.details.as_ref().unwrap();
        // Year, platform, rating, genre, player count, age rating, and the disc
        // the screen is pointing at.
        assert_eq!(details.chips().len(), 7);
        // Companies, play modes, players, released, regions (from the file tags
        // RomM sent, since `regions` is empty), languages and file size.
        assert_eq!(details.facts().len(), 7);
        assert_eq!(details.current_version_label(), Some("Disc 1"));
    }

    #[test]
    fn a_sparse_game_shows_no_empty_chips_or_facts() {
        let mut app = details_app(0, 0);
        app.details.as_mut().unwrap().info = Some(RetroGameDetails {
            platform_name: None,
            summary: None,
            cover_path: None,
            cover_large_path: None,
            screenshot_paths: Vec::new(),
            genres: Vec::new(),
            player_count: None,
            release_year: None,
            release_date: None,
            regions: Vec::new(),
            languages: Vec::new(),
            tags: Vec::new(),
            companies: Vec::new(),
            game_modes: Vec::new(),
            age_ratings: Vec::new(),
            average_rating: None,
            file_size_bytes: None,
            favorite: false,
            backlogged: false,
        });

        let _ = app.view_details();

        let details = app.details.as_ref().unwrap();
        assert!(details.chips().is_empty());
        assert!(details.facts().is_empty());
        // A single-file game does not repeat its title next to Play.
        assert_eq!(details.current_version_label(), None);
    }

    #[test]
    fn stale_details_results_do_not_paint_onto_the_next_screen() {
        let mut app = details_app(0, 0);
        app.details.as_mut().unwrap().rom_id = 99;

        // An answer for the game the user already left is dropped...
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::DetailsLoaded {
                rom_id: 4014,
                result: Err("too slow".to_owned()),
            },
        );
        assert!(app.details.as_ref().unwrap().error.is_none());

        // ...while one for the game being shown is kept.
        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::DetailsLoaded {
                rom_id: 99,
                result: Err("too slow".to_owned()),
            },
        );
        assert_eq!(
            app.details.as_ref().unwrap().error.as_deref(),
            Some("too slow")
        );
    }

    #[test]
    fn shoulder_buttons_do_not_change_section_from_the_details_screen() {
        let mut app = details_app(0, 0);
        let section = app.cur_section;

        let _ = <HearthDeck as cosmic::Application>::update(
            &mut app,
            super::Message::GamepadEvent(super::GamepadEvent::NextGroup),
        );

        assert_eq!(app.cur_section, section);
        assert_eq!(app.page, super::Page::Details);
    }
}
