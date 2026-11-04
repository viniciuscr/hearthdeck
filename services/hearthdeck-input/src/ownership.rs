use crate::mapping::Control;
use hearthdeck_protocol::input::Action;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Owner {
    #[default]
    Observe,
    Frontend,
    TextEntry,
    Desktop,
    Overlay,
    Keyboard,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Signal {
    Action(Action),
    Keyboard,
}

#[derive(Default)]
struct Pad {
    held: HashSet<Control>,
    armed: bool,
    guide_used: bool,
    select_used: bool,
    start_used: bool,
    repeat: Option<(Action, u64)>,
}

#[derive(Default)]
pub(crate) struct Ownership {
    pub owner: Owner,
    pads: HashMap<u64, Pad>,
}

impl Ownership {
    pub fn set_owner(&mut self, owner: Owner) {
        if self.owner != owner {
            self.owner = owner;
            self.suspend();
        }
    }

    pub fn suspend(&mut self) {
        for pad in self.pads.values_mut() {
            pad.armed = false;
            pad.repeat = None;
            pad.guide_used = true;
            pad.select_used = true;
            pad.start_used = true;
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.pads.remove(&id);
    }

    pub fn armed(&self, id: u64) -> bool {
        self.pads.get(&id).is_some_and(|pad| pad.armed)
    }

    pub fn guide_held(&self, id: u64) -> bool {
        self.pads
            .get(&id)
            .is_some_and(|pad| pad.held.contains(&Control::Guide))
    }

    pub fn button(
        &mut self,
        id: u64,
        control: Control,
        pressed: bool,
        now: u64,
        keyboard_enabled: bool,
    ) -> Vec<Signal> {
        let pad = self.pads.entry(id).or_default();
        let changed = if pressed {
            pad.held.insert(control)
        } else {
            pad.held.remove(&control)
        };
        if !changed || !pad.armed {
            return Vec::new();
        }
        if control == Control::Guide {
            if pressed {
                pad.guide_used = false;
                pad.repeat = None;
            } else if !pad.guide_used {
                return vec![Signal::Action(Action::ToggleOverlay)];
            }
            return Vec::new();
        }
        if pressed && control == Control::West && pad.held.contains(&Control::Guide) {
            if !pad.guide_used
                && keyboard_enabled
                && matches!(self.owner, Owner::Observe | Owner::Desktop)
            {
                pad.guide_used = true;
                return vec![Signal::Keyboard];
            }
            return Vec::new();
        }
        if pad.held.contains(&Control::Guide) {
            return Vec::new();
        }
        if self.owner == Owner::TextEntry {
            return if control == Control::East && pressed {
                vec![Signal::Action(Action::Back)]
            } else {
                Vec::new()
            };
        }
        if !matches!(self.owner, Owner::Frontend | Owner::Overlay) {
            return Vec::new();
        }
        if self.owner == Owner::Frontend {
            if pressed && control == Control::Start {
                pad.start_used = false;
            }
            if pressed && control == Control::Select {
                pad.select_used = false;
            }
            if pressed
                && matches!(control, Control::Start | Control::Select)
                && pad.held.contains(&Control::Start)
                && pad.held.contains(&Control::Select)
                && !pad.start_used
                && !pad.select_used
            {
                pad.start_used = true;
                pad.select_used = true;
                return vec![Signal::Action(Action::Keyboard)];
            }
            if control == Control::Select && !pressed && !pad.select_used {
                return vec![Signal::Action(Action::FilterPanel)];
            }
        }
        let Some(action) = action(control) else {
            return Vec::new();
        };
        if !pressed {
            if pad.repeat.is_some_and(|(active, _)| active == action) {
                pad.repeat = None;
            }
            return Vec::new();
        }
        if matches!(
            action,
            Action::MoveUp | Action::MoveDown | Action::MoveLeft | Action::MoveRight
        ) {
            pad.repeat = Some((action, now + 400));
        }
        vec![Signal::Action(action)]
    }

    pub fn tick(&mut self, now: u64) -> Vec<Signal> {
        let mut events = Vec::new();
        for pad in self.pads.values_mut() {
            if !pad.armed && pad.held.is_empty() {
                pad.armed = true;
            }
            if pad.armed
                && let Some((action, at)) = &mut pad.repeat
                && now >= *at
            {
                events.push(Signal::Action(*action));
                *at = now + 180;
            }
        }
        events
    }
}

fn action(control: Control) -> Option<Action> {
    Some(match control {
        Control::South => Action::Confirm,
        Control::East => Action::Back,
        Control::West => Action::ContextMenu,
        Control::North => Action::Search,
        Control::LeftBumper => Action::PrevGroup,
        Control::RightBumper => Action::NextGroup,
        Control::LeftTrigger => Action::PrevTab,
        Control::RightTrigger => Action::NextTab,
        Control::DpadUp | Control::StickUp => Action::MoveUp,
        Control::DpadDown | Control::StickDown => Action::MoveDown,
        Control::DpadLeft | Control::StickLeft => Action::MoveLeft,
        Control::DpadRight | Control::StickRight => Action::MoveRight,
        Control::Other(_)
        | Control::Start
        | Control::Select
        | Control::Guide
        | Control::PointerLeft
        | Control::PointerRight
        | Control::PointerUp
        | Control::PointerDown => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(owner: Owner) -> Ownership {
        let mut state = Ownership::default();
        state.set_owner(owner);
        state.button(1, Control::South, false, 0, true);
        state.button(2, Control::South, false, 0, true);
        state.tick(0);
        state
    }

    #[test]
    fn guide_release_and_same_device_chord_are_exclusive() {
        let mut state = ready(Owner::Observe);
        assert!(state.button(1, Control::Guide, true, 0, true).is_empty());
        assert!(state.button(2, Control::West, true, 0, true).is_empty());
        assert_eq!(
            state.button(1, Control::West, true, 0, true),
            vec![Signal::Keyboard]
        );
        state.button(1, Control::West, false, 0, true);
        assert!(state.button(1, Control::West, true, 0, true).is_empty());
        assert!(state.button(1, Control::Guide, false, 0, true).is_empty());
        state.button(2, Control::Guide, true, 0, true);
        assert_eq!(
            state.button(2, Control::Guide, false, 0, true),
            vec![Signal::Action(Action::ToggleOverlay)]
        );
    }

    #[test]
    fn select_waits_for_release_and_both_chord_orders_work() {
        for (first, second) in [
            (Control::Start, Control::Select),
            (Control::Select, Control::Start),
        ] {
            let mut state = ready(Owner::Frontend);
            assert!(state.button(1, first, true, 0, true).is_empty());
            assert_eq!(
                state.button(1, second, true, 0, true),
                vec![Signal::Action(Action::Keyboard)]
            );
            assert!(state.button(1, Control::Select, false, 0, true).is_empty());
        }
        let mut state = ready(Owner::Frontend);
        state.button(1, Control::Select, true, 0, true);
        state.button(2, Control::Start, true, 0, true);
        assert_eq!(
            state.button(1, Control::Select, false, 0, true),
            vec![Signal::Action(Action::FilterPanel)]
        );
    }

    #[test]
    fn ownership_change_requires_each_device_to_be_neutral_and_clears_repeat() {
        let mut state = ready(Owner::Frontend);
        assert_eq!(
            state.button(1, Control::StickUp, true, 0, false),
            vec![Signal::Action(Action::MoveUp)]
        );
        state.set_owner(Owner::Overlay);
        assert!(state.tick(1000).is_empty());
        assert!(!state.armed(1));
        assert!(state.armed(2));
        state.button(1, Control::StickUp, false, 0, false);
        state.tick(1001);
        assert!(state.armed(1));
        state.remove(1);
        assert!(state.armed(2));
    }

    #[test]
    fn repeat_stops_on_release_and_disconnect() {
        let mut state = ready(Owner::Frontend);
        state.button(1, Control::DpadDown, true, 0, false);
        assert!(state.tick(399).is_empty());
        assert_eq!(state.tick(400), vec![Signal::Action(Action::MoveDown)]);
        state.remove(1);
        assert!(state.tick(1000).is_empty());
    }

    #[test]
    fn guide_stops_navigation_repeat_until_a_new_direction_press() {
        let mut state = ready(Owner::Frontend);
        state.button(1, Control::DpadDown, true, 0, false);
        state.button(1, Control::Guide, true, 1, false);
        assert!(state.tick(1000).is_empty());
        state.button(1, Control::Guide, false, 1001, false);
        assert!(state.tick(2000).is_empty());
    }

    #[test]
    fn unknown_held_buttons_prevent_arming_the_next_owner() {
        let mut state = ready(Owner::Frontend);
        state.button(1, Control::Other(317), true, 0, false);
        state.set_owner(Owner::Overlay);
        state.tick(1);
        assert!(!state.armed(1));
        state.button(1, Control::Other(317), false, 2, false);
        state.tick(3);
        assert!(state.armed(1));
    }
}
