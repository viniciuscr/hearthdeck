//! Takes a launched app's window fullscreen, through the compositor.
//!
//! On a desktop session an app opens at whatever size it asks for, which on a TV
//! is usually wrong twice over: too small, and wrapped in decorations with a
//! title bar and a close button. Fullscreen fixes both at once, because a
//! fullscreen window is undecorated by definition.
//!
//! COSMIC splits this over two protocols: `ext_foreign_toplevel_list_v1` lists
//! the open windows and reports their app id and title, and COSMIC's own
//! `zcosmic_toplevel_manager_v1` is what can act on one. Both are requests the
//! compositor may ignore. A compositor that implements neither is not an error
//! path to handle: the kiosk session runs Gamescope, which implements neither,
//! and there the watcher finds no global and exits without touching anything.
//!
//! The watcher is one thread for the life of the app, driven entirely by
//! compositor events. A launch registers what its window should look like, and
//! the window's own arrival is what wakes the thread up: there is nothing to
//! poll and no timer to miss.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use cosmic_protocols::toplevel_info::v1::client::{
    zcosmic_toplevel_handle_v1::{Event as CosmicHandleEvent, ZcosmicToplevelHandleV1},
    zcosmic_toplevel_info_v1::{Event as CosmicInfoEvent, ZcosmicToplevelInfoV1},
};
use cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1::{
    Event as ManagerEvent, ZcosmicToplevelManagerV1,
};
use log::{debug, info};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::{Event as RegistryEvent, WlRegistry};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{Event as WindowEvent, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, Event as ListEvent, ExtForeignToplevelListV1},
};

/// How long a launch's window is expected to appear. Generous: a launch goes
/// through the daemon, the bridge and a transient systemd unit before the app
/// even starts, and a first run of a large app takes a while.
const HINT_LIFETIME: Duration = Duration::from_secs(60);

/// Shortest app name worth matching against a window title. Below this the name
/// is as likely to be a word inside an unrelated window's title as the app's own
/// name.
const MIN_TITLE_CANDIDATE: usize = 4;

/// The window names a launch is expected to produce.
#[derive(Clone, Debug)]
pub struct WindowHint {
    /// Lowercased strings the window's app id or title may equal or contain.
    candidates: Vec<String>,
    /// What to call this in a log line.
    label: String,
}

impl WindowHint {
    /// Names a window from the entry that launched it.
    ///
    /// A Wayland window's `app_id` is whatever the app itself sets, which for a
    /// `.desktop`-launched app is usually the file's own name; an entry that
    /// declares `StartupWMClass` reports that instead. The entry's *name* is the
    /// last resort, and the only one that works for a web app handed to a
    /// browser (`--app=<url>`): that window belongs to the browser, and only its
    /// title says which page it is.
    pub fn for_entry(id: &str, name: &str, wm_class: Option<&str>) -> Self {
        let mut candidates: Vec<String> = Vec::new();
        let mut push = |candidate: String| {
            if !candidate.is_empty() && !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        };

        push(id.trim_end_matches(".desktop").trim().to_lowercase());
        if let Some(wm_class) = wm_class.map(str::trim).filter(|value| !value.is_empty()) {
            push(wm_class.to_lowercase());
        }
        let name = name.trim().to_lowercase();
        if name.chars().count() >= MIN_TITLE_CANDIDATE {
            push(name);
        }

        Self {
            candidates,
            label: id.to_owned(),
        }
    }

    /// Whether this window belongs to the app the hint describes.
    fn matches(&self, app_id: &str, title: &str) -> bool {
        let app_id = app_id.to_lowercase();
        let title = title.to_lowercase();
        self.candidates.iter().any(|candidate| {
            (!app_id.is_empty() && (app_id == *candidate || app_id.contains(candidate.as_str())))
                || (!title.is_empty()
                    && (title == *candidate || title.contains(candidate.as_str())))
        })
    }
}

/// Launches whose windows have not been seen yet, shared with the watcher
/// thread. A plain list behind a mutex rather than a channel: the thread is
/// woken by the compositor, not by the sender, so a channel would only add a
/// second thing to wait on.
static HINTS: OnceLock<Mutex<Vec<(WindowHint, Instant)>>> = OnceLock::new();

/// Whether the watcher thread is already running, so a second launch does not
/// start a second listener.
static WATCHING: AtomicBool = AtomicBool::new(false);

