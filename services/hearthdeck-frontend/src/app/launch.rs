//! Starting a launch and watching for the session it starts.

use cosmic::app::Task;
use hearthdeck_protocol::InputProfile;

use crate::input_ownership::Event as InputEvent;
use crate::launch_state::{Effect as LaunchEffect, Event as LaunchEvent};
use crate::toplevel::{WindowHint, fullscreen_when_it_appears};

use super::{App, Message};

impl App {
    /// Launches the entry the tile at `index` names.
    pub(super) fn activate(&mut self, index: usize) -> Task<Message> {
        let Some(entry) = self.visible.get(index).cloned() else {
            return Task::none();
        };
        self.launch_managed(entry.id.clone(), entry.name.clone())
    }

    /// Starts a launch: guards input ownership, shows the overlay, registers the
    /// window hint with the compositor watcher, and posts the item id to the
    /// daemon's single launch route - which source that is stays the daemon's
    /// side of the wire.
    pub(super) fn launch_managed(&mut self, item_id: String, title: String) -> Task<Message> {
        // A game is already starting, or one owns the screen; a second launch
        // would be lost.
        if !self.input_ownership.frontend_has_control() {
            return Task::none();
        }
        let input_profile = if self.config.desktop_input_enabled(&item_id) {
            InputProfile::Desktop
        } else {
            InputProfile::Native
        };
        // Built before the launch so the compositor watcher is already listening
        // when the new window appears.
        let window_hint = self.launch_window_hint(&item_id, &title);
        if self.launch_state.update(LaunchEvent::Start(title)) != LaunchEffect::Launch {
            return Task::none();
        }
        self.input_ownership.update(InputEvent::LaunchStarted);
        fullscreen_when_it_appears(window_hint);
        let client = self.daemon_client.clone();
        Task::perform(
            async move { client.launch_item(&item_id, input_profile).await },
            |result| {
                cosmic::Action::App(Message::DaemonLaunchResult(
                    result.map(|_| ()).map_err(|error| error.to_string()),
                ))
            },
        )
    }

    /// The window class the entry declares, for the compositor watcher that
    /// takes the launched window fullscreen.
    pub(super) fn launch_window_hint(&self, app_id: &str, title: &str) -> WindowHint {
        let wm_class = self
            .catalog
            .iter()
            .find(|entry| entry.id == app_id)
            .and_then(|entry| entry.wm_class.clone());
        WindowHint::for_entry(app_id, title, wm_class.as_deref())
    }

    /// Polls the daemon for a running session. Input ownership needs this to know
    /// when a launched game exits and the frontend can act again.
    pub(super) fn poll_active_session(&self, delay: std::time::Duration) -> Task<Message> {
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
}
