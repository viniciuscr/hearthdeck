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
use crate::providers::daemon::{DaemonClient, DaemonConfig, DaemonProvider};
use crate::providers::filter::{Facet, cycle};
use crate::screens::library::console::{self, ConsoleListing};
use crate::screens::library::focus::{self, Chrome, Confirm, Cursors, Direction, Layout, Outcome};
use crate::screens::library::sidebar::{self, Sidebar};
use crate::screens::library::{filters, grid, header};
use crate::style::{
    DIVIDER_WIDTH, ICON_LARGE, WINDOW_HEIGHT, WINDOW_WIDTH, content_horizontal_padding,
    filter_drawer_width, grid_viewport_height_fallback, launch_overlay,
    primary_action_button_class, root_background, sidebar_divider,
};
use crate::subscriptions::gamepad::{GamepadEvent, gamepad_events};
use crate::toplevel::{WindowHint, fullscreen_when_it_appears};
use crate::ui;

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
    /// The console list a tab strip is built from: `(platform id, label)`.
    ConsolesLoaded(Vec<(i64, String)>),
    SelectSection(Section),
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
    /// A tile was right-clicked. The context menu lands in a later slice.
    TileContext(usize),
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

pub struct App {
    core: Core,
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
    facet_selection: BTreeMap<String, String>,
    /// Whether the filter sidebar is open.
    filter_open: bool,
    cur_section: Section,
    cur_group: Option<usize>,
    /// Where the pad is: the tile it is on and the column it came down, the row
    /// of the open drawer, and the chrome stop the keyboard's ring is on.
    ///
    /// The shell owns this rather than reading iced's focus, because Confirm has
    /// to act on what the user can see is highlighted; iced is told about it
    /// afterwards, to paint the ring.
    cursors: Cursors,
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
            config,
            config_store,
            daemon_client,
            catalog: Vec::new(),
            console: ConsoleListing::default(),
            visible: Vec::new(),
            facets: Vec::new(),
            facet_selection: BTreeMap::new(),
            filter_open: false,
            cur_section: Section::PcGames,
            cur_group: None,
            // The first tile: with nothing loaded yet there is nothing to point
            // at, and every read of the cursor tolerates that.
            cursors: Cursors::default(),
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
        (app, Task::batch([load, poll, consoles, disk]))
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
            // The context menu moves here next.
            Message::TileContext(_index) => {}
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
        match focus::step(self.cursors, direction, self.layout()) {
            Outcome::Tile { tile, column } => {
                self.cursors.tile = tile;
                self.cursors.column = column;
                self.focus_task()
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
        // Inside the drawer, X is the one thing a drawer cannot do for itself:
        // clear every facet without travelling to a button for it. The clear
        // action is deliberately not a cursor row, so the cursor never has to
        // walk to it.
        if self.filter_open {
            return self.update(Message::ClearFilters);
        }
        // The tile's own menu, which the next slice fills in: X is the button
        // that opens it, wherever the tile is.
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
        // click - gives it back to the content. With nothing else open that is
        // all Back means on this screen; the page behind the library is the
        // Dashboard, which is the one rung still missing.
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
            facet_selection: BTreeMap::new(),
            filter_open: false,
            cur_section: Section::PcGames,
            cur_group: None,
            cursors: Cursors::default(),
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

    /// A shell showing `count` launchable entries in its grid.
    fn showing(count: usize) -> App {
        let mut app = navigable();
        let entries = (0..count)
            .map(|index| entry(&format!("hearthdeck:game-{index}"), &["Game"]))
            .collect();
        let _ = app.update(Message::CatalogLoaded(entries));
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
}
