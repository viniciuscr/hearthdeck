//! The HearthDeck frontend shell: one `App` that owns the state and routes every
//! message, over four pages - dashboard, library, details and settings.
//!
//! This file holds the state, `update`, `view` and the subscriptions. What a
//! message *does* lives beside it, one module per concern: `navigation` (steps,
//! confirm and the single Back), `dashboard_page`, `console_source` (the paged
//! console grid), `settings_source`, `launch`, `details` and `animation`. The
//! screens' own state and views are under `crate::screens`.
//!
//! The sidebar is deliberately *ours* (`screens/library/sidebar.rs`): COSMIC's
//! `nav_bar` was tried here and does not fit the design — its segmented list,
//! type/icon sizes and grey panel are all wrong for the kiosk look, and it is
//! not ours to restyle. That is the line we use to decide "use COSMIC" vs
//! "own it".

mod animation;
mod console_source;
mod dashboard_page;
mod details;
mod launch;
mod navigation;
mod refresh;
mod settings_source;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use crate::library_item::LibraryItem;
use cosmic::app::{Core, Settings, Task};
use cosmic::cosmic_config::{Config, CosmicConfigEntry};
use cosmic::iced::Alignment;
use cosmic::iced::core::keyboard::{Key, key::Named};
use cosmic::iced::core::widget::operation::focusable::focus;
use cosmic::iced::core::window::{Event as WindowEvent, Id as SurfaceId};
use cosmic::iced::event::{Status, listen_with};
use cosmic::iced::widget::stack;
use cosmic::iced::{self, Length, Subscription, runtime, window};
use cosmic::theme;
use cosmic::widget::{Id, button, column, container, context_drawer, icon, row, space, text};
use cosmic::{Application, Element};

use crate::app::animation::{ScrollTarget, Scrolls, section_direction, tab_direction};
use crate::app_group::{AppLibraryConfig, RommFacet, Section, Sections, section_facets};
use crate::fl;
use crate::form_factor::{self, FormFactor};
use crate::input_ownership::{Event as InputEvent, InputOwnership};
use crate::launch_state::{Effect as LaunchEffect, Event as LaunchEvent, LaunchState, Overlay};
use crate::motion::{Fade, Slide};
use crate::providers::daemon::{Collection, DaemonClient, DaemonConfig, RetroDetails};
use crate::providers::filter::Facet;
use crate::screens::dashboard::{self};
use crate::screens::details::{Details as GameDetails, Flag as DetailsFlag};
use crate::screens::library::console::{self, ConsoleScope, ConsoleScopes};
use crate::screens::library::focus::{self, Cursors, Direction, Layout};
use crate::screens::library::sidebar::{self, Sidebar};
use crate::screens::library::{filters, grid, header};
use crate::screens::settings;
use crate::style::{
    DIVIDER_WIDTH, ICON_LARGE, SECTION_TRANSITION_DURATION, TAB_TRANSITION_DURATION, WINDOW_HEIGHT,
    WINDOW_WIDTH, backdrop_fit, content_horizontal_padding, filter_drawer_width,
    grid_viewport_height_fallback, launch_overlay, primary_action_button_class, root_background,
    sidebar_divider,
};
use crate::subscriptions::config_watch::config_watch;
use crate::subscriptions::daemon_events::{FeedEvent, daemon_events};
use crate::subscriptions::gamepad::{GamepadEvent, gamepad_events};
use crate::system_status::SystemStatus;
use crate::text_entry::{Field, TextEntry};
use crate::ui;
use crate::wallpaper::{self, Wallpaper};
use crate::widgets::menu::{self, Menu, Row};
use crate::widgets::top_bar;
use crate::widgets::transition::{PageTransition, SectionTransition, TabTransition};

/// One page of RomM console records per request.
const ROMM_PAGE_SIZE: u32 = 60;

/// How long the launch overlay stays up after the daemon accepts, so the handover
/// to the game is not a flash.
const LAUNCH_OVERLAY_DELAY: std::time::Duration = std::time::Duration::from_millis(1200);

/// How often the frontend re-checks whether a managed game session owns the
/// screen, so input returns when the game exits.
const SESSION_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// How long the console grid waits after the last keystroke before it searches.
///
/// The console grid is searched by the source, so without this every character
/// typed would be a daemon request with a RomM query behind it.
const CONSOLE_SEARCH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);

/// How often the sidebar's free-space figure is re-read.
///
/// Free space moves on the order of minutes, and reading it is a filesystem
/// call, so it rides its own slow poll rather than a frame subscription.
const DISK_FREE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// How often the top bar's system status is re-read.
///
/// Wi-Fi and Bluetooth move on the order of minutes, but this poll is also
/// what makes the bar's clock tick: the view reads the time fresh, and a poll
/// is what makes the view run again. The inherited shell ran the same loop at
/// the same pace.
const SYSTEM_STATUS_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

/// How often the settings screen asks the daemon about a pending Stremio link.
///
/// Short, because somebody is standing in front of the television waiting for
/// the phone in their hand to finish. It is a local call: the daemon is the one
/// that talks to Stremio, and what it reports is already polled and coalesced.
const STREMIO_LINK_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// How long after a link the dashboard's collections are read back.
///
/// Publishing the account's rows is a network round trip to Stremio on the
/// daemon's side, so reading the collections now would find the row still
/// empty. Five seconds matches the delay the categorization screen already uses
/// for a change whose end it cannot see.
const STREMIO_PUBLISH_SETTLE: std::time::Duration = std::time::Duration::from_secs(5);

