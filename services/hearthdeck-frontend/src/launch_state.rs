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

/// What a launch draws over the page, when it covers one at all.
///
/// The data the launch layer renders. The decision of *whether* it covers a
/// page lives on [`LaunchState::overlay`], so the rule is one answer every
/// page draws by instead of a guard each view has to remember.
#[derive(Debug, PartialEq, Eq)]
pub enum Overlay<'a> {
    /// The game is starting.
    Progress { title: &'a str },
    /// The launch failed, and the reason is what the user reads.
    Failed { title: &'a str, error: &'a str },
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

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }

    /// The layer this launch draws over a page, or `None` when it covers none.
    ///
    /// A launch in flight takes the page over; a failure stays until it is
    /// read. An idle launch covers nothing - the page is what the user asked
    /// to see, never a layer claiming a game is starting.
    pub fn overlay(&self) -> Option<Overlay<'_>> {
        match self {
            Self::Idle => None,
            Self::Launching { title } => Some(Overlay::Progress { title }),
            Self::Failed { title, error } => Some(Overlay::Failed { title, error }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Effect, Event, LaunchState, Overlay};

    #[test]
    fn start_makes_launch_progress_visible() {
        let mut state = LaunchState::default();

        assert_eq!(
            state.update(Event::Start("Celeste".to_owned())),
            Effect::Launch
        );
        assert!(state.is_visible());
        assert_eq!(
            state,
            LaunchState::Launching {
                title: "Celeste".to_owned()
            }
        );
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
        assert_eq!(
            state,
            LaunchState::Failed {
                title: "Celeste".to_owned(),
                error: "bridge unavailable".to_owned()
            }
        );
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
        assert_eq!(
            state,
            LaunchState::Launching {
                title: "Celeste".to_owned()
            }
        );
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

    /// An idle launch covers nothing: the page is what the user asked to see,
    /// never a layer saying a game is starting. This is the rule every page
    /// draws its launch layer by.
    #[test]
    fn an_idle_launch_draws_no_overlay_over_the_page() {
        assert!(LaunchState::Idle.overlay().is_none());
        assert!(LaunchState::default().overlay().is_none());
    }

    #[test]
    fn a_launch_in_flight_draws_the_progress_overlay() {
        let state = LaunchState::Launching {
            title: "Celeste".to_owned(),
        };

        assert_eq!(
            state.overlay(),
            Some(Overlay::Progress { title: "Celeste" })
        );
    }

    /// A failure is a message to read, so it stays over the page until it is
    /// dismissed - and goes with the dismissal.
    #[test]
    fn a_failed_launch_draws_the_failure_until_it_is_dismissed() {
        let mut state = LaunchState::Failed {
            title: "Celeste".to_owned(),
            error: "bridge unavailable".to_owned(),
        };

        assert_eq!(
            state.overlay(),
            Some(Overlay::Failed {
                title: "Celeste",
                error: "bridge unavailable",
            })
        );
        state.update(Event::Dismiss);
        assert!(state.overlay().is_none());
    }
}
