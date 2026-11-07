//! Which surface the Settings destination opens.
//!
//! Settings ships as its own GPUI process: gpui renders through its own window
//! and event loop, so it cannot be mounted inside an iced `Element`. The split
//! is drawn where it is cheap - the window only reads - and this module owns the
//! half of it the shell has to know: which binary, whether to launch it, and
//! whether one is already up.
//!
//! Two environment variables decide, and neither is needed in a normal start:
//!
//! - `HEARTHDECK_GPUI_SETTINGS` -- a binary to launch instead of the packaged
//!   one, for running an uninstalled build.
//! - `HEARTHDECK_SETTINGS_UI=iced` -- the rollback: keep the in-app page even
//!   when the window is installed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Where the package installs the settings window.
const PACKAGED_BINARY: &str = "/usr/lib/hearthdeck/hearthdeck-settings";

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
        }
    }

    /// A shell that always draws the in-app page - the tests' shell, so no test
    /// can start a process.
    #[cfg(test)]
    pub fn in_app() -> Self {
        Self::from_parts(None, true)
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
        let child = match Command::new(&binary).spawn() {
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
        Open::Launched
    }

    fn is_open(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| running.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::{Open, SettingsWindow};
    use std::path::PathBuf;

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
