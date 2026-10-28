/// How many entries the quick menu has in its normal state. The menu tables in
/// `overlay.rs` declare their arrays with this length, so a table that grows
/// without this following is a compile error rather than a stuck D-pad.
pub const MENU_ITEM_COUNT: usize = 3;

/// How many entries a retry menu has (after a failed close or log out): the
/// failed action and Resume. Held separately because navigation wraps within
/// whichever menu is on screen.
pub const RETRY_ITEM_COUNT: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Menu,
    Closing,
    CloseFailed,
    LoggingOut,
    LogOutFailed,
}

impl Status {
    /// Whether a close or log out is already committed to its effect. There is
    /// nothing to navigate or activate until that reports back.
    fn busy(self) -> bool {
        matches!(self, Status::Closing | Status::LoggingOut)
    }

    /// How many entries the menu shown in this status has, for D-pad wrap-around.
    fn item_count(self) -> usize {
        match self {
            Status::Menu => MENU_ITEM_COUNT,
            Status::CloseFailed | Status::LogOutFailed => RETRY_ITEM_COUNT,
            // No menu is navigable while busy; the count is never consulted.
            Status::Closing | Status::LoggingOut => RETRY_ITEM_COUNT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Toggle,
    NavigateUp,
    NavigateDown,
    Activate,
    Resume,
    Close,
    CloseSucceeded,
    CloseFailed,
    HideAfterClose,
    LogOut,
    LogOutSucceeded,
    LogOutFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    None,
    Show,
    Hide,
    StopApplication,
    DelayHide,
    LogOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayState {
    pub visible: bool,
    pub status: Status,
    pub selected: usize,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            visible: false,
            status: Status::Menu,
            selected: 0,
        }
    }
}

impl OverlayState {
    pub fn update(&mut self, event: Event) -> Effect {
        match event {
            Event::Toggle if self.status.busy() => Effect::None,
            Event::Toggle if self.visible => {
                self.visible = false;
                Effect::Hide
            }
            Event::Toggle => {
                self.visible = true;
                self.status = Status::Menu;
                self.selected = 0;
                Effect::Show
            }
            Event::NavigateUp | Event::NavigateDown if self.visible && !self.status.busy() => {
                let item_count = self.status.item_count();
                self.selected = if event == Event::NavigateUp {
                    (self.selected + item_count - 1) % item_count
                } else {
                    (self.selected + 1) % item_count
                };
                Effect::None
            }
            Event::NavigateUp | Event::NavigateDown => Effect::None,
            Event::Activate if !self.visible || self.status.busy() => Effect::None,
            Event::Activate => match (self.status, self.selected) {
                (Status::Menu, 0) | (Status::CloseFailed, 1) | (Status::LogOutFailed, 1) => {
                    self.update(Event::Resume)
                }
                (Status::Menu, 1) | (Status::CloseFailed, 0) => self.update(Event::Close),
                (Status::Menu, 2) | (Status::LogOutFailed, 0) => self.update(Event::LogOut),
                _ => Effect::None,
            },
            Event::Resume => {
                self.visible = false;
                self.status = Status::Menu;
                Effect::Hide
            }
            Event::Close if self.status.busy() => Effect::None,
            Event::Close => {
                self.status = Status::Closing;
                Effect::StopApplication
            }
            Event::CloseSucceeded => Effect::DelayHide,
            Event::CloseFailed => {
                self.status = Status::CloseFailed;
                self.selected = 0;
                Effect::None
            }
            Event::HideAfterClose => {
                self.visible = false;
                self.status = Status::Menu;
                Effect::Hide
            }
            Event::LogOut if self.status.busy() => Effect::None,
            Event::LogOut => {
                self.status = Status::LoggingOut;
                Effect::LogOut
            }
            // The session is going away on success, so hide and reset exactly
            // like Resume: nothing waits on the other side.
            Event::LogOutSucceeded => self.update(Event::Resume),
            Event::LogOutFailed => {
                self.status = Status::LogOutFailed;
                self.selected = 0;
                Effect::None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Effect, Event, OverlayState, Status};

    #[test]
    fn toggle_opens_and_closes_the_menu() {
        let mut state = OverlayState::default();

        assert_eq!(state.update(Event::Toggle), Effect::Show);
        assert!(state.visible);
        assert_eq!(state.update(Event::Toggle), Effect::Hide);
        assert!(!state.visible);
    }

    #[test]
    fn opening_resets_selection_and_failure_state() {
        for status in [Status::CloseFailed, Status::LogOutFailed] {
            let mut state = OverlayState {
                visible: false,
                status,
                selected: 1,
            };

            assert_eq!(state.update(Event::Toggle), Effect::Show);
            assert_eq!(state.status, Status::Menu, "status: {status:?}");
            assert_eq!(state.selected, 0);
        }
    }

    #[test]
    fn navigation_wraps_around_all_menu_items() {
        let mut state = OverlayState {
            visible: true,
            ..OverlayState::default()
        };

        state.update(Event::NavigateUp);
        assert_eq!(state.selected, 2);
        state.update(Event::NavigateDown);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn retry_menu_navigation_stays_within_two_items() {
        for status in [Status::CloseFailed, Status::LogOutFailed] {
            let mut state = OverlayState {
                visible: true,
                status,
                selected: 0,
            };

            state.update(Event::NavigateUp);
            assert_eq!(state.selected, 1, "status: {status:?}");
            state.update(Event::NavigateDown);
            assert_eq!(state.selected, 0, "status: {status:?}");
        }
    }

    #[test]
    fn activation_runs_the_selected_menu_action() {
        let mut menu = OverlayState {
            visible: true,
            ..OverlayState::default()
        };

        assert_eq!(menu.update(Event::Activate), Effect::Hide);

        let mut close = OverlayState {
            visible: true,
            selected: 1,
            ..OverlayState::default()
        };
        assert_eq!(close.update(Event::Activate), Effect::StopApplication);

        let mut retry = OverlayState {
            visible: true,
            status: Status::CloseFailed,
            selected: 0,
        };
        assert_eq!(retry.update(Event::Activate), Effect::StopApplication);

        let mut resume = OverlayState {
            visible: true,
            status: Status::CloseFailed,
            selected: 1,
        };
        assert_eq!(resume.update(Event::Activate), Effect::Hide);

        let mut logout = OverlayState {
            visible: true,
            selected: 2,
            ..OverlayState::default()
        };
        assert_eq!(logout.update(Event::Activate), Effect::LogOut);
    }

    #[test]
    fn log_out_failure_menu_activation_retries_or_resumes() {
        let mut retry = OverlayState {
            visible: true,
            status: Status::LogOutFailed,
            selected: 0,
        };
        assert_eq!(retry.update(Event::Activate), Effect::LogOut);

        let mut resume = OverlayState {
            visible: true,
            status: Status::LogOutFailed,
            selected: 1,
        };
        assert_eq!(resume.update(Event::Activate), Effect::Hide);
    }

    #[test]
    fn close_stays_visible_until_delayed_hide() {
        let mut state = OverlayState {
            visible: true,
            ..OverlayState::default()
        };

        assert_eq!(state.update(Event::Close), Effect::StopApplication);
        assert_eq!(state.status, Status::Closing);
        assert!(state.visible);
        assert_eq!(state.update(Event::CloseSucceeded), Effect::DelayHide);
        assert!(state.visible);
        assert_eq!(state.update(Event::HideAfterClose), Effect::Hide);
        assert!(!state.visible);
    }

    #[test]
    fn close_failure_remains_visible_and_can_retry() {
        let mut state = OverlayState {
            visible: true,
            status: Status::Closing,
            selected: 1,
        };

        assert_eq!(state.update(Event::CloseFailed), Effect::None);
        assert_eq!(state.status, Status::CloseFailed);
        assert_eq!(state.selected, 0);
        assert!(state.visible);
        assert_eq!(state.update(Event::Close), Effect::StopApplication);
        assert_eq!(state.status, Status::Closing);
    }

    #[test]
    fn closing_ignores_duplicate_close_toggle_and_navigation() {
        let mut state = OverlayState {
            visible: true,
            status: Status::Closing,
            selected: 0,
        };

        assert_eq!(state.update(Event::Close), Effect::None);
        assert_eq!(state.update(Event::Toggle), Effect::None);
        assert_eq!(state.update(Event::NavigateDown), Effect::None);
        assert!(state.visible);
    }

    #[test]
    fn resume_always_hides_and_resets_the_menu() {
        let mut state = OverlayState {
            visible: true,
            status: Status::CloseFailed,
            selected: 1,
        };

        assert_eq!(state.update(Event::Resume), Effect::Hide);
        assert!(!state.visible);
        assert_eq!(state.status, Status::Menu);
    }

    #[test]
    fn logging_out_ignores_further_input_until_it_reports() {
        let mut state = OverlayState {
            visible: true,
            ..OverlayState::default()
        };

        assert_eq!(state.update(Event::LogOut), Effect::LogOut);
        assert_eq!(state.status, Status::LoggingOut);
        assert!(state.visible);

        assert_eq!(state.update(Event::LogOut), Effect::None);
        assert_eq!(state.update(Event::Toggle), Effect::None);
        assert_eq!(state.update(Event::NavigateDown), Effect::None);
        assert_eq!(state.update(Event::Activate), Effect::None);
        assert_eq!(state.status, Status::LoggingOut);
        assert!(state.visible);
    }

    #[test]
    fn successful_logout_hides_and_resets_the_menu() {
        let mut state = OverlayState {
            visible: true,
            status: Status::LoggingOut,
            ..OverlayState::default()
        };

        assert_eq!(state.update(Event::LogOutSucceeded), Effect::Hide);
        assert!(!state.visible);
        assert_eq!(state.status, Status::Menu);
    }

    #[test]
    fn logout_failure_remains_visible_and_can_retry() {
        let mut state = OverlayState {
            visible: true,
            status: Status::LoggingOut,
            selected: 2,
        };

        assert_eq!(state.update(Event::LogOutFailed), Effect::None);
        assert_eq!(state.status, Status::LogOutFailed);
        assert_eq!(state.selected, 0);
        assert!(state.visible);
        assert_eq!(state.update(Event::LogOut), Effect::LogOut);
        assert_eq!(state.status, Status::LoggingOut);
    }
}
