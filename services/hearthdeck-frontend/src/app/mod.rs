//! The rewritten HearthDeck frontend.
//!
//! Built from scratch, one screen at a time, reusing the plumbing below the UI
//! (`providers`, `app_group`, `icon_cache`, `style`, `ui`, `widgets`). The
//! inherited app lives on as [`crate::app_legacy`] until this one covers every
//! screen; `HEARTHDECK_LEGACY=1` runs the old one.
//!
//! The library shell is in place: the sidebar, section selection, the header's
//! tabs, and the daemon-backed grid - virtualized, paged, and searched at the
//! source - over the console grid's filter facets. Launch, focus navigation, the
//! context menu and the other screens land in the next slices.
//!
//! The sidebar is deliberately *ours* (`screens/library/sidebar.rs`): COSMIC's
//! `nav_bar` was tried here and does not fit the design — its segmented list,
//! type/icon sizes and grey panel are all wrong for the kiosk look, and it is
//! not ours to restyle. That is the line we use to decide "use COSMIC" vs
//! "own it".

use std::collections::BTreeMap;
use std::sync::Arc;

use cosmic::app::{Core, Settings, Task};
use cosmic::cosmic_config::{Config, CosmicConfigEntry};
use cosmic::desktop::DesktopEntryData;
use cosmic::iced::Alignment;
use cosmic::iced::core::keyboard::{Key, key::Named};
use cosmic::iced::core::widget::operation::focusable::focus;
use cosmic::iced::core::window::{Event as WindowEvent, Id as SurfaceId};
use cosmic::iced::event::{Status, listen_with};
use cosmic::iced::widget::scrollable::{AbsoluteOffset, scroll_to};
use cosmic::iced::widget::stack;
use cosmic::iced::{self, Length, Subscription, runtime, window};
use cosmic::theme;
use cosmic::widget::{
    Id, button, column, container, context_drawer, icon, row, space, text, text_input,
};
use cosmic::{Application, Element};
use hearthdeck_protocol::InputProfile;

use crate::app_group::{AppLibraryConfig, RommFacet, Section, section_facets};
use crate::fl;
use crate::input_ownership::{
    Event as InputEvent, InputOwnership, LaunchTarget, managed_launch_target,
};
use crate::launch_state::{Effect as LaunchEffect, Event as LaunchEvent, LaunchState};
use crate::providers::GameProvider;
use crate::providers::daemon::{
    Collection, CollectionItem, DaemonClient, DaemonConfig, DaemonProvider,
};
use crate::providers::filter::{Facet, cycle};
use crate::screens::dashboard::{self, Card, Shelf};
use crate::screens::library::console::{self, ConsoleListing};
use crate::screens::library::focus::{self, Chrome, Confirm, Cursors, Direction, Layout, Outcome};
use crate::screens::library::sidebar::{self, Sidebar};
use crate::screens::library::{filters, grid, header};
use crate::style::{
    DASHBOARD_RAIL_TILES, DIVIDER_WIDTH, ICON_LARGE, WINDOW_HEIGHT, WINDOW_WIDTH,
    content_horizontal_padding, dashboard_tile_size, filter_drawer_width,
    grid_viewport_height_fallback, launch_overlay, primary_action_button_class, root_background,
    sidebar_divider,
};
use crate::subscriptions::gamepad::{GamepadEvent, gamepad_events};
use crate::toplevel::{WindowHint, fullscreen_when_it_appears};
use crate::ui;
use crate::widgets::menu::{self, Menu, Row};

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

/// Everything the new shell can react to so far.
#[derive(Debug, Clone)]
pub enum Message {
    /// The catalog fetched from the daemon, rendered as tiles.
    CatalogLoaded(Vec<Arc<DesktopEntryData>>),
    /// A page of console records arrived, for the load it was started for.
    ConsolePageLoaded {
        /// The load this page answers. A page from an older one describes a
        /// scope the user has already left, and is dropped.
        generation: u64,
        result: Result<console::Page, String>,
    },
    /// The filter facets the current console scope offers, from the daemon.
    FacetsLoaded(Vec<Facet>),
    /// The dashboard's collections arrived, which is what its rails are made of.
    CollectionsLoaded(Result<Vec<Collection>, String>),
    /// The console list a tab strip is built from: `(platform id, label)`.
    ConsolesLoaded(Vec<(i64, String)>),
    SelectSection(Section),
    /// The dashboard's top bar: show the dashboard itself.
    OpenDashboard,
    /// The dashboard's top bar: show the library as it was left.
    OpenLibrary,
    /// The dashboard's top bar: show the library with the caret in its search
    /// box, which is what this app's search means.
    OpenSearch,
    /// A dashboard card was pressed: show the library at the section the entry
    /// on that card belongs to.
    ShowLibrary(Section),
    SelectGroup(Option<usize>),
    InputChanged(String),
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
    /// The pad's context button: open the tile menu on the tile at this index,
    /// or close it when it is already open on that tile.
    TileContext(usize),
    /// The menu's scrim was pressed: close it.
    CloseMenu,
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
    /// The main window appeared: go fullscreen, matching the session's kiosk
    /// model.
    Opened,
    /// The main window's width, so proportional layout follows it.
    WindowResized(f32),
    /// The main window gained or lost compositor focus.
    WindowFocusChanged(bool),
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
}

pub struct App {
    core: Core,
    /// Which page the shell is showing.
    page: Page,
    config: AppLibraryConfig,
    /// The config file the shell writes through, resolved once at startup so the
    /// shell has exactly one write target. `None` when there is no store to
    /// write to, which is also what keeps a test off the real config file.
    config_store: Option<Config>,
    daemon_client: DaemonClient,
    /// The full catalog as the daemon reported it (PC games and applications).
    catalog: Vec<Arc<DesktopEntryData>>,
    /// The console scope's records, a page at a time, as the daemon reports
    /// them, with the cursor that extends them.
    console: ConsoleListing,
    /// What the grid draws: the section's source after filtering. The console
    /// grid's facets and search are applied by the daemon (see `load_console`);
    /// every other section is filtered in memory. Kept in state so `view` stays
    /// pure.
    visible: Vec<Arc<DesktopEntryData>>,
    /// The facets the current source offers, and the selection keyed by facet id.
    facets: Vec<Facet>,
    /// The daemon's collections, in the order it composes them. The dashboard's
    /// rails are these, so a collection nobody wrote code for - which is what a
    /// feature's own categories, and so the AI's, are - appears without a
    /// frontend change.
    collections: Vec<Collection>,
    facet_selection: BTreeMap<String, String>,
    /// Whether the filter sidebar is open.
    filter_open: bool,
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
    /// Where each dashboard rail is scrolled to, by rail index. The shell keeps
    /// it because the rail cannot: the movement rules need to know whether the
    /// card the cursor lands on would leave the view.
    rail_offsets: Vec<f32>,
    /// Where the dashboard page is scrolled to, which the rail follow needs the
    /// same way each rail needs its own offset: the dashboard is a stack of rails
    /// and only some of it is on screen.
    page_offset: f32,
    search_value: String,
    user_name: String,
    /// Free space on the home filesystem, as the last poll read it. Blank until
    /// the first poll answers.
    disk_free: String,
    window_width: f32,
    /// Where the grid is scrolled to, and how tall its viewport is. The
    /// virtualization builds the rows between them, and the console source asks
    /// for its next page when they reach the end of the records it holds.
    grid_scroll_offset: f32,
    grid_viewport_height: f32,
    /// The launch overlay: progress while a game starts, or its failure.
    launch_state: LaunchState,
    /// Who owns input: the frontend, a launch in flight, or a running session.
    input_ownership: InputOwnership,
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
        let config = config_store
            .as_ref()
            .map(|store| {
                AppLibraryConfig::get_entry(store).unwrap_or_else(|(errors, config)| {
                    for error in errors {
                        log::error!("{error:?}");
                    }
                    config
                })
            })
            .unwrap_or_default();

        let daemon_client = DaemonClient::new(DaemonConfig {
            base_url: std::env::var("HEARTHDECK_BACKEND_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:38400".to_string()),
            token: std::env::var("HEARTHDECK_PAIRING_TOKEN").unwrap_or_default(),
        });
        let load = Task::perform(
            {
                let daemon_client = daemon_client.clone();
                async move {
                    let provider = DaemonProvider::with_client(daemon_client);
                    match GameProvider::discover(&provider).await {
                        Ok(records) => records
                            .into_iter()
                            .map(|record| Arc::new(record.into_desktop_entry()))
                            .collect(),
                        Err(error) => {
                            log::error!("catalog fetch failed: {error:?}");
                            Vec::new()
                        }
                    }
                }
            },
            |entries| cosmic::Action::App(Message::CatalogLoaded(entries)),
        );

