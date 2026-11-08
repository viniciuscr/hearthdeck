//! Which surface the Settings destination opens.
//!
//! Settings ships as its own GPUI process: gpui renders through its own window
//! and event loop, so it cannot be mounted inside an iced `Element`. The split
//! is drawn where it is cheap - the window only reads - and this module owns the
//! half of it the shell has to know: which binary, whether to launch it, whether
//! one is already up, and how the pad reaches it.
//!
//! Two environment variables decide, and neither is needed in a normal start:
//!
//! - `HEARTHDECK_GPUI_SETTINGS` -- a binary to launch instead of the packaged
//!   one, for running an uninstalled build.
//! - `HEARTHDECK_SETTINGS_UI=iced` -- the rollback: keep the in-app page even
//!   when the window is installed.
//!
//! The pad is forwarded, not re-read: the broker gives the controller to one
//! process (this one), so this module relays the semantic action it already
//! received - `up`, `confirm`, `back` - over a socket the window binds. The
//! window turns each word into the same action its keys are bound to, so both
//! surfaces run one navigation vocabulary.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use crate::subscriptions::gamepad::GamepadEvent;

/// Where the package installs the settings window.
const PACKAGED_BINARY: &str = "/usr/lib/hearthdeck/hearthdeck-settings";

/// The socket the window binds, named in the child's environment.
const INPUT_ENV: &str = "HEARTHDECK_SETTINGS_INPUT";

/// What opening Settings did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Open {
    /// Nothing to launch, or the rollback is set: draw the in-app page.
    InApp,
    /// The window was started.
    Launched,
    /// The window is already up; bringing it forward is the compositor's job.
    AlreadyRunning,
    /// The binary is there but would not start. The caller falls back to the
    /// in-app page: a broken window must not be a dead end.
    Failed,
}

/// The settings window, as the shell sees it.
pub struct SettingsWindow {
    binary: Option<PathBuf>,
    forced_in_app: bool,
    /// Set while the child runs, cleared by the thread that reaps it. A thread
    /// per launch is cheap here - the window opens once in a while - and it
    /// means no zombie and no `Child` bookkeeping in `update`.
    running: Option<Arc<AtomicBool>>,
    /// Where forwarded pad actions go while the window is up.
    input: Option<Sender<GamepadEvent>>,
}

impl SettingsWindow {
    /// Resolve once, at startup: a dev override, else the packaged binary.
    pub fn from_environment() -> Self {
        let binary = std::env::var_os("HEARTHDECK_GPUI_SETTINGS")
            .map(PathBuf::from)
            .or_else(|| {
                let packaged = Path::new(PACKAGED_BINARY);
                packaged.is_file().then(|| packaged.to_path_buf())
            });
        Self::from_parts(
            binary,
            matches!(
                std::env::var("HEARTHDECK_SETTINGS_UI").as_deref(),
                Ok("iced")
            ),
        )
    }

    /// The environment is read in [`Self::from_environment`]; this is the
    /// decision itself, so it can be tested without touching the process env.
    pub fn from_parts(binary: Option<PathBuf>, forced_in_app: bool) -> Self {
        Self {
            binary,
            forced_in_app,
            running: None,
            input: None,
        }
    }

    /// A shell that always draws the in-app page - the tests' shell, so no test
    /// can start a process.
    #[cfg(test)]
    pub fn in_app() -> Self {
        Self::from_parts(None, true)
    }

    /// Whether the window is up, which is what decides whether the shell keeps
    /// the pad to itself or hands it on.
    pub fn is_running(&self) -> bool {
        self.is_open()
    }

    /// Hand one semantic action to the window, if it is up. `false` means the
    /// shell keeps it - no window, or the write failed - so the caller's normal
    /// navigation runs unchanged.
    pub fn forward(&self, event: GamepadEvent) -> bool {
        if !self.is_open() {
            return false;
        }
        let Some(input) = &self.input else {
            return false;
        };
        input.send(event).is_ok()
    }