/// How often the dashboard re-reads its own rails while it is on screen.
///
/// The daemon re-reads the arrivals feed every half hour and says so on its
/// change feed, which is the fast path; this is the floor under it. A kiosk can
/// sit on the dashboard for days, and a feed that never came back would leave
/// the banner on whatever it fetched at startup. An hour keeps the banner well
/// inside the once-a-day it promises, for the cost of one local request.
const DASHBOARD_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Everything the shell can react to.
#[derive(Debug, Clone)]
pub enum Message {
    Romm(settings::romm::Message),
    /// The catalog fetched from the daemon, rendered as tiles, or why it could
    /// not be.
    CatalogLoaded(Result<Vec<Arc<LibraryItem>>, String>),
    /// A page of console records arrived, for the scope and load it was started
    /// for.
    ConsolePageLoaded {
        /// The scope the page belongs to. Pages of different scopes cache side
        /// by side and are never mixed into each other's lists.
        scope: ConsoleScope,
        /// The load this page answers. A page from an older one describes a
        /// reload the user has already superseded, and is dropped.
        generation: u64,
        result: Result<console::Page, String>,
    },
    /// The filter facets the current console scope offers, from the daemon.
    FacetsLoaded(Result<Vec<Facet>, String>),
    /// The dashboard's collections arrived, which is what its rails are made of.
    CollectionsLoaded(Result<Vec<Collection>, String>),
    /// The daemon said something changed (or its feed came back).
    DaemonChanged(FeedEvent),
    /// The settings screen's smart-categories state, as the daemon reports it.
    CategorizationLoaded(Result<crate::providers::daemon::CategorizationSnapshot, String>),
    /// The emulator readiness report the settings card draws.
    RefreshRetroReadiness,
    RetroReadinessLoaded(Result<Vec<crate::providers::daemon::RetroSystemReadiness>, String>),
    /// Places the missing BIOS files from the user's own BIOS source.
    SyncRetroBios,
    RetroBiosSynced(Result<crate::providers::daemon::RetroBiosSyncReport, String>),
    /// Installs the missing libretro core packages through the daemon's
    /// privileged helper.
    InstallRetroCores,
    RetroCoresInstalled(Result<crate::providers::daemon::RetroCoreInstallReport, String>),
    /// A categorization action finished. The shell re-reads the state either way,
    /// because a failed action is still news about what the daemon thinks.
    CategorizationDone(Result<(), String>),
    /// The dashboard's top bar: show the settings screen.
    OpenSettings,
    /// The smart-categories switch was flipped.
    SetCategorization(bool),
    /// Classify the library and publish its dashboard rails.
    ScanCategorization,
    /// Ask the model which applications are couch-friendly.
    DetectCouchFriendly,
    /// The daemon's health reply landed: what this deployment provides at all.
    HealthLoaded(Result<crate::providers::daemon::HealthResponse, String>),
    /// Restore default categories was pressed: every section's tabs go back to
    /// the ones derived from the library and the console platforms.
    /// The Stremio section's link button: ask the daemon for a code.
    StartStremioLink,
    /// The daemon answered the request for a code: the one to approve, or why
    /// it could not issue one.
    StremioLinkStarted(Result<settings::stremio_link::Pending, String>),
    /// One poll of the pending link came back.
    StremioLinkPolled(Result<crate::providers::daemon::StremioLink, String>),
    /// Whether a Stremio account is linked, as the daemon reports it.
    StremioConnectionLoaded(Result<Option<crate::providers::daemon::StremioConnection>, String>),
    /// The Stremio section's unlink button: forget the account.
    UnlinkStremio,
    /// The daemon answered the unlink request.
    StremioUnlinked(Result<(), String>),
    /// The Stremio source was asked to refresh. The failure is all this carries:
    /// there is nothing for the user to do about it, and the provider's own
    /// interval will try again.
    StremioSourceRefreshed(Result<(), String>),
    /// The console list a tab strip is built from: `(platform id, label)`.
    ConsolesLoaded(Result<Vec<crate::providers::daemon::RetroConsole>, String>),
    SelectSection(Section),
    /// The dashboard's top bar: show the dashboard itself.
    OpenDashboard,
    /// Restore dashboard was pressed: drop the model's rails and read everything
    /// the dashboard is drawn from again.
    RestoreDashboard,
    /// The model's rails are gone, or could not be removed.
    ModelRailsRemoved(Result<(), String>),
    /// The dashboard's top bar: show the library as it was left.
    OpenLibrary,
    /// The dashboard's top bar: show the library with the caret in its search
    /// box, which is what this app's search means.
    OpenSearch,
    /// A dashboard card was pressed: launch the item it names.
    ActivateCard {
        /// The item's id, sent exactly as the rail carried it.
        id: String,
        /// What the launch overlay shows while it starts.
        title: String,
    },
    /// A console was selected from the dashboard's consoles rail: switch to
    /// the Console Games section and select that platform's tab.
    ActivateConsole(i64),
    SelectGroup(Option<usize>),
    InputChanged(String),
    /// A text field took the caret. The on-screen keyboard follows that focus -
    /// the compositor activates the input method when the field takes it - so
    /// this is also what tells the shell that the pad belongs to the keyboard
    /// now instead of to the screen behind it.
    TextFieldFocused(Field),
    /// A text field gave the caret up. Only the field that has it can take it
    /// away, because a blur from the field the user just left arrives after the
    /// next field already has the caret.
    TextFieldBlurred(Field),
    /// The focused field was asked to accept what was typed: Enter on a
    /// keyboard, including the on-screen one. The shell decides what accepting
    /// means for each field, so no screen has to know the next step.
    TextFieldSubmitted(Field),
    /// A filter chip was pressed: set (or clear) one facet's value.
    SelectFacet {
        facet: String,
        value: Option<String>,
    },
    /// Open or close the filter sidebar.
    ToggleFilterPanel,
    /// The filter sidebar's own close control was used.
    CloseFilterPanel,
    /// Drop every facet selection.
    ClearFilters,
    /// A tile was pressed: launch the entry it names.
    Activate(usize),
    /// The menu's controller-compatibility row: turn the per-application
    /// pointer-driving mode on or off for this entry. It is what makes an
    /// application that does not handle a controller itself usable with the pad.
    ToggleControllerCompatibility(usize),
    /// The menu's move row: put the application at this tile in the other of
    /// Couch and Desktop, which is what a person does when the guess was wrong.
    MoveToForm(usize, FormFactor),
    /// The pad's context button: open the tile menu on the tile at this index,
    /// or close it when it is already open on that tile.
    TileContext(usize),
    /// The menu's scrim was pressed: close it.
    CloseMenu,
    /// The pad's context button on a console game: show its details screen,
    /// seeded from the tile it was opened from.
    OpenDetails(usize),
    /// Leave the details screen for the grid it was opened from.
    CloseDetails,
    /// The details screen's Play button: launch the file the screen points at.
    DetailsPlay,
    /// A screenshot in the details strip was pressed: make it the big image.
    DetailsSelectScreenshot(usize),
    /// The details screen's favorite button.
    DetailsToggleFavorite,
    /// The details screen's play-later button.
    DetailsToggleBacklog,
    /// The details screen's disc button: open or close the picker.
    DetailsToggleDiscPicker,
    /// A file was picked from the disc picker: point the screen at it.
    DetailsSelectDisc(i64),
    /// One game's detail read arrived, for the file it was started for. The
    /// payload is boxed: `RetroDetails` is large enough that carrying it inline
    /// would make every message carry its size.
    DetailsLoaded {
        rom_id: i64,
        result: Box<Result<RetroDetails, String>>,
    },
    /// A favorite or play-later write settled against the daemon.
    DetailsFlagWritten {
        rom_id: i64,
        flag: DetailsFlag,
        previous: Option<bool>,
        result: Result<bool, String>,
    },
    /// A dashboard rail scrolled: remembered, so the next step can tell whether
    /// the card it lands on would leave the view.
    RailScrolled {
        rail: usize,
        offset: f32,
    },
    /// The dashboard page scrolled, remembered for the same reason a rail's
    /// offset is: the follow needs to know whether the focused rail would leave
    /// the view.
    PageScrolled(f32),
    /// The dashboard's slow poll elapsed while it is on screen: its rails are
    /// read again, whatever the change feed is doing.
    DashboardTick,
    /// The library's tab strip scrolled: remembered, so the next tab the
    /// selection lands on can be told whether it would leave the view.
    TabsScrolled(f32),
    /// A navigation event, from the pad or from the keyboard - both speak this
    /// one vocabulary, so there is a single place that moves the cursor.
    GamepadEvent(GamepadEvent),
    /// Walk the chrome one stop forward or back, which is the keyboard's way to
    /// what the pad reaches by a button.
    ChromeNext,
    ChromePrevious,
    /// The grid's scroll position moved, so the console grid can tell whether
    /// the end of the pages it holds is on screen.
    GridScrolled {
        offset: f32,
        viewport_height: f32,
    },
    /// The console grid's search settled: the user stopped typing.
    ConsoleSearchSettled(String),
    /// The daemon accepted, or refused, a launch.
    DaemonLaunchResult(Result<(), String>),
    /// The launch overlay's failure was dismissed.
    DismissLaunch,
    /// Whether a managed game session currently owns the screen.
    ActiveSessionResult(Result<bool, String>),
    /// The home filesystem's free space, for the sidebar footer.
    DiskFreeLoaded(String),
    /// The top bar's system status: Wi-Fi and Bluetooth, as the poll read them.
    SystemStatusLoaded(SystemStatus),
    /// cosmic-bg's configuration was saved: the wallpaper the dashboard draws
    /// may have moved with it.
    CosmicBackgroundChanged,
    /// The desktop wallpaper, re-read from cosmic-config. `None` when cosmic-bg
    /// has no image configured - a colour background counts as none - and the
    /// dashboard then keeps the theme's own background.
    WallpaperLoaded(Option<Wallpaper>),
    /// Hearthdeck's own settings were saved outside this shell: what the
    /// pages show may have moved with them.
    AppConfigChanged,
    /// The icon worker resolved artwork a view asked for, so the views draw
    /// again and pick it up.
    IconsRendered,
    /// The settings, re-read from cosmic-config. Only what actually changed is
    /// rebuilt, so a save that touches nothing here moves nothing.
    AppConfigLoaded(AppLibraryConfig),
    /// The main window appeared: go fullscreen, matching the session's kiosk
    /// model.
    Opened,
    /// The main window's size, so proportional layout follows it: the window is
    /// the display's own size under the kiosk's compositor, not the size it
    /// opened at.
    WindowResized {
        width: f32,
        height: f32,
    },
    /// The main window gained or lost compositor focus.
    WindowFocusChanged(bool),
    /// A frame from `window::frames()` while a transition is in flight, carrying
    /// the frame time: libcosmic's `update` gets no `now` of its own, so the
    /// clock rides the message.
    Animate(Instant),
}

/// Which page the shell is showing.
///
/// The dashboard is where the app opens, matching the inherited app: a front
/// door onto the library rather than a section of it. Everything that is not the
/// dashboard is reached from it, and Back comes back here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Dashboard,
    Library,
    /// One console game's own screen: play, favorite, play later, and the
    /// files the game collapsed. Opened over the library grid it came from,
    /// which is where Back puts the user back.
    Details,
    /// What Hearthdeck is allowed to do to the library. A sibling of the other
    /// two rather than a page inside the library.
    Settings,
}

impl Page {
    /// The destination this page marks as current on the top bar it draws,
    /// which is what scopes that bar's button ids to the page wearing it - see
    /// [`top_bar::button_id`] for why the ids carry the page at all.
    ///
    /// The two views that wear the bar pass the same destination to
    /// [`App::top_bar`]; the pages that wear chrome of their own instead - the
    /// library, and the details screen over it - have none.
    fn bar_destination(self) -> Option<top_bar::Destination> {
        match self {
            Self::Dashboard => Some(top_bar::Destination::Dashboard),
            Self::Settings => Some(top_bar::Destination::Settings),
            Self::Library | Self::Details => None,
        }
    }
}

/// One thing the settings screen can ask the daemon to do.
///
/// A list rather than a task per button, because they all end the same way: the
/// daemon is asked, and then asked what it now thinks.
enum CategorizationAction {
    SetEnabled(bool),
    Scan,
    DetectCouch,
}