        let app = Self {
            core,
            page: Page::default(),
            config,
            config_store,
            daemon_client,
            catalog: Vec::new(),
            console: ConsoleListing::default(),
            visible: Vec::new(),
            facets: Vec::new(),
            collections: Vec::new(),
            facet_selection: BTreeMap::new(),
            filter_open: false,
            menu: menu::State::default(),
            cur_section: Section::PcGames,
            cur_group: None,
            // The first tile: with nothing loaded yet there is nothing to point
            // at, and every read of the cursor tolerates that.
            cursors: Cursors::default(),
            dashboard_cursor: dashboard::focus::Cursor::default(),
            rail_offsets: Vec::new(),
            page_offset: 0.0,
            search_value: String::new(),
            user_name: crate::system_status::current_user_name(),
            disk_free: String::new(),
            window_width: WINDOW_WIDTH,
            grid_scroll_offset: 0.0,
            grid_viewport_height: grid_viewport_height_fallback(),
            launch_state: LaunchState::default(),
            input_ownership: InputOwnership::default(),
        };
        let poll = app.poll_active_session(std::time::Duration::ZERO);
        let consoles = app.load_consoles();
        let disk = app.poll_disk_free(std::time::Duration::ZERO);
        let collections = app.load_collections();
        (app, Task::batch([load, poll, consoles, disk, collections]))
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        // The cursor is repaired before anything acts on it, and again after:
        // entries load, facets come and go and sections swap, so both the cursor
        // and what it can point at change under it. Before, so no action acts on
        // something that is gone; after, so no observer sees one.
        self.cursors = focus::clamp(self.cursors, self.layout());
        match message {
            Message::CatalogLoaded(entries) => {
                self.catalog = entries;
                // The local sections' tabs are the ones their entries imply: a
                // store or category nothing installed carries is not a tab. The
                // catalog the grid draws is therefore what the strip is built
                // from. Console Games is the exception - its tabs are the
                // platforms the daemon reported (see `ConsolesLoaded`).
                if self.config.sync_category_groups(&self.catalog) {
                    self.persist_config();
                }
                self.rebuild_local_facets();
                self.refilter();
                return self.list_replaced();
            }
            Message::ConsolesLoaded(platforms) => {
                // An empty list means the fetch failed; do not wipe the tabs.
                if platforms.is_empty() {
                    return Task::none();
                }
                if self.config.sync_console_groups(&platforms) {
                    self.persist_config();
                    // The chrome the ring walks changed, so its stop may no
                    // longer be one that exists.
                    self.cursors = focus::clamp(self.cursors, self.layout());
                    // Indices may have moved, so drop the selection and reload.
                    if self.cur_section == Section::ConsoleGames {
                        self.cur_group = None;
                        return Task::batch([self.load_console(0), self.load_facets()]);
                    }
                }
            }
            Message::ConsolePageLoaded { generation, result } => {
                match result {
                    Ok(page) => {
                        let first_page = page.offset == 0;
                        if !self.console.apply(generation, page) {
                            return Task::none();
                        }
                        // The console grid is not what is on screen. The page is
                        // still logged, so the tab is warm when it is next
                        // opened, but it must not repaint the screen it left.
                        if self.cur_section != Section::ConsoleGames {
                            return Task::none();
                        }
                        self.refilter();
                        if first_page {
                            return self.list_replaced();
                        }
                    }
                    Err(error) => {
                        if self.console.failed(generation) {
                            log::error!("console fetch failed: {error}");
                        }
                    }
                }
            }
            Message::GridScrolled {
                offset,
                viewport_height,
            } => {
                self.grid_scroll_offset = offset;
                self.grid_viewport_height = viewport_height;
                // Reaching the end of the pages loaded so far is what pulls the
                // next one in; a scope with nothing left asks for nothing.
                if self.cur_section == Section::ConsoleGames
                    && let Some(next) = self.console.extend_offset(
                        offset,
                        viewport_height,
                        grid::metrics(self.window_width),
                    )
                {
                    return self.load_console(next);
                }
            }
            Message::ConsoleSearchSettled(value) => {
                // One of these is scheduled per keystroke; the term the user
                // actually stopped on is the one still in the field.
                if value != self.search_value || self.cur_section != Section::ConsoleGames {
                    return Task::none();
                }
                return self.load_console(0);
            }
            Message::CollectionsLoaded(result) => {
                match result {
                    Ok(collections) => {
                        self.collections = collections;
                        // The rails just changed under the cursor, so it may be
                        // pointing past the end of them now.
                        let rails = self.dashboard_rail_lengths();
                        self.dashboard_cursor =
                            dashboard::focus::clamp(self.dashboard_cursor, &rails);
                    }
                    Err(error) => log::warn!("collection fetch failed: {error}"),
                }
            }
            Message::FacetsLoaded(facets) => {
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
                self.page = Page::Library;
                self.cur_section = section;
                self.cur_group = None;
                self.search_value.clear();
                self.facet_selection.clear();
                self.facets.clear();
                self.filter_open = false;
                if section == Section::ConsoleGames {
                    // The console grid's facets and records both come from the
                    // daemon; it is the one source that loads asynchronously.
                    self.refilter();
                    return Task::batch([self.load_console(0), self.load_facets()]);
                }
                self.rebuild_local_facets();
                self.refilter();
                return self.list_replaced();
            }
            Message::SelectGroup(group) => {
                self.cur_group = group;
                self.facet_selection.clear();
                self.facets.clear();
                self.filter_open = false;
                if self.cur_section == Section::ConsoleGames {
                    self.refilter();
                    return Task::batch([self.load_console(0), self.load_facets()]);
                }
                self.rebuild_local_facets();
                self.refilter();
                return self.list_replaced();
            }
            Message::InputChanged(value) => {
                self.search_value = value;
                self.refilter();
                if self.cur_section == Section::ConsoleGames {
                    // The console grid is searched by the source, so the query
                    // waits for the user to stop typing.
                    return self.settle_console_search();
                }
                return self.list_replaced();
            }
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
                    return self.load_console(0);
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
                    return self.load_console(0);
                }
                self.refilter();
                return self.list_replaced();
            }
            Message::Activate(index) => return self.activate(index),
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
            Message::TileContext(index) => {
                // The button that opens the menu is the one that closes it, so a
                // second press hands the pad back to the grid.
                if index < self.visible.len() {
                    self.menu.toggle(index);
                }
            }
            Message::CloseMenu => self.menu.close(),
            Message::RailScrolled { rail, offset } => {
                if self.rail_offsets.len() <= rail {
                    self.rail_offsets.resize(rail + 1, 0.0);
                }
                self.rail_offsets[rail] = offset;
            }
            Message::PageScrolled(offset) => self.page_offset = offset,
            // A dashboard card is a way into the library, so pressing one leaves
            // the dashboard for it - which the section change itself does.
            Message::ShowLibrary(section) => return self.update(Message::SelectSection(section)),
            Message::OpenDashboard => self.page = Page::Dashboard,
            Message::OpenLibrary => self.page = Page::Library,
            Message::OpenSearch => {
                self.page = Page::Library;
                // The caret goes in the search box, which the library header
                // draws - so the page has to be the library first.
                return self.focus_search();
            }
            Message::GamepadEvent(event) => {
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
                self.launch_state.update(LaunchEvent::Dismiss);
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
            Message::Opened => {
                return window::set_mode(SurfaceId::RESERVED, window::Mode::Fullscreen);
            }
            Message::WindowResized(width) => self.window_width = width,
            Message::WindowFocusChanged(focused) => {
                self.input_ownership.update(if focused {
                    InputEvent::FrontendFocused
                } else {
                    InputEvent::FrontendUnfocused
                });
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        // The dashboard is a sibling of the library, not a page inside it: it
        // carries its own top bar and has no sidebar, so it is the whole window.
        // Drawing the library's shell around it would make it read as a section
        // of the library, which is exactly what it is not.
        if self.page == Page::Dashboard {
            return self.view_dashboard();
        }

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
        let content: Element<'_, Self::Message> = if self.launch_state.is_visible() {
            view_launch_overlay(&self.launch_state)
        } else {
            let header = header::view(header::Header {
                section: self.cur_section,
                selected_group: self.cur_group,
                groups: self.config.sections.get(self.cur_section),
                search_value: &self.search_value,
                filter_active: self.facet_selection.len(),
                show_filter: !self.facets.is_empty(),
            });

            let body: Element<'_, Self::Message> = if self.visible.is_empty() {
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
                })
            };

            let page = container(column![header, body])
                .width(Length::Fill)
                .height(Length::Fill)
                .padding([0, content_horizontal_padding()]);

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
                page.into()
            }
        };

        // The menu is a surface over the page: it is stacked over the content
        // column, so it draws over the grid and the filter drawer alike, and its
        // scrim dismisses it. The rows come from the entry it is open on, which
        // is why the menu is built here and not in the grid.
        let content: Element<'_, Self::Message> = match self.menu_target() {
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

    fn subscription(&self) -> Subscription<Self::Message> {
        // The pad and the keyboard feed one message, because they are one
        // vocabulary: a step is a step whichever made it. Neither is acted on
        // while something else owns the screen - the update arm drops events
        // then - so there is nothing to gate here.
        let navigation = gamepad_events().map(Message::GamepadEvent);
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
                Some(Message::WindowResized(size.width))
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
        Subscription::batch([navigation, input])
    }
}

