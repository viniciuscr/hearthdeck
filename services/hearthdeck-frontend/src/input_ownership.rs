#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputOwnership {
    #[default]
    Checking,
    Frontend,
    Launching,
    ManagedSession,
    Unfocused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    SessionObserved(bool),
    SessionCheckFailed,
    LaunchStarted,
    LaunchAccepted,
    LaunchFailed,
    FrontendFocused,
    FrontendUnfocused,
}

impl InputOwnership {
    pub fn update(&mut self, event: Event) {
        match event {
            Event::SessionObserved(true)
                if matches!(
                    self,
                    Self::Checking | Self::Launching | Self::ManagedSession
                ) =>
            {
                *self = Self::ManagedSession;
            }
            Event::SessionObserved(false)
                if matches!(self, Self::Checking | Self::ManagedSession) =>
            {
                *self = Self::Frontend;
            }
            Event::LaunchStarted if matches!(self, Self::Frontend) => *self = Self::Launching,
            Event::LaunchAccepted if matches!(self, Self::Launching) => {
                *self = Self::ManagedSession;
            }
            Event::LaunchFailed if matches!(self, Self::Launching) => *self = Self::Frontend,
            Event::FrontendFocused if matches!(self, Self::ManagedSession | Self::Unfocused) => {
                *self = Self::Frontend;
            }
            Event::FrontendUnfocused
                if matches!(
                    self,
                    Self::Frontend | Self::ManagedSession | Self::Unfocused
                ) =>
            {
                *self = Self::Unfocused;
            }
            Event::SessionObserved(false)
            | Event::SessionObserved(true)
            | Event::SessionCheckFailed
            | Event::LaunchStarted
            | Event::LaunchAccepted
            | Event::LaunchFailed
            | Event::FrontendFocused
            | Event::FrontendUnfocused => {}
        }
    }

    pub fn frontend_has_control(self) -> bool {
        matches!(self, Self::Frontend)
    }
}

pub fn managed_catalog_id(value: &str) -> Option<&str> {
    value
        .strip_prefix("hearthdeck:")
        .filter(|identifier| !identifier.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{Event, InputOwnership, managed_catalog_id};

    #[test]
    fn only_daemon_catalog_ids_are_launchable() {
        assert_eq!(
            managed_catalog_id("hearthdeck:desktop:example"),
            Some("desktop:example")
        );
        assert_eq!(managed_catalog_id("flatpak:example"), None);
        assert_eq!(managed_catalog_id("hearthdeck:"), None);
    }

    #[test]
    fn startup_disables_input_until_session_state_is_known() {
        let mut ownership = InputOwnership::default();

        assert!(!ownership.frontend_has_control());
        ownership.update(Event::SessionObserved(false));
        assert!(ownership.frontend_has_control());
    }

    #[test]
    fn focus_events_cannot_bypass_the_initial_session_check() {
        let mut ownership = InputOwnership::default();

        ownership.update(Event::FrontendFocused);
        ownership.update(Event::FrontendUnfocused);
        ownership.update(Event::FrontendFocused);

        assert_eq!(ownership, InputOwnership::Checking);
        assert!(!ownership.frontend_has_control());
    }

    #[test]
    fn active_session_owns_input_until_the_bridge_reports_exit() {
        let mut ownership = InputOwnership::Frontend;

        ownership.update(Event::LaunchStarted);
        assert!(!ownership.frontend_has_control());
        ownership.update(Event::LaunchAccepted);
        ownership.update(Event::SessionObserved(true));
        assert!(!ownership.frontend_has_control());
        ownership.update(Event::SessionObserved(false));
        assert!(ownership.frontend_has_control());
    }

    #[test]
    fn stale_inactive_poll_cannot_reenable_input_during_launch() {
        let mut ownership = InputOwnership::Frontend;

        ownership.update(Event::LaunchStarted);
        ownership.update(Event::SessionObserved(false));

        assert_eq!(ownership, InputOwnership::Launching);
        assert!(!ownership.frontend_has_control());
    }

    #[test]
    fn session_lookup_failure_fails_closed_after_launch() {
        let mut ownership = InputOwnership::ManagedSession;

        ownership.update(Event::SessionCheckFailed);

        assert_eq!(ownership, InputOwnership::ManagedSession);
        assert!(!ownership.frontend_has_control());
    }

    #[test]
    fn initial_session_lookup_failure_keeps_input_disabled() {
        let mut ownership = InputOwnership::default();

        ownership.update(Event::SessionCheckFailed);

        assert_eq!(ownership, InputOwnership::Checking);
        assert!(!ownership.frontend_has_control());
    }

    #[test]
    fn another_launch_cannot_start_while_external_session_owns_input() {
        let mut ownership = InputOwnership::ManagedSession;

        ownership.update(Event::LaunchStarted);

        assert_eq!(ownership, InputOwnership::ManagedSession);
        assert!(!ownership.frontend_has_control());
    }

    #[test]
    fn compositor_focus_return_restores_input_for_a_persistent_session() {
        let mut ownership = InputOwnership::ManagedSession;

        ownership.update(Event::FrontendFocused);
        ownership.update(Event::SessionObserved(true));

        assert_eq!(ownership, InputOwnership::Frontend);
        assert!(ownership.frontend_has_control());
    }

    #[test]
    fn losing_focus_disables_input_until_focus_returns() {
        let mut ownership = InputOwnership::Frontend;

        ownership.update(Event::FrontendUnfocused);
        ownership.update(Event::SessionObserved(false));
        assert_eq!(ownership, InputOwnership::Unfocused);
        assert!(!ownership.frontend_has_control());

        ownership.update(Event::FrontendFocused);
        assert!(ownership.frontend_has_control());
    }

    #[test]
    fn failed_launch_returns_control_to_the_frontend() {
        let mut ownership = InputOwnership::Frontend;

        ownership.update(Event::LaunchStarted);
        ownership.update(Event::LaunchFailed);

        assert!(ownership.frontend_has_control());
    }
}
