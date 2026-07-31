use std::{
    collections::HashMap,
    io::ErrorKind,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use evdev::{AbsoluteAxisCode, Device, EventSummary, KeyCode};
use tracing::{debug, warn};

const DEVICE_RESCAN_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
pub enum InputCommand {
    SetVisible(bool),
}

#[derive(Clone, Copy, Debug)]
pub enum GamepadAction {
    Toggle,
    Hide,
    ToggleSelection,
    Activate,
}

pub struct InputHandle {
    commands: mpsc::Sender<InputCommand>,
    pub actions: mpsc::Receiver<GamepadAction>,
}

impl InputHandle {
    pub fn set_visible(&self, visible: bool) {
        if let Err(error) = self.commands.send(InputCommand::SetVisible(visible)) {
            warn!(%error, "gamepad input worker is unavailable");
        }
    }
}

pub fn start() -> InputHandle {
    let (actions, action_receiver) = mpsc::channel();
    let (commands, command_receiver) = mpsc::channel();
    thread::Builder::new()
        .name("hearthdeck-gamepad".to_owned())
        .spawn(move || InputWorker::new(actions, command_receiver).run())
        .expect("could not start the gamepad input worker");
    InputHandle {
        commands,
        actions: action_receiver,
    }
}

struct InputWorker {
    devices: HashMap<PathBuf, Device>,
    event_sender: mpsc::Sender<GamepadAction>,
    command_receiver: mpsc::Receiver<InputCommand>,
    last_guide_device: Option<PathBuf>,
    grabbed_device: Option<PathBuf>,
    visible: bool,
    last_rescan: Instant,
}

impl InputWorker {
    fn new(
        event_sender: mpsc::Sender<GamepadAction>,
        command_receiver: mpsc::Receiver<InputCommand>,
    ) -> Self {
        Self {
            devices: HashMap::new(),
            event_sender,
            command_receiver,
            last_guide_device: None,
            grabbed_device: None,
            visible: false,
            last_rescan: Instant::now() - DEVICE_RESCAN_INTERVAL,
        }
    }

    fn run(mut self) {
        loop {
            if self.last_rescan.elapsed() >= DEVICE_RESCAN_INTERVAL {
                self.refresh_devices();
                self.last_rescan = Instant::now();
            }
            let commands = self.command_receiver.try_iter().collect::<Vec<_>>();
            for command in commands {
                self.set_visible(matches!(command, InputCommand::SetVisible(true)));
            }
            let paths = self.devices.keys().cloned().collect::<Vec<_>>();
            for path in paths {
                self.read_events(&path);
            }
            thread::sleep(Duration::from_millis(8));
        }
    }

    fn refresh_devices(&mut self) {
        let discovered = evdev::enumerate()
            .filter_map(|(path, device)| is_gamepad(&device).then_some((path, device)))
            .collect::<HashMap<_, _>>();
        self.devices.retain(|path, _| discovered.contains_key(path));
        for (path, device) in discovered {
            self.devices.entry(path).or_insert_with(|| {
                let _ = device.set_nonblocking(true);
                debug!(device = ?device.name(), "gamepad input device attached");
                device
            });
        }
    }

    fn read_events(&mut self, path: &PathBuf) {
        let mut actions = Vec::new();
        let disconnected = {
            let Some(device) = self.devices.get_mut(path) else {
                return;
            };
            match device.fetch_events() {
                Ok(events) => {
                    for event in events {
                        if let Some(action) = action_for(event.destructure(), self.visible) {
                            actions.push(action);
                        }
                    }
                    false
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return,
                Err(error) => {
                    debug!(device = %path.display(), %error, "gamepad input device disconnected");
                    true
                }
            }
        };
        if disconnected {
            self.devices.remove(path);
            return;
        }
        for action in actions {
            if matches!(action, GamepadAction::Toggle) {
                self.last_guide_device = Some(path.clone());
            }
            let _ = self.event_sender.send(action);
        }
    }

    fn set_visible(&mut self, visible: bool) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible {
            let Some(path) = self.last_guide_device.clone() else {
                return;
            };
            let Some(device) = self.devices.get_mut(&path) else {
                return;
            };
            match device.grab() {
                Ok(()) => self.grabbed_device = Some(path),
                Err(error) => warn!(%error, "could not grab the active gamepad for the overlay"),
            }
        } else if let Some(path) = self.grabbed_device.take()
            && let Some(device) = self.devices.get_mut(&path)
            && let Err(error) = device.ungrab()
        {
            warn!(%error, "could not release the gamepad after closing the overlay");
        }
    }
}

fn is_gamepad(device: &Device) -> bool {
    device
        .supported_keys()
        .is_some_and(|keys| keys.contains(KeyCode::BTN_SOUTH) && keys.contains(KeyCode::BTN_MODE))
}

fn action_for(event: EventSummary, visible: bool) -> Option<GamepadAction> {
    match event {
        EventSummary::Key(_, key, value) => action_for_key(key, value, visible),
        EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_HAT0X, value)
            if visible && value != 0 =>
        {
            Some(GamepadAction::ToggleSelection)
        }
        _ => None,
    }
}

fn action_for_key(key: KeyCode, value: i32, visible: bool) -> Option<GamepadAction> {
    match (key, value, visible) {
        (KeyCode::BTN_MODE, 1, _) => Some(GamepadAction::Toggle),
        (KeyCode::BTN_EAST, 1, true) => Some(GamepadAction::Hide),
        (KeyCode::BTN_SOUTH, 1, true) => Some(GamepadAction::Activate),
        (KeyCode::BTN_DPAD_LEFT | KeyCode::BTN_DPAD_RIGHT, 1, true) => {
            Some(GamepadAction::ToggleSelection)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use evdev::KeyCode;

    use super::{GamepadAction, action_for_key};

    #[test]
    fn guide_toggles_from_any_state() {
        assert!(matches!(
            action_for_key(KeyCode::BTN_MODE, 1, false),
            Some(GamepadAction::Toggle)
        ));
    }

    #[test]
    fn overlay_controls_are_ignored_while_hidden() {
        assert!(action_for_key(KeyCode::BTN_SOUTH, 1, false).is_none());
        assert!(matches!(
            action_for_key(KeyCode::BTN_SOUTH, 1, true),
            Some(GamepadAction::Activate)
        ));
        assert!(matches!(
            action_for_key(KeyCode::BTN_DPAD_LEFT, 1, true),
            Some(GamepadAction::ToggleSelection)
        ));
    }
}