pub struct App {
    core: Core,
    /// Which page the shell is showing.
    page: Page,
    /// The page this one arrived from, while the fade-through between them runs.
    /// Bounded by `style::PAGE_TRANSITION_DURATION` and dropped by
    /// [`App::expire_page_fade`], which runs for every message.
    page_fade: Option<Fade<Page>>,
    /// The timestamp of the current frame, refreshed by [`Message::Animate`]
    /// while a transition is in flight; `view` derives progress from it.
    now: Instant,
    /// The section this one slides to, while the slide runs. The section being
    /// drawn stays the current one until the cover is opaque.
    section_slide: Option<Slide<Section>>,
    /// The tab (group) this one slides to, while the slide runs. The same rule:
    /// the current tab stays until the cover is opaque.
    tab_slide: Option<Slide<Option<usize>>>,
    /// The eased focus-follow scrolls in flight, one per list.
    scrolls: Scrolls,
    config: AppLibraryConfig,
    /// The library's tab strips, derived from the catalog and the consoles.
    tabs: Sections,
    /// The config file the shell writes through, resolved once at startup so the
    /// shell has exactly one write target. `None` when there is no store to
    /// write to, which is also what keeps a test off the real config file.
    config_store: Option<Config>,
    daemon_client: DaemonClient,
    /// The full catalog as the daemon reported it (PC games and applications).
    catalog: Vec<Arc<LibraryItem>>,
    /// Every console scope's records, a page at a time, as the daemon reports
    /// them, cached per scope with the cursor that extends them.
    console: ConsoleScopes,
    /// The console platforms RomM reports, with their cached artwork. Used for
    /// the dashboard's consoles rail and for the library's console tabs.
    console_platforms: Vec<crate::providers::daemon::RetroConsole>,
    /// What the grid draws: the section's source after filtering. The console
    /// grid's facets and search are applied by the daemon (see
    /// `fetch_console_page`); every other section is filtered in memory. Kept
    /// in state so `view` stays pure.
    visible: Vec<Arc<LibraryItem>>,
    /// The facets the current source offers, and the selection keyed by facet id.
    facets: Vec<Facet>,
    /// The daemon's collections, in the order it composes them. The dashboard's
    /// rails are these, so a collection nobody wrote code for - which is what a
    /// feature's own categories, and so the AI's, are - appears without a
    /// frontend change.
    collections: Vec<Collection>,
    /// The settings screen's smart-categories state, as the daemon last reported
    /// it. `None` until the first read lands, which is what lets the screen say
    /// it is checking rather than show a switch in a state nobody reported.
    categorization: Option<crate::providers::daemon::CategorizationSnapshot>,
    /// Whether the deployment provides categorization at all, from `/v1/health`.
    /// `None` until the health reply lands: the switch is not drawn for a
    /// feature this installation does not have, so its answer has to arrive
    /// before the screen can decide what it is.
    categorization_capable: Option<bool>,
    /// What the last categorization action did, for the section's own status
    /// text. There is no toast layer yet, so the section is where the user
    /// reads the outcome.
    /// Why the last categorization action was refused.
    categorization_notice: Option<String>,
    /// The Stremio link flow: the account the daemon holds, the code being
    /// approved elsewhere, and what the shell must do next. The flow is the
    /// decision layer - each answer from the daemon moves it - so nothing in
    /// `update` decides what a reply means, it only runs what the flow asks for.
    stremio: settings::stremio_link::Flow,
    romm: settings::romm::Form,
    /// The emulator-readiness card's state.
    retro_readiness: settings::readiness::Flow,
    facet_selection: BTreeMap<String, String>,
    /// Whether the filter sidebar is open.
    filter_open: bool,
    /// Which text field owns the caret, if one does, which is also what says
    /// the on-screen keyboard is up: focusing a field is what activates the
    /// input method. While a field has it the pad belongs to the keyboard -
    /// every button but Back is dropped before it reaches the screen - so the
    /// app does not move behind the keys the user is pressing.
    text_entry: TextEntry,
    /// The tile menu: which tile it is open on, and which of its rows is
    /// highlighted. The menu owns both, so "is it open" cannot disagree with
    /// "what is it on".
    menu: menu::State,
    cur_section: Section,
    cur_group: Option<usize>,
    /// Where the pad is: the tile it is on and the column it came down, the row
    /// of the open drawer, and the chrome stop the keyboard's ring is on.
    ///
    /// The shell owns this rather than reading iced's focus, because Confirm has
    /// to act on what the user can see is highlighted; iced is told about it
    /// afterwards, to paint the ring.
    cursors: Cursors,
    /// Where the pad is on the dashboard, which is a different surface with a
    /// different shape: rails of cards rather than a grid of tiles.
    dashboard_cursor: dashboard::focus::Cursor,
    /// Which card the window's background is showing, and the fade in flight
    /// from the card it was showing. Kept beside the cursor rather than read
    /// from it, because a change only moves if something records that it moved.
    backdrop_at: dashboard::backdrop::At,
    backdrop_fade: Option<Fade<dashboard::backdrop::At>>,
    /// Where the pad is on the top bar both sibling pages wear: the destination
    /// button at this index, or `None` when the bar does not hold the cursor.
    ///
    /// The bar is the one row the dashboard and the settings screen share, so
    /// its cursor is shell state rather than either page's - but what a step
    /// along it does is the bar's own rule, in `widgets::top_bar`.
    bar_cursor: Option<usize>,
    /// Where each dashboard rail is scrolled to, by rail index. The shell keeps
    /// it because the rail cannot: the movement rules need to know whether the
    /// card the cursor lands on would leave the view.
    rail_offsets: Vec<f32>,
    /// Where the dashboard page is scrolled to, which the rail follow needs the
    /// same way each rail needs its own offset: the dashboard is a stack of rails
    /// and only some of it is on screen.
    page_offset: f32,
    /// Where the library's tab strip is scrolled to. The follow needs it for the
    /// same reason a rail needs its offset: the strip only moves when the tab
    /// the selection lands on would leave the view.
    tabs_offset: f32,
    search_value: String,
    user_name: String,
    /// Free space on the home filesystem, as the last poll read it. Blank until
    /// the first poll answers.
    disk_free: String,
    /// Wi-Fi and Bluetooth, as the last poll read them - what the top bar's
    /// status icons draw. Defaulted, so the bar says "unavailable" rather than
    /// nothing until the first read lands.
    system_status: SystemStatus,
    /// What cosmic-bg is showing as the desktop wallpaper, as the last read got
    /// it: the dashboard's page background. `None` draws the theme's own
    /// background instead.
    wallpaper: Option<Wallpaper>,
    window_width: f32,
    window_height: f32,
    /// Where the grid is scrolled to, and how tall its viewport is. The
    /// virtualization builds the rows between them, and the console source asks
    /// for its next page when they reach the end of the records it holds.
    grid_scroll_offset: f32,
    grid_viewport_height: f32,
    /// Whether the grid's follow is owed a run. The grid is dropped and rebuilt
    /// at its top by every trip off the library - a sibling page, a launch
    /// overlay - and the scroll has to be put back under the focus once the
    /// tree it lands on has settled. Set where the grid is re-shown, taken by
    /// the frame that follows.
    grid_restore_pending: bool,
    /// The launch overlay: progress while a game starts, or its failure.
    launch_state: LaunchState,
    /// Who owns input: the frontend, a launch in flight, or a running session.
    input_ownership: InputOwnership,
    /// The console game's details screen. Seeded when it opens, and kept until
    /// its fade out of view has run: the fade draws the page it is leaving, so
    /// the state outlives the page by exactly that long.
    details: Option<GameDetails>,
    /// Which failed reads have a retry waiting.
    retries: refresh::Retries,
}

impl Application for App {
    type Message = Message;
    type Executor = cosmic::executor::Default;
    type Flags = ();
    const APP_ID: &'static str = "org.hearthdeck.HearthDeck";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(mut core: Core, _flags: ()) -> (Self, Task<Self::Message>) {
        core.set_keyboard_nav(false);
        core.set_app_type(cosmic::core::AppType::Window);
        core.window.use_template = false;

        let config_store = AppLibraryConfig::helper();
        let config = AppLibraryConfig::read(config_store.as_ref());

