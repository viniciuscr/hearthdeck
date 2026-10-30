use super::*;
use crate::app_group::{AppGroup, FilterType, GroupSource, SectionUi};
use crate::providers::daemon::RetroRomVersion;
use crate::providers::filter::FacetOption;
use crate::style::PAGE_TRANSITION_DURATION;

use super::animation::ScrollTarget;

/// A shell with no config store: nothing it does can reach the real config
/// file, which is what lets a test drive `update` and read the state it
/// leaves behind. The daemon client is never called either - a test feeds
/// `update` the answer a daemon would have given.
fn shell() -> App {
    App {
        core: Core::default(),
        page: Page::default(),
        page_fade: None,
        now: Instant::now(),
        backdrop_at: dashboard::backdrop::At::default(),
        backdrop_fade: None,
        section_slide: None,
        tab_slide: None,
        scrolls: Scrolls::default(),
        config: AppLibraryConfig::default(),
        config_store: None,
        daemon_client: DaemonClient::new(DaemonConfig {
            base_url: String::new(),
            token: String::new(),
        }),
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
        cursors: Cursors::default(),
        dashboard_cursor: dashboard::focus::Cursor::default(),
        bar_cursor: None,
        rail_offsets: Vec::new(),
        page_offset: 0.0,
        tabs_offset: 0.0,
        search_value: String::new(),
        user_name: String::new(),
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
        retries: super::refresh::Retries::default(),
    }
}

/// Display fixtures with the section and form a daemon response supplies.
fn entry(id: &str, categories: &[&str]) -> Arc<LibraryItem> {
    let mut entry = LibraryItem {
        id: id.to_owned(),
        name: id.to_owned(),
        section: if categories.contains(&"Game") {
            if categories
                .iter()
                .any(|value| value.starts_with("hearthdeck-console:"))
            {
                Section::ConsoleGames
            } else {
                Section::PcGames
            }
        } else {
            Section::Applications
        },
        categories: categories
            .iter()
            .filter(|value| !value.starts_with("hearthdeck-store:"))
            .filter(|value| !value.starts_with("hearthdeck-console:"))
            .map(|category| (*category).to_owned())
            .collect(),
        store: categories
            .iter()
            .find_map(|value| value.strip_prefix("hearthdeck-store:"))
            .map(str::to_owned),
        console: categories.iter().find_map(|value| {
            value
                .strip_prefix("hearthdeck-console:")
                .and_then(|id| id.parse().ok())
                .map(|platform_id| crate::library_item::ConsoleMetadata {
                    platform_id,
                    genres: Vec::new(),
                    regions: Vec::new(),
                })
        }),
        ..LibraryItem::default()
    };
    if Section::Applications.matches(&entry) {
        entry.form_factor = Some(crate::form_factor::FormFactor::Desktop);
    }
    Arc::new(entry)
}

#[test]
fn an_unchanged_catalog_keeps_the_grid_scroll_position() {
    let mut app = shell();
    let entries = vec![entry("game", &["Game"])];
    let _ = app.update(Message::CatalogLoaded(Ok(entries.clone())));
    app.grid_scroll_offset = 240.0;

    let _ = app.update(Message::CatalogLoaded(Ok(entries)));

    assert_eq!(app.grid_scroll_offset, 240.0);
}

#[test]
fn a_romm_sign_in_reloads_consoles_and_removal_clears_the_url() {
    use crate::providers::daemon::RommSettings;
    use settings::romm::Message as Romm;

    let mut app = showing_a_console_game(Vec::new());
    app.config.sync_console_groups(&[(7, "SNES".into())]);
    let scope = app.console_scope();
    assert!(!app.console.entries(&scope).is_empty());
    let connection = || RommSettings {
        base_url: "http://localhost:8080".into(),
        configured: true,
    };
    let _ = app.update(Message::Romm(Romm::Loaded(Ok(Some(connection())))));
    assert_eq!(app.romm.base_url, "http://localhost:8080");
    let _ = app.update(Message::Romm(Romm::SignInOpened));
    let _ = app.update(Message::Romm(Romm::UsernameChanged("admin".into())));
    let _ = app.update(Message::Romm(Romm::PasswordChanged("hunter2".into())));
    let _ = app.update(Message::Romm(Romm::SignInPressed));
    assert!(app.romm.busy);
    let _ = app.update(Message::Romm(Romm::SignedIn(Ok(connection()))));
    assert!(app.romm.sign_in.is_none());
    assert!(app.console.entries(&scope).is_empty());
    assert!(app.config.sections.console_games.is_empty());
    let _ = app.update(Message::Romm(Romm::Removed(Ok(()))));
    assert!(app.romm.base_url.is_empty());
    assert!(app.romm.connection.is_none());
}