impl App {
    /// The dashboard screen: the app's front door, with chrome of its own.
    ///
    /// It is drawn as the whole window rather than as the library's content, so
    /// the library's sidebar never appears around it - the two are sibling
    /// destinations, and the sidebar belongs to the library.
    fn view_dashboard(&self) -> Element<'_, Message> {
        let screen = dashboard::view(
            &self.dashboard_shelves(),
            dashboard::TopBar {
                user_name: self.user_name.clone(),
                app_icon: ui::app_icon(),
                on_home: Message::OpenDashboard,
                on_library: Message::OpenLibrary,
                on_search: Message::OpenSearch,
            },
            self.window_width,
            |rail, offset| Message::RailScrolled { rail, offset },
            Message::PageScrolled,
        );
        // A launch is a layer over whatever page is behind it, the dashboard
        // included.
        let screen: Element<'_, Message> = if self.launch_state.is_visible() {
            stack![screen, view_launch_overlay(&self.launch_state)].into()
        } else {
            screen
        };
        container(screen)
            .width(Length::Fill)
            .height(Length::Fill)
            .class(theme::Container::Custom(Box::new(root_background)))
            .into()
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
            self.visible = self.console.entries().to_vec();
        } else {
            self.visible = self.config.filtered_by_facets(
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
        if self.search_value.is_empty() {
            return Task::batch([drawn, self.focus_task()]);
        }
        drawn
    }

    /// Re-queries the console grid once the user has stopped typing.
    ///
    /// The wait is a task rather than a subscription: the timeout that fires
    /// carries the term it was scheduled for, and only a term still in the field
    /// is searched, so a superseded wait needs no cancelling.
    fn settle_console_search(&mut self) -> Task<Message> {
        let value = self.search_value.clone();
        Task::perform(tokio::time::sleep(CONSOLE_SEARCH_DEBOUNCE), move |()| {
            cosmic::Action::App(Message::ConsoleSearchSettled(value))
        })
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
            tabs: self.config.sections.get(self.cur_section).len() + 1,
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
        let compatibility = if self.config.desktop_input_enabled(&entry.id) {
            fl!("disable-controller-compatibility")
        } else {
            fl!("enable-controller-compatibility")
        };
        vec![
            Row::new(fl!("run"), Message::Activate(index)),
            Row::new(compatibility, Message::ToggleControllerCompatibility(index)),
        ]
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

    /// The dashboard's rails: the daemon's collections, in the order it composes
    /// them.
    ///
    /// Drawn from the collections rather than from a list of known shelves, so a
    /// collection nobody wrote frontend code for - which is what a feature's own
    /// categories, and so the AI's, are - appears on the dashboard without a
    /// change here. With nothing composed yet the library still gets a rail,
    /// because a blank front door is worse than a short one.
    fn dashboard_shelves(&self) -> Vec<Shelf> {
        let spacing = theme::spacing();
        let size = dashboard_tile_size(
            self.window_width,
            content_horizontal_padding(),
            spacing.space_l,
        );
        let shelves: Vec<Shelf> = self
            .collections
            .iter()
            .filter(|collection| !collection.items.is_empty())
            .map(|collection| {
                let cards = collection
                    .items
                    .iter()
                    .take(DASHBOARD_RAIL_TILES)
                    .enumerate()
                    .map(|(index, item)| {
                        Card::new(
                            dashboard_card_id(&collection.slug, index),
                            item.name.clone(),
                            self.collection_item_icon(item, size as u32),
                            self.collection_item_message(item),
                        )
                    })
                    .collect();
                Shelf::new(false, dashboard::rail_title(collection), cards)
            })
            .collect();
        if shelves.is_empty() {
            return self.library_shelf(size);
        }
        shelves
    }

    /// The library as a single rail, for when the daemon has composed nothing yet
    /// - a fresh install, or a fetch that has not landed.
    fn library_shelf(&self, size: f32) -> Vec<Shelf> {
        let cards: Vec<Card> = self
            .catalog
            .iter()
            .filter(|entry| Section::PcGames.matches(entry))
            .take(DASHBOARD_RAIL_TILES)
            .enumerate()
            .map(|(index, entry)| {
                Card::new(
                    dashboard_card_id("library", index),
                    entry.name.clone(),
                    crate::icon_cache::entry_icon_handle(&entry.icon, size as u32),
                    Some(Message::ShowLibrary(Section::PcGames)),
                )
            })
            .collect();
        if cards.is_empty() {
            return Vec::new();
        }
        vec![Shelf::new(false, fl!("all-games"), cards)]
    }

    /// A collection item's icon.
    ///
    /// The library's own entry when the item is one of its, which is the common
    /// case and draws the real cover. An item the library does not hold - a play
    /// the daemon derived, or a title from a linked account - gets a generic mark
    /// rather than nothing.
    fn collection_item_icon(&self, item: &CollectionItem, size: u32) -> icon::Handle {
        self.catalog
            .iter()
            .find(|entry| entry.id == item.item_id)
            .map_or_else(
                || {
                    crate::icon_cache::icon_cache_handle(
                        "application-x-executable-symbolic",
                        size as u16,
                    )
                },
                |entry| crate::icon_cache::entry_icon_handle(&entry.icon, size),
            )
    }

    /// What pressing a collection card does.
    ///
    /// A way into the library, at the section that item belongs to - which is
    /// what a tile does everywhere else until the details screen exists. An item
    /// the library does not hold has nowhere to go, so its card is inert rather
    /// than a way into some other section.
    fn collection_item_message(&self, item: &CollectionItem) -> Option<Message> {
        self.catalog
            .iter()
            .find(|entry| entry.id == item.item_id)
            .map(|entry| Message::ShowLibrary(section_of(entry)))
    }

    /// How many cards each rail holds, which is the shape the rail navigation
    /// walks.
    fn dashboard_rail_lengths(&self) -> Vec<usize> {
        self.dashboard_shelves()
            .iter()
            .map(|shelf| shelf.cards.len())
            .collect()
    }

    /// Steps the dashboard's cursor, and keeps the card it lands on on screen.
    fn step_dashboard(&mut self, direction: Direction) -> Task<Message> {
        let rails = self.dashboard_rail_lengths();
        let before = self.dashboard_cursor;
        self.dashboard_cursor = dashboard::focus::step(self.dashboard_cursor, direction, &rails);
        if self.dashboard_cursor == before {
            return Task::none();
        }
        self.focus_dashboard_card()
    }

    /// Runs the card the dashboard's cursor is on.
    fn confirm_dashboard(&mut self) -> Task<Message> {
        let rails = self.dashboard_rail_lengths();
        let message =
            dashboard::focus::confirm(self.dashboard_cursor, &rails).and_then(|(rail, card)| {
                self.dashboard_shelves()
                    .get(rail)
                    .and_then(|shelf| shelf.cards.get(card))
                    .and_then(|card| card.message.clone())
            });
        match message {
            Some(message) => self.update(message),
            None => Task::none(),
        }
    }

    /// Points the renderer's focus at the card the dashboard's cursor is on, and
    /// scrolls its rail so that card stays on screen.
    ///
    /// A rail moves only once the cursor crosses one of its edges, which is the
    /// rule the grid follows and for the same reason: a row that slid on every
    /// step reads as the row moving rather than the selection.
    fn focus_dashboard_card(&mut self) -> Task<Message> {
        let shelves = self.dashboard_shelves();
        let Some(shelf) = shelves.get(self.dashboard_cursor.rail) else {
            return Task::none();
        };
        let Some(card) = shelf.cards.get(self.dashboard_cursor.card) else {
            return Task::none();
        };
        let mut tasks = vec![widget_focus(card.id.clone())];

        // The rail follows the cursor sideways...
        let (card_width, gap) = dashboard::card_pitch(shelf, self.window_width);
        if let Some(offset) = dashboard::focus::rail_scroll_target(
            self.dashboard_cursor.card,
            shelf.cards.len(),
            self.rail_offsets
                .get(self.dashboard_cursor.rail)
                .copied()
                .unwrap_or(0.0),
            dashboard::rail_viewport(self.window_width),
            card_width,
            gap,
        ) {
            tasks.push(scroll_rail(self.dashboard_cursor.rail, offset));
        }

        // ...and the page follows it down the stack of rails, which is the same
        // rule a third time: rails of unequal height rather than cards of equal
        // width. The dashboard fills the content column, so the page's viewport
        // is the window's height.
        let heights = dashboard::rail_heights(&shelves, self.window_width);
        // The page sits below the top bar, so its viewport is the window's height
        // less the bar rather than the whole window.
        let page_viewport = WINDOW_HEIGHT - dashboard::top_bar_height();
        if let Some(offset) = dashboard::focus::page_scroll_target(
            &heights,
            self.dashboard_cursor.rail,
            dashboard::rail_gap(),
            self.page_offset,
            page_viewport,
        ) {
            tasks.push(dashboard::scroll_to_page(offset));
        }

        Task::batch(tasks)
    }

    /// Acts on one navigation event.
    ///
    /// The pad and the keyboard both arrive here, so there is one place that
    /// knows what a step, a confirm and Back mean - and no control that decides
    /// it for itself.
    fn navigate(&mut self, event: GamepadEvent) -> Task<Message> {
        match event {
            GamepadEvent::MoveUp => self.step_focus(Direction::Up),
            GamepadEvent::MoveDown => self.step_focus(Direction::Down),
            GamepadEvent::MoveLeft => self.step_focus(Direction::Left),
            GamepadEvent::MoveRight => self.step_focus(Direction::Right),
            GamepadEvent::Confirm => self.confirm_focus(),
            GamepadEvent::Back => self.back(),
            // Search is the one action that puts the caret in the search box
            // from wherever the cursor is, which is what a pad needs to search.
            GamepadEvent::Search => self.focus_search(),
            GamepadEvent::ContextMenu => self.context_action(),
            // The shoulder buttons switch section and the triggers switch tab.
            // They are chords rather than places on the screen, so they wrap
            // where a step stops at the end of the grid.
            GamepadEvent::PrevGroup => self.switch_section(-1),
            GamepadEvent::NextGroup => self.switch_section(1),
            GamepadEvent::PrevTab => self.switch_tab(-1),
            GamepadEvent::NextTab => self.switch_tab(1),
            // The drawer is a surface over the grid rather than a place in it,
            // so it has its own button instead of being walked to.
            GamepadEvent::FilterPanel => self.update(Message::ToggleFilterPanel),
        }
    }

    /// Moves the cursor one step, and the renderer's focus with it.
    fn step_focus(&mut self, direction: Direction) -> Task<Message> {
        // A step belongs to the content: a D-pad press takes the cursor back
        // from the keyboard's ring rather than walking along the chrome.
        self.cursors.ring = None;
        // The menu is drawn over whichever page is behind it, so it takes the
        // step before the page does.
        if self.menu_is_up() {
            return self.move_menu(direction);
        }
        if self.page == Page::Dashboard {
            return self.step_dashboard(direction);
        }
        match focus::step(self.cursors, direction, self.layout()) {
            Outcome::Tile { tile, column } => {
                self.cursors.tile = tile;
                self.cursors.column = column;
                let focus = self.focus_task();
                let scroll = self.scroll_to_tile(tile);
                Task::batch([focus, scroll])
            }
            // Inside the drawer the row is the control, so the cursor moves
            // between rows and nothing is repainted: the drawer draws its own
            // cursor, because a row is a container and cannot hold focus.
            Outcome::FilterRow(row) => {
                self.cursors.filter_row = row;
                Task::none()
            }
            // Sideways on a row is that row's own control: the value steps and
            // the cursor stays where the user is looking.
            Outcome::FilterValue { row, delta } => self.cycle_facet(row, delta),
            Outcome::Stayed => Task::none(),
        }
    }

    /// Acts on whatever holds the cursor: a ring stop, a drawer row, or a tile.
    fn confirm_focus(&mut self) -> Task<Message> {
        // The menu is the innermost surface, on either page.
        if self.menu_is_up() {
            return self.activate_menu_row();
        }
        if self.page == Page::Dashboard {
            return self.confirm_dashboard();
        }
        match focus::confirm(self.cursors, self.layout()) {
            Confirm::Tile(index) => self.update(Message::Activate(index)),
            // A pad whose left and right do not work still has to be able to
            // change a facet, so confirming a row steps it forward.
            Confirm::Filter { row } => self.cycle_facet(row, 1),
            Confirm::Chrome(chrome) => self.activate_chrome(chrome),
            Confirm::Nothing => Task::none(),
        }
    }

    /// Acts on a chrome stop the keyboard's ring is on.
    ///
    /// These are the actions the pad's own buttons ask for, so a keyboard
    /// reaches exactly what a controller does and the two cannot drift apart.
    fn activate_chrome(&mut self, chrome: Chrome) -> Task<Message> {
        match chrome {
            Chrome::Section(index) => match Section::ALL.get(index) {
                Some(section) => self.update(Message::SelectSection(*section)),
                None => Task::none(),
            },
            // 0 is the section's "all" tab; what follows it are its groups,
            // which is the numbering the header and the ring share.
            Chrome::Tab(index) => self.update(Message::SelectGroup(index.checked_sub(1))),
            Chrome::Search => self.focus_search(),
            Chrome::FilterButton => self.update(Message::ToggleFilterPanel),
        }
    }

    /// Steps the facet drawn on `row` through its values, which is what the
    /// drawer's own arrows do.
    fn cycle_facet(&mut self, row: usize, delta: i32) -> Task<Message> {
        let Some(facet) = self.facets.get(row) else {
            return Task::none();
        };
        let current = self.facet_selection.get(&facet.id).map(String::as_str);
        let value = cycle(facet, current, delta);
        // The same message the drawer's arrows send, so a pad-driven selection
        // and a clicked one cannot drift apart.
        self.update(Message::SelectFacet {
            facet: facet.id.clone(),
            value,
        })
    }

    /// The X button: what the user can do with what the cursor is on.
    fn context_action(&mut self) -> Task<Message> {
        // An open menu takes the button back: it is the innermost surface, so
        // the press closes it before it reaches anything behind it.
        if self.menu.is_open() {
            self.menu.close();
            return Task::none();
        }
        // Inside the drawer, X is the one thing a drawer cannot do for itself:
        // clear every facet without travelling to a button for it. The clear
        // action is deliberately not a cursor row, so the cursor never has to
        // walk to it.
        if self.filter_open {
            return self.update(Message::ClearFilters);
        }
        // The tile's own menu: X is the button that opens it, on the tile the
        // cursor is on.
        if self.cursors.tile < self.visible.len() {
            return self.update(Message::TileContext(self.cursors.tile));
        }
        Task::none()
    }

    /// Back: what leaves the screen, innermost surface first.
    ///
    /// This is the whole global Back contract. Back that behaves differently
    /// depending on where the user is would be worse than no Back at all, so no
    /// screen and no drawer handles it for itself - they are unwound here, in
    /// the order the surfaces stack. The one rung still missing is the page
    /// behind the library: the Dashboard, which is where Back will end up.
    fn back(&mut self) -> Task<Message> {
        // A failed launch is a message to read, so it is the innermost surface.
        if self.launch_state.error().is_some() {
            return self.update(Message::DismissLaunch);
        }
        // A launch in flight is not something to back out of.
        if self.launch_state.is_visible() {
            return Task::none();
        }
        // The menu is drawn over everything there is, so it is the surface Back
        // reaches first.
        if self.menu.is_open() {
            self.menu.close();
            return Task::none();
        }
        // The drawer is a surface of its own: it closes before Back reaches the
        // grid behind it, and the pad goes back to the tile it left - which the
        // cursors kept, because the drawer has a row of its own rather than
        // being a place in the grid.
        if self.filter_open {
            return self.update(Message::CloseFilterPanel);
        }
        // The ring is the keyboard's surface: Back steps out of it into the
        // content, the same way it leaves the drawer.
        if self.cursors.ring.is_some() {
            self.cursors.ring = None;
            return self.focus_task();
        }
        // Whatever took the caret - the search box, through its button or a
        // click - gives it back to the content. With nothing else open, Back from
        // the library returns to the dashboard, and on the dashboard itself there
        // is nothing behind it to go back to.
        if self.page == Page::Library {
            self.page = Page::Dashboard;
            let rails = self.dashboard_rail_lengths();
            self.dashboard_cursor = dashboard::focus::clamp(self.dashboard_cursor, &rails);
            return self.focus_dashboard_card();
        }
        self.focus_task()
    }

    /// Puts the caret in the search box, which is what the pad's search button
    /// and the ring's stop at it both ask for.
    ///
    /// The cursor itself stays where it is: the box takes the *caret*, not the
    /// pad, so leaving it again puts the user back on the tile they were on.
    fn focus_search(&mut self) -> Task<Message> {
        self.cursors.ring = None;
        text_input::focus(header::search_id())
    }

    /// Switches section, wrapping at either end.
    fn switch_section(&mut self, delta: i32) -> Task<Message> {
        let sections = Section::ALL.len() as i32;
        let index = (self.cur_section.index() as i32 + delta).rem_euclid(sections);
        match Section::ALL.get(index as usize) {
            Some(section) => self.update(Message::SelectSection(*section)),
            None => Task::none(),
        }
    }

    /// Switches tab within the section on screen, wrapping over that section's
    /// "all" tab and its groups.
    fn switch_tab(&mut self, delta: i32) -> Task<Message> {
        let tabs = self.config.sections.get(self.cur_section).len() as i32 + 1;
        let current = self.cur_group.map_or(0, |group| group as i32 + 1);
        let next = (current + delta).rem_euclid(tabs);
        self.update(Message::SelectGroup(
            next.checked_sub(1).map(|group| group as usize),
        ))
    }

    /// One stop along the chrome ring, which is how a keyboard reaches what the
    /// pad reaches by a button. Walking past an end hands the cursor back to the
    /// content, which is where the pad always leaves it.
    ///
    /// The ring is input, so it waits for the same ownership the pad's events do
    /// rather than moving the cursor of a screen a game is covering.
    fn walk_ring(&mut self, backwards: bool) -> Task<Message> {
        if !self.input_ownership.frontend_has_control() {
            return Task::none();
        }
        self.cursors.ring = focus::ring_step(self.cursors.ring, backwards, self.layout());
        self.focus_task()
    }

    /// Points the renderer's focus at whatever holds the cursor, which is where
    /// a button's `focused` style flag - and so the design system's focus ring -
    /// comes from.
    ///
    /// The drawer is the exception: its rows are containers, and a container
    /// cannot hold focus, so they are drawn from the cursor itself (see
    /// `Filters::cursor`).
    fn focus_task(&self) -> Task<Message> {
        if let Some(chrome) = self
            .cursors
            .ring
            .and_then(|index| focus::ring_at(index, self.layout()))
        {
            return match chrome {
                Chrome::Section(index) => Section::ALL.get(index).map_or(Task::none(), |section| {
                    widget_focus(sidebar::section_id(*section))
                }),
                Chrome::Tab(index) => widget_focus(header::tab_id(index)),
                Chrome::Search => text_input::focus(header::search_id()),
                Chrome::FilterButton => widget_focus(header::filter_button_id()),
            };
        }
        self.visible
            .get(self.cursors.tile)
            .map_or(Task::none(), |entry| widget_focus(grid::tile_id(entry)))
    }

    /// Moves the grid so the cursor stays on screen.
    ///
    /// The viewport follows the cursor rather than the other way round: a step
    /// that would take the tile off the visible area scrolls the grid to put its
    /// row back inside, and a step within the viewport does not move the page at
    /// all. Without this the cursor walks off the bottom of the screen and the
    /// grid behind it never moves.
    ///
    /// The new offset is recorded before the scrollable reports it, because the
    /// grid is virtualized: the rows built are the rows around the offset, so a
    /// row not yet inside it does not exist as a widget - and `focus` on a widget
    /// that does not exist resolves to nothing. The scrollable confirms the
    /// offset on the next frame, through `GridScrolled`.
    fn scroll_to_tile(&mut self, tile: usize) -> Task<Message> {
        let metrics = grid::metrics(self.window_width);
        let row = tile / metrics.columns.max(1);
        let Some(offset) = metrics.scroll_to_row(
            self.visible.len(),
            row,
            self.grid_scroll_offset,
            self.grid_viewport_height,
        ) else {
            return Task::none();
        };
        self.grid_scroll_offset = offset;
        grid::scroll_to_offset(offset)
    }

    /// The locally-loaded sections derive their facets from the catalog they
    /// show; only the console grid's come from the daemon.
    fn rebuild_local_facets(&mut self) {
        if self.cur_section != Section::ConsoleGames {
            self.facets = section_facets(self.cur_section, &self.catalog);
        }
    }

    /// The RomM platform id of the console group in scope, when one is selected.
    fn console_platform_id(&self) -> Option<i64> {
        self.cur_group
            .and_then(|index| self.config.sections.console_games.get(index))
            .and_then(|group| group.romm_platform_id())
    }

    /// Fetches one page of console records for the scope on screen.
    ///
    /// The tab, the facet selection and the search term all narrow the query at
    /// the source, so the grid is never limited to what the pages it already
    /// holds happened to carry. The request is tagged with the listing's
    /// generation, so the page that comes back is merged only if the scope it
    /// describes is still the scope being shown.
    fn load_console(&mut self, offset: u32) -> Task<Message> {
        let Some(generation) = self.console.start(offset) else {
            return Task::none();
        };
        let platform_id = self.console_platform_id();
        let search = self.search_value.clone();
        let filters = self.facet_selection.clone();
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .list_retro_records(
                        platform_id,
                        (!search.is_empty()).then_some(search.as_str()),
                        &filters,
                        ROMM_PAGE_SIZE,
                        offset,
                    )
                    .await
                    .map(|page| console::Page {
                        entries: page
                            .items
                            .into_iter()
                            .map(|record| Arc::new(record.into_desktop_entry()))
                            .collect(),
                        total: page.total,
                        offset: page.offset,
                    })
                    .map_err(|error| error.to_string())
            },
            move |result| cosmic::Action::App(Message::ConsolePageLoaded { generation, result }),
        )
    }

    /// Fetches the dashboard's collections, which are what its rails are made of.
    ///
    /// One request for all of them, in the order the daemon composes them, rather
    /// than a request per shelf: a collection nobody wrote frontend code for has
    /// to arrive the same way as one that did.
    fn load_collections(&self) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .list_collections()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::CollectionsLoaded(result)),
        )
    }

    /// Fetches the console list, so the Console Games tabs match the user's
    /// actual RomM platforms rather than whatever was last persisted.
    fn load_consoles(&self) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                match client.list_retro_consoles().await {
                    Ok(consoles) => consoles,
                    Err(error) => {
                        log::error!("console list fetch failed: {error:?}");
                        Vec::new()
                    }
                }
            },
            |consoles| {
                let platforms = consoles
                    .into_iter()
                    .map(|console| (console.id, console.label))
                    .collect();
                cosmic::Action::App(Message::ConsolesLoaded(platforms))
            },
        )
    }

    /// Writes the config back to disk, so a derived tab set survives a restart.
    fn persist_config(&self) {
        if let Some(store) = self.config_store.as_ref() {
            let _ = self.config.write_entry(store);
        }
    }

    /// Fetches the filter facets the current console scope offers.
    fn load_facets(&self) -> Task<Message> {
        let platform_id = self.console_platform_id();
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                match client.retro_facets(platform_id).await {
                    Ok(facets) => facets,
                    Err(error) => {
                        log::error!("facet fetch failed: {error:?}");
                        Vec::new()
                    }
                }
            },
            |facets| cosmic::Action::App(Message::FacetsLoaded(facets)),
        )
    }

    /// Launches the entry the tile at `index` names.
    fn activate(&mut self, index: usize) -> Task<Message> {
        let Some(entry) = self.visible.get(index).cloned() else {
            return Task::none();
        };
        let Some(target) = managed_launch_target(&entry.id) else {
            log::error!("refusing unmanaged application launch: {}", entry.id);
            return Task::none();
        };
        self.launch_managed(target, entry.id.clone(), entry.name.clone())
    }

    /// Starts a launch: guards input ownership, shows the overlay, registers the
    /// window hint with the compositor watcher, and calls the daemon route for
    /// the target kind.
    fn launch_managed(
        &mut self,
        target: LaunchTarget,
        app_id: String,
        title: String,
    ) -> Task<Message> {
        // A game is already starting, or one owns the screen; a second launch
        // would be lost.
        if !self.input_ownership.frontend_has_control() {
            return Task::none();
        }
        let input_profile = if self.config.desktop_input_enabled(&app_id) {
            InputProfile::Desktop
        } else {
            InputProfile::Native
        };
        // Built before the launch so the compositor watcher is already listening
        // when the new window appears.
        let window_hint = self.launch_window_hint(&app_id, &title);
        if self.launch_state.update(LaunchEvent::Start(title)) != LaunchEffect::Launch {
            return Task::none();
        }
        self.input_ownership.update(InputEvent::LaunchStarted);
        fullscreen_when_it_appears(window_hint);
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                match target {
                    LaunchTarget::Catalog(id) => client.launch_app(&id, input_profile).await,
                    LaunchTarget::Romm(id) => client.launch_retro_rom(id, input_profile).await,
                }
            },
            |result| {
                cosmic::Action::App(Message::DaemonLaunchResult(
                    result.map(|_| ()).map_err(|error| error.to_string()),
                ))
            },
        )
    }

    /// The window class the entry declares, for the compositor watcher that
    /// takes the launched window fullscreen.
    fn launch_window_hint(&self, app_id: &str, title: &str) -> WindowHint {
        let wm_class = self
            .catalog
            .iter()
            .find(|entry| entry.id == app_id)
            .and_then(|entry| entry.wm_class.clone());
        WindowHint::for_entry(app_id, title, wm_class.as_deref())
    }

    /// Polls the daemon for a running session. Input ownership needs this to know
    /// when a launched game exits and the frontend can act again.
    fn poll_active_session(&self, delay: std::time::Duration) -> Task<Message> {
        let client = self.daemon_client.clone();
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
}