        let daemon_client = DaemonClient::new(DaemonConfig {
            base_url: std::env::var("HEARTHDECK_BACKEND_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:38400".to_string()),
            token: std::env::var("HEARTHDECK_PAIRING_TOKEN").unwrap_or_default(),
        });
        let mut app = Self {
            core,
            page: Page::default(),
            page_fade: None,
            now: Instant::now(),
            section_slide: None,
            tab_slide: None,
            scrolls: Scrolls::default(),
            config,
            tabs: Sections::default(),
            config_store,
            daemon_client,
            catalog: Vec::new(),
            console: ConsoleScopes::default(),
            console_platforms: Vec::new(),
            visible: Vec::new(),
            facets: Vec::new(),
            collections: Vec::new(),
            facet_selection: BTreeMap::new(),
            filter_open: false,
            text_entry: TextEntry::default(),
            menu: menu::State::default(),
            cur_section: Section::PcGames,
            cur_group: None,
            categorization: None,
            categorization_capable: None,
            categorization_notice: None,
            stremio: settings::stremio_link::Flow::default(),
            romm: settings::romm::Form::default(),
            retro_readiness: settings::readiness::Flow::default(),
            // The first tile: with nothing loaded yet there is nothing to point
            // at, and every read of the cursor tolerates that.
            cursors: Cursors::default(),
            dashboard_cursor: dashboard::focus::Cursor::default(),
            backdrop_at: dashboard::backdrop::At::default(),
            backdrop_fade: None,
            bar_cursor: None,
            rail_offsets: Vec::new(),
            page_offset: 0.0,
            tabs_offset: 0.0,
            search_value: String::new(),
            user_name: crate::system_status::current_user_name(),
            disk_free: String::new(),
            system_status: SystemStatus::default(),
            wallpaper: None,
            window_width: WINDOW_WIDTH,
            window_height: WINDOW_HEIGHT,
            grid_scroll_offset: 0.0,
            grid_viewport_height: grid_viewport_height_fallback(),
            grid_restore_pending: false,
            launch_state: LaunchState::default(),
            input_ownership: InputOwnership::default(),
            details: None,
            retries: refresh::Retries::default(),
        };
        let load = app.load_catalog(std::time::Duration::ZERO);
        let poll = app.poll_active_session(std::time::Duration::ZERO);
        let consoles = app.load_consoles();
        let disk = app.poll_disk_free(std::time::Duration::ZERO);
        let status = app.poll_system_status(std::time::Duration::ZERO);
        let collections = app.load_collections();
        let categorization = app.load_categorization();
        let health = app.load_health();
        // The settings screen's Stremio half has something to read whatever the
        // deployment provides, so it is read here rather than only on the way
        // in: a session can be revoked while the shell is running.
        let stremio = app.load_stremio_connection();
        let wallpaper = App::load_wallpaper();
        (
            app,
            Task::batch([
                load,
                poll,
                consoles,
                disk,
                status,
                collections,
                categorization,
                health,
                stremio,
                wallpaper,
            ]),
        )
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        // A transition must never outlive its duration, whatever message happens
        // to arrive next.
        self.expire_page_fade();
        self.expire_backdrop_fade();
        // The cursor is repaired before anything acts on it: entries load, facets
        // come and go and sections swap, so what it can point at changes under it.
        // The arms that replace what it walks repair it again themselves.
        self.cursors = focus::clamp(self.cursors, self.layout());
        match message {
            Message::CatalogLoaded(result) => {
                // A failed read leaves the library as it was: an empty catalog
                // would also prune the tabs its entries imply, and persist that.
                let mut entries = match result {
                    Ok(entries) => entries,
                    Err(error) => {
                        log::error!("catalog fetch failed: {error}");
                        return self.catalog_failed();
                    }
                };
                self.retries.catalog = false;
                self.config.apply_form_overrides(&mut entries);
                // The feed re-reads on every library event, and most change
                // nothing; replacing the list would throw the cursor to the top.
                if entries == self.catalog {
                    return Task::none();
                }
                self.catalog = entries;
                // The local sections' tabs are the ones their entries imply: a
                // store or category nothing installed carries is not a tab. The
                // catalog the grid draws is therefore what the strip is built
                // from. Console Games is the exception - its tabs are the
                // platforms the daemon reported (see `ConsolesLoaded`).
                self.tabs.sync_category_groups(&self.catalog);
                self.rebuild_local_facets();
                self.refilter();
                return self.list_replaced();
            }
            Message::ConsolesLoaded(result) => {
                let platforms = match result {
                    Ok(platforms) => platforms,
                    Err(error) => {
                        log::error!("console list fetch failed: {error}");
                        return self.consoles_failed();
                    }
                };
                self.retries.consoles = false;
                // None reported: keep the tabs the last answer built.
                if platforms.is_empty() {
                    return Task::none();
                }
                // Store the consoles for the dashboard rail. The rail inserts
                // itself below the arrivals and renumbers the rails under the
                // pad, so the pad is named by the card it was on and put back
                // on it once the list is rebuilt.
                let was = self.dashboard_cursor_card();
                self.console_platforms = platforms.clone();
                let platform_tuples: Vec<(i64, String)> =
                    platforms.into_iter().map(|c| (c.id, c.label)).collect();
                let mut tasks = vec![self.queue_console_prefetch(), self.reland_dashboard(was)];
                if self.tabs.sync_console_groups(&platform_tuples) {
                    // The chrome the ring walks changed, so its stop may no
                    // longer be one that exists.
                    self.cursors = focus::clamp(self.cursors, self.layout());
                    // Indices may have moved, so drop the selection and reload.
                    if self.cur_section == Section::ConsoleGames {
                        self.cur_group = None;
                        self.refilter();
                        tasks.push(self.load_console_page(self.console_scope(), 0));
                        tasks.push(self.load_facets());
                    }
                }
                return Task::batch(tasks);
            }
            Message::ConsolePageLoaded {
                scope,
                generation,
                result,
            } => {
                match result {
                    Ok(page) => {
                        let first_page = page.offset == 0;
                        // The page is logged in its own scope whether or not
                        // that scope is on screen - a warm cache is what the
                        // next tab switch reads - but only the scope the grid
                        // shows may repaint what is drawn.
                        if self.console.apply(&scope, generation, page)
                            && self.cur_section == Section::ConsoleGames
                            && scope == self.console_scope()
                        {
                            self.refilter();
                            if first_page {
                                return Task::batch([
                                    self.list_replaced(),
                                    self.pump_console_prefetch(),
                                ]);
                            }
                        }
                    }
                    Err(error) => {
                        if self.console.failed(&scope, generation) {
                            log::error!("console fetch failed: {error}");
                        }
                    }
                }
                return self.pump_console_prefetch();
            }
            Message::GridScrolled {
                offset,
                viewport_height,
            } => {
                self.grid_scroll_offset = offset;
                self.grid_viewport_height = viewport_height;
                // Reaching the end of the pages loaded so far is what pulls the
                // next one in; a scope with nothing left asks for nothing.
                if self.cur_section == Section::ConsoleGames {
                    let scope = self.console_scope();
                    if let Some(next) = self.console.extend_offset(
                        &scope,
                        offset,
                        viewport_height,
                        grid::metrics(self.window_width),
                    ) {
                        return self.load_console_page(scope, next);
                    }
                }
            }
            Message::ConsoleSearchSettled(value) => {
                // One of these is scheduled per keystroke; the term the user
                // actually stopped on is the one still in the field.
                if value != self.search_value || self.cur_section != Section::ConsoleGames {
                    return Task::none();
                }
                // The query changed, so the scope did: draw the scope's own
                // pages (nothing, while it has none yet) and fetch its first
                // page. The scope the user typed over stays cached for when it
                // comes back to it.
                return self.ensure_console_scope();
            }
            Message::DaemonChanged(event) => return self.daemon_changed(event),
            Message::CollectionsLoaded(result) => {
                match result {
                    Ok(collections) => {
                        // Rails can appear and drop out between reads, which
                        // renumbers every rail: the pad is named by the card it
                        // was on and put back on it.
                        let was = self.dashboard_cursor_card();
                        self.collections = collections;
                        self.retries.collections = false;
                        return self.reland_dashboard(was);
                    }
                    Err(error) => {
                        log::warn!("collection fetch failed: {error}");
                        return self.collections_failed();
                    }
                }
            }
            Message::FacetsLoaded(result) => {
                let facets = result.unwrap_or_else(|error| {
                    log::error!("facet fetch failed: {error}");
                    Vec::new()
                });
                // The daemon reports the facet ids; the labels are ours so they
                // follow the locale, falling back to the source's own label for
                // an id we do not know.
                self.facets = facets
                    .into_iter()
                    .map(|mut facet| {
                        if let Some(known) = RommFacet::from_id(&facet.id) {
                            facet.label = known.name();
                        }
                        facet
                    })
                    .collect();
                // The drawer's rows changed with them, so its cursor may be
                // pointing past the end of the new list.
                self.cursors = focus::clamp(self.cursors, self.layout());
            }
            Message::SelectSection(section) => {
                // The sidebar is the library's own navigation, so reaching for it
                // leaves the dashboard.
                let arriving = self.page != Page::Library;
                self.switch_page(Page::Library);
                self.bar_cursor = None;
                // A slide travels the library's own content column, so it needs
                // that column on screen: a section picked as the page changes, or
                // the section already showing, applies at once instead.
                if arriving || section == self.cur_section {
                    return self.apply_section(section);
                }
                let direction = section_direction(self.cur_section, section);
                self.section_slide = Some(Slide::new(
                    section,
                    direction,
                    Instant::now(),
                    SECTION_TRANSITION_DURATION,
                ));
                return Task::none();
            }
            Message::ActivateConsole(platform_id) => {
                self.switch_page(Page::Library);
                self.bar_cursor = None;
                // A platform with no tab yet opens the "all consoles" tab.
                let group = self
                    .tabs
                    .console_games
                    .iter()
                    .position(|group| group.romm_platform_id() == Some(platform_id));
                return self.apply_section_with_group(Section::ConsoleGames, group);
            }
            Message::SelectGroup(group) => {
                // Re-selecting the tab already showing would be motion for its own
                // sake, so it applies at once.
                if group == self.cur_group {
                    return self.apply_group(group);
                }
                let direction = tab_direction(self.cur_group, group);
                self.tab_slide = Some(Slide::new(
                    group,
                    direction,
                    Instant::now(),
                    TAB_TRANSITION_DURATION,
                ));
                return Task::none();
            }
            Message::InputChanged(value) => {
                self.search_value = value;
                if self.cur_section == Section::ConsoleGames {
                    // The console grid is searched by the source, so the query
                    // waits for the user to stop typing - and the list on
                    // screen waits with it, rather than being replaced by the
                    // empty list of a scope that has not answered yet.
                    return self.settle_console_search();
                }
                self.refilter();
                return self.list_replaced();
            }
            Message::TextFieldFocused(field) => self.text_entry.focus(field),
            Message::TextFieldBlurred(field) => self.text_entry.blur(field),
            Message::TextFieldSubmitted(field) => return self.submit_text_field(field),
            Message::SelectFacet { facet, value } => {
                match value {
                    Some(value) => {
                        self.facet_selection.insert(facet, value);
                    }
                    None => {
                        self.facet_selection.remove(&facet);
                    }
                }
                // The console grid is filtered by the source, so a selection
                // re-queries; a locally-loaded section re-filters in memory.
                if self.cur_section == Section::ConsoleGames {
                    return self.ensure_console_scope();
                }
                self.refilter();
                return self.list_replaced();
            }
            Message::ToggleFilterPanel => {
                // The drawer is a surface over the grid, not a place in it:
                // opening it gives it the pad, and closing it gives the pad back
                // to the tile the user was on, because the cursors kept it.
                self.filter_open = !self.filter_open;
            }
            Message::CloseFilterPanel => self.filter_open = false,
            Message::ClearFilters => {
                self.facet_selection.clear();
                if self.cur_section == Section::ConsoleGames {
                    return self.ensure_console_scope();
                }
                self.refilter();
                return self.list_replaced();
            }
            Message::Activate(index) => return self.activate(index),
            // A card carries the id the daemon composed the rail with, exactly
            // as it came: what that names is the daemon's side of the wire.
            Message::ActivateCard { id, title } => return self.launch_managed(id, title),
            Message::ToggleControllerCompatibility(index) => {
                // Keyed on the entry's id rather than its tile index: the flag
                // belongs to the application, so it must survive the list being
                // filtered or reordered under it.
                let id = self.visible.get(index).map(|entry| entry.id.clone());
                if let Some(id) = id {
                    self.config.toggle_desktop_input(&id);
                    self.persist_config();
                }
            }
            Message::MoveToForm(index, form) => return self.move_to_form(index, form),
            Message::TileContext(index) => {
                // The button that opens the menu is the one that closes it, so a
                // second press hands the pad back to the grid.
                if index < self.visible.len() {
                    self.menu.toggle(index);
                }
            }
            Message::CloseMenu => self.menu.close(),
            Message::OpenDetails(index) => return self.open_details(index),
            Message::CloseDetails => return self.close_details(),
            Message::DetailsPlay => return self.play_details(),
            Message::DetailsSelectScreenshot(index) => {
                if let Some(details) = self.details.as_mut() {
                    details.select_shot(index);
                }
            }
            Message::DetailsToggleFavorite => {
                return self.toggle_details_flag(DetailsFlag::Favorite);
            }
            Message::DetailsToggleBacklog => {
                return self.toggle_details_flag(DetailsFlag::Backlog);
            }
            Message::DetailsToggleDiscPicker => return self.toggle_details_disc_picker(),
            Message::DetailsSelectDisc(rom_id) => return self.select_details_disc(rom_id),
            Message::DetailsLoaded { rom_id, result } => return self.load_details(rom_id, *result),
            Message::DetailsFlagWritten {
                rom_id,
                flag,
                previous,
                result,
            } => return self.settle_details_flag(rom_id, flag, previous, result),
            Message::RailScrolled { rail, offset } => {
                // A report from a scroll the shell is driving describes where
                // the travel happens to be, not where the shell sent it; taking
                // it would make the next follow measure from the wrong place
                // and visibly correct.
                if !self.scrolls.is_moving(ScrollTarget::Rail(rail)) {
                    self.set_rail_offset(rail, offset);
                }
            }
            Message::PageScrolled(offset) => {
                if !self.scrolls.is_moving(ScrollTarget::Page) {
                    self.page_offset = offset;
                }
            }
            Message::DashboardTick => return self.dashboard_ticked(),
            Message::TabsScrolled(offset) => self.tabs_offset = offset,
            Message::OpenDashboard => {
                // Landing on a page hands the cursor to its content, so neither
                // the library's ring nor the bar's row survives the trip.
                self.switch_page(Page::Dashboard);
                self.cursors.ring = None;
                self.bar_cursor = None;
                return self.refresh_dashboard();
            }
            Message::RestoreDashboard => return self.restore_dashboard(),
            Message::ModelRailsRemoved(result) => {
                if let Err(error) = result {
                    log::warn!("could not remove the model's dashboard rails: {error}");
                }
                return self.load_collections();
            }
            Message::OpenLibrary => {
                self.switch_page(Page::Library);
                self.bar_cursor = None;
            }
            Message::OpenSettings => {
                self.switch_page(Page::Settings);
                self.cursors.ring = None;
                self.bar_cursor = None;
                // Opening it asks for fresh reads, rather than showing whatever
                // was fetched at startup. A deployment that has already said it
                // cannot run the feature is not asked for a state it has not got.
                let mut reads = vec![self.load_health()];
                if self.categorization_capable != Some(false) {
                    reads.push(self.load_categorization());
                }
                // The Stremio half is not behind that check: it has something to
                // read whatever the deployment provides, and the code a link is
                // approved with is the one control an unsupported deployment can
                // still offer.
                reads.push(self.load_stremio_connection());
                reads.push(self.load_romm_settings());
                reads.push(self.load_retro_readiness());
                return Task::batch(reads);
            }
            Message::SetCategorization(on) => {
                return self.categorization_action(CategorizationAction::SetEnabled(on));
            }
            Message::ScanCategorization => {
                return self.categorization_action(CategorizationAction::Scan);
            }
            Message::DetectCouchFriendly => {
                return self.categorization_action(CategorizationAction::DetectCouch);
            }
            Message::CategorizationDone(result) => {
                if let Err(error) = result {
                    log::warn!("categorization action failed: {error}");
                    self.categorization_notice = Some(error);
                }
                return self.load_categorization();
            }
            Message::CategorizationLoaded(result) => match result {
                Ok(snapshot) => self.categorization = Some(snapshot),
                Err(error) => log::warn!("categorization read failed: {error}"),
            },
            Message::RefreshRetroReadiness => return self.load_retro_readiness(),
            Message::RetroReadinessLoaded(result) => match result {
                Ok(readiness) => self.retro_readiness.readiness = Some(readiness),
                Err(error) => log::warn!("readiness read failed: {error}"),
            },
            Message::SyncRetroBios => {
                log::info!("readiness sync requested");
                self.retro_readiness.syncing = true;
                self.retro_readiness.notice = None;
                return self.sync_retro_bios();
            }
            Message::InstallRetroCores => {
                log::info!("core install requested");
                self.retro_readiness.installing = true;
                self.retro_readiness.notice = None;
                return self.install_retro_cores();
            }
            Message::RetroCoresInstalled(result) => {
                self.retro_readiness.installing = false;
                match result {
                    Ok(report) => {
                        let notice = settings::readiness::install_notice(&report);
                        if report.failed.is_empty() {
                            log::info!("core install finished: {notice}");
                        } else {
                            log::warn!("core install finished: {notice}");
                        }
                        self.retro_readiness.notice = Some(notice);
                    }
                    Err(error) => {
                        log::warn!("core install failed: {error}");
                        self.retro_readiness.notice = Some(format!("core install failed: {error}"));
                    }
                }
                // The installed flags live in the readiness report: reread it
                // so the rows and the button reflect what just landed.
                return self.load_retro_readiness();
            }
            Message::RetroBiosSynced(result) => {
                self.retro_readiness.syncing = false;
                match result {
                    Ok(report) => {
                        let notice = settings::readiness::sync_notice(&report);
                        if report.failed.is_empty() {
                            log::info!("readiness sync finished: {notice}");
                        } else {
                            log::warn!("readiness sync finished: {notice}");
                        }
                        self.retro_readiness.readiness = Some(report.readiness);
                        self.retro_readiness.notice = Some(notice);
                    }
                    Err(error) => {
                        log::warn!("BIOS sync failed: {error}");
                        self.retro_readiness.notice = Some(format!("BIOS sync failed: {error}"));
                    }
                }
            }
            // The deployment says whether the feature exists here at all, which
            // is the one answer that outranks everything the daemon reports.
            Message::HealthLoaded(result) => match result {
                Ok(health) => {
                    self.categorization_capable = Some(health.capabilities.categorization)
                }
                // The last answer stands: a failed read is not news that the
                // deployment lost the feature.
                Err(error) => log::warn!("health read failed: {error}"),
            },
            Message::Romm(message) => return self.update_romm(message),
            // The Stremio section: the flow decides what an answer means and
            // what must happen next, and the shell only runs that. Nothing here
            // reads a reply to decide anything itself.
            Message::StartStremioLink => return self.start_stremio_link(),
            Message::StremioLinkStarted(result) => {
                let effect = self.stremio.link_started(result);
                return self.run_stremio_effect(effect);
            }
            Message::StremioLinkPolled(result) => {
                let effect = self.stremio.link_polled(result);
                return self.run_stremio_effect(effect);
            }
            Message::StremioConnectionLoaded(result) => self.stremio.connection_read(result),
            Message::UnlinkStremio => return self.unlink_stremio(),
            Message::StremioUnlinked(result) => {
                let effect = self.stremio.unlinked(result);
                return self.run_stremio_effect(effect);
            }
            // Nothing for the user to do about a failed publish: the provider's
            // own interval will try again, and `/v1/health` is where it shows.
            Message::StremioSourceRefreshed(result) => {
                if let Err(error) = result {
                    log::warn!("could not refresh the Stremio source: {error}");
                }
            }
            Message::OpenSearch => {
                self.switch_page(Page::Library);
                // The caret goes in the search box, which the library header
                // draws - so the page has to be the library first.
                return self.focus_search();
            }
            Message::GamepadEvent(event) => {
                // A text field owns the caret, so the on-screen keyboard is up
                // and the pad is how its keys get pressed. Nothing else reaches
                // the screen until the caret is gone - and Back is the one
                // button that does it.
                //
                // This is checked before ownership, deliberately. The field may
                // be in the RomM dialog, and the compositor may consider this
                // window unfocused while the keyboard is up - cosmic-osk hides
                // itself on East without telling anyone - so whether the
                // keyboard is up is a fact about the caret, not about which
                // window is focused. Dropping Back here would leave the pad
                // dead with the keyboard gone.
                //
                // The RomM sign-in dialog is a surface over the settings page,
                // not a place in its rows, so the pad cannot walk into it. It
                // is driven by the one button that means yes and the one that
                // means no, and while it is up nothing behind it may move.
                // Read before ownership as well: losing focus to the keyboard
                // on the way in must not leave the dialog impossible to close.
                //
                // Both surfaces are rungs of `back`, which unwinds them.
                if self.text_entry.is_active() || self.romm.sign_in.is_some() {
                    return match event {
                        GamepadEvent::Back => self.back(),
                        GamepadEvent::Confirm
                            if !self.text_entry.is_active() && self.romm.can_sign_in() =>
                        {
                            self.update(Message::Romm(
                                crate::screens::settings::romm::Message::SignInPressed,
                            ))
                        }
                        _ => Task::none(),
                    };
                }
                // A game owns the screen, or the window is not focused: the
                // pad's events are not ours to act on. (This is what keeps a
                // session's controller input from driving the library behind
                // it.)
                if !self.input_ownership.frontend_has_control() {
                    return Task::none();
                }
                return self.navigate(event);
            }
            Message::ChromeNext => return self.walk_ring(false),
            Message::ChromePrevious => return self.walk_ring(true),
            Message::DaemonLaunchResult(result) => {
                let effect = match result {
                    Ok(()) => {
                        self.input_ownership.update(InputEvent::LaunchAccepted);
                        self.launch_state.update(LaunchEvent::Accepted)
                    }
                    Err(error) => {
                        log::error!("daemon launch failed: {error}");
                        self.input_ownership.update(InputEvent::LaunchFailed);
                        self.launch_state.update(LaunchEvent::Failed(error))
                    }
                };
                if effect == LaunchEffect::DelayDismiss {
                    return Task::perform(tokio::time::sleep(LAUNCH_OVERLAY_DELAY), |_| {
                        cosmic::Action::App(Message::DismissLaunch)
                    });
                }
            }
            Message::DismissLaunch => {
                let launch_was_up = self.launch_state.is_visible();
                self.launch_state.update(LaunchEvent::Dismiss);
                // The launch layer stood in for the grid the whole launch, so
                // the grid it drops here is rebuilt at its top - while a game
                // may run for hours. The follow is owed now, not when the game
                // exits and the user comes back to a grid scrolled away from
                // the tile they launched.
                if launch_was_up && self.page == Page::Library {
                    self.grid_restore_pending = true;
                }
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
            Message::DiskFreeLoaded(label) => {
                // Blank means the filesystem could not be read; keep what was
                // last shown rather than blanking a figure that was valid.
                if !label.is_empty() {
                    self.disk_free = label;
                }
                return self.poll_disk_free(DISK_FREE_POLL_INTERVAL);
            }
            Message::SystemStatusLoaded(status) => {
                self.system_status = status;
                return self.poll_system_status(SYSTEM_STATUS_POLL_INTERVAL);
            }
            Message::CosmicBackgroundChanged => {
                return App::load_wallpaper();
            }
            Message::WallpaperLoaded(wallpaper) => {
                self.wallpaper = wallpaper;
            }
            Message::IconsRendered => return Task::none(),
            Message::AppConfigChanged => {
                return App::load_config();
            }
            Message::AppConfigLoaded(config) => {
                return self.apply_live_config(config);
            }
            Message::Opened => {
                return window::set_mode(SurfaceId::RESERVED, window::Mode::Fullscreen);
            }
            Message::WindowResized { width, height } => {
                self.window_width = width;
                self.window_height = height;
            }
            Message::WindowFocusChanged(focused) => {
                self.input_ownership.update(if focused {
                    InputEvent::FrontendFocused
                } else {
                    InputEvent::FrontendUnfocused
                });
            }
            // The frame clock while motion is in flight. Expiry of the page fade
            // has already run for this message at the top of `update`; what the
            // tick drives is everything else.
            Message::Animate(now) => {
                self.now = now;
                let mut tasks = vec![
                    self.advance_section_slide(),
                    self.advance_tab_slide(),
                    self.advance_scroll(),
                ];
                // The rebuilt grid is only in its lasting tree once the fade is
                // over, so the owed follow waits for that and runs on the first
                // frame after it - the scroll it drives is what lands and stays.
                if self.grid_restore_pending && self.page_fade.is_none() {
                    self.grid_restore_pending = false;
                    tasks.push(self.restore_grid_scroll());
                }
                return Task::batch(tasks);
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        // A fade-through draws both sides: the page being left until the
        // midpoint, the page being arrived at from there. `PageTransition` paints
        // only the near side of the swap each frame.
        if let Some(fade) = &self.page_fade {
            return PageTransition::new(
                self.view_page(*fade.origin()),
                self.view_page(self.page),
                fade.progress(self.now),
            )
            .into();
        }
        self.view_page(self.page)
    }

    fn dialog(&self) -> Option<Element<'_, Self::Message>> {
        crate::screens::settings::romm::sign_in_dialog(&self.romm)
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        // The pad and the keyboard feed one message, because they are one
        // vocabulary: a step is a step whichever made it. Neither is acted on
        // while something else owns the screen - the update arm drops events
        // then - so there is nothing to gate here.
        let navigation = gamepad_events().map(Message::GamepadEvent);
        let daemon = daemon_events(self.daemon_client.clone()).map(Message::DaemonChanged);
        // Only the primary window drives the layout; dialogs are separate
        // surfaces and must not overwrite the width.
        let input = listen_with(|event, status, id| match event {
            cosmic::iced::Event::Window(WindowEvent::Opened { .. })
                if id == SurfaceId::RESERVED =>
            {
                Some(Message::Opened)
            }
            cosmic::iced::Event::Window(WindowEvent::Focused) if id == SurfaceId::RESERVED => {
                Some(Message::WindowFocusChanged(true))
            }
            cosmic::iced::Event::Window(WindowEvent::Unfocused) if id == SurfaceId::RESERVED => {
                Some(Message::WindowFocusChanged(false))
            }
            cosmic::iced::Event::Window(WindowEvent::Resized(size))
                if id == SurfaceId::RESERVED =>
            {
                Some(Message::WindowResized {
                    width: size.width,
                    height: size.height,
                })
            }
            // Keys are ours only when nothing else has claimed them: a focused
            // text input takes the arrows for its own caret, so typing in the
            // search box never moves the cursor.
            cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                key,
                modifiers,
                ..
            }) if status == Status::Ignored => match key {
                Key::Named(Named::ArrowUp) => Some(Message::GamepadEvent(GamepadEvent::MoveUp)),
                Key::Named(Named::ArrowDown) => Some(Message::GamepadEvent(GamepadEvent::MoveDown)),
                Key::Named(Named::ArrowLeft) => Some(Message::GamepadEvent(GamepadEvent::MoveLeft)),
                Key::Named(Named::ArrowRight) => {
                    Some(Message::GamepadEvent(GamepadEvent::MoveRight))
                }
                Key::Named(Named::Enter) => Some(Message::GamepadEvent(GamepadEvent::Confirm)),
                // The chrome, which the arrows deliberately never reach. Tab
                // walks it the way it walks the controls of any desktop window,
                // and walking off either end puts the cursor back in the grid.
                Key::Named(Named::Tab) if modifiers.shift() => Some(Message::ChromePrevious),
                Key::Named(Named::Tab) => Some(Message::ChromeNext),
                _ => None,
            },
            // Escape leaves whatever is over the screen, and is read on release
            // so that a surface which closed on the press does not also close
            // what was behind it. Its status is deliberately not checked: a
            // focused input must not be able to trap Back.
            cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyReleased {
                key: Key::Named(Named::Escape),
                ..
            }) => Some(Message::GamepadEvent(GamepadEvent::Back)),
            _ => None,
        });
        let mut subs = vec![navigation, input, daemon];
        // The dashboard's wallpaper follows cosmic-bg live: its config is one
        // of the watched settings, so changing the background in COSMIC
        // Settings reaches the shell without a restart.
        let wallpaper = config_watch(wallpaper::CONFIG_ID, wallpaper::CONFIG_VERSION)
            .map(|_keys| Message::CosmicBackgroundChanged);
        subs.push(wallpaper);
        // Hearthdeck's own settings ride the same wire: a save from anywhere -
        // this shell, a sync tool, a dotfiles restore - reaches the pages
        // without a restart.
        let settings = config_watch(crate::config::APP_ID, AppLibraryConfig::version())
            .map(|_keys| Message::AppConfigChanged);
        subs.push(settings);
        subs.push(crate::icon_cache::rendered_icons().map(|()| Message::IconsRendered));
        // The frame clock runs only while motion is in flight: an idle window
        // schedules no redraws at all, and an ungated frame subscription would pin
        // the compositor at the refresh rate forever.
        if self.is_animating() {
            subs.push(window::frames().map(|(_window, at)| Message::Animate(at)));
        }
        // The dashboard's own slow read: the page on screen keeps following the
        // daemon at least this often, whatever the change feed is doing.
        if refresh::dashboard_tick_runs(self.page) {
            subs.push(iced::time::every(DASHBOARD_POLL_INTERVAL).map(|_| Message::DashboardTick));
        }
        Subscription::batch(subs)
    }
}