/// Asks the compositor to fullscreen the window `hint` describes, when it
/// appears.
///
/// Best effort by design: a compositor without these protocols, or a window that
/// never matches, means nothing happens — never a failure the caller has to
/// handle.
pub fn fullscreen_when_it_appears(hint: WindowHint) {
    let pending = HINTS.get_or_init(|| Mutex::new(Vec::new()));
    {
        let mut pending = pending.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        pending.retain(|(_, registered)| now.duration_since(*registered) < HINT_LIFETIME);
        pending.push((hint, now));
    }

    if WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("hearthdeck-toplevel".to_owned())
        .spawn(|| {
            if let Err(error) = watch() {
                debug!("stopped watching windows: {error}");
            }
            WATCHING.store(false, Ordering::SeqCst);
        });
    if spawned.is_err() {
        WATCHING.store(false, Ordering::SeqCst);
    }
}

/// Listens for windows until the connection ends, fullscreening the ones a
/// launch is waiting for.
fn watch() -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<Watcher>(&connection)?;
    let qh = queue.handle();

    // The list is what reports a window's app id and title.
    let Ok(_windows) = globals.bind::<ExtForeignToplevelListV1, Watcher, ()>(&qh, 1..=1, ()) else {
        debug!("compositor does not list windows; launched apps keep the size they open at");
        return Ok(());
    };
    // COSMIC's own pair is what can act on one. `get_cosmic_toplevel`, the way to
    // reach a listed window, arrived in version 2 of the info protocol.
    let Ok(window_info) = globals.bind::<ZcosmicToplevelInfoV1, Watcher, ()>(&qh, 2..=3, ()) else {
        debug!("compositor has no COSMIC window info; launched apps are left as they open");
        return Ok(());
    };
    let Ok(manager) = globals.bind::<ZcosmicToplevelManagerV1, Watcher, ()>(&qh, 1..=4, ()) else {
        debug!("compositor cannot manage windows; launched apps are left as they open");
        return Ok(());
    };

    let mut watcher = Watcher {
        window_info: Some(window_info),
        manager: Some(manager),
        asked: Vec::new(),
    };
    loop {
        if let Err(error) = queue.blocking_dispatch(&mut watcher) {
            return Err(error.into());
        }
    }
}

struct Watcher {
    /// COSMIC's extension of the window list: what a window can be addressed
    /// through.
    window_info: Option<ZcosmicToplevelInfoV1>,
    /// What can be asked of a window.
    manager: Option<ZcosmicToplevelManagerV1>,
    /// The window handles fullscreen was asked through. Kept for the life of the
    /// watcher because destroying the handle destroys the object the request went
    /// to, and a compositor is entitled to drop what that object asked for with
    /// it.
    asked: Vec<ZcosmicToplevelHandleV1>,
}

impl Watcher {
    /// Fullscreens the window if a launch is waiting for it, and retires the
    /// hint so the window is not asked for twice.
    fn consider(
        &mut self,
        window: &ExtForeignToplevelHandleV1,
        qh: &QueueHandle<Self>,
        app_id: &str,
        title: &str,
    ) {
        let (Some(window_info), Some(manager)) = (self.window_info.clone(), self.manager.clone())
        else {
            return;
        };
        let Some(pending) = HINTS.get() else {
            return;
        };
        let mut pending = pending.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        pending.retain(|(_, registered)| now.duration_since(*registered) < HINT_LIFETIME);

        let Some(index) = pending
            .iter()
            .position(|(hint, _)| hint.matches(app_id, title))
        else {
            // Not every window belongs to a launch: a dialog, or an app the user
            // started elsewhere. Worth a line, since it is the only clue when a
            // window a launch did produce fails to match.
            debug!("window {app_id:?} {title:?} is not one a launch is waiting for");
            return;
        };
        let (hint, _) = pending.remove(index);

        let cosmic = window_info.get_cosmic_toplevel(window, qh, ());
        // `None` for the output: the request lets the compositor pick, which is
        // the only sensible choice for a TV with one display.
        manager.set_fullscreen(&cosmic, None);
        info!(
            "asked the compositor to fullscreen {app_id:?} {title:?}, launched as {}",
            hint.label
        );
        self.asked.push(cosmic);
    }
}

