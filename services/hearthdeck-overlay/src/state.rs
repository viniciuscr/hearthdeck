#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Menu,
    Closing,
    CloseFailed,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    None,
    Show,
    Hide,
    StopApplication,
    DelayHide,
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
            Event::Toggle if self.status == Status::Closing => Effect::None,
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
            Event::NavigateUp | Event::NavigateDown
                if self.visible && self.status != Status::Closing =>
            {
                let item_count = 2;
                self.selected = if event == Event::NavigateUp {
                    (self.selected + item_count - 1) % item_count
                } else {
                    (self.selected + 1) % item_count
                };
                Effect::None
            }
            Event::NavigateUp | Event::NavigateDown => Effect::None,
            Event::Activate if !self.visible || self.status == Status::Closing => Effect::None,
            Event::Activate => match (self.status, self.selected) {
                (Status::Menu, 0) | (Status::CloseFailed, 1) => self.update(Event::Resume),
                (Status::Menu, 1) | (Status::CloseFailed, 0) => self.update(Event::Close),
                _ => Effect::None,
            },
            Event::Resume => {
                self.visible = false;
                self.status = Status::Menu;
                Effect::Hide
            }
            Event::Close if self.status == Status::Closing => Effect::None,
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
        let mut state = OverlayState {
            visible: false,
            status: Status::CloseFailed,
            selected: 1,
        };

        assert_eq!(state.update(Event::Toggle), Effect::Show);
        assert_eq!(state.status, Status::Menu);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn navigation_wraps_both_menu_items() {
        let mut state = OverlayState {
            visible: true,
            ..OverlayState::default()
        };

        state.update(Event::NavigateUp);
        assert_eq!(state.selected, 1);
        state.update(Event::NavigateDown);
        assert_eq!(state.selected, 0);
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
}