impl App {
    /// One page, drawn as the whole window. This is the dispatcher a fade-through
    /// needs: rendering both sides of a transition means asking for a page by
    /// value rather than for whichever page happens to be current.
    fn view_page(&self, page: Page) -> Element<'_, Message> {
        match page {
            // The dashboard is a sibling of the library, not a page inside it: it
            // carries its own top bar and has no sidebar, so it is the whole
            // window. Drawing the library's shell around it would make it read as a
            // section of the library, which is exactly what it is not.
            Page::Dashboard => self.view_dashboard(),
            // Settings is a sibling of the library as well, and carries no sidebar
            // of its own.
            Page::Settings => self.view_settings(),
            Page::Library => self.view_library(),
            // The details screen is the console game's own page. It carries no
            // sidebar - it is over the grid it was opened from.
            Page::Details => self.view_details(),
        }
    }
    /// The library: the sidebar, the content column beside it, and the surfaces
    /// that open over both.
    fn view_library(&self) -> Element<'_, Message> {
        let sidebar = sidebar::view(
            Sidebar {
                current: self.cur_section,
                user_name: &self.user_name,
                disk_free: &self.disk_free,
                app_icon: ui::app_icon(),
                window_width: self.window_width,
            },
            Message::SelectSection,
        );

        let divider = container(space::horizontal())
            .width(Length::Fixed(DIVIDER_WIDTH))
            .height(Length::Fill)
            .class(theme::Container::Custom(Box::new(sidebar_divider)));

        // A launch takes over the content column: the game is starting, so the
        // library behind it is no longer what the user is acting on.
        let content: Element<'_, Message> = if let Some(overlay) = self.launch_state.overlay() {
            view_launch_overlay(overlay)
        } else {
            let header = header::view(header::Header {
                section: self.cur_section,
                selected_group: self.cur_group,
                groups: self.tabs.get(self.cur_section),
                search_value: &self.search_value,
                filter_active: self.facet_selection.len(),
                show_filter: !self.facets.is_empty(),
            });

            let body: Element<'_, Message> = if self.visible.is_empty() {
                container(space::horizontal())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                grid::view(grid::Grid {
                    entries: &self.visible,
                    window_width: self.window_width,
                    scroll_offset: self.grid_scroll_offset,
                    viewport_height: self.grid_viewport_height,
                    // The same question `focus_task` answers, so the border
                    // and the label land on one tile together.
                    focused_tile: focus::focused_tile(self.cursors, self.layout()),
                })
            };

            // A tab change slides the grid sideways, under the cover: the strip
            // above it is navigation, and stays put.
            let body: Element<'_, Message> = match &self.tab_slide {
                Some(slide) => {
                    TabTransition::new(body, slide.progress(self.now), slide.direction()).into()
                }
                None => body,
            };

            let column: Element<'_, Message> = container(column![header, body])
                .width(Length::Fill)
                .height(Length::Fill)
                .padding([0, content_horizontal_padding()])
                .into();

            // A section change slides the whole content column - title, tabs and
            // grid together - while the sidebar stays put as the fixed navigation.
            let page: Element<'_, Message> = match &self.section_slide {
                Some(slide) => {
                    SectionTransition::new(column, slide.progress(self.now), slide.direction())
                        .into()
                }
                None => column,
            };

            // The filters live in a sidebar the header's own button toggles, the
            // way the console drawer did - one filter surface for every section,
            // driven by whatever facets the source reported, not a strip.
            if self.filter_open && !self.facets.is_empty() {
                context_drawer(
                    Some(fl!("filter").into()),
                    None,
                    None,
                    None,
                    Message::CloseFilterPanel,
                    page,
                    filters::view(filters::Filters {
                        facets: &self.facets,
                        selected: &self.facet_selection,
                        cursor: self.filter_open.then_some(self.cursors.filter_row),
                    }),
                    filter_drawer_width(self.window_width),
                )
                .into()
            } else {
                page
            }
        };

        // The menu is a surface over the page: it is stacked over the content
        // column, so it draws over the grid and the filter drawer alike, and its
        // scrim dismisses it. The rows come from the entry it is open on, which
        // is why the menu is built here and not in the grid.
        let content: Element<'_, Message> = match self.menu_target() {
            Some(target) => {
                let title = self
                    .visible
                    .get(target)
                    .map_or("", |entry| entry.name.as_str());
                stack![
                    content,
                    Menu::new(
                        title,
                        self.menu_rows_for(target),
                        self.menu.selected(),
                        Message::CloseMenu,
                    )
                ]
                .into()
            }
            None => content,
        };

        container(row![sidebar, divider, content])
            .width(Length::Fill)
            .height(Length::Fill)
            .class(theme::Container::Custom(Box::new(root_background)))
            .into()
    }
    /// The dashboard screen: the app's front door, with chrome of its own.
    ///
    /// It is drawn as the whole window rather than as the library's content, so
    /// the library's sidebar never appears around it - the two are sibling
    /// destinations, and the sidebar belongs to the library.
    fn view_dashboard(&self) -> Element<'_, Message> {
        let shelves = self.dashboard_shelves();
        let screen = dashboard::view(
            &shelves,
            self.dashboard_cursor,
            &self.top_bar(top_bar::Destination::Dashboard),
            self.window_width,
            self.window_height,
            navigate,
            |rail, offset| Message::RailScrolled { rail, offset },
            Message::PageScrolled,
        );
        // A launch is a layer over whatever page is behind it, the dashboard
        // included.
        let screen: Element<'_, Message> = if let Some(overlay) = self.launch_state.overlay() {
            stack![screen, view_launch_overlay(overlay)].into()
        } else {
            screen
        };
        // The focused title's wide art fills the window behind the page,
        // cross-fading as the pad walks; wherever the pad is on a card whose
        // feed sent no art, this draws nothing and the wallpaper shows.
        let screen: Element<'_, Message> =
            match dashboard::backdrop::view(&shelves, self.backdrop_motion()) {
                Some(art) => stack![art, screen].into(),
                None => screen,
            };
        // The dashboard wears the desktop: whatever cosmic-bg is showing is
        // drawn full-bleed as the page, over the theme's own background - which
        // shows through where the image does not reach, and stands in for it
        // when there is no image at all. The library keeps the plain surface:
        // its grid is dense art already.
        let page: Element<'_, Message> = match self.wallpaper.as_ref() {
            Some(wallpaper) => stack![
                backdrop_fit(&wallpaper.handle, wallpaper.fit, Length::Fill, Length::Fill),
                screen
            ]
            .into(),
            None => screen,
        };
        container(page)
            .width(Length::Fill)
            .height(Length::Fill)
            .class(theme::Container::Custom(Box::new(root_background)))
            .into()
    }
    /// The settings screen, drawn as the whole window for the same reason the
    /// dashboard is: it is a sibling destination, and the library's sidebar
    /// belongs to the library.
    fn view_settings(&self) -> Element<'_, Message> {
        container(settings::view(
            &settings::Settings {
                categorization: self.categorization.as_ref(),
                capable: self.categorization_capable,
                notice: self.categorization_notice.as_deref(),
                stremio: &self.stremio,
                romm: &self.romm,
                readiness: &self.retro_readiness,
            },
            &self.top_bar(top_bar::Destination::Settings),
            navigate,
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .class(theme::Container::Custom(Box::new(root_background)))
        .into()
    }
    /// The chrome every top-level destination wears, marked with the one that is
    /// drawing it - so the bar shows where you are as well as where you can go.
    fn top_bar(&self, current: top_bar::Destination) -> top_bar::TopBar<'_> {
        top_bar::TopBar {
            current,
            user_name: self.user_name.clone(),
            app_icon: ui::app_icon(),
            status: &self.system_status,
        }
    }
    /// Re-projects the current section into `visible`.
    ///
    /// The console grid arrives already narrowed: the daemon applies the facet
    /// selection and the search term to the source's own query, so the pages it
    /// hands over are drawn as they are. Every other section is filtered here,
    /// in memory, over the catalog it already holds.
    ///
    /// The cursor is re-seated here because the list is what it walks: every
    /// path that replaces `visible` comes through this, so there is one place
    /// that knows the cursor may now point past the end of what exists.
    fn refilter(&mut self) {
        if self.cur_section == Section::ConsoleGames {
            // Only the scope on screen's own records: one scope's pages are
            // never another scope's list.
            self.visible = self.console.entries(&self.console_scope()).to_vec();
        } else {
            self.visible = self.tabs.filtered_by_facets(
                self.cur_section,
                self.cur_group,
                &self.search_value,
                &self.catalog,
                &self.facet_selection,
            );
        }
        // A menu belongs to a tile, and the tile it was opened on may no longer
        // be in the list: a search, a tab and a section all replace it.
        if self
            .menu
            .target()
            .is_some_and(|target| target >= self.visible.len())
        {
            self.menu.close();
        }
        self.cursors = focus::clamp(self.cursors, self.layout());
    }
    /// What a list that has just been replaced asks for: the grid drawn from its
    /// first row, and the pad put on the first tile.
    ///
    /// A new list is read from its start, and a cursor left deep in the old one
    /// would point at a tile that is no longer on screen. It is not moved at all
    /// when the search box narrowed the list: that happens on every keystroke,
    /// and taking the caret away mid-word is not a redraw.
    fn list_replaced(&mut self) -> Task<Message> {
        self.grid_scroll_offset = 0.0;
        self.cursors.tile = 0;
        self.cursors.column = 0;
        let drawn = grid::scroll_to_top();
        // A search box that replaced the list keeps the caret: this runs on
        // every keystroke, and focusing the grid here would take the caret - and
        // with it the keyboard - out of the box mid-word.
        if self.search_value.is_empty() && !self.text_entry.is_active() {
            return Task::batch([drawn, self.focus_task()]);
        }
        drawn
    }
    /// What the navigation can move between right now, derived from the state
    /// the view reads.
    ///
    /// The drawer's row count is here whether or not it is open, because that is
    /// what keeps its cursor valid while it is closed; `drawer_open` is what
    /// gives it the pad.
    fn layout(&self) -> Layout {
        Layout {
            sections: Section::ALL.len(),
            tabs: self.tabs.get(self.cur_section).len() + 1,
            filter_button: !self.facets.is_empty(),
            facets: self.facets.len(),
            drawer_open: self.filter_open && !self.facets.is_empty(),
            tiles: self.visible.len(),
            columns: grid::metrics(self.window_width).columns,
        }
    }
    /// The tile the menu is open on, when that tile is still in the list.
    ///
    /// A menu belongs to a tile, and the list under it can be replaced - a
    /// search narrows, a tab or a section swaps - so the target is only good
    /// while the tile it names is still there.
    fn menu_target(&self) -> Option<usize> {
        self.menu
            .target()
            .filter(|target| *target < self.visible.len())
    }
    /// How many rows the open menu holds.
    ///
    /// Counted from the same list the view draws, so the two cannot disagree
    /// about how far the pad can walk.
    fn menu_rows(&self) -> usize {
        self.menu_target()
            .map_or(0, |target| self.menu_rows_for(target).len())
    }
    /// The rows the menu shows for the tile at `index`.
    ///
    /// This is the only place they are defined: the navigation counts them and
    /// the view draws them, so a row cannot appear in one and be missing from
    /// the other.
    ///
    /// A row is a label and the message it sends, so a switch carries its state
    /// in the label - there is no tick to draw beside it.
    fn menu_rows_for(&self, index: usize) -> Vec<Row<Message>> {
        let Some(entry) = self.visible.get(index) else {
            return Vec::new();
        };
        let compatibility = if self.config.desktop_input_for(entry) {
            fl!("disable-controller-compatibility")
        } else {
            fl!("enable-controller-compatibility")
        };
        let mut rows = vec![
            Row::new(fl!("run"), Message::Activate(index)),
            Row::new(compatibility, Message::ToggleControllerCompatibility(index)),
        ];
        if self.cur_section == Section::Applications {
            let other = form_factor::of(entry).other();
            let label = match other {
                FormFactor::Couch => fl!("move-to-couch"),
                FormFactor::Desktop => fl!("move-to-desktop"),
            };
            rows.push(Row::new(label, Message::MoveToForm(index, other)));
        }
        rows
    }
    /// Runs the row the open menu has highlighted.
    ///
    /// The menu closes before the row acts, so nothing is done from behind it: a
    /// launch must not leave a menu on screen, and an action that replaces the
    /// list must not leave one open on a tile that is gone. The highlighted row
    /// is therefore read before the close, which resets it.
    fn activate_menu_row(&mut self) -> Task<Message> {
        let Some(target) = self.menu_target() else {
            return Task::none();
        };
        let selected = self.menu.selected();
        let action = self.menu_rows_for(target).into_iter().nth(selected);
        self.menu.close();
        match action {
            Some(row) => self.update(row.message),
            None => Task::none(),
        }
    }
    /// Whether the menu is up with something in it. A menu with no rows is not a
    /// surface, so it does not take the navigation.
    fn menu_is_up(&self) -> bool {
        self.menu.is_open() && self.menu_rows() > 0
    }
    /// Steps the open menu's highlight. A menu is a list, so both vertical
    /// directions walk it and wrap; there is nothing beside a row to step to.
    fn move_menu(&mut self, direction: Direction) -> Task<Message> {
        let rows = self.menu_rows();
        match direction {
            Direction::Up => self.menu.move_selection(-1, rows),
            Direction::Down => self.menu.move_selection(1, rows),
            Direction::Left | Direction::Right => {}
        }
        Task::none()
    }
    /// The locally-loaded sections derive their facets from the catalog they
    /// show; only the console grid's come from the daemon.
    fn rebuild_local_facets(&mut self) {
        if self.cur_section != Section::ConsoleGames {
            self.facets = section_facets(self.cur_section, &self.catalog);
        }
    }
    /// Puts the application at tile `index` in `form`, and remembers the choice.
    ///
    /// The Desktop tab exists only while something wants it, so the strip is
    /// rebuilt too, and a selection left past its end falls back to the default
    /// view.
    fn move_to_form(&mut self, index: usize, form: FormFactor) -> Task<Message> {
        let Some(id) = self.visible.get(index).map(|entry| entry.id.clone()) else {
            return Task::none();
        };
        self.config.set_form_override(&id, form);
        if let Some(entry) = self.catalog.iter_mut().find(|entry| entry.id == id) {
            form_factor::set(Arc::make_mut(entry), form);
        }
        self.tabs.sync_category_groups(&self.catalog);
        self.persist_config();
        let tabs = self.tabs.get(self.cur_section).len();
        if self.cur_group.is_some_and(|group| group >= tabs) {
            self.cur_group = None;
        }
        self.rebuild_local_facets();
        self.refilter();
        self.focus_task()
    }

    /// Writes the config back to disk, so the user's choices survive a restart.
    fn persist_config(&self) {
        if let Some(store) = self.config_store.as_ref()
            && let Err(error) = self.config.write_entry(store)
        {
            // A failed write does not undo the change - the in-memory state is
            // still what the user asked for - but it must not be silent, or a
            // restart looks like it lost their work.
            log::warn!("config write failed: {error}");
        }
    }
    /// Reads the home filesystem's free space, and schedules the next read.
    ///
    /// The read is a filesystem call, so it happens in the effect rather than in
    /// `update`: the shell stores the answer when it comes back.
    fn poll_disk_free(&self, delay: std::time::Duration) -> Task<Message> {
        Task::perform(
            async move {
                tokio::time::sleep(delay).await;
                crate::system_status::disk_free_label()
            },
            |label| cosmic::Action::App(Message::DiskFreeLoaded(label)),
        )
    }
    /// Reads Wi-Fi and Bluetooth, and schedules the next read - which is also
    /// what makes the top bar's clock tick, since the view reads the time fresh
    /// and a poll is what makes the view run again.
    fn poll_system_status(&self, delay: std::time::Duration) -> Task<Message> {
        Task::perform(
            async move {
                tokio::time::sleep(delay).await;
                SystemStatus::load().await
            },
            |status| cosmic::Action::App(Message::SystemStatusLoaded(status)),
        )
    }
    /// Reads the desktop wallpaper cosmic-bg has configured, and reports it.
    ///
    /// The read is a config lookup plus a possible folder listing, so it
    /// happens in the effect rather than in `update`.
    fn load_wallpaper() -> Task<Message> {
        Task::perform(async { wallpaper::load() }, |wallpaper| {
            cosmic::Action::App(Message::WallpaperLoaded(wallpaper))
        })
    }
    /// Re-reads the app's own settings.
    ///
    /// The read is a few config lookups, so it happens in the effect rather
    /// than in `update`.
    fn load_config() -> Task<Message> {
        Task::perform(
            async {
                let store = AppLibraryConfig::helper();
                AppLibraryConfig::read(store.as_ref())
            },
            |config| cosmic::Action::App(Message::AppConfigLoaded(config)),
        )
    }
}