#[test]
fn pending_romm_requests_cannot_be_overwritten_or_submitted_twice() {
    use settings::romm::Message as Romm;

    let mut app = shell();
    let _ = app.update(Message::Romm(Romm::UrlChanged(
        "http://localhost:8080".into(),
    )));
    let _ = app.update(Message::Romm(Romm::ConnectPressed));
    assert!(app.romm.busy);
    let _ = app.update(Message::Romm(Romm::UrlChanged(
        "http://localhost:9090".into(),
    )));
    let _ = app.update(Message::Romm(Romm::SignInOpened));
    assert_eq!(app.romm.base_url, "http://localhost:8080");
    assert!(app.romm.sign_in.is_none());
    assert!(!app.romm.can_connect());
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

/// A legacy folder, kept when the catalog rebuilds the derived tabs.
fn add_user_group(app: &mut App, section: Section, name: &str) {
    app.config.sections.get_mut(section).push(AppGroup {
        name: name.to_owned(),
        icon: "folder-symbolic".to_string(),
        filter: FilterType::AppIds(Vec::new()),
        source: GroupSource::Local,
    });
}

/// The tabs a section shows are the ones its entries imply. Without them the
/// strip is the "all" tab alone: the stores and categories the grid switches
/// between have nowhere to be reached from.
#[test]
fn a_loaded_catalog_gives_each_section_its_category_tabs() {
    let mut app = shell();

    let _ = app.update(Message::CatalogLoaded(Ok(vec![
        entry("half-life", &["Game", "hearthdeck-store:Steam"]),
        entry("writer", &["Office"]),
    ])));

    assert_eq!(tabs(&app, Section::PcGames), ["Steam"]);
    assert_eq!(tabs(&app, Section::Applications), ["Desktop"]);
}

/// A tab is a store that installed something: the catalog is where the local
/// sections' tabs come from, so a store leaving the library takes its tab
/// with it.
#[test]
fn a_store_no_installed_game_comes_from_is_not_a_tab() {
    let mut app = shell();
    let _ = app.update(Message::CatalogLoaded(Ok(vec![entry(
        "half-life",
        &["Game", "hearthdeck-store:Steam"],
    )])));
    let _ = app.update(Message::CatalogLoaded(Ok(vec![entry(
        "writer",
        &["Office"],
    )])));

    assert!(
        tabs(&app, Section::PcGames).is_empty(),
        "the Steam tab outlived the game it came from: {:?}",
        tabs(&app, Section::PcGames)
    );
    assert_eq!(tabs(&app, Section::Applications), ["Desktop"]);
}

/// Console Games' tabs are the platforms the daemon reported, which no
/// catalog can imply, so adopting one must leave them where they are.
#[test]
fn a_loaded_catalog_leaves_the_console_tabs_alone() {
    let mut app = shell();
    let _ = app.update(Message::ConsolesLoaded(Ok(vec![
        crate::providers::daemon::RetroConsole {
            id: 7,
            label: "PlayStation".to_owned(),
            artwork: None,
        },
    ])));

    let _ = app.update(Message::CatalogLoaded(Ok(vec![entry(
        "half-life",
        &["Game", "hearthdeck-store:Steam"],
    )])));

    assert_eq!(tabs(&app, Section::ConsoleGames), ["PlayStation"]);
}

/// A console picked on the dashboard opens the library on that console's
/// tab, not on the "all consoles" tab the section opens on by itself.
#[test]
fn a_console_picked_on_the_dashboard_opens_its_own_tab() {
    let mut app = shell();
    let console = |id, label: &str| crate::providers::daemon::RetroConsole {
        id,
        label: label.to_owned(),
        artwork: None,
    };
    let _ = app.update(Message::ConsolesLoaded(Ok(vec![
        console(7, "SNES"),
        console(9, "PlayStation"),
    ])));
    assert_eq!(app.page, Page::Dashboard);

    let _ = app.update(Message::ActivateConsole(9));

    assert_eq!(app.page, Page::Library);
    assert_eq!(app.cur_section, Section::ConsoleGames);
    assert_eq!(app.console_platform_id(), Some(9));
}

/// A failed read is not an empty library: the entries and the tabs they
/// imply stay, instead of being pruned and persisted away.
#[test]
fn a_failed_catalog_read_keeps_the_library_and_its_tabs() {
    let mut app = shell();
    let _ = app.update(Message::CatalogLoaded(Ok(vec![entry(
        "half-life",
        &["Game", "hearthdeck-store:Steam"],
    )])));

    let _ = app.update(Message::CatalogLoaded(Err("daemon down".to_owned())));

    assert_eq!(app.catalog.len(), 1);
    assert_eq!(tabs(&app, Section::PcGames), ["Steam"]);
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
    let _ = app.update(Message::CatalogLoaded(Ok(entries)));
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
    let _ = app.update(Message::FacetsLoaded(Ok(vec![
        facet(
            "store",
            "Store",
            ["hearthdeck-store:Steam", "hearthdeck-store:GOG"],
        ),
        facet("category", "Category", ["Utility", "Development"]),
    ])));
    app
}

/// A shell on the console grid whose one record is a RomM game, so the
/// details screen has a tile to open from. `versions` are the files that
/// game collapsed to; empty means a single-file game.
fn showing_a_console_game(versions: Vec<RetroRomVersion>) -> App {
    let mut app = navigable();
    app.page = Page::Library;
    app.cur_section = Section::ConsoleGames;
    let scope = app.console_scope();
    let generation = app.console.begin(&scope, 0).expect("the load is allowed");
    let mut by_id = BTreeMap::new();
    if !versions.is_empty() {
        by_id.insert("romm:42".to_owned(), versions);
    }
    let _ = app.update(Message::ConsolePageLoaded {
        scope,
        generation,
        result: Ok(console::Page {
            entries: vec![entry("romm:42", &["Game", "hearthdeck-console:7"])],
            versions: by_id,
            total: 1,
            offset: 0,
        }),
    });
    app
}

/// A shell on the console grid with two console tabs behind it - the
/// platforms the daemon reported - showing the first tab's one game.
fn showing_console_tabs() -> App {
    let mut app = navigable();
    app.page = Page::Library;
    app.cur_section = Section::ConsoleGames;
    app.config
        .sync_console_groups(&[(7, "SNES".to_owned()), (9, "PlayStation".to_owned())]);
    app.cur_group = Some(0);
    console_page(
        &mut app,
        vec![entry("romm:snes", &["Game", "hearthdeck-console:7"])],
    );
    app
}

/// Feeds the console grid one page the way a daemon answer would, for the
/// scope the shell is currently showing.
fn console_page(app: &mut App, entries: Vec<Arc<LibraryItem>>) {
    let scope = app.console_scope();
    let generation = app.console.begin(&scope, 0).expect("the load is allowed");
    let total = entries.len() as u64;
    let _ = app.update(Message::ConsolePageLoaded {
        scope,
        generation,
        result: Ok(console::Page {
            entries,
            versions: BTreeMap::new(),
            total,
            offset: 0,
        }),
    });
}

/// The entry ids on the grid, in the order they are drawn.
fn drawn(app: &App) -> Vec<String> {
    app.visible.iter().map(|entry| entry.id.clone()).collect()
}

/// Switching console tabs draws the tab that was switched to - its own pages
/// when it has any, nothing while its first page is still out - and never
/// the games of the tab the user left.
#[test]
fn switching_console_tabs_does_not_leave_the_last_tabs_games_on_the_grid() {
    let mut app = showing_console_tabs();
    assert_eq!(
        drawn(&app),
        ["romm:snes"],
        "the SNES tab shows its own game"
    );

    let _ = app.update(Message::SelectGroup(Some(1)));
    settle(&mut app);

    assert_eq!(app.cur_group, Some(1), "the PlayStation tab is showing");
    assert!(
        drawn(&app).is_empty(),
        "the SNES game is still on the grid under the PlayStation tab: {:?}",
        drawn(&app)
    );
}

/// A tab that has already answered does not ask again: switching back reads
/// its cached pages the moment the swap happens.
#[test]
fn switching_back_to_a_loaded_console_tab_paints_its_pages_at_once() {
    let mut app = showing_console_tabs();
    let _ = app.update(Message::SelectGroup(Some(1)));
    settle(&mut app);
    console_page(
        &mut app,
        vec![entry("romm:ps", &["Game", "hearthdeck-console:9"])],
    );

    let _ = app.update(Message::SelectGroup(Some(0)));
    settle(&mut app);

    assert_eq!(
        drawn(&app),
        ["romm:snes"],
        "switching back must read the cached pages, not wait for a fetch"
    );
}

/// The console grid is narrowed at the source, so typing cannot re-project
/// it: the list the user is leaving stays under their hands while they
/// type. Settling the query is different - there the grid snaps to the
/// settled scope's own records, nothing at all until its first page lands.
#[test]
fn typing_a_console_search_keeps_the_grid_while_the_query_is_not_settled() {
    let mut app = showing_console_tabs();

    let _ = app.update(Message::InputChanged("zeld".to_owned()));

    assert_eq!(
        drawn(&app),
        ["romm:snes"],
        "the grid emptied itself mid-typing"
    );

    let _ = app.update(Message::ConsoleSearchSettled("zeld".to_owned()));
    assert!(
        drawn(&app).is_empty(),
        "the settled query is still showing the games of the search it replaced: {:?}",
        drawn(&app)
    );

    console_page(
        &mut app,
        vec![entry("romm:z", &["Game", "hearthdeck-console:7"])],
    );
    assert_eq!(drawn(&app), ["romm:z"]);
}

/// Coming back to a query that already answered draws its own games at
/// once: the answer is in the cache, so nothing may stay up from the query
/// it replaced.
#[test]
fn clearing_a_console_search_shows_the_unfiltered_scope_again() {
    let mut app = showing_console_tabs();

    let _ = app.update(Message::InputChanged("zeld".to_owned()));
    let _ = app.update(Message::ConsoleSearchSettled("zeld".to_owned()));
    console_page(
        &mut app,
        vec![entry("romm:z", &["Game", "hearthdeck-console:7"])],
    );
    assert_eq!(drawn(&app), ["romm:z"]);

    let _ = app.update(Message::InputChanged(String::new()));
    let _ = app.update(Message::ConsoleSearchSettled(String::new()));

    assert_eq!(
        drawn(&app),
        ["romm:snes"],
        "the cleared search is still showing the searched games: {:?}",
        drawn(&app)
    );
}

/// A page that warms a tab in the background lands in that tab's cache and
/// repaints nothing - but it is what that tab opens from.
#[test]
fn a_prefetched_page_lands_in_its_own_scope_without_repainting_the_grid() {
    let mut app = showing_console_tabs();

    let scope = ConsoleScope::plain(Some(9));
    let generation = app
        .console
        .begin(&scope, 0)
        .expect("the prefetch request is allowed");
    let _ = app.update(Message::ConsolePageLoaded {
        scope,
        generation,
        result: Ok(console::Page {
            entries: vec![entry("romm:ps", &["Game", "hearthdeck-console:9"])],
            versions: BTreeMap::new(),
            total: 1,
            offset: 0,
        }),
    });

    assert_eq!(
        drawn(&app),
        ["romm:snes"],
        "the warm-up page repainted the screen it does not describe"
    );

    let _ = app.update(Message::SelectGroup(Some(1)));
    settle(&mut app);
    assert_eq!(
        drawn(&app),
        ["romm:ps"],
        "the warmed tab does not open from its cache"
    );
}

/// Sends one navigation event, the way the pad or the keyboard would.
fn send(app: &mut App, event: GamepadEvent) {
    let _ = app.update(Message::GamepadEvent(event));
}

/// Runs the frame clock past every duration, so the slides and the scroll land
/// and retire: the first tick applies what is pending under the cover, the
/// second drops it. Nothing here waits on the real clock - the tick carries its
/// own timestamp.
fn settle(app: &mut App) {
    let ahead = Instant::now() + std::time::Duration::from_secs(1);
    let _ = app.update(Message::Animate(ahead));
    let _ = app.update(Message::Animate(ahead));
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
fn the_keyboard_chord_puts_the_caret_in_the_search_box() {
    let mut app = shell();
    app.input_ownership
        .update(crate::input_ownership::Event::SessionObserved(false));

    // The shell's own search box is where the pad can type, so the chord shows
    // the library and focuses the box - which is what makes the keyboard
    // appear. On a sibling page that means showing the library first.
    send(&mut app, GamepadEvent::Keyboard);

    assert_eq!(app.page, Page::Library);
    assert_eq!(
        app.text_entry.active(),
        Some(Field::Search),
        "the chord did not hand the pad to the keyboard"
    );
}

/// The keyboard takes the pad: while a text field owns the caret no button but
/// Back reaches the screen, and Back gives the pad back without leaving the
/// page it was typing on.
#[test]
fn the_pad_belongs_to_the_keyboard_while_a_text_field_has_the_caret() {
    let mut app = showing(12);
    app.input_ownership
        .update(crate::input_ownership::Event::SessionObserved(false));
    let before = app.cursors;

    let _ = app.update(Message::TextFieldFocused(Field::Search));
    for event in [
        GamepadEvent::MoveDown,
        GamepadEvent::MoveRight,
        GamepadEvent::Confirm,
        GamepadEvent::Search,
        GamepadEvent::FilterPanel,
    ] {
        send(&mut app, event);
    }

    assert_eq!(
        app.cursors, before,
        "a button moved the screen behind the keys"
    );
    assert!(
        !app.filter_open,
        "Select opened the drawer under the keyboard"
    );
    assert!(
        app.text_entry.is_active(),
        "only Back dismisses the keyboard"
    );

    send(&mut app, GamepadEvent::Back);

    assert!(
        !app.text_entry.is_active(),
        "Back did not give the pad back"
    );
    assert_eq!(
        app.page,
        Page::Library,
        "dismissing the keyboard also left the page"
    );

    // With the caret gone the pad drives the screen again.
    send(&mut app, GamepadEvent::MoveDown);
    assert_ne!(app.cursors, before);
}

/// The caret is what decides that the keyboard is up, not which window the
/// compositor considers focused. cosmic-osk hides itself on East without
/// telling the shell, and a focus change can land while the keyboard is still
/// up, so ownership being anything but `Frontend` must not swallow Back.
#[test]
fn back_dismisses_the_keyboard_even_when_the_window_lost_focus() {
    let mut app = showing(12);
    app.input_ownership
        .update(crate::input_ownership::Event::SessionObserved(false));
    let _ = app.update(Message::TextFieldFocused(Field::RommPassword));
    app.input_ownership
        .update(crate::input_ownership::Event::FrontendUnfocused);
    assert!(!app.input_ownership.frontend_has_control());

    send(&mut app, GamepadEvent::Back);

    assert!(
        !app.text_entry.is_active(),
        "Back was swallowed by the ownership gate"
    );
    assert_eq!(
        app.page,
        Page::Library,
        "dismissing the keyboard also left the page"
    );
}

/// A blur names its field: the one the user already left cannot take the caret
/// away from the one they are typing in.
#[test]
fn a_stale_blur_does_not_hand_the_pad_back() {
    let mut app = showing(12);
    app.input_ownership
        .update(crate::input_ownership::Event::SessionObserved(false));
    let _ = app.update(Message::TextFieldFocused(Field::RommPassword));
    let before = app.cursors;

    let _ = app.update(Message::TextFieldBlurred(Field::RommUsername));
    send(&mut app, GamepadEvent::MoveDown);

    assert_eq!(
        app.text_entry.active(),
        Some(Field::RommPassword),
        "a stale blur took the caret out of the field that had it"
    );
    assert_eq!(app.cursors, before, "the pad moved under the keyboard");
}

/// The sign-in dialog cannot be walked to, so Enter is how the pad moves from
/// the username to the password, and how it sends the form.
#[test]
fn accepting_a_credential_field_moves_to_the_next_one() {
    let mut app = shell();
    app.romm.base_url = "http://127.0.0.1:8080".to_owned();
    app.romm.sign_in = Some(settings::romm::SignIn {
        username: "player".to_owned(),
        password: "secret".to_owned(),
    });

    let _ = app.update(Message::TextFieldSubmitted(Field::RommUsername));
    assert_eq!(app.text_entry.active(), Some(Field::RommPassword));

    let _ = app.update(Message::TextFieldSubmitted(Field::RommPassword));

    assert!(app.romm.busy, "the password did not send the sign-in form");
    assert!(
        !app.text_entry.is_active(),
        "the keyboard stayed up over a form that was sent"
    );
}

/// Opening the sign-in dialog puts the caret in the username itself: the pad
/// has no cursor that could walk to the field, so the keyboard has to come up
/// with the dialog.
#[test]
fn opening_the_sign_in_dialog_puts_the_caret_in_the_username() {
    let mut app = shell();

    let _ = app.update(Message::Romm(settings::romm::Message::SignInOpened));
    assert_eq!(
        app.text_entry.active(),
        Some(Field::RommUsername),
        "the dialog opened without a field to type into"
    );

    let _ = app.update(Message::Romm(settings::romm::Message::SignInClosed));
    assert!(
        !app.text_entry.is_active(),
        "the caret outlived the dialog it was in"
    );
}

/// The sign-in dialog is a surface over the settings page: Back closes it and
/// Confirm submits it, and neither reaches the page behind.
#[test]
fn the_pad_answers_the_sign_in_dialog() {
    let mut app = showing(12);
    app.input_ownership
        .update(crate::input_ownership::Event::SessionObserved(false));
    app.romm.base_url = "http://127.0.0.1:8080".to_owned();
    app.romm.sign_in = Some(settings::romm::SignIn {
        username: "player".to_owned(),
        password: "secret".to_owned(),
    });

    send(&mut app, GamepadEvent::Confirm);
    assert!(app.romm.busy, "the pad could not submit the dialog");

    app.romm.busy = false;
    send(&mut app, GamepadEvent::Back);

    assert!(app.romm.sign_in.is_none(), "Back did not close the dialog");
    assert_eq!(app.page, Page::Library, "Back also left the page");
}

/// A page that is left takes its fields with it, so the pad cannot stay handed
/// to a keyboard nothing is typing into.
#[test]
fn leaving_a_page_gives_the_pad_back() {
    let mut app = showing(12);
    let _ = app.update(Message::TextFieldFocused(Field::Search));

    app.switch_page(Page::Settings);

    assert!(
        !app.text_entry.is_active(),
        "the caret outlived the field it was in"
    );
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
        app.launch_state,
        LaunchState::Launching {
            title: "hearthdeck:game-1".to_owned()
        },
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
    let _ = app.update(Message::CatalogLoaded(Ok(entries)));
    app
}

/// The triggers switch tab, which is how the pad reaches them now that the
/// D-pad stays in the grid. The same path a click takes.
#[test]
fn the_trigger_chord_switches_tab() {
    let mut app = showing_two_tabs();
    assert_eq!(app.cur_group, None, "the section opens on its all tab");

    send(&mut app, GamepadEvent::NextTab);
    let slide = app.tab_slide.as_ref().expect("the tab change should slide");
    assert_eq!(*slide.target(), Some(0));
    assert_eq!(app.cur_group, None, "the grid swaps under the cover");

    settle(&mut app);

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
    settle(&mut app);

    assert_eq!(app.cur_section, Section::ConsoleGames);
    // Nothing is loaded for that section, so its grid is empty and there is
    // no tile to be on: the cursor sits on the first one of a list that is
    // not there yet, which every read of it tolerates.
    assert_eq!(app.cursors.tile, 0);

    send(&mut app, GamepadEvent::PrevGroup);
    settle(&mut app);

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

/// A library whose section carries more tabs than its strip can show at once:
/// the far ones start off the right edge.
fn showing_a_long_tab_strip() -> App {
    let mut app = showing(4);
    for name in [
        "Nintendo Entertainment System",
        "Super Nintendo Entertainment System",
        "Nintendo 64",
        "Game Boy Advance",
        "Sega Mega Drive",
        "Sony PlayStation",
    ] {
        add_user_group(&mut app, Section::PcGames, name);
    }
    app
}

/// Whether tab `index` sits entirely inside the strip's visible width, measured
/// the way the follow measures it.
fn tab_is_on_screen(app: &App, index: usize) -> bool {
    let widths = header::tab_widths(app.cur_section, app.config.sections.get(app.cur_section));
    let viewport = header::tab_viewport(app.window_width, !app.facets.is_empty());
    let gap = f32::from(theme::spacing().space_m);
    let start = widths.iter().take(index).sum::<f32>() + index as f32 * gap;
    start >= app.tabs_offset - 0.5 && start + widths[index] <= app.tabs_offset + viewport + 0.5
}

/// Triggering through the tabs scrolls the strip once the selection reaches its
/// edge, so the highlight cannot walk off the right of the window.
#[test]
fn a_tab_past_the_strips_edge_scrolls_the_strip_to_it() {
    let mut app = showing_a_long_tab_strip();
    assert_eq!(app.tabs_offset, 0.0, "the strip starts at its beginning");

    // The first group's tab is already in view, so the strip stays put for it:
    // the row does not move until the selection reaches the edge.
    send(&mut app, GamepadEvent::NextTab);
    settle(&mut app);
    assert_eq!(app.tabs_offset, 0.0, "the strip moved for a tab on screen");

    for _ in 0..5 {
        send(&mut app, GamepadEvent::NextTab);
        settle(&mut app);
    }

    assert_eq!(app.cur_group, Some(5), "the last group's tab is selected");
    assert!(
        app.tabs_offset > 0.0,
        "the strip never followed the selection"
    );
    assert!(
        tab_is_on_screen(&app, 6),
        "the selected tab is off screen at {}",
        app.tabs_offset
    );
}

/// The keyboard's ring walks the same tabs, so a tab it stops on has to be
/// brought into view the same way - the ring is a cursor on the chrome, and a
/// cursor the user cannot see is not a cursor.
#[test]
fn the_ring_brings_the_tab_it_stops_on_into_view() {
    let mut app = showing_a_long_tab_strip();

    // Backwards enters the ring at its last stop, which without a filter button
    // is the last tab.
    let _ = app.update(Message::ChromePrevious);

    assert_eq!(
        app.cursors.ring,
        Some(focus::ring_len(app.layout()) - 1),
        "the ring is on the last stop"
    );
    assert_eq!(
        app.scrolls.targets(),
        vec![ScrollTarget::Tabs],
        "the strip eases to the tab rather than snapping"
    );
    assert!(
        tab_is_on_screen(&app, 6),
        "the tab the ring is on is off screen at {}",
        app.tabs_offset
    );
}

/// The strip belongs to the section on screen: switching sections puts the new
/// one's strip back at its start instead of leaving it scrolled where the old
/// tabs ended.
#[test]
fn a_new_section_starts_its_strip_at_the_beginning() {
    let mut app = showing_a_long_tab_strip();
    for _ in 0..6 {
        send(&mut app, GamepadEvent::NextTab);
        settle(&mut app);
    }
    assert!(app.tabs_offset > 0.0);

    let _ = app.update(Message::SelectSection(Section::Applications));
    settle(&mut app);

    assert_eq!(
        app.tabs_offset, 0.0,
        "the new section's strip kept the old one's offset"
    );
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
    let visible_rows = ((app.grid_viewport_height / metrics.row_height).floor() as usize).max(1);

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

/// Walks the grid past the fold the way the pad does, so the follow has
/// scrolled and the cursor sits rows below the top.
fn walked_past_the_fold(app: &mut App) {
    let metrics = grid::metrics(app.window_width);
    let visible_rows = ((app.grid_viewport_height / metrics.row_height).floor() as usize).max(1);
    for _ in 0..visible_rows + 3 {
        send(app, GamepadEvent::MoveDown);
    }
    settle(app);
}

/// Coming back to the library rebuilds its grid at the top of the list - the
/// scrollable's offset lives in the widget tree the trip drops - while the
/// cursors keep their tile. The follow has to run again for the rebuilt grid,
/// or the user returns to a viewport scrolled away from where the focus is.
#[test]
fn returning_to_the_library_puts_the_scroll_back_under_the_focus() {
    let mut app = showing(8 * 40);
    walked_past_the_fold(&mut app);
    let focused = app.cursors.tile;
    let followed = app.grid_scroll_offset;
    assert!(followed > 0.0, "the walk should have scrolled the grid");

    // Off to a sibling page and back: the round trip a tab switch makes.
    let _ = app.update(Message::OpenDashboard);
    age_page_fade(&mut app);
    let _ = app.update(Message::OpenLibrary);
    age_page_fade(&mut app);
    let _ = app.update(Message::Animate(Instant::now()));

    assert_eq!(app.cursors.tile, focused, "the cursors keep their tile");
    assert_eq!(
        app.scrolls.targets(),
        vec![ScrollTarget::Grid],
        "the follow is re-run for the rebuilt grid"
    );
    assert_eq!(
        app.grid_scroll_offset, followed,
        "the viewport lands where the focus left it"
    );
}

/// A launch takes the screen over: the overlay stands in for the grid until
/// the game runs, and a session can last hours. The follow has to run when the
/// overlay drops, not when the game exits - the user comes back from the game
/// to the tile they launched.
#[test]
fn a_dismissed_launch_puts_the_scroll_back_under_the_launched_tile() {
    let mut app = showing(8 * 40);
    walked_past_the_fold(&mut app);
    let focused = app.cursors.tile;
    let followed = app.grid_scroll_offset;
    assert!(followed > 0.0, "the walk should have scrolled the grid");

    send(&mut app, GamepadEvent::Confirm);
    // The daemon's acceptance is what schedules the delayed dismissal of the
    // overlay; the dismissal itself is the message that drops it.
    let _ = app.update(Message::DismissLaunch);
    let _ = app.update(Message::Animate(Instant::now()));

    assert_eq!(
        app.cursors.tile, focused,
        "the cursors keep the launched tile"
    );
    assert_eq!(
        app.scrolls.targets(),
        vec![ScrollTarget::Grid],
        "the follow is re-run when the overlay drops"
    );
    assert_eq!(
        app.grid_scroll_offset, followed,
        "the grid waits under the focus for the game to end"
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

    assert_eq!(
        app.launch_state,
        LaunchState::Launching {
            title: "hearthdeck:game-1".to_owned()
        }
    );
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

#[test]
fn library_tabs_offer_no_membership_editing() {
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
        "the all tab has no folder to take an app out of"
    );

    // X's menu only exists on the library grid, and the app opens on the
    // dashboard - so the test has to stand where the menu lives.
    let mut app = showing_two_tabs();
    app.page = Page::Library;
    let _ = app.update(Message::SelectGroup(Some(0)));
    settle(&mut app);
    send(&mut app, GamepadEvent::ContextMenu);
    assert_eq!(
        labels(&app),
        [fl!("run"), fl!("enable-controller-compatibility")],
        "a category tab must not offer to change its membership"
    );
    assert_eq!(tabs(&app, Section::PcGames), ["Steam"]);
    assert_eq!(app.visible.len(), 4);
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
/// card launches the item it names - without leaving the dashboard, which the
/// launch overlay covers.
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
    assert_eq!(
        app.launch_state,
        LaunchState::Launching {
            title: "hearthdeck:game-5".to_owned()
        },
        "Confirm launches the card the cursor is on"
    );
    assert_eq!(app.page, Page::Dashboard, "the launch does not navigate");
}

/// One step can move two lists: landing on a rail below the fold scrolls the
/// page down to it and that rail across to the remembered card. The two easings
/// have to coexist; when the page replaced the rail, the cursor landed on a card
/// no one could see.
#[test]
fn a_step_into_an_offscreen_rail_eases_the_page_and_the_rail_together() {
    let mut app = two_rails();
    app.dashboard_cursor = dashboard::focus::Cursor {
        rail: 0,
        card: 8,
        remembered: 8,
    };

    send(&mut app, GamepadEvent::MoveDown);

    let targets = app.scrolls.targets();
    assert!(
        targets.contains(&ScrollTarget::Rail(1)),
        "the rail follows the remembered card: {targets:?}"
    );
    assert!(
        targets.contains(&ScrollTarget::Page),
        "and the page follows the rail: {targets:?}"
    );
}

/// The rail below the fold keeps following the cursor sideways after the page
/// has settled on it - the step's page scroll must not take the rail's scroll
/// with it on every move.
#[test]
fn a_rail_below_the_fold_still_follows_the_cursor_sideways() {
    let mut app = two_rails();
    send(&mut app, GamepadEvent::MoveDown);
    settle(&mut app);
    assert_eq!(
        app.dashboard_cursor,
        dashboard::focus::Cursor {
            rail: 1,
            card: 0,
            remembered: 0,
        }
    );

    // With the smaller tiles, more cards fit the rail: walk past them until
    // the focused one leaves the rail's view.
    for _ in 0..8 {
        send(&mut app, GamepadEvent::MoveRight);
    }

    assert_eq!(
        app.scrolls.targets(),
        vec![ScrollTarget::Rail(1)],
        "a card past the rail's view moves the rail, and it is the only list moving"
    );
}

/// A rail that arrives late - the RomM consoles, whose server is still loading -
/// is inserted under the cursor's feet. The pad stays on the rail it chose and
/// the page stays under it: no snap back to the top.
#[test]
fn a_rail_that_arrives_late_does_not_snap_the_page_back_to_the_top() {
    let mut app = shell();
    let _ = app.update(Message::CollectionsLoaded(Ok(vec![
        arrivals(&["hearthdeck:stremio-arrivals:tt1"]),
        collection("favorites", None, &["hearthdeck:game-0"]),
        collection("play-later", None, &["hearthdeck:game-1"]),
    ])));

    // Down onto the rail below the arrivals: the page moves down to it.
    let _ = app.step_dashboard(Direction::Down);
    let landed = app.page_offset;
    assert!(landed > 0.0, "the page moved down to the rail: {landed}");
    assert_eq!(app.dashboard_cursor.rail, 1);

    // The consoles arrive: a rail is inserted right below the arrivals, under
    // the cursor's feet.
    let _ = app.update(Message::ConsolesLoaded(Ok(vec![
        crate::providers::daemon::RetroConsole {
            id: 1,
            label: "Mega Drive".to_owned(),
            artwork: None,
        },
    ])));

    assert_eq!(
        app.dashboard_cursor.rail, 2,
        "the cursor stays on the rail it chose"
    );
    assert!(
        app.page_offset >= landed,
        "the page does not snap back to the top: {}",
        app.page_offset
    );
}

/// A rail card draws the icon its own item carries, resolved by the provider,
/// and never one looked up in this client's library by item id.
///
/// The lookup was the bug. No media record is in the application library at
/// all, no console game is either, and a curated item has no library entry to
/// find - so those cards drew the generic mark, or worse, whatever the library
/// happened to hold under the same id.
#[test]
fn a_rail_card_draws_the_icon_its_own_item_carries() {
    use cosmic::desktop::fde::IconSource;
    use std::path::PathBuf;

    let mut app = shell();
    // A library entry under the rail item's own id, wearing artwork of its
    // own: it is not what the card draws.
    let _ = app.update(Message::CatalogLoaded(Ok(vec![Arc::new(LibraryItem {
        id: "hearthdeck:game-0".to_owned(),
        name: "Library's own".to_owned(),
        icon: IconSource::Path(PathBuf::from("/usr/share/icons/library-only.png")),
        ..LibraryItem::default()
    })])));
    let rail = |app: &mut App, icon: Option<&str>| {
        let _ = app.update(Message::CollectionsLoaded(Ok(vec![Collection {
            slug: "favorites".to_owned(),
            kind: "membership".to_owned(),
            rule: None,
            role: "favorite".to_owned(),
            position: 0,
            owner: "system".to_owned(),
            name: None,
            items: vec![CollectionItem {
                item_id: "hearthdeck:game-0".to_owned(),
                name: "Game 0".to_owned(),
                icon: icon.map(str::to_owned),
                background: None,
                logo: None,
                description: None,
                played: None,
            }],
        }])));
    };

    // The cover the provider resolved for a console game. That game is not in
    // the library at all, and the card draws the file it was sent.
    rail(
        &mut app,
        Some("/home/user/.cache/hearthdeck/icons/cover.png"),
    );
    let drawn = &app.dashboard_shelves()[0].cards[0].icon;
    assert!(!drawn.symbolic, "the card draws the cover it was sent");
    assert!(
        matches!(drawn.data, cosmic::widget::icon::Data::Image(_)),
        "a file is drawn as the image it is"
    );

    // A poster the provider could not fetch is not artwork, and it does not
    // borrow the library's icon for the same id: the card draws the mark.
    rail(
        &mut app,
        Some("https://images.metahub.space/poster/medium/tt0149460/img"),
    );
    assert!(
        app.dashboard_shelves()[0].cards[0].icon.symbolic,
        "an unfetched poster is the generic mark, not the library's icon"
    );
}

/// The new-arrivals collection as the daemon composes it: its items carry the
/// wide art the window's background draws with, which no other rail's items
/// have.
fn arrivals(ids: &[&str]) -> Collection {
    let mut collection = collection(NEW_ARRIVALS_COLLECTION, None, ids);
    for (index, item) in collection.items.iter_mut().enumerate() {
        item.background = Some(format!("/home/user/.cache/hearthdeck/backdrop-{index}.png"));
    }
    collection
}

/// What has just arrived is the dashboard's first rail - the banner's old
/// place, above the blank space - and it is a rail like every other.
#[test]
fn what_has_just_arrived_is_the_dashboards_first_rail() {
    let mut app = shell();
    let _ = app.update(Message::CollectionsLoaded(Ok(vec![
        collection("favorites", None, &["hearthdeck:game-0"]),
        arrivals(&[
            "hearthdeck:stremio-arrivals:tt1",
            "hearthdeck:stremio-arrivals:tt2",
        ]),
    ])));

    let shelves = app.dashboard_shelves();
    assert_eq!(
        shelves[0].title,
        fl!("new-arrivals"),
        "the arrivals rail takes the banner's old place, whatever order they arrive in"
    );
    assert_eq!(
        app.dashboard_rail_lengths(),
        vec![2, 1],
        "and the pad walks it like it walks every other rail"
    );
}

/// The window's background follows the pad across the rail: one title's art
/// cross-fading into the next's, and out into the wallpaper when the pad steps
/// onto a card whose feed sent no wide art.
#[test]
fn the_background_follows_the_pad_across_the_rail() {
    use crate::style::BACKGROUND_TRANSITION_DURATION;

    let mut app = shell();
    let _ = app.update(Message::CollectionsLoaded(Ok(vec![
        arrivals(&[
            "hearthdeck:stremio-arrivals:tt1",
            "hearthdeck:stremio-arrivals:tt2",
        ]),
        collection("favorites", None, &["hearthdeck:game-0"]),
    ])));

    // Along the arrivals rail the art cross-fades from card to card.
    let _ = app.step_dashboard(Direction::Right);
    let motion = app.backdrop_motion();
    assert_eq!(
        motion.at,
        dashboard::backdrop::At { rail: 0, card: 1 },
        "the background is the card the pad is on"
    );
    assert!(motion.fading.is_some(), "and the change is a cross-fade");
    let settle = app.now + BACKGROUND_TRANSITION_DURATION * 2;
    let _ = app.update(Message::Animate(settle));
    assert_eq!(app.backdrop_motion().fading, None);

    // Down onto a rail whose cards carry no wide art hands the background back
    // to the wallpaper: the last art fades out, it does not vanish.
    let _ = app.step_dashboard(Direction::Down);
    let motion = app.backdrop_motion();
    assert_eq!(motion.at.rail, 1);
    assert!(motion.fading.is_some(), "the art fades out under the pad");
    let _ = app.update(Message::Animate(
        app.now + BACKGROUND_TRANSITION_DURATION * 2,
    ));
    assert_eq!(app.backdrop_motion().fading, None);
}

/// The first rail snaps the page to its top: the blank space above it is kept
/// for the art and comes with the rail into view - the rail at the bottom of
/// the screen and the window onto the art filling everything above it, at
/// whatever size the display is.
#[test]
fn the_first_rail_snaps_the_page_to_the_top() {
    for window_height in [WINDOW_HEIGHT, 1080.0] {
        let mut app = shell();
        app.window_height = window_height;
        let _ = app.update(Message::CollectionsLoaded(Ok(vec![
            arrivals(&["hearthdeck:stremio-arrivals:tt1"]),
            collection("favorites", None, &["hearthdeck:game-0"]),
            collection("play-later", None, &["hearthdeck:game-1"]),
        ])));

        // Down past the first rail, the page follows the pad to the rails below.
        let _ = app.step_dashboard(Direction::Down);
        let landed = app.page_offset;
        let _ = app.step_dashboard(Direction::Down);
        assert!(app.page_offset > 0.0, "the page followed the pad down");
        // Back up onto the first rail snaps it to the top again.
        let _ = app.step_dashboard(Direction::Up);
        let _ = app.step_dashboard(Direction::Up);
        assert_eq!(app.page_offset, 0.0, "the first rail pins the page's top");
        // And the rail below lands in the same place every time: one scroll,
        // one landing, no snap that comes back around.
        let _ = app.step_dashboard(Direction::Down);
        assert_eq!(
            app.page_offset, landed,
            "the second rail lands where it landed before"
        );
    }
}

/// A report from a scroll the shell is driving describes where the travel
/// happens to be, not where the shell sent it: taking it would leave the next
/// follow measuring from the wrong place, which reads as the screen snapping
/// around after the pad leaves the banner.
#[test]
fn a_follow_scroll_is_not_overwritten_by_its_own_reports() {
    use crate::style::SCROLL_TRANSITION_DURATION;

    let mut app = shell();
    let _ = app.update(Message::CollectionsLoaded(Ok(vec![
        arrivals(&["hearthdeck:stremio-arrivals:tt1"]),
        collection("favorites", None, &["hearthdeck:game-0"]),
    ])));

    // Down off the banner drives the page to the rail below the fold.
    let _ = app.step_dashboard(Direction::Down);
    let target = app.page_offset;
    assert!(
        app.scrolls.is_moving(ScrollTarget::Page),
        "the page is travelling to the rail"
    );
    let _ = app.update(Message::PageScrolled(5.0));
    assert_eq!(
        app.page_offset, target,
        "mid-travel reports are not the target"
    );

    // Once it has arrived a report is the truth again: a scroll the person
    // made themselves.
    let _ = app.update(Message::Animate(app.now + SCROLL_TRANSITION_DURATION * 2));
    let _ = app.update(Message::PageScrolled(500.0));
    assert_eq!(app.page_offset, 500.0);
}

/// A collection as the daemon composes one.
fn collection(slug: &str, name: Option<&str>, ids: &[&str]) -> Collection {
    Collection {
        slug: slug.to_owned(),
        kind: "membership".to_owned(),
        rule: None,
        role: String::new(),
        position: 0,
        owner: String::new(),
        name: name.map(str::to_owned),
        items: ids
            .iter()
            .map(|id| CollectionItem {
                item_id: (*id).to_owned(),
                name: (*id).to_owned(),
                icon: None,
                background: None,
                logo: None,
                description: None,
                played: None,
            })
            .collect(),
    }
}

/// A shell whose dashboard has two twelve-card rails, the second below the fold.
/// The rails are wide enough to scroll sideways, which is what the follow cares
/// about, and tall enough that the second one needs the page to move.
fn two_rails() -> App {
    let mut app = loaded(1);
    let ids: Vec<String> = (0..12)
        .map(|index| format!("hearthdeck:game-{index}"))
        .collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let _ = app.update(Message::CollectionsLoaded(Ok(vec![
        collection("last-played", None, &ids),
        collection("stremio:continue-watching", None, &ids),
    ])));
    app
}

/// The dashboard's rails are the daemon's collections - so a collection this
/// client has never heard of, which is what a feature's own categories and so
/// the AI's are, becomes a shelf without a change in the frontend. That is
/// the whole reason the daemon sends a name with one.
#[test]
fn the_dashboards_rails_are_the_daemons_collections() {
    let mut app = loaded(2);
    let _ = app.update(Message::CollectionsLoaded(Ok(vec![
        collection("favorites", None, &["hearthdeck:game-0"]),
        collection("smart:action", Some("Action picks"), &["hearthdeck:game-1"]),
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
        None,
        &["hearthdeck:game-0"],
    )])));

    assert_eq!(app.dashboard_shelves()[0].title, "smart:mystery");
}

/// A card launches the item it names: the launch overlay comes up and the
/// frontend gives input up, staying on the dashboard underneath.
#[test]
fn a_collection_card_launches_the_item_it_names() {
    let mut app = loaded(2);
    let _ = app.update(Message::CollectionsLoaded(Ok(vec![collection(
        "favorites",
        None,
        &["hearthdeck:game-1"],
    )])));

    send(&mut app, GamepadEvent::Confirm);

    assert_eq!(
        app.launch_state,
        LaunchState::Launching {
            title: "hearthdeck:game-1".to_owned()
        },
        "the card did not launch the item it names"
    );
    assert!(!app.input_ownership.frontend_has_control());
    assert_eq!(
        app.page,
        Page::Dashboard,
        "the launch stays on the dashboard"
    );
}

/// A film card carries the id the daemon composed the rail with, so it
/// launches even though the application library has never held it - which
/// is every Continue Watching title there is.
#[test]
fn a_media_card_the_library_never_held_launches_anyway() {
    let mut app = loaded(1);
    let _ = app.update(Message::CollectionsLoaded(Ok(vec![collection(
        "stremio:continue-watching",
        None,
        &["hearthdeck:stremio:tt0111161"],
    )])));

    send(&mut app, GamepadEvent::Confirm);

    assert_eq!(
        app.launch_state,
        LaunchState::Launching {
            title: "hearthdeck:stremio:tt0111161".to_owned()
        },
        "an item outside the local library is still launchable"
    );
    assert_eq!(app.page, Page::Dashboard);
}

/// The frontend never parses an id: whatever the rail carried goes to the
/// launch route exactly as it came, and what that names is the daemon's
/// answer to give. The client-side prefix gate that used to stand here is
/// what made cards dead for no reason the daemon would have agreed with.
#[test]
fn a_card_id_reaches_the_launch_route_exactly_as_the_rail_carried_it() {
    let mut app = loaded(1);
    let _ = app.update(Message::CollectionsLoaded(Ok(vec![collection(
        "stremio:continue-watching",
        None,
        &["stremio:tt0111161"],
    )])));

    let shelves = app.dashboard_shelves();
    assert_eq!(shelves[0].title, fl!("continue-watching"));
    assert!(
        matches!(
            shelves[0].cards[0].message,
            Some(Message::ActivateCard { ref id, .. }) if id == "stremio:tt0111161"
        ),
        "the card carries the id the rail did, unparsed"
    );

    send(&mut app, GamepadEvent::Confirm);
    assert_eq!(
        app.launch_state,
        LaunchState::Launching {
            title: "stremio:tt0111161".to_owned()
        },
        "the raw id went to the launch route; the daemon decides what it means"
    );
    assert_eq!(app.page, Page::Dashboard);
}

use crate::providers::daemon::{
    CategorizationPhase, CategorizationSnapshot, CollectionItem, ModelStatus,
    NEW_ARRIVALS_COLLECTION,
};

/// A health reply that carries one capability and nothing else, so a test can
/// say what this deployment provides without building a whole host.
fn health(categorization: bool) -> crate::providers::daemon::HealthResponse {
    crate::providers::daemon::HealthResponse {
        version: String::new(),
        lan_enabled: false,
        transport: String::new(),
        providers: Vec::new(),
        capabilities: hearthdeck_protocol::HostCapabilities {
            launch: false,
            application_sessions: false,
            retro_launch: false,
            categorization,
        },
    }
}

/// A categorization answer with nothing but the switch set - the state the
/// settings screen reads to decide what to draw.
fn snapshot(enabled: bool) -> CategorizationSnapshot {
    CategorizationSnapshot {
        enabled,
        status: hearthdeck_protocol::CategorizationStatus {
            running: false,
            phase: CategorizationPhase::Idle,
            completed: 0,
            total: 0,
            model: ModelStatus::Absent,
            last_error: None,
            last_completed_at: None,
        },
        report: None,
    }
}

/// The account the daemon reports as linked, so a test can land one without
/// a daemon behind it.
fn stremio_account() -> crate::providers::daemon::StremioConnection {
    crate::providers::daemon::StremioConnection {
        linked_at: "2026-10-07T10:00:00Z".to_owned(),
    }
}

/// A code as the daemon issues one: the four characters, the page that
/// already carries them, and the window.
fn stremio_code() -> settings::stremio_link::Pending {
    settings::stremio_link::Pending {
        code: "ABCD".to_owned(),
        link: "https://link.stremio.com/#/ABCD".to_owned(),
        seconds_left: 120,
    }
}

/// The dashboard's Settings destination opens the settings screen, and Back
/// leaves it for the dashboard: a sibling destination, not a place inside the
/// library.
#[test]
fn the_settings_destination_opens_settings_and_back_leaves_it() {
    let mut app = showing(4);

    let _ = app.update(Message::OpenSettings);
    assert_eq!(app.page, Page::Settings);

    send(&mut app, GamepadEvent::Back);
    assert_eq!(
        app.page,
        Page::Dashboard,
        "settings is as deep as the library, and no deeper"
    );
}

/// The screen draws the daemon's answer and nothing else, so what lands is
/// what is kept.
#[test]
fn the_settings_screen_keeps_the_daemons_answer() {
    let mut app = showing(4);
    assert!(
        app.categorization.is_none(),
        "nothing has been read yet, so there is nothing to draw"
    );

    let _ = app.update(Message::CategorizationLoaded(Ok(snapshot(false))));
    assert!(!app.categorization.as_ref().is_some_and(|snap| snap.enabled));

    let _ = app.update(Message::CategorizationLoaded(Ok(snapshot(true))));
    assert!(app.categorization.as_ref().is_some_and(|snap| snap.enabled));
}

/// A read that fails does not blank the screen: the last answer stands, so a
/// transient failure does not turn the switch into a question mark.
#[test]
fn a_failed_categorization_read_keeps_the_last_answer() {
    let mut app = showing(4);
    let _ = app.update(Message::CategorizationLoaded(Ok(snapshot(true))));

    let _ = app.update(Message::CategorizationLoaded(Err(
        "daemon is down".to_owned()
    )));

    assert!(
        app.categorization.as_ref().is_some_and(|snap| snap.enabled),
        "the last answer the daemon gave stands"
    );
}

/// The health reply is what says whether this deployment provides the
/// feature at all, and the screen draws nothing on offer until it has one.
#[test]
fn the_health_reply_says_whether_the_feature_exists_here() {
    let mut app = showing(4);
    assert_eq!(
        app.categorization_capable, None,
        "nothing has asked the deployment yet"
    );

    let _ = app.update(Message::HealthLoaded(Ok(health(false))));
    assert_eq!(
        app.categorization_capable,
        Some(false),
        "this installation cannot run it"
    );

    let _ = app.update(Message::HealthLoaded(Ok(health(true))));
    assert_eq!(app.categorization_capable, Some(true), "and this one can");
}

/// Restoring the defaults rebuilds the derived tabs, drops what an older scan
/// left behind, and keeps the user's own folders.
#[test]
fn restoring_the_defaults_rebuilds_the_tabs_and_keeps_the_users_folders() {
    let mut app = showing(4);
    app.cur_section = Section::Applications;
    let _ = app.update(Message::CatalogLoaded(Ok(vec![entry(
        "writer",
        &["Office"],
    )])));
    add_user_group(&mut app, Section::Applications, "Mine");
    app.config.sections.applications.push(AppGroup {
        name: "Speedruns".to_owned(),
        icon: "folder-symbolic".to_owned(),
        filter: FilterType::AppIds(vec!["app.one".to_owned()]),
        source: GroupSource::Categorization,
    });
    app.config.sections.applications.remove(0);
    app.cur_group = Some(1);

    let _ = app.update(Message::RestoreDefaultCategories);

    assert_eq!(
        tabs(&app, Section::Applications),
        ["Desktop", "Mine"],
        "the derived tab is back, the scan's is gone and the user's stays"
    );
    assert_eq!(app.cur_group, None, "the library lands on its all tab");
}

/// Settings saved outside the shell reach the pages when the watch fires:
/// the tabs come back rebuilt, and the library lands on its all tab the way
/// a restore does.
#[test]
fn a_live_settings_change_rebuilds_the_tabs() {
    let mut app = showing(4);
    app.cur_section = Section::Applications;
    let _ = app.update(Message::CatalogLoaded(Ok(vec![entry(
        "writer",
        &["Office"],
    )])));
    add_user_group(&mut app, Section::Applications, "Mine");
    app.cur_group = Some(1);

    let mut saved = app.config.clone();
    saved.sections.applications = vec![AppGroup {
        name: "FromDisk".to_owned(),
        icon: "folder-symbolic".to_owned(),
        filter: FilterType::AppIds(Vec::new()),
        source: GroupSource::Local,
    }];
    let _ = app.update(Message::AppConfigLoaded(saved));

    assert_eq!(
        tabs(&app, Section::Applications),
        ["FromDisk"],
        "the tabs are what the save said"
    );
    assert_eq!(app.cur_group, None, "the library lands on its all tab");
}

/// A save that does not touch these tabs must not move the user: the cursor
/// stays where it was. The toggle itself is live.
#[test]
fn a_live_change_to_the_input_toggles_leaves_the_cursor_alone() {
    let mut app = showing(4);
    app.cur_section = Section::Applications;
    let _ = app.update(Message::CatalogLoaded(Ok(vec![entry(
        "writer",
        &["Office"],
    )])));
    app.cur_group = Some(1);

    let mut saved = app.config.clone();
    saved.desktop_input_apps.push("app.one".to_owned());
    let _ = app.update(Message::AppConfigLoaded(saved));

    assert_eq!(
        app.cur_group,
        Some(1),
        "the cursor stays on the tab it was on"
    );
    assert!(
        app.config.desktop_input_enabled("app.one"),
        "the toggle is live"
    );
}

/// A form overridden outside the shell moves its tiles between the lists at
/// once, without waiting for the next catalog load.
#[test]
fn a_live_form_override_moves_the_tiles() {
    let mut app = showing(4);
    app.cur_section = Section::Applications;
    let _ = app.update(Message::CatalogLoaded(Ok(vec![entry(
        "writer",
        &["Office"],
    )])));
    assert_eq!(
        app.visible.len(),
        0,
        "a desktop form is not what the couch shows"
    );

    let mut saved = app.config.clone();
    saved
        .form_overrides
        .insert("writer".to_owned(), crate::form_factor::FormFactor::Couch);
    let _ = app.update(Message::AppConfigLoaded(saved));

    assert_eq!(app.visible.len(), 1, "the default view shows it now");
}

/// Turning smart categorization off must also undo a scan's leftovers: it is
/// the way out for someone whose tabs it broke.
#[test]
fn turning_smart_categorization_off_restores_the_default_tabs() {
    let mut app = showing(4);
    app.cur_section = Section::Applications;
    let _ = app.update(Message::CatalogLoaded(Ok(vec![entry(
        "writer",
        &["Office"],
    )])));
    app.config.sections.applications = vec![AppGroup {
        name: "Speedruns".to_owned(),
        icon: "folder-symbolic".to_owned(),
        filter: FilterType::AppIds(vec!["app.one".to_owned()]),
        source: GroupSource::Categorization,
    }];

    let _ = app.update(Message::SetCategorization(false));

    assert_eq!(tabs(&app, Section::Applications), ["Desktop"]);
}

/// A refused categorization action lands in the section's own status text,
/// and asking for the next one starts it clean.
#[test]
fn a_refused_action_lands_in_the_sections_own_status_text() {
    let mut app = showing(4);

    let _ = app.update(Message::CategorizationDone(Err(
        "the scan was refused".to_owned()
    )));
    assert_eq!(
        app.categorization_notice.as_deref(),
        Some("the scan was refused")
    );

    let _ = app.update(Message::DetectCouchFriendly);
    assert_eq!(
        app.categorization_notice, None,
        "a new action starts from a clean slate"
    );
}

/// The Stremio section draws whatever the daemon last said about the
/// account: a read lands it, and a successful "none" clears it - an account
/// that is not there is a fact, not a missing one.
#[test]
fn a_stremio_connection_read_lands_the_account_the_section_draws() {
    let mut app = showing(4);

    let _ = app.update(Message::StremioConnectionLoaded(Ok(
        Some(stremio_account()),
    )));

    assert_eq!(
        app.stremio
            .connection
            .as_ref()
            .map(|found| found.linked_at.as_str()),
        Some("2026-10-07T10:00:00Z"),
        "the account the daemon holds is the one kept"
    );
    assert!(
        !app.stremio.reading,
        "the answer ends the checking state the request started"
    );

    let _ = app.update(Message::StremioConnectionLoaded(Ok(None)));

    assert!(
        app.stremio.connection.is_none(),
        "nothing is linked, and the section says so rather than keeping a ghost"
    );
}

/// Unlinking takes the account off the section - and while the request is
/// unanswered nothing presses again, so the account cannot be forgotten
/// twice.
#[test]
fn unlinking_takes_the_account_off_the_section_once_it_is_answered() {
    let mut app = showing(4);
    let _ = app.update(Message::StremioConnectionLoaded(Ok(
        Some(stremio_account()),
    )));

    let _ = app.update(Message::UnlinkStremio);

    assert!(app.stremio.busy, "the request is in flight");
    assert!(
        !settings::stremio_link::pressable(settings::stremio_link::Action::Unlink, &app.stremio),
        "a request in flight owns the section"
    );

    let _ = app.update(Message::StremioUnlinked(Ok(())));

    assert!(app.stremio.connection.is_none(), "the account is gone");
    assert!(!app.stremio.busy, "and the buttons come back");
}

/// A refused unlink is never news that the account is gone: it stays, with
/// the daemon's own reason beside it for the section to read.
#[test]
fn a_refused_unlink_keeps_the_account_and_says_why() {
    let mut app = showing(4);
    let _ = app.update(Message::StremioConnectionLoaded(Ok(
        Some(stremio_account()),
    )));
    let _ = app.update(Message::UnlinkStremio);

    let _ = app.update(Message::StremioUnlinked(Err("daemon is down".to_owned())));

    assert!(
        app.stremio.connection.is_some(),
        "the account is still linked as far as anyone knows"
    );
    assert_eq!(app.stremio.error.as_deref(), Some("daemon is down"));
    assert!(
        !app.stremio.busy,
        "the buttons come back even when the request failed"
    );
}

/// Asking for a code locks the section while the request is in flight, and
/// the code the daemon answers with is what the section then shows - each
/// poll updating the countdown through the same flow.
#[test]
fn a_started_link_locks_the_section_and_then_shows_its_code() {
    let mut app = showing(4);
    assert!(
        settings::stremio_link::pressable(settings::stremio_link::Action::Link, &app.stremio),
        "idle, linking is the way in"
    );

    let _ = app.update(Message::StartStremioLink);

    assert!(app.stremio.busy, "the code is being asked for");

    let _ = app.update(Message::StremioLinkStarted(Ok(stremio_code())));

    assert!(!app.stremio.busy, "the answer releases the buttons");
    assert_eq!(
        app.stremio
            .pending
            .as_ref()
            .map(|pending| pending.code.as_str()),
        Some("ABCD"),
        "the code the daemon issued is the one on screen"
    );

    let _ = app.update(Message::StremioLinkPolled(Ok(
        crate::providers::daemon::StremioLink::Waiting {
            code: "ABCD".to_owned(),
            link: "https://link.stremio.com/#/ABCD".to_owned(),
            seconds_left: 98,
        },
    )));

    assert_eq!(
        app.stremio
            .pending
            .as_ref()
            .map(|pending| pending.seconds_left),
        Some(98),
        "the countdown follows the daemon's answer, not a local clock"
    );
}

/// The bar's buttons work from whichever sibling is showing, and each one
/// goes to its own screen - including back to the dashboard.
#[test]
fn the_top_bar_moves_between_siblings_whichever_is_showing() {
    // `loaded` rather than `showing`: the point is the page the app opens on,
    // which is the dashboard.
    let mut app = loaded(4);
    assert_eq!(app.page, Page::Dashboard);

    let _ = app.update(navigate(top_bar::Destination::Settings));
    assert_eq!(app.page, Page::Settings);

    let _ = app.update(navigate(top_bar::Destination::Search));
    assert_eq!(
        app.page,
        Page::Library,
        "search is a way into the library, not a screen of its own"
    );

    let _ = app.update(navigate(top_bar::Destination::Dashboard));
    assert_eq!(app.page, Page::Dashboard, "and back");
}

/// The pad reaches the top bar from the top rail of the dashboard, walks it
/// sideways, and a button confirms to its own screen.
#[test]
fn the_pad_walks_the_top_bar_from_the_dashboard_and_back() {
    let mut app = loaded(4);
    assert_eq!(app.page, Page::Dashboard);

    send(&mut app, GamepadEvent::MoveUp);
    assert_eq!(
        app.bar_cursor,
        Some(top_bar::Destination::Dashboard.index()),
        "Up off the top rail lands on the bar, on the page's own button"
    );

    send(&mut app, GamepadEvent::MoveRight);
    assert_eq!(app.bar_cursor, Some(top_bar::Destination::Library.index()));
    send(&mut app, GamepadEvent::MoveRight);
    send(&mut app, GamepadEvent::MoveRight);
    assert_eq!(app.bar_cursor, Some(top_bar::Destination::Settings.index()));
    send(&mut app, GamepadEvent::MoveRight);
    assert_eq!(
        app.bar_cursor,
        Some(top_bar::Destination::Settings.index()),
        "the end of the bar is an end"
    );

    send(&mut app, GamepadEvent::Confirm);
    assert_eq!(app.page, Page::Settings, "the button goes to its screen");
}

/// Down from the bar leaves it for the rails below, and Back off the bar
/// goes back to the card: the bar is chrome, not a surface over anything.
#[test]
fn the_bar_gives_the_cursor_back_to_the_rails() {
    let mut app = loaded(4);

    send(&mut app, GamepadEvent::MoveUp);
    send(&mut app, GamepadEvent::MoveRight);
    assert_eq!(app.bar_cursor, Some(top_bar::Destination::Library.index()));

    send(&mut app, GamepadEvent::MoveDown);
    assert_eq!(app.bar_cursor, None, "Down leaves the bar");
    assert_eq!(app.dashboard_cursor.rail, 0, "for the rails below it");

    send(&mut app, GamepadEvent::MoveUp);
    send(&mut app, GamepadEvent::Back);
    assert_eq!(app.bar_cursor, None, "Back steps off the bar too");
    assert_eq!(app.page, Page::Dashboard, "without leaving the page");
}

/// The keyboard walks the bar the way the pad does - entering at either end
/// and leaving past an end, back to the content.
#[test]
fn tab_walks_the_bar_on_the_dashboard() {
    let mut app = loaded(4);

    let _ = app.update(Message::ChromeNext);
    assert_eq!(
        app.bar_cursor,
        Some(0),
        "Tab enters the bar at its first button"
    );
    let _ = app.update(Message::ChromePrevious);
    assert_eq!(app.bar_cursor, None, "and leaves it the way it came");
    let _ = app.update(Message::ChromePrevious);
    assert_eq!(
        app.bar_cursor,
        Some(top_bar::Destination::Settings.index()),
        "backwards enters at the far end"
    );
}

/// The settings screen's rows are not walkable yet, so the pad's step lands
/// on the bar it wears, and Confirm there acts on the bar - and on nothing
/// else. The screen used to answer Confirm by launching whichever tile the
/// library cursor happened to be on, invisibly.
#[test]
fn the_pad_on_settings_walks_the_bar_and_acts_on_nothing_else() {
    let mut app = showing(4);
    let _ = app.update(Message::OpenSettings);

    send(&mut app, GamepadEvent::MoveDown);
    assert_eq!(
        app.bar_cursor,
        Some(top_bar::Destination::Settings.index()),
        "any step lands on the bar, on the page's own button"
    );

    send(&mut app, GamepadEvent::Confirm);
    assert_eq!(app.page, Page::Settings, "the page's own button is a no-op");
    assert!(
        !app.launch_state.is_visible(),
        "nothing was launched behind the screen"
    );
}

/// The context menu belongs to the library grid: on a sibling page there is
/// no tile behind it to act on, so the button does nothing rather than
/// opening a menu over the bar and taking the pad from there.
#[test]
fn the_context_button_opens_no_menu_off_the_library() {
    let mut app = loaded(4);

    send(&mut app, GamepadEvent::ContextMenu);

    assert!(
        !app.menu_is_up(),
        "there is no grid here for it to belong to"
    );
}

/// The context button answers "what can I do with this": for a console game
/// that answer lives on the game's own screen, so X opens it rather than
/// stacking a menu of the same actions. Everything else keeps its menu.
#[test]
fn the_context_button_opens_details_only_for_console_games() {
    let mut app = showing_a_console_game(Vec::new());

    send(&mut app, GamepadEvent::ContextMenu);

    assert_eq!(app.page, Page::Details, "X opens the console game's screen");
    assert!(app.details.is_some());

    // A local game keeps its menu instead.
    let mut local = showing(2);
    send(&mut local, GamepadEvent::ContextMenu);
    assert!(local.menu.is_open(), "a local game keeps its menu");
    assert!(local.details.is_none());
}

/// Seeing the game's screen is a read, not a launch: the button opens the
/// page and starts nothing, so no layer claiming a game is starting comes
/// up with it.
#[test]
fn opening_the_details_screen_starts_no_launch() {
    let mut app = showing_a_console_game(Vec::new());

    send(&mut app, GamepadEvent::ContextMenu);

    assert_eq!(app.page, Page::Details);
    assert!(
        !app.launch_state.is_visible(),
        "the context button opens the screen; it launches nothing"
    );
    assert!(
        app.launch_state.overlay().is_none(),
        "and no launch layer covers the page while nothing is in flight"
    );
}

/// Back from the details screen leaves it for the grid it was opened from,
/// with the cursor back on the tile the user came in through.
#[test]
fn details_back_returns_to_the_tile_that_opened_it() {
    let mut app = showing_a_console_game(Vec::new());
    send(&mut app, GamepadEvent::ContextMenu);
    assert_eq!(app.page, Page::Details);

    send(&mut app, GamepadEvent::Back);

    assert_eq!(app.page, Page::Library);
    assert_eq!(app.cursors.tile, 0, "the cursor is back on the tile");
}

/// Back closes the disc picker before it leaves the screen: the picker is a
/// surface stacked over the page, so it unwinds first.
#[test]
fn back_closes_the_disc_picker_before_leaving_the_details_screen() {
    let mut app = showing_a_console_game(vec![
        RetroRomVersion {
            id: 42,
            title: "Disc 1".to_owned(),
            is_main_sibling: true,
        },
        RetroRomVersion {
            id: 43,
            title: "Disc 2".to_owned(),
            is_main_sibling: false,
        },
    ]);
    send(&mut app, GamepadEvent::ContextMenu);
    assert_eq!(app.page, Page::Details);

    // Open the picker.
    let _ = app.update(Message::DetailsToggleDiscPicker);
    assert!(
        app.details
            .as_ref()
            .expect("the screen is open")
            .disc_picker,
        "the picker is open"
    );

    send(&mut app, GamepadEvent::Back);

    assert_eq!(
        app.page,
        Page::Details,
        "Back closes the picker, not the screen"
    );
    assert!(
        !app.details
            .as_ref()
            .expect("the screen is open")
            .disc_picker,
        "and the picker is what closed"
    );

    send(&mut app, GamepadEvent::Back);
    assert_eq!(app.page, Page::Library, "a second Back leaves the screen");
}

/// The shoulder buttons are chords of the library behind the details screen:
/// on the details screen they do nothing, because switching section there
/// would change what the screen describes.
#[test]
fn shoulder_buttons_do_not_change_section_from_the_details_screen() {
    let mut app = showing_a_console_game(Vec::new());
    send(&mut app, GamepadEvent::ContextMenu);
    assert_eq!(app.page, Page::Details);
    let section = app.cur_section;

    send(&mut app, GamepadEvent::NextGroup);
    send(&mut app, GamepadEvent::PrevGroup);

    assert_eq!(app.cur_section, section, "the section did not change");
    assert_eq!(app.page, Page::Details, "and the screen stayed put");
}

/// The library's ring is the library's: it does not survive a trip to a
/// sibling page, where Back would otherwise step out of a ring that nothing
/// on screen shows.
#[test]
fn leaving_the_library_takes_the_ring_with_it() {
    let mut app = showing(4);
    let _ = app.update(Message::ChromeNext);
    assert!(app.cursors.ring.is_some());

    let _ = app.update(Message::OpenDashboard);

    assert!(app.cursors.ring.is_none());
}

/// The pad's Search button reaches the search box from wherever it is, the
/// same way the bar's Search button does - off the library it shows the
/// library first.
#[test]
fn the_search_button_reaches_the_search_from_any_page() {
    let mut app = loaded(4);
    assert_eq!(app.page, Page::Dashboard);

    send(&mut app, GamepadEvent::Search);

    assert_eq!(app.page, Page::Library);
}

/// The bar's status icons read the same snapshot the poll landed in the
/// shell, and until the first read they say the service is unavailable
/// rather than drawing nothing.
#[test]
fn the_bar_reads_the_status_the_poll_landed() {
    use crate::system_status::WifiStatus;

    let mut app = shell();
    assert!(app.system_status.wifi.is_none(), "nothing read yet");

    let _ = app.update(Message::SystemStatusLoaded(SystemStatus {
        wifi: Some(WifiStatus {
            enabled: true,
            connected: true,
            strength: 80,
        }),
        bluetooth: None,
    }));

    let bar = app.top_bar(top_bar::Destination::Dashboard);
    assert!(
        bar.status.wifi.as_ref().is_some_and(|wifi| wifi.connected),
        "the bar reads the shell's snapshot, not its own copy"
    );
    assert!(bar.status.bluetooth.is_none());
}

/// Switching pages flips at once - routing and input read `self.page`
/// immediately - and records where the fade came from, so the transition can
/// still draw the page being left.
#[test]
fn switching_pages_starts_a_fade_through() {
    let mut app = shell();
    assert_eq!(app.page, Page::Dashboard);

    let _ = app.update(Message::OpenSettings);

    assert_eq!(app.page, Page::Settings);
    let fade = app.page_fade.as_ref().expect("the switch should be fading");
    assert_eq!(
        *fade.origin(),
        Page::Dashboard,
        "fades from the page it left"
    );
}

#[test]
fn reselecting_the_current_page_snaps_instead_of_fading() {
    let mut app = shell();
    let _ = app.update(Message::OpenSettings);
    assert!(app.page_fade.is_some());

    // Asking for the page already on screen again: fading to where we already
    // are would be motion for its own sake.
    let _ = app.update(Message::OpenSettings);

    assert_eq!(app.page, Page::Settings);
    assert!(app.page_fade.is_none());
}

/// Ages the in-flight page fade past its duration, so the next message
/// retires it rather than carrying it along.
fn age_page_fade(app: &mut App) {
    let origin = *app
        .page_fade
        .as_ref()
        .expect("page fade in flight")
        .origin();
    app.page_fade = Some(Fade::new(
        origin,
        Instant::now() - std::time::Duration::from_secs(1),
        PAGE_TRANSITION_DURATION,
    ));
}

/// A fade is bounded: it cannot outlive `PAGE_TRANSITION_DURATION`, even when
/// the message that arrives next is not a frame tick.
#[test]
fn any_next_message_retires_a_fade_that_ran_its_course() {
    let mut app = shell();
    let _ = app.update(Message::OpenLibrary);

    // Inside its duration, an unrelated message leaves it running.
    let _ = app.update(Message::DismissLaunch);
    assert!(app.page_fade.is_some());

    age_page_fade(&mut app);
    let _ = app.update(Message::DismissLaunch);

    assert!(
        app.page_fade.is_none(),
        "expiry is not the tick's job alone"
    );
    assert_eq!(app.page, Page::Library, "the page it landed on stays");
}

/// The frame tick is what moves a fade along: `view` reads its progress from
/// the clock the tick carries.
#[test]
fn a_frame_tick_moves_a_fade_along() {
    let mut app = shell();
    let _ = app.update(Message::OpenLibrary);
    let at_landing = app
        .page_fade
        .as_ref()
        .expect("page fade in flight")
        .progress(app.now);

    let _ = app.update(Message::Animate(app.now + PAGE_TRANSITION_DURATION / 2));

    let midpoint = app
        .page_fade
        .as_ref()
        .expect("page fade still in flight")
        .progress(app.now);
    assert!(
        midpoint > at_landing,
        "the tick advanced the fade: {midpoint} after {at_landing}"
    );
}

/// Back is a page change like any other, so it fades home instead of cutting.
#[test]
fn back_from_another_page_fades_to_the_dashboard() {
    let mut app = showing(0);
    assert_eq!(app.page, Page::Library);

    let _ = app.back();

    assert_eq!(app.page, Page::Dashboard);
    let fade = app.page_fade.as_ref().expect("Back should fade home");
    assert_eq!(*fade.origin(), Page::Library);
}

/// The RomM sign-in dialog is modal, so Back closes it and stays on the page.
#[test]
fn back_closes_the_romm_sign_in_dialog_before_leaving_the_page() {
    let mut app = showing(0);
    app.romm.sign_in = Some(crate::screens::settings::romm::SignIn::default());

    let _ = app.back();

    assert!(app.romm.sign_in.is_none());
    assert_eq!(app.page, Page::Library);
}

/// A section change slides the content column vertically: the section being
/// drawn stays the current one until the cover is opaque, so the outgoing grid
/// is still what the user sees while it travels off.
#[test]
fn switching_sections_starts_a_vertical_slide() {
    let mut app = showing(4);

    let _ = app.update(Message::SelectSection(Section::Applications));

    let slide = app
        .section_slide
        .as_ref()
        .expect("the section change should slide");
    assert_eq!(*slide.target(), Section::Applications);
    assert_eq!(slide.direction(), 1.0, "Applications sits below Pc Games");
    assert_eq!(
        app.cur_section,
        Section::PcGames,
        "the swap waits for cover"
    );
    assert!(app.is_animating(), "and the frames are subscribed for it");
}

#[test]
fn section_direction_follows_sidebar_order() {
    let mut app = showing(4);
    let _ = app.update(Message::SelectSection(Section::Applications));
    settle(&mut app);

    let _ = app.update(Message::SelectSection(Section::PcGames));

    let slide = app
        .section_slide
        .as_ref()
        .expect("the section change should slide");
    assert_eq!(slide.direction(), -1.0, "Pc Games sits above Applications");
}

#[test]
fn the_section_slide_swaps_under_cover_then_clears() {
    let mut app = showing(4);
    let _ = app.update(Message::SelectSection(Section::ConsoleGames));

    settle(&mut app);

    assert_eq!(app.cur_section, Section::ConsoleGames);
    assert!(app.section_slide.is_none(), "and the slide is dropped");
}

#[test]
fn reselecting_the_current_section_does_not_slide() {
    let mut app = showing(4);

    let _ = app.update(Message::SelectSection(Section::PcGames));

    assert!(
        app.section_slide.is_none(),
        "nothing changed, so nothing moves"
    );
    assert_eq!(app.cur_section, Section::PcGames);
}

/// A section picked on the way in from the dashboard applies at once: the slide
/// travels the library's content column, which is not on screen yet.
#[test]
fn a_section_picked_as_the_page_changes_applies_at_once() {
    let mut app = loaded(4);
    assert_eq!(app.page, Page::Dashboard);

    let _ = app.update(Message::SelectSection(Section::Applications));

    assert!(app.section_slide.is_none());
    assert_eq!(app.cur_section, Section::Applications);
    assert_eq!(app.page, Page::Library);
}

/// A tab change slides the grid sideways, and the tab it is travelling to is
/// applied under the cover exactly as a section is.
#[test]
fn switching_tabs_starts_a_directional_slide() {
    let mut app = showing_two_tabs();

    send(&mut app, GamepadEvent::NextTab);

    let slide = app.tab_slide.as_ref().expect("the tab change should slide");
    assert_eq!(*slide.target(), Some(0));
    assert_eq!(slide.direction(), 1.0, "the store's tab sits to the right");
    assert_eq!(app.cur_group, None, "the swap waits for cover");

    settle(&mut app);

    assert_eq!(app.cur_group, Some(0));
    assert!(app.tab_slide.is_none(), "and the slide is dropped");
}

#[test]
fn tab_direction_follows_strip_order() {
    let mut app = showing_two_tabs();
    let _ = app.update(Message::SelectGroup(Some(0)));
    settle(&mut app);

    let _ = app.update(Message::SelectGroup(None));

    let slide = app.tab_slide.as_ref().expect("the tab change should slide");
    assert_eq!(slide.direction(), -1.0, "the all tab sits to the left");
}

#[test]
fn reselecting_the_current_tab_does_not_slide() {
    let mut app = showing_two_tabs();

    let _ = app.update(Message::SelectGroup(None));

    assert!(app.tab_slide.is_none(), "nothing changed, so nothing moves");
}

/// A focus step that drags a list eases it there: the lists that follow the
/// cursor are what the frame clock is subscribed for.
#[test]
fn a_focus_step_eases_the_grid_to_its_row() {
    let mut app = showing(8 * 40);
    let metrics = grid::metrics(app.window_width);
    let visible_rows = ((app.grid_viewport_height / metrics.row_height).floor() as usize).max(1);

    for _ in 0..visible_rows.saturating_sub(1) {
        send(&mut app, GamepadEvent::MoveDown);
    }
    assert!(
        !app.scrolls.is_animating(),
        "a step inside the viewport drags nothing"
    );

    send(&mut app, GamepadEvent::MoveDown);

    assert_eq!(
        app.scrolls.targets(),
        vec![ScrollTarget::Grid],
        "the grid should be easing"
    );
    assert!(app.is_animating(), "and the frames are subscribed for it");

    settle(&mut app);
    assert!(!app.scrolls.is_animating(), "arriving drops the animation");
}

/// The daemon is often not answering yet when the app starts; a failed read used
/// to leave the dashboard on its fallback shelf for the whole session.
#[test]
fn a_failed_dashboard_read_is_retried_until_one_succeeds() {
    let mut app = shell();

    let _ = app.update(Message::CollectionsLoaded(Err("daemon down".to_owned())));
    assert!(app.retries.collections, "a retry is on its way");

    let _ = app.update(Message::CollectionsLoaded(Ok(Vec::new())));
    assert!(
        !app.retries.collections,
        "a read that landed ends the chain"
    );
}

#[test]
fn restoring_the_dashboard_shows_it_with_the_cursor_on_the_first_card() {
    let mut app = showing(4);
    app.page = Page::Settings;
    app.dashboard_cursor = dashboard::focus::Cursor {
        rail: 2,
        card: 3,
        remembered: 3,
    };

    let _ = app.update(Message::RestoreDashboard);

    assert_eq!(app.page, Page::Dashboard);
    assert!(app.dashboard_cursor == dashboard::focus::Cursor::default());
}

/// Moving an application to the other form is a choice that sticks, and the
/// Desktop tab goes when nothing wants it.
#[test]
fn moving_an_application_to_the_couch_takes_it_off_the_desktop_tab() {
    let mut app = showing(1);
    let _ = app.update(Message::CatalogLoaded(Ok(vec![entry(
        "writer",
        &["Office"],
    )])));
    app.cur_section = Section::Applications;
    app.cur_group = Some(0);
    app.refilter();
    assert_eq!(app.visible.len(), 1, "the Desktop tab shows it");

    let _ = app.update(Message::MoveToForm(
        0,
        crate::form_factor::FormFactor::Couch,
    ));

    assert!(tabs(&app, Section::Applications).is_empty());
    assert_eq!(app.cur_group, None, "the tab it was on is gone");
    assert_eq!(app.visible.len(), 1, "the default view shows it now");
    assert_eq!(
        app.config.form_overrides.get("writer"),
        Some(&crate::form_factor::FormFactor::Couch)
    );
}