/// The registry's own event stream: the backend records the global list into the
/// dispatch data for us, so there is nothing to handle here.
impl Dispatch<WlRegistry, GlobalListContents> for Watcher {
    fn event(
        _state: &mut Self,
        _proxy: &WlRegistry,
        _event: RegistryEvent,
        _data: &GlobalListContents,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for Watcher {
    fn event(
        _state: &mut Self,
        _proxy: &ExtForeignToplevelListV1,
        event: ListEvent,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Each listed window arrives with its own handle, and it is on that
        // handle's events that matching happens.
        if let ListEvent::Finished = event {
            debug!("the compositor stopped listing windows");
        }
    }

    // The list's `toplevel` event creates the window handle, so the queue needs
    // to know which interface those objects have.
    wayland_client::event_created_child!(Watcher, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE
            => (ExtForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Watcher {
    fn event(
        state: &mut Self,
        window: &ExtForeignToplevelHandleV1,
        event: WindowEvent,
        _data: &(),
        _connection: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // A window reports its app id and its title as separate events, and
        // either one can be what a launch's hint matches on.
        match event {
            WindowEvent::AppId { app_id } => state.consider(window, qh, &app_id, ""),
            WindowEvent::Title { title } => state.consider(window, qh, "", &title),
            _ => {}
        }
    }
}

impl Dispatch<ZcosmicToplevelInfoV1, ()> for Watcher {
    fn event(
        _state: &mut Self,
        _proxy: &ZcosmicToplevelInfoV1,
        _event: CosmicInfoEvent,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // The window handles we ask for carry the state this protocol reports.
    }
}

impl Dispatch<ZcosmicToplevelHandleV1, ()> for Watcher {
    fn event(
        _state: &mut Self,
        _proxy: &ZcosmicToplevelHandleV1,
        _event: CosmicHandleEvent,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZcosmicToplevelManagerV1, ()> for Watcher {
    fn event(
        _state: &mut Self,
        _proxy: &ZcosmicToplevelManagerV1,
        event: ManagerEvent,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Worth recording: request 5 is fullscreen, and its absence is the
        // explanation when a window is asked for fullscreen and stays as it was.
        if let ManagerEvent::Capabilities { capabilities } = event {
            let announced: Vec<u32> = capabilities
                .chunks_exact(4)
                .map(|chunk| u32::from_ne_bytes(chunk.try_into().unwrap_or_default()))
                .collect();
            debug!("compositor window-management capabilities: {announced:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WindowHint;

    #[test]
    fn a_hint_matches_the_names_a_desktop_launch_reports() {
        let hint =
            WindowHint::for_entry("org.example.App.desktop", "Example App", Some("ExampleApp"));

        // The window may report the entry id, the declared window class, or a
        // title carrying the app's name.
        assert!(hint.matches("org.example.app", ""));
        assert!(hint.matches("", "Example App — untitled"));
        assert!(hint.matches("exampleapp", "Example App"));
        assert!(!hint.matches("stremio", "Stremio"));
        assert!(!hint.matches("", ""));
    }

    #[test]
    fn a_web_app_matches_by_title_because_the_window_is_the_browsers() {
        // A hand-written web app entry: the window belongs to the browser, and
        // its app id says so. Only the title says which page it is.
        let hint = WindowHint::for_entry("prime-video.desktop", "Prime Video", None);

        assert!(hint.matches("google-chrome", "Prime Video"));
        assert!(hint.matches("chromium", "prime video — watch"));
        assert!(!hint.matches("chromium", "New Tab"));
    }

    #[test]
    fn short_names_do_not_become_title_matches() {
        // A three-letter name is not a candidate of its own: it would match far
        // too much of any unrelated title. The entry's id still carries it.
        let dotted = WindowHint::for_entry("org.videolan.VLC.desktop", "VLC", None);

        assert!(dotted.matches("org.videolan.vlc", ""));
        assert!(!dotted.matches("", "VLC media player"));

        // A full name is a candidate, and is what a window titled after the app
        // matches on.
        let named = WindowHint::for_entry("vlc.desktop", "VLC media player", None);
        assert!(named.matches("", "VLC media player — playing"));
    }

    #[test]
    fn a_console_game_matches_on_its_title() {
        // Emulators title their window after the game, while the app id is the
        // emulator's own.
        let hint = WindowHint::for_entry("romm:4014", "Zoop", None);

        assert!(hint.matches("retroarch", "Zoop (USA)"));
        assert!(!hint.matches("retroarch", "RetroArch"));
    }

    /// Fullscreens a window, on the running session.
    ///
    /// Opens a short-lived dialog of its own rather than touching a window the
    /// user has open, asks for it the way a launch would, and checks the state
    /// the compositor reports back — the one part of this module that only a real
    /// compositor can answer. Ignored by default because it opens a window:
    ///
    /// `cargo test -p hearthdeck-frontend fullscreens_a_started_window -- --ignored --nocapture`
    #[test]
    #[ignore = "opens a window on the running session"]
    fn fullscreens_a_started_window() {
        use std::collections::HashMap;
        use std::process::Command;
        use std::time::{Duration, Instant};

        use cosmic_protocols::toplevel_info::v1::client::{
            zcosmic_toplevel_handle_v1::{
                Event as CosmicEvent, State as CosmicState, ZcosmicToplevelHandleV1,
            },
            zcosmic_toplevel_info_v1::{Event as InfoEvent, ZcosmicToplevelInfoV1},
        };
        use cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1::{
            Event as ManagerEvent, ZcosmicToplevelManagerV1,
        };
        use wayland_client::globals::{GlobalListContents, registry_queue_init};
        use wayland_client::protocol::wl_registry::{Event as RegistryEvent, WlRegistry};
        use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
        use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
            ext_foreign_toplevel_handle_v1::{Event as WindowEvent, ExtForeignToplevelHandleV1},
            ext_foreign_toplevel_list_v1::{self, Event as ListEvent, ExtForeignToplevelListV1},
        };

        const TITLE: &str = "hearthdeck-fullscreen-check";

        #[derive(Default)]
        struct Probe {
            /// Every listed window: its handle, and what it has reported.
            windows: HashMap<u32, (ExtForeignToplevelHandleV1, String, String)>,
            /// The window this test asked for, once it was found.
            asked: Option<ZcosmicToplevelHandleV1>,
            fullscreen: bool,
        }

        impl Dispatch<WlRegistry, GlobalListContents> for Probe {
            fn event(
                _state: &mut Self,
                _proxy: &WlRegistry,
                _event: RegistryEvent,
                _data: &GlobalListContents,
                _connection: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
            }
        }
        impl Dispatch<ExtForeignToplevelListV1, ()> for Probe {
            fn event(
                _state: &mut Self,
                _proxy: &ExtForeignToplevelListV1,
                _event: ListEvent,
                _data: &(),
                _connection: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
            }

            wayland_client::event_created_child!(Probe, ExtForeignToplevelListV1, [
                ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE
                    => (ExtForeignToplevelHandleV1, ())
            ]);
        }
        impl Dispatch<ExtForeignToplevelHandleV1, ()> for Probe {
            fn event(
                state: &mut Self,
                window: &ExtForeignToplevelHandleV1,
                event: WindowEvent,
                _data: &(),
                _connection: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
                let entry = state
                    .windows
                    .entry(window.id().protocol_id())
                    .or_insert_with(|| (window.clone(), String::new(), String::new()));
                match event {
                    WindowEvent::AppId { app_id } => entry.1 = app_id,
                    WindowEvent::Title { title } => entry.2 = title,
                    _ => {}
                }
            }
        }
        impl Dispatch<ZcosmicToplevelInfoV1, ()> for Probe {
            fn event(
                _state: &mut Self,
                _proxy: &ZcosmicToplevelInfoV1,
                _event: InfoEvent,
                _data: &(),
                _connection: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
            }
        }
        impl Dispatch<ZcosmicToplevelHandleV1, ()> for Probe {
            fn event(
                state: &mut Self,
                window: &ZcosmicToplevelHandleV1,
                event: CosmicEvent,
                _data: &(),
                _connection: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
                if state.asked.as_ref() != Some(window) {
                    return;
                }
                if let CosmicEvent::State { state: reported } = event {
                    state.fullscreen = reported.chunks_exact(4).any(|value| {
                        CosmicState::try_from(u32::from_ne_bytes(
                            value[0..4].try_into().unwrap_or_default(),
                        )) == Ok(CosmicState::Fullscreen)
                    });
                }
            }
        }
        impl Dispatch<ZcosmicToplevelManagerV1, ()> for Probe {
            fn event(
                _state: &mut Self,
                _proxy: &ZcosmicToplevelManagerV1,
                _event: ManagerEvent,
                _data: &(),
                _connection: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
            }
        }

        let mut window = Command::new("zenity")
            .args([
                "--info",
                &format!("--title={TITLE}"),
                "--text=Window for the fullscreen check.",
                // Closes itself, so a failed assertion cannot leave a dialog on
                // the desktop.
                "--timeout=20",
            ])
            .spawn()
            .expect("zenity opens a window to test with");

        let connection = Connection::connect_to_env().expect("a Wayland session");
        let (globals, mut queue) =
            registry_queue_init::<Probe>(&connection).expect("the registry reads");
        let qh = queue.handle();
        let _windows = globals
            .bind::<ExtForeignToplevelListV1, Probe, ()>(&qh, 1..=1, ())
            .expect("the compositor lists windows");
        let window_info = globals
            .bind::<ZcosmicToplevelInfoV1, Probe, ()>(&qh, 2..=3, ())
            .expect("the compositor reports window info");
        let manager = globals
            .bind::<ZcosmicToplevelManagerV1, Probe, ()>(&qh, 1..=4, ())
            .expect("the compositor manages windows");

        // The same matching a launch uses, against the window this test opened.
        let hint = WindowHint::for_entry("zenity", TITLE, None);
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut probe = Probe::default();
        while Instant::now() < deadline && !probe.fullscreen {
            queue.roundtrip(&mut probe).expect("a round trip");
            if probe.asked.is_none()
                && let Some((window, app_id, title)) = probe
                    .windows
                    .values()
                    .find(|(_, app_id, title)| hint.matches(app_id, title))
            {
                println!("found app_id={app_id:?} title={title:?}");
                // COSMIC's manager acts on its own view of a window, which is
                // requested for the handle the list gave us.
                let cosmic = window_info.get_cosmic_toplevel(window, &qh, ());
                manager.set_fullscreen(&cosmic, None);
                probe.asked = Some(cosmic);
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        let _ = window.kill();
        // Reaped rather than left running: the dialog closes itself after its
        // own timeout, so this only shortens the wait.
        let _ = window.wait();
        assert!(
            probe.fullscreen,
            "the compositor did not take the window fullscreen; windows seen: {:?}",
            probe.windows
        );
    }

    /// What the running compositor offers, and what its windows report.
    ///
    /// Ignored by default because it needs a live session:
    /// `cargo test -p hearthdeck-frontend -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a live Wayland session"]
    fn reports_the_compositors_windows() {
        use std::collections::HashMap;

        use wayland_client::globals::{GlobalListContents, registry_queue_init};
        use wayland_client::protocol::wl_registry::{Event as RegistryEvent, WlRegistry};
        use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
        use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
            ext_foreign_toplevel_handle_v1::{Event as WindowEvent, ExtForeignToplevelHandleV1},
            ext_foreign_toplevel_list_v1::{self, Event as ListEvent, ExtForeignToplevelListV1},
        };

        #[derive(Default)]
        struct Probe {
            ids: HashMap<u32, (String, String)>,
        }

        impl Dispatch<WlRegistry, GlobalListContents> for Probe {
            fn event(
                _state: &mut Self,
                _proxy: &WlRegistry,
                _event: RegistryEvent,
                _data: &GlobalListContents,
                _connection: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
            }
        }
        impl Dispatch<ExtForeignToplevelListV1, ()> for Probe {
            fn event(
                _state: &mut Self,
                _proxy: &ExtForeignToplevelListV1,
                _event: ListEvent,
                _data: &(),
                _connection: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
            }

            wayland_client::event_created_child!(Probe, ExtForeignToplevelListV1, [
                ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE
                    => (ExtForeignToplevelHandleV1, ())
            ]);
        }
        impl Dispatch<ExtForeignToplevelHandleV1, ()> for Probe {
            fn event(
                state: &mut Self,
                window: &ExtForeignToplevelHandleV1,
                event: WindowEvent,
                _data: &(),
                _connection: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
                let entry = state.ids.entry(window.id().protocol_id()).or_default();
                match event {
                    WindowEvent::AppId { app_id } => entry.0 = app_id,
                    WindowEvent::Title { title } => entry.1 = title,
                    _ => {}
                }
            }
        }

        let connection = match Connection::connect_to_env() {
            Ok(connection) => connection,
            Err(error) => {
                println!("no Wayland session: {error}");
                return;
            }
        };
        let (globals, mut queue) =
            registry_queue_init::<Probe>(&connection).expect("the registry reads");
        let qh = queue.handle();
        globals
            .bind::<ExtForeignToplevelListV1, Probe, ()>(&qh, 1..=1, ())
            .expect("the compositor lists windows");

        let mut probe = Probe::default();
        // Two round trips: the first lists the windows, the second delivers the
        // app ids and titles that follow their handles.
        queue.roundtrip(&mut probe).expect("roundtrip");
        queue.roundtrip(&mut probe).expect("roundtrip");

        for (_, (app_id, title)) in probe.ids {
            println!("app_id={app_id:?} title={title:?}");
        }
    }
}
