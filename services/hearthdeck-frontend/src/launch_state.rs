#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LaunchState {
    #[default]
    Idle,
    Launching {
        title: String,
    },
    Failed {
        title: String,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Start(String),
    Accepted,
    Failed(String),
    Dismiss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    None,
    Launch,
    DelayDismiss,
}

impl LaunchState {
    pub fn update(&mut self, event: Event) -> Effect {
        match event {
            Event::Start(title) if matches!(self, Self::Idle) => {
                *self = Self::Launching { title };
                Effect::Launch
            }
            Event::Start(_) => Effect::None,
            Event::Accepted if matches!(self, Self::Launching { .. }) => Effect::DelayDismiss,
            Event::Accepted => Effect::None,
            Event::Failed(error) => {
                let Self::Launching { title } = self else {
                    return Effect::None;
                };
                *self = Self::Failed {
                    title: std::mem::take(title),
                    error,
                };
                Effect::None
            }
            Event::Dismiss => {
                *self = Self::Idle;
                Effect::None
            }
        }
    }

    pub fn is_visible(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    pub fn title(&self) -> Option<&str> {
        match self {
            Self::Launching { title } | Self::Failed { title, .. } => Some(title),
            Self::Idle => None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Effect, Event, LaunchState};

    #[test]
    fn start_makes_launch_progress_visible() {
        let mut state = LaunchState::default();

        assert_eq!(
            state.update(Event::Start("Celeste".to_owned())),
            Effect::Launch
        );
        assert!(state.is_visible());
        assert_eq!(state.title(), Some("Celeste"));
        assert_eq!(state.error(), None);
    }

    #[test]
    fn acceptance_keeps_progress_visible_until_delayed_dismissal() {
        let mut state = LaunchState::Launching {
            title: "Celeste".to_owned(),
        };

        assert_eq!(state.update(Event::Accepted), Effect::DelayDismiss);
        assert!(state.is_visible());
        state.update(Event::Dismiss);
        assert_eq!(state, LaunchState::Idle);
    }

    #[test]
    fn failure_stays_visible_with_the_error_until_dismissed() {
        let mut state = LaunchState::Launching {
            title: "Celeste".to_owned(),
        };

        assert_eq!(
            state.update(Event::Failed("bridge unavailable".to_owned())),
            Effect::None
        );
        assert_eq!(state.title(), Some("Celeste"));
        assert_eq!(state.error(), Some("bridge unavailable"));
        state.update(Event::Dismiss);
        assert_eq!(state, LaunchState::Idle);
    }

    #[test]
    fn duplicate_start_is_ignored_while_launching() {
        let mut state = LaunchState::Launching {
            title: "Celeste".to_owned(),
        };

        assert_eq!(
            state.update(Event::Start("Another game".to_owned())),
            Effect::None
        );
        assert_eq!(state.title(), Some("Celeste"));
    }

    #[test]
    fn stale_results_do_not_open_an_overlay() {
        let mut state = LaunchState::Idle;

        assert_eq!(state.update(Event::Accepted), Effect::None);
        assert_eq!(
            state.update(Event::Failed("old request".to_owned())),
            Effect::None
        );
        assert_eq!(state, LaunchState::Idle);
    }
}