/// The launch overlay: progress while a game starts, or the failure with a way
/// to dismiss it. It draws an [`Overlay`] - a value only a launch in flight or
/// a failure produces - so the layer cannot cover a page while nothing is
/// being launched.
fn view_launch_overlay(overlay: Overlay<'_>) -> Element<'_, Message> {
    let spacing = theme::spacing();
    let content: Element<'_, Message> = match overlay {
        Overlay::Failed { title, error } => column![
            icon::icon(icon::from_name("dialog-error-symbolic").into()).size(ICON_LARGE),
            text::title2(fl!("launch-failed")),
            text::body(title),
            text::caption(error),
            button::custom(text::body(fl!("dismiss")))
                .class(primary_action_button_class())
                .on_press(Message::DismissLaunch)
                .padding([spacing.space_xs, spacing.space_l]),
        ]
        .spacing(spacing.space_s)
        .align_x(Alignment::Center)
        .into(),
        Overlay::Progress { title } => column![
            icon::icon(ui::app_icon()).size(ICON_LARGE),
            text::title2(fl!("launching", title = title)),
        ]
        .spacing(spacing.space_m)
        .align_x(Alignment::Center)
        .into(),
    };

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .class(theme::Container::Custom(Box::new(launch_overlay)))
        .into()
}

