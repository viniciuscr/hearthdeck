use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    South,
    East,
    West,
    North,
    LeftBumper,
    RightBumper,
    Start,
    Select,
    DpadLeft,
    DpadRight,
    DpadUp,
    DpadDown,
    StickLeft,
    StickRight,
    StickUp,
    StickDown,
    LeftTrigger,
    RightTrigger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutputKey {
    Enter,
    Escape,
    Space,
    Tab,
    PageUp,
    PageDown,
    Left,
    Right,
    Up,
    Down,
    MouseLeft,
    MouseRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputEvent {
    Key { key: OutputKey, pressed: bool },
    MouseMove { x: i32, y: i32 },
}

#[derive(Default)]
pub struct Mapper {
    active: bool,
    sources: HashMap<(u64, Control), OutputKey>,
    output_counts: HashMap<OutputKey, usize>,
    right_sticks: HashMap<u64, (i32, i32)>,
}

impl Mapper {
    pub fn set_active(&mut self, active: bool) -> Vec<OutputEvent> {
        if self.active == active {
            return Vec::new();
        }
        self.active = active;
        if active {
            return Vec::new();
        }

        self.sources.clear();
        self.right_sticks.clear();
        self.output_counts
            .drain()
            .map(|(key, _)| OutputEvent::Key {
                key,
                pressed: false,
            })
            .collect()
    }

    pub fn button(&mut self, device: u64, control: Control, pressed: bool) -> Vec<OutputEvent> {
        if !self.active {
            return Vec::new();
        }
        let Some(key) = output_for(control) else {
            return Vec::new();
        };
        self.set_source((device, control), key, pressed)
    }

    pub fn directional_axis(
        &mut self,
        device: u64,
        negative: Control,
        positive: Control,
        value: i32,
    ) -> Vec<OutputEvent> {
        if !self.active {
            return Vec::new();
        }
        let mut events = self.button(device, negative, value < -DEADZONE);
        events.extend(self.button(device, positive, value > DEADZONE));
        events
    }

    pub fn right_stick_axis(&mut self, device: u64, horizontal: bool, value: i32) {
        if !self.active {
            return;
        }
        let position = self.right_sticks.entry(device).or_default();
        if horizontal {
            position.0 = value;
        } else {
            position.1 = value;
        }
    }

    pub fn mouse_tick(&self) -> Option<OutputEvent> {
        if !self.active {
            return None;
        }
        let (x, y) = self
            .right_sticks
            .values()
            .fold((0, 0), |(sum_x, sum_y), (x, y)| {
                (sum_x + mouse_delta(*x), sum_y + mouse_delta(*y))
            });
        (x != 0 || y != 0).then_some(OutputEvent::MouseMove { x, y })
    }

    pub fn remove_device(&mut self, device: u64) -> Vec<OutputEvent> {
        self.right_sticks.remove(&device);
        let sources = self
            .sources
            .keys()
            .filter(|(source_device, _)| *source_device == device)
            .copied()
            .collect::<Vec<_>>();
        sources
            .into_iter()
            .flat_map(|source| self.release_source(source))
            .collect()
    }

    fn set_source(
        &mut self,
        source: (u64, Control),
        key: OutputKey,
        pressed: bool,
    ) -> Vec<OutputEvent> {
        if pressed {
            if self.sources.insert(source, key).is_some() {
                return Vec::new();
            }
            let count = self.output_counts.entry(key).or_default();
            *count += 1;
            if *count == 1 {
                return vec![OutputEvent::Key { key, pressed: true }];
            }
            Vec::new()
        } else {
            self.release_source(source)
        }
    }

    fn release_source(&mut self, source: (u64, Control)) -> Vec<OutputEvent> {
        let Some(key) = self.sources.remove(&source) else {
            return Vec::new();
        };
        let Some(count) = self.output_counts.get_mut(&key) else {
            return Vec::new();
        };
        *count -= 1;
        if *count == 0 {
            self.output_counts.remove(&key);
            return vec![OutputEvent::Key {
                key,
                pressed: false,
            }];
        }
        Vec::new()
    }
}

const DEADZONE: i32 = 350;

fn output_for(control: Control) -> Option<OutputKey> {
    Some(match control {
        Control::South | Control::Start => OutputKey::Enter,
        Control::East => OutputKey::Escape,
        Control::West => OutputKey::Space,
        Control::North | Control::Select => OutputKey::Tab,
        Control::LeftBumper => OutputKey::PageUp,
        Control::RightBumper => OutputKey::PageDown,
        Control::DpadLeft | Control::StickLeft => OutputKey::Left,
        Control::DpadRight | Control::StickRight => OutputKey::Right,
        Control::DpadUp | Control::StickUp => OutputKey::Up,
        Control::DpadDown | Control::StickDown => OutputKey::Down,
        Control::LeftTrigger => OutputKey::MouseRight,
        Control::RightTrigger => OutputKey::MouseLeft,
    })
}

fn mouse_delta(value: i32) -> i32 {
    if value.abs() <= DEADZONE {
        0
    } else {
        value * 14 / 1000
    }
}

#[cfg(test)]
mod tests {
    use super::{Control, Mapper, OutputEvent, OutputKey};

    fn key(key: OutputKey, pressed: bool) -> OutputEvent {
        OutputEvent::Key { key, pressed }
    }

    #[test]
    fn face_buttons_map_to_common_keyboard_controls() {
        let mut mapper = Mapper::default();
        mapper.set_active(true);

        assert_eq!(
            mapper.button(1, Control::South, true),
            vec![key(OutputKey::Enter, true)]
        );
        assert_eq!(
            mapper.button(1, Control::South, false),
            vec![key(OutputKey::Enter, false)]
        );
        assert_eq!(
            mapper.button(1, Control::East, true),
            vec![key(OutputKey::Escape, true)]
        );
    }

    #[test]
    fn shared_outputs_remain_pressed_until_every_source_releases() {
        let mut mapper = Mapper::default();
        mapper.set_active(true);

        assert_eq!(
            mapper.button(1, Control::South, true),
            vec![key(OutputKey::Enter, true)]
        );
        assert!(mapper.button(1, Control::Start, true).is_empty());
        assert!(mapper.button(1, Control::South, false).is_empty());
        assert_eq!(
            mapper.button(1, Control::Start, false),
            vec![key(OutputKey::Enter, false)]
        );
    }

    #[test]
    fn dpad_and_stick_share_directional_keys_safely() {
        let mut mapper = Mapper::default();
        mapper.set_active(true);

        assert_eq!(
            mapper.directional_axis(1, Control::DpadLeft, Control::DpadRight, -1000),
            vec![key(OutputKey::Left, true)]
        );
        assert!(
            mapper
                .directional_axis(1, Control::StickLeft, Control::StickRight, -1000)
                .is_empty()
        );
        assert!(
            mapper
                .directional_axis(1, Control::DpadLeft, Control::DpadRight, 0)
                .is_empty()
        );
        assert_eq!(
            mapper.directional_axis(1, Control::StickLeft, Control::StickRight, 0),
            vec![key(OutputKey::Left, false)]
        );
    }

    #[test]
    fn triggers_map_to_mouse_buttons() {
        let mut mapper = Mapper::default();
        mapper.set_active(true);

        assert_eq!(
            mapper.button(1, Control::RightTrigger, true),
            vec![key(OutputKey::MouseLeft, true)]
        );
        assert_eq!(
            mapper.button(1, Control::LeftTrigger, true),
            vec![key(OutputKey::MouseRight, true)]
        );
    }

    #[test]
    fn right_stick_moves_mouse_outside_deadzone() {
        let mut mapper = Mapper::default();
        mapper.set_active(true);
        mapper.right_stick_axis(1, true, 1000);
        mapper.right_stick_axis(1, false, -500);

        assert_eq!(
            mapper.mouse_tick(),
            Some(OutputEvent::MouseMove { x: 14, y: -7 })
        );
    }

    #[test]
    fn disabling_mapping_releases_all_outputs() {
        let mut mapper = Mapper::default();
        mapper.set_active(true);
        mapper.button(1, Control::South, true);
        mapper.button(1, Control::RightTrigger, true);

        let events = mapper.set_active(false);
        assert_eq!(events.len(), 2);
        assert!(events.contains(&key(OutputKey::Enter, false)));
        assert!(events.contains(&key(OutputKey::MouseLeft, false)));
        assert!(mapper.mouse_tick().is_none());
    }

    #[test]
    fn removing_a_device_releases_only_its_sources() {
        let mut mapper = Mapper::default();
        mapper.set_active(true);
        mapper.button(1, Control::South, true);
        mapper.button(2, Control::South, true);

        assert!(mapper.remove_device(1).is_empty());
        assert_eq!(mapper.remove_device(2), vec![key(OutputKey::Enter, false)]);
    }
}