    /// Open the settings surface, launching the window when there is one.
    pub fn open(&mut self) -> Open {
        if self.forced_in_app {
            return Open::InApp;
        }
        let Some(binary) = self.binary.clone() else {
            return Open::InApp;
        };
        if self.is_open() {
            return Open::AlreadyRunning;
        }
        let socket = input_socket();
        let mut command = Command::new(&binary);
        if let Some(socket) = &socket {
            command.env(INPUT_ENV, socket);
        }
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!(
                    %error,
                    binary = %binary.display(),
                    "the settings window would not start; drawing the in-app page"
                );
                return Open::Failed;
            }
        };
        tracing::info!(
            pid = child.id(),
            binary = %binary.display(),
            "settings window launched"
        );
        let running = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&running);
        std::thread::spawn(move || {
            let mut child = child;
            if let Err(error) = child.wait() {
                tracing::warn!(%error, "waiting on the settings window failed");
            }
            flag.store(false, Ordering::Relaxed);
        });
        self.running = Some(running);
        self.input = socket.map(forward_actions);
        Open::Launched
    }

    fn is_open(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| running.load(Ordering::Relaxed))
    }
}

/// Where the window binds its input socket. `None` when there is no runtime
/// directory, which only means the pad cannot reach it.
fn input_socket() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(|dir| PathBuf::from(dir).join("hearthdeck-settings-input.sock"))
}

/// Relays forwarded actions to the window on its own thread, connecting when the
/// first one arrives - the window binds its socket a moment after it starts.
fn forward_actions(socket: PathBuf) -> Sender<GamepadEvent> {
    let (sender, receiver) = channel::<GamepadEvent>();
    std::thread::spawn(move || {
        let mut stream: Option<UnixStream> = None;
        while let Ok(event) = receiver.recv() {
            let Some(name) = crate::subscriptions::gamepad::name(event) else {
                continue;
            };
            if stream.is_none() {
                stream = connect(&socket);
            }
            let Some(active) = stream.as_mut() else {
                continue;
            };
            if writeln!(active, "{name}").is_err() {
                stream = None;
            }
        }
    });
    sender
}

/// A bounded wait: the window binds after `spawn` returns, and a window that
/// never comes up must not leave this thread retrying forever.
fn connect(socket: &Path) -> Option<UnixStream> {
    for _ in 0..20 {
        if let Ok(stream) = UnixStream::connect(socket) {
            return Some(stream);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{Open, SettingsWindow, forward_actions};
    use crate::subscriptions::gamepad::GamepadEvent;
    use std::io::BufRead as _;
    use std::path::PathBuf;

    /// With no window up the shell keeps the pad: forwarding reports it did not
    /// take the action, so normal navigation runs untouched.
    #[test]
    fn nothing_is_forwarded_when_no_window_is_up() {
        assert!(!SettingsWindow::in_app().forward(GamepadEvent::Confirm));
    }

    /// The relay writes the words the window's parser reads, in order, over a
    /// real socket - the two halves of the contract, together.
    #[test]
    fn forwarded_actions_reach_the_socket_in_order() {
        let path = std::env::temp_dir().join(format!(
            "hearthdeck-forward-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("a listening socket");

        let sender = forward_actions(path.clone());
        sender.send(GamepadEvent::MoveDown).expect("a send");
        sender.send(GamepadEvent::Confirm).expect("a send");
        drop(sender);

        let (stream, _) = listener.accept().expect("the relay connects");
        let mut lines = std::io::BufReader::new(stream).lines();
        assert_eq!(lines.next().expect("a line").expect("a line"), "down");
        assert_eq!(lines.next().expect("a line").expect("a line"), "confirm");
        let _ = std::fs::remove_file(&path);
    }

    /// With nothing installed and nothing named, the destination stays the
    /// in-app page: the window is an addition, never a dependency.
    #[test]
    fn no_binary_keeps_the_in_app_page() {
        assert_eq!(SettingsWindow::from_parts(None, false).open(), Open::InApp);
        assert_eq!(SettingsWindow::in_app().open(), Open::InApp);
    }

    /// The rollback wins over an installed window, so a bad deployment can be
    /// recovered by one environment variable.
    #[test]
    fn the_rollback_flag_beats_an_installed_window() {
        let binary = Some(PathBuf::from("/bin/true"));
        assert_eq!(SettingsWindow::from_parts(binary, true).open(), Open::InApp);
    }

    /// A binary that cannot run reports the failure instead of panicking or
    /// pretending the window opened; the caller draws the page instead.
    #[test]
    fn a_binary_that_cannot_run_reports_failure() {
        let mut window =
            SettingsWindow::from_parts(Some(PathBuf::from("/nonexistent/settings")), false);
        assert_eq!(window.open(), Open::Failed);
    }
}