/// Where the top bar's buttons go.
///
/// A plain function rather than a closure, because the bar takes it as one: the
/// mapping is the same for every screen that wears the bar.
fn navigate(destination: top_bar::Destination) -> Message {
    match destination {
        top_bar::Destination::Dashboard => Message::OpenDashboard,
        top_bar::Destination::Library => Message::OpenLibrary,
        top_bar::Destination::Search => Message::OpenSearch,
        top_bar::Destination::Settings => Message::OpenSettings,
    }
}

/// The widget id of a dashboard card: the rail's slug and the card's place in
/// it, so the pad can be pointed at it and a card keeps its identity while the
/// rails around it change.
fn dashboard_card_id(slug: &str, index: usize) -> Id {
    Id::new(format!("dashboard-card-{slug}-{index}"))
}

/// What pressing a dashboard card does: launch the item it names.
///
/// The card carries the id the daemon composed the rail with, and it goes out
/// exactly as it came - this client never parses a prefix to choose a route.
/// The lookup against its own catalog that used to decide whether a card did
/// anything is what left every film and console game dead: media records are
/// not in the application library at all. Which ids this daemon can launch,
/// and what that means, is decided on the other side of the wire.
fn launch_card_message(id: &str, title: &str) -> Message {
    Message::ActivateCard {
        id: id.to_owned(),
        title: title.to_owned(),
    }
}