/// The launch overlay: progress while a game starts, or the failure with a way
/// to dismiss it.
fn view_launch_overlay(state: &LaunchState) -> Element<'_, Message> {
    let spacing = theme::spacing();
    let title = state.title().unwrap_or_default();
    let content: Element<'_, Message> = if let Some(error) = state.error() {
        column![
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
        .into()
    } else {
        column![
            icon::icon(ui::app_icon()).size(ICON_LARGE),
            text::title2(fl!("launching", title = title)),
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

/// The widget id of a dashboard card: the rail's slug and the card's place in
/// it, so the pad can be pointed at it and a card keeps its identity while the
/// rails around it change.
fn dashboard_card_id(slug: &str, index: usize) -> Id {
    Id::new(format!("dashboard-card-{slug}-{index}"))
}

/// The library section an entry belongs to, for a card that opens the library at
/// it.
fn section_of(entry: &DesktopEntryData) -> Section {
    Section::ALL
        .into_iter()
        .find(|section| section.matches(entry))
        .unwrap_or(Section::PcGames)
}

/// Scrolls one dashboard rail to a horizontal offset, leaving it where it is
/// vertically - a rail only ever moves sideways.
fn scroll_rail(rail: usize, offset: f32) -> Task<Message> {
    scroll_to(
        dashboard::rail_id(rail),
        AbsoluteOffset {
            x: Some(offset),
            y: None,
        },
    )
}

/// Puts the renderer's focus on one widget.
///
/// The shell's cursor decides what the user is on; this only reproduces it in
/// iced, which is where a button's `focused` style flag - and so the design
/// system's focus ring - comes from.
fn widget_focus(id: Id) -> Task<Message> {
    runtime::task::widget(focus(id))
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
mod tests {
    use super::*;
    use crate::app_group::AppGroup;
    use crate::providers::filter::FacetOption;

    /// A shell with no config store: nothing it does can reach the real config
    /// file, which is what lets a test drive `update` and read the state it
    /// leaves behind. The daemon client is never called either - a test feeds
    /// `update` the answer a daemon would have given.
    fn shell() -> App {
        App {
            core: Core::default(),
            page: Page::default(),
            config: AppLibraryConfig::default(),
            config_store: None,
            daemon_client: DaemonClient::new(DaemonConfig {
                base_url: String::new(),
                token: String::new(),
            }),
            catalog: Vec::new(),
            console: ConsoleListing::default(),
            visible: Vec::new(),
            facets: Vec::new(),
            collections: Vec::new(),
            facet_selection: BTreeMap::new(),
            filter_open: false,
            menu: menu::State::default(),
            cur_section: Section::PcGames,
            cur_group: None,
            cursors: Cursors::default(),
            dashboard_cursor: dashboard::focus::Cursor::default(),
            rail_offsets: Vec::new(),
            page_offset: 0.0,
            search_value: String::new(),
            user_name: String::new(),
            disk_free: String::new(),
            window_width: WINDOW_WIDTH,
            grid_scroll_offset: 0.0,
            grid_viewport_height: grid_viewport_height_fallback(),
            launch_state: LaunchState::default(),
            input_ownership: InputOwnership::default(),
        }
    }

    fn entry(id: &str, categories: &[&str]) -> Arc<DesktopEntryData> {
        Arc::new(DesktopEntryData {
            id: id.to_owned(),
            name: id.to_owned(),
            categories: categories
                .iter()
                .map(|category| (*category).to_owned())
                .collect(),
            ..DesktopEntryData::default()
        })
    }

    /// The labels of a section's tabs, in the order the strip shows them.
    fn tabs(app: &App, section: Section) -> Vec<String> {
        app.config
            .sections
            .get(section)
            .iter()
            .map(AppGroup::name)
            .collect()
    }

    /// The tabs a section shows are the ones its entries imply. Without them the
    /// strip is the "all" tab alone: the stores and categories the grid switches
    /// between have nowhere to be reached from.
    #[test]
    fn a_loaded_catalog_gives_each_section_its_category_tabs() {
        let mut app = shell();

        let _ = app.update(Message::CatalogLoaded(vec![
            entry("half-life", &["Game", "hearthdeck-store:Steam"]),
            entry("writer", &["Office"]),
        ]));

        assert_eq!(tabs(&app, Section::PcGames), ["Steam"]);
        assert_eq!(tabs(&app, Section::Applications), ["Office"]);
    }

    /// A tab is a store that installed something: the catalog is where the local
    /// sections' tabs come from, so a store leaving the library takes its tab
    /// with it.
    #[test]
    fn a_store_no_installed_game_comes_from_is_not_a_tab() {
        let mut app = shell();
        let _ = app.update(Message::CatalogLoaded(vec![entry(
            "half-life",
            &["Game", "hearthdeck-store:Steam"],
        )]));
        let _ = app.update(Message::CatalogLoaded(vec![entry("writer", &["Office"])]));

        assert!(
            tabs(&app, Section::PcGames).is_empty(),
            "the Steam tab outlived the game it came from: {:?}",
            tabs(&app, Section::PcGames)
        );
        assert_eq!(tabs(&app, Section::Applications), ["Office"]);
    }

    /// Console Games' tabs are the platforms the daemon reported, which no
    /// catalog can imply, so adopting one must leave them where they are.
    #[test]
    fn a_loaded_catalog_leaves_the_console_tabs_alone() {
        let mut app = shell();
        let _ = app.update(Message::ConsolesLoaded(vec![(7, "PlayStation".to_owned())]));

        let _ = app.update(Message::CatalogLoaded(vec![entry(
            "half-life",
            &["Game", "hearthdeck-store:Steam"],
        )]));

        assert_eq!(tabs(&app, Section::ConsoleGames), ["PlayStation"]);
    }

    /// A shell the pad can drive: the session has been checked and the frontend
    /// owns input, which is what navigation waits for before acting.
    fn navigable() -> App {
        let mut app = shell();
        app.input_ownership
            .update(InputEvent::SessionObserved(false));
        app
    }

    /// A shell that has loaded `count` launchable entries, still on the page the
    /// app opens on.
    fn loaded(count: usize) -> App {
        let mut app = navigable();
        let entries = (0..count)
            .map(|index| entry(&format!("hearthdeck:game-{index}"), &["Game"]))
            .collect();
        let _ = app.update(Message::CatalogLoaded(entries));
        app
    }

    /// The same shell on the library page - which is where the grid, its filters
    /// and its tile menu live. A test about the grid has to be on the page that
    /// has one, and the app opens on the dashboard.
    fn showing(count: usize) -> App {
        let mut app = loaded(count);
        app.page = Page::Library;
        app
    }

    /// A shell whose section offers two facets, so its filter button exists and
    /// its drawer has rows to walk.
    fn showing_a_filter() -> App {
        let mut app = showing(8);
        let facet = |id: &str, label: &str, values: [&str; 2]| Facet {
            id: id.to_owned(),
            label: label.to_owned(),
            options: values
                .into_iter()
                .map(|value| FacetOption {
                    value: value.to_owned(),
                    label: value.to_owned(),
                })
                .collect(),
        };
        let _ = app.update(Message::FacetsLoaded(vec![
            facet(
                "store",
                "Store",
                ["hearthdeck-store:Steam", "hearthdeck-store:GOG"],
            ),
            facet("category", "Category", ["Utility", "Development"]),
        ]));
        app
    }

    /// Sends one navigation event, the way the pad or the keyboard would.
    fn send(app: &mut App, event: GamepadEvent) {
        let _ = app.update(Message::GamepadEvent(event));
    }

    /// The pad is not the input path while a game owns the screen, or before the
    /// frontend has been told it has input at all.
    #[test]
    fn the_pad_does_nothing_until_the_frontend_owns_input() {
        let mut app = shell();
        let before = app.cursors;

        send(&mut app, GamepadEvent::MoveDown);

        assert_eq!(app.cursors, before);
    }

    #[test]
    fn the_pad_walks_the_grid_and_holds_at_the_ends_of_rows() {
        let mut app = showing(12);

        send(&mut app, GamepadEvent::MoveRight);
        send(&mut app, GamepadEvent::MoveDown);

        assert_eq!(app.cursors.tile, 5);

        // A step past the end of a row stays on that row's last tile: it does
        // not walk into the next row, and it does not wrap to the far end of the
        // library.
        for _ in 0..20 {
            send(&mut app, GamepadEvent::MoveRight);
        }
        assert_eq!(app.cursors.tile, 7, "the last tile of its row");

        // And from the last row, down does nothing at all.
        send(&mut app, GamepadEvent::MoveDown);
        assert_eq!(app.cursors.tile, 11);
        send(&mut app, GamepadEvent::MoveDown);
        assert_eq!(app.cursors.tile, 11, "there is no row below it");
    }

    /// What the user complained about, pinned: from the grid's first row, up
    /// does nothing at all. It does not carry them to a tab they were not
    /// looking at, and it does not leave the content.
    #[test]
    fn the_pad_never_leaves_the_content() {
        let mut app = showing(12);

        send(&mut app, GamepadEvent::MoveUp);
        send(&mut app, GamepadEvent::MoveUp);
        send(&mut app, GamepadEvent::MoveLeft);

        assert_eq!(app.cursors.tile, 0, "the first tile, and it stays there");
        assert_eq!(
            app.cursors.ring, None,
            "and the content still holds the cursor"
        );
    }

    /// The remembered column: stepping down out of a row that ends early does
    /// not drag the cursor to the first column, and stepping back up returns to
    /// the column the user came down in.
    #[test]
    fn a_column_survives_a_short_row() {
        // Seven entries in rows of four: the last row holds three.
        let mut app = showing(7);

        for _ in 0..3 {
            send(&mut app, GamepadEvent::MoveRight);
        }
        assert_eq!(app.cursors.tile, 3, "the fourth column of the first row");

        send(&mut app, GamepadEvent::MoveDown);
        assert_eq!(app.cursors.tile, 6, "the short row ends here");
        assert_eq!(app.cursors.column, 3, "and the column is remembered");

        send(&mut app, GamepadEvent::MoveUp);
        assert_eq!(app.cursors.tile, 3, "back to the column it came down");
    }

    /// Confirm acts on the tile the cursor is on, and not on the first one.
    #[test]
    fn confirm_launches_the_tile_the_cursor_is_on() {
        let mut app = showing(4);
        send(&mut app, GamepadEvent::MoveRight);
        assert_eq!(app.cursors.tile, 1);

        send(&mut app, GamepadEvent::Confirm);

        assert_eq!(
            app.launch_state.title(),
            Some("hearthdeck:game-1"),
            "the wrong entry was launched"
        );
        // And the frontend gives input up while that launch is in flight.
        assert!(!app.input_ownership.frontend_has_control());
    }

    /// A shell whose section offers two tabs: the "all" tab, plus the store its
    /// entries came from.
    fn showing_two_tabs() -> App {
        let mut app = navigable();
        let entries = (0..4)
            .map(|index| {
                entry(
                    &format!("hearthdeck:game-{index}"),
                    &["Game", "hearthdeck-store:Steam"],
                )
            })
            .collect();
        let _ = app.update(Message::CatalogLoaded(entries));
        app
    }

    /// The triggers switch tab, which is how the pad reaches them now that the
    /// D-pad stays in the grid. The same path a click takes.
    #[test]
    fn the_trigger_chord_switches_tab() {
        let mut app = showing_two_tabs();
        assert_eq!(app.cur_group, None, "the section opens on its all tab");

        send(&mut app, GamepadEvent::NextTab);

        assert_eq!(app.cur_group, Some(0), "the store's tab should be showing");
        assert_eq!(
            app.cursors.tile, 0,
            "and the list it shows starts at the top"
        );
    }

    /// The shoulder buttons switch section, wrapping.
    #[test]
    fn the_shoulder_buttons_switch_section_and_wrap() {
        let mut app = showing(4);

        send(&mut app, GamepadEvent::NextGroup);

        assert_eq!(app.cur_section, Section::ConsoleGames);
        // Nothing is loaded for that section, so its grid is empty and there is
        // no tile to be on: the cursor sits on the first one of a list that is
        // not there yet, which every read of it tolerates.
        assert_eq!(app.cursors.tile, 0);

        send(&mut app, GamepadEvent::PrevGroup);

        assert_eq!(app.cur_section, Section::PcGames);
        assert_eq!(
            app.cursors.tile, 0,
            "and the list it came back to starts over"
        );
    }

    /// The select button opens the drawer - it is a surface over the grid rather
    /// than a place in it, so it cannot be walked to - and Back gives the pad
    /// back to the tile the user was on.
    #[test]
    fn the_select_button_opens_the_drawer_and_back_gives_the_pad_back() {
        let mut app = showing_a_filter();

        // Walk into the grid first, so there is a tile to come back to.
        send(&mut app, GamepadEvent::MoveRight);
        send(&mut app, GamepadEvent::MoveDown);
        assert_eq!(app.cursors.tile, 5);

        send(&mut app, GamepadEvent::FilterPanel);

        assert!(app.filter_open, "select opens the drawer");
        assert_eq!(
            app.cursors.filter_row, 0,
            "and its cursor starts at the top"
        );

        // While it is up, the pad belongs to it, and the grid's cursor is kept
        // rather than lost.
        send(&mut app, GamepadEvent::MoveDown);
        assert_eq!(app.cursors.filter_row, 1);
        assert_eq!(app.cursors.tile, 5, "the tile is untouched");

        send(&mut app, GamepadEvent::Back);

        assert!(!app.filter_open);
        assert_eq!(app.cursors.tile, 5, "and the pad is back on that tile");
    }

    /// Back reads a failed launch before anything else: that message is the
    /// innermost thing on screen.
    #[test]
    fn back_reads_a_failed_launch_before_anything_else() {
        let mut app = showing(4);
        app.launch_state
            .update(LaunchEvent::Start("Half-Life".to_owned()));
        app.launch_state
            .update(LaunchEvent::Failed("no such file".to_owned()));

        send(&mut app, GamepadEvent::Back);

        assert!(
            app.launch_state.error().is_none(),
            "Back dismisses the failure"
        );
    }

    /// Tab walks the chrome, which is how a keyboard reaches what the pad
    /// reaches by a button. Stepping past either end puts the cursor back in the
    /// content, and a D-pad press takes it back at once.
    #[test]
    fn the_ring_walks_the_chrome_and_comes_back_to_the_content() {
        let mut app = showing_a_filter();
        assert_eq!(app.cursors.ring, None);

        let _ = app.update(Message::ChromeNext);
        assert_eq!(app.cursors.ring, Some(0), "the first section");

        // All the way round and out the far end: three sections, the search box,
        // the section's tab, and the filter button.
        for _ in 0..6 {
            let _ = app.update(Message::ChromeNext);
        }
        assert_eq!(app.cursors.ring, None, "past the last stop is the content");

        // Backwards enters at the far end.
        let _ = app.update(Message::ChromePrevious);
        assert_eq!(app.cursors.ring, Some(5), "the filter button");

        // The pad does not walk the chrome: a direction takes the cursor back to
        // the content instead of moving along it.
        send(&mut app, GamepadEvent::MoveDown);
        assert_eq!(app.cursors.ring, None);
    }

    /// The search button puts the caret in the box without taking the pad out of
    /// the content, so leaving the box puts the user back where they were.
    #[test]
    fn the_search_button_leaves_the_pad_where_it_is() {
        let mut app = showing(4);
        send(&mut app, GamepadEvent::MoveRight);
        assert_eq!(app.cursors.tile, 1);

        send(&mut app, GamepadEvent::Search);

        assert_eq!(app.cursors.tile, 1, "the box takes the caret, not the pad");
        assert_eq!(app.cursors.ring, None);

        send(&mut app, GamepadEvent::Back);

        assert_eq!(app.cursors.tile, 1, "and Back leaves it there");
    }

    /// Inside the drawer, X is the one thing a drawer cannot do for itself, and
    /// it needs no cursor row to do it from.
    #[test]
    fn the_context_button_clears_the_filters_inside_the_drawer() {
        let mut app = showing_a_filter();
        let _ = app.update(Message::SelectFacet {
            facet: "store".to_owned(),
            value: Some("hearthdeck-store:Steam".to_owned()),
        });
        let _ = app.update(Message::ToggleFilterPanel);
        assert_eq!(app.facet_selection.len(), 1);

        send(&mut app, GamepadEvent::ContextMenu);

        assert!(app.facet_selection.is_empty(), "X clears every facet");
    }

    /// The sidebar's footer reads the last figure the poll produced. A poll that
    /// could not read the filesystem must not blank a figure that was already
    /// valid, or the footer would flicker empty every time a read failed.
    #[test]
    fn the_sidebar_footer_shows_the_storage_figure_the_poll_reported() {
        let mut app = shell();
        assert!(
            app.disk_free.is_empty(),
            "nothing is shown before the first poll answers"
        );

        let _ = app.update(Message::DiskFreeLoaded("128.4 GB".to_owned()));
        assert_eq!(app.disk_free, "128.4 GB");

        let _ = app.update(Message::DiskFreeLoaded(String::new()));
        assert_eq!(
            app.disk_free, "128.4 GB",
            "an unreadable filesystem keeps the last figure instead of blanking it"
        );
    }

    /// A list that narrows under the cursor leaves it on a tile that is really
    /// there, so Confirm can never act on an entry the grid stopped drawing.
    #[test]
    fn a_list_that_narrows_leaves_the_cursor_on_a_tile_that_exists() {
        let mut app = showing(2);
        send(&mut app, GamepadEvent::MoveRight);
        assert_eq!(app.cursors.tile, 1);

        let _ = app.update(Message::InputChanged("game-0".to_owned()));

        assert_eq!(app.cursors.tile, 0, "only one entry matches now");
    }

    /// The pad walks the grid and the grid follows: steps inside the rows the
    /// viewport already shows leave the page alone, and the step that would take
    /// the cursor off them scrolls the grid instead of losing it. Without this
    /// the selection walks off the bottom of the screen and the content never
    /// moves - which is what the rewrite shipped without.
    #[test]
    fn the_grid_follows_the_cursor_out_of_the_viewport() {
        // Far more rows than a viewport holds, so there is somewhere to scroll.
        let mut app = showing(8 * 40);
        let metrics = grid::metrics(app.window_width);
        let visible_rows =
            ((app.grid_viewport_height / metrics.row_height).floor() as usize).max(1);

        // Down the rows the viewport already shows: the page stays where it is.
        for _ in 0..visible_rows.saturating_sub(1) {
            send(&mut app, GamepadEvent::MoveDown);
        }
        assert_eq!(
            app.grid_scroll_offset, 0.0,
            "a step inside the viewport moved the page"
        );

        // One more row down leaves through the bottom, and the grid follows.
        send(&mut app, GamepadEvent::MoveDown);
        assert!(
            app.grid_scroll_offset > 0.0,
            "the cursor left the viewport and the grid did not follow it"
        );
    }

    /// The pad's context button opens the tile menu on the tile the cursor is
    /// on, and the menu takes the pad: the grid behind it stops moving.
    #[test]
    fn the_context_button_opens_the_menu_and_the_menu_owns_the_pad() {
        let mut app = showing(6);
        send(&mut app, GamepadEvent::MoveRight);
        assert_eq!(app.cursors.tile, 1);

        send(&mut app, GamepadEvent::ContextMenu);

        assert!(app.menu.is_open(), "X opens the tile menu");
        assert_eq!(app.menu.target(), Some(1), "on the tile it was pressed on");

        // The grid does not move while the menu is up, so the tile it was opened
        // on is still where the user left it when the menu closes.
        send(&mut app, GamepadEvent::MoveDown);
        assert_eq!(app.cursors.tile, 1, "the grid moved behind the menu");

        // The button that opens it is the one that closes it.
        send(&mut app, GamepadEvent::ContextMenu);
        assert!(!app.menu.is_open());
    }

    /// Back closes the menu before it reaches anything behind it: it is the
    /// innermost surface, so it unwinds first.
    #[test]
    fn back_closes_the_menu_first() {
        let mut app = showing(4);
        send(&mut app, GamepadEvent::ContextMenu);
        assert!(app.menu.is_open());

        send(&mut app, GamepadEvent::Back);

        assert!(!app.menu.is_open(), "Back closes the menu");
        assert_eq!(app.cursors.tile, 0, "and the grid cursor never moved");
    }

    /// Confirm runs the row the menu is open on, for the entry the menu belongs
    /// to - not the first entry, and not whatever the grid cursor is over.
    #[test]
    fn confirm_runs_the_row_the_menu_is_open_on() {
        let mut app = showing(4);
        send(&mut app, GamepadEvent::MoveRight);
        send(&mut app, GamepadEvent::ContextMenu);

        send(&mut app, GamepadEvent::Confirm);

        assert_eq!(app.launch_state.title(), Some("hearthdeck:game-1"));
        assert!(
            !app.menu.is_open(),
            "the menu is not left on screen behind the action"
        );
    }

    /// A menu belongs to a tile, and a list that drops that tile closes it -
    /// otherwise a menu would sit open over an entry that is no longer there.
    #[test]
    fn a_list_that_drops_the_menu_target_closes_the_menu() {
        let mut app = showing(4);
        send(&mut app, GamepadEvent::MoveRight);
        send(&mut app, GamepadEvent::MoveRight);
        send(&mut app, GamepadEvent::ContextMenu);
        assert_eq!(app.menu.target(), Some(2));

        let _ = app.update(Message::InputChanged("game-0".to_owned()));

        assert_eq!(app.visible.len(), 1, "only one entry matches now");
        assert!(
            !app.menu.is_open(),
            "the menu outlived the tile it was opened on"
        );
    }

    /// The controller compatibility row is the switch that makes an application
    /// the pad cannot drive usable on its own: the desktop profile hands the
    /// controller to a pointer. Its label carries the state, because a row here
    /// is a label and the message it sends - there is no tick to draw beside it.
    #[test]
    fn the_menu_offers_the_controller_compatibility_switch() {
        let labels = |app: &App| -> Vec<String> {
            app.menu_rows_for(app.menu_target().expect("the menu is open"))
                .into_iter()
                .map(|row| row.label)
                .collect()
        };

        let mut app = showing(2);
        send(&mut app, GamepadEvent::ContextMenu);

        assert_eq!(
            labels(&app),
            [fl!("run"), fl!("enable-controller-compatibility")],
            "an application that has never been switched on offers to switch it on"
        );

        // The second row is the switch, and confirming it flips the flag.
        send(&mut app, GamepadEvent::MoveDown);
        assert_eq!(app.menu.selected(), 1, "the switch is the second row");
        send(&mut app, GamepadEvent::Confirm);

        assert!(
            app.config.desktop_input_enabled("hearthdeck:game-0"),
            "the switch never reached the config"
        );
        assert!(
            !app.menu.is_open(),
            "the switch closes the menu behind it like any other row"
        );

        send(&mut app, GamepadEvent::ContextMenu);
        assert_eq!(
            labels(&app),
            [fl!("run"), fl!("disable-controller-compatibility")],
            "and it now offers to switch it back off"
        );
    }

    /// The menu is as tall as the rows it was built with, so the pad can reach
    /// every one of them: a count that drifted from the rows would leave a row
    /// drawn but unreachable.
    #[test]
    fn the_menu_walks_every_row_it_shows() {
        let mut app = showing(2);
        send(&mut app, GamepadEvent::ContextMenu);

        let target = app.menu_target().expect("the menu is open");
        assert_eq!(
            app.menu_rows(),
            app.menu_rows_for(target).len(),
            "the count the navigation uses is not the list the view draws"
        );

        send(&mut app, GamepadEvent::MoveDown);
        assert_eq!(app.menu.selected(), 1, "the second row is reachable");

        // And the rows wrap, so the pad reaches either one from either end.
        send(&mut app, GamepadEvent::MoveDown);
        assert_eq!(app.menu.selected(), 0);
    }

    /// The dashboard is where the app opens, and the sidebar is the way out of
    /// it: reaching for a section leaves for the library.
    #[test]
    fn the_app_opens_on_the_dashboard_and_the_sidebar_leaves_it() {
        let mut app = loaded(4);
        assert_eq!(app.page, Page::Dashboard, "the app opens on its front door");

        let _ = app.update(Message::SelectSection(Section::Applications));

        assert_eq!(app.page, Page::Library);
        assert_eq!(app.cur_section, Section::Applications);
    }

    /// Back from the library returns to the dashboard, and on the dashboard there
    /// is nothing behind it to reach.
    #[test]
    fn back_returns_to_the_dashboard_and_goes_no_further() {
        let mut app = showing(4);
        let _ = app.update(Message::SelectSection(Section::PcGames));
        assert_eq!(app.page, Page::Library);

        send(&mut app, GamepadEvent::Back);
        assert_eq!(app.page, Page::Dashboard, "Back returns to the dashboard");

        send(&mut app, GamepadEvent::Back);
        assert_eq!(
            app.page,
            Page::Dashboard,
            "the dashboard is the page behind everything"
        );
    }

    /// The pad walks the dashboard's rail and stops at its ends, and Confirm on a
    /// card leaves for the library - which is what a card is a way into.
    #[test]
    fn the_pad_walks_the_dashboard_rail() {
        let mut app = loaded(6);
        assert_eq!(
            app.dashboard_rail_lengths(),
            vec![6],
            "one rail so far, one card per entry"
        );

        send(&mut app, GamepadEvent::MoveRight);
        assert_eq!(app.dashboard_cursor.card, 1, "the rail takes the step");

        // The ends of a rail are ends: it does not wrap, and it does not leave
        // for another rail.
        for _ in 0..10 {
            send(&mut app, GamepadEvent::MoveRight);
        }
        assert_eq!(app.dashboard_cursor.card, 5, "the last card");
        send(&mut app, GamepadEvent::MoveRight);
        assert_eq!(app.dashboard_cursor.card, 5, "and it stays there");

        send(&mut app, GamepadEvent::Confirm);
        assert_eq!(app.page, Page::Library, "Confirm leaves for the library");
    }

    /// A collection as the daemon composes one.
    fn collection(slug: &str, owner: &str, name: Option<&str>, ids: &[&str]) -> Collection {
        Collection {
            slug: slug.to_owned(),
            owner: owner.to_owned(),
            name: name.map(str::to_owned),
            items: ids
                .iter()
                .map(|id| CollectionItem {
                    item_id: (*id).to_owned(),
                    name: (*id).to_owned(),
                    icon: None,
                    played: None,
                })
                .collect(),
        }
    }

    /// The dashboard's rails are the daemon's collections - so a collection this
    /// client has never heard of, which is what a feature's own categories and so
    /// the AI's are, becomes a shelf without a change in the frontend. That is
    /// the whole reason the daemon sends a name with one.
    #[test]
    fn the_dashboards_rails_are_the_daemons_collections() {
        let mut app = loaded(2);
        let _ = app.update(Message::CollectionsLoaded(Ok(vec![
            collection("favorites", "user", None, &["hearthdeck:game-0"]),
            collection(
                "smart:action",
                "ai",
                Some("Action picks"),
                &["hearthdeck:game-1"],
            ),
        ])));

        let shelves = app.dashboard_shelves();
        assert_eq!(shelves.len(), 2, "one rail per collection");
        // A slug the client knows is shown in the user's own language.
        assert_eq!(shelves[0].title, fl!("favorites"));
        // One it does not uses the name the daemon sent with it.
        assert_eq!(shelves[1].title, "Action picks");
        // And the pad walks what the view draws.
        assert_eq!(app.dashboard_rail_lengths(), vec![1, 1]);
    }

    /// A collection with neither a known slug nor a name falls back to its slug
    /// rather than to a blank heading.
    #[test]
    fn a_collection_with_no_name_reads_as_its_slug() {
        let mut app = loaded(1);
        let _ = app.update(Message::CollectionsLoaded(Ok(vec![collection(
            "smart:mystery",
            "ai",
            None,
            &["hearthdeck:game-0"],
        )])));

        assert_eq!(app.dashboard_shelves()[0].title, "smart:mystery");
    }

    /// A card for an item the library holds is a way into it, at the section that
    /// item belongs to.
    #[test]
    fn a_collection_card_that_is_a_library_entry_opens_the_library_at_it() {
        let mut app = loaded(2);
        let _ = app.update(Message::CollectionsLoaded(Ok(vec![collection(
            "favorites",
            "user",
            None,
            &["hearthdeck:game-1"],
        )])));

        send(&mut app, GamepadEvent::Confirm);

        assert_eq!(
            app.page,
            Page::Library,
            "the card is a way into the library"
        );
        assert_eq!(app.cur_section, Section::PcGames);
    }

    /// A card for an item the library does not hold has nowhere to go, so it is
    /// inert rather than a way into some other section.
    #[test]
    fn a_collection_item_the_library_does_not_hold_is_a_dead_card() {
        let mut app = loaded(1);
        let _ = app.update(Message::CollectionsLoaded(Ok(vec![collection(
            "stremio:continue-watching",
            "stremio",
            None,
            &["stremio:tt0111161"],
        )])));

        let shelves = app.dashboard_shelves();
        assert_eq!(shelves[0].title, fl!("continue-watching"));
        assert!(
            shelves[0].cards[0].message.is_none(),
            "an item the library cannot show has nowhere to go"
        );

        send(&mut app, GamepadEvent::Confirm);
        assert_eq!(app.page, Page::Dashboard, "Confirm did nothing");
    }
}