/// Creates a message to activate a console from the dashboard's consoles rail.
fn launch_console_message(platform_id: i64) -> Message {
    Message::ActivateConsole(platform_id)
}

/// Puts the renderer's focus on one widget.
///
/// The shell's cursor decides what the user is on; this only reproduces it in
/// iced, which is where a button's `focused` style flag - and so the design
/// system's focus ring - comes from.
fn widget_focus(id: Id) -> Task<Message> {
    runtime::task::widget(focus(id))
}

/// Takes the renderer's focus off whatever holds it.
///
/// A text field that loses focus this way stops asking for the input method,
/// and the compositor takes the on-screen keyboard down with it - so this is
/// how Back dismisses the keyboard without also leaving the screen.
fn unfocus() -> Task<Message> {
    use cosmic::iced::core::widget::operation::focusable;

    runtime::task::widget(focusable::unfocus())
}

pub fn run() -> cosmic::iced::Result {
    let settings = Settings::default()
        .antialiasing(true)
        .client_decorations(true)
        .debug(false)
        .default_text_size(16.0)
        .scale_factor(1.0)
        .size(iced::Size::new(WINDOW_WIDTH, WINDOW_HEIGHT))
        .resizable(None)
        .exit_on_close(true);
    cosmic::app::run::<App>(settings, ())
}

#[cfg(test)]
mod tests;
