use std::{
    collections::HashMap,
    io::ErrorKind,
    path::PathBuf,
    sync::{OnceLock, mpsc},
    thread,
    time::Duration,
};

use cosmic::iced::{Subscription, futures::SinkExt, stream};
use evdev::{AbsoluteAxisCode, Device, EventSummary, KeyCode};
use tokio::sync::mpsc as tokio_mpsc;
use tracing::{debug, warn};

static COMMANDS: OnceLock<mpsc::Sender<InputCommand>> = OnceLock::new();

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

pub fn subscription() -> Subscription<GamepadAction> {
    Subscription::run(messages)
}

pub fn set_visible(visible: bool) {
    let Some(sender) = COMMANDS.get() else {
        return;
    };
    if let Err(error) = sender.send(InputCommand::SetVisible(visible)) {
        warn!(%error, "gamepad input worker is unavailable");
    }
}

fn messages() -> impl cosmic::iced::futures::Stream<Item = GamepadAction> {
    stream::channel(16, |mut output| async move {
        let (events, mut receiver) = tokio_mpsc::unbounded_channel();
        let (commands, command_receiver) = mpsc::channel();
        let _ = COMMANDS.set(commands);
        thread::Builder::new()
            .name("hearthdeck-gamepad".to_owned())
            .spawn(move || InputWorker::new(events, command_receiver).run())
            .expect("could not start the gamepad input worker");
        while let Some(event) = receiver.recv().await {
            if output.send(event).await.is_err() {
                return;
            }
        }
    })
}

struct InputWorker {
    devices: HashMap<PathBuf, Device>,
    event_sender: tokio_mpsc::UnboundedSender<GamepadAction>,
    command_receiver: mpsc::Receiver<InputCommand>,
    last_guide_device: Option<PathBuf>,
    grabbed_device: Option<PathBuf>,
    visible: bool,
}

impl InputWorker {
    fn new(
        event_sender: tokio_mpsc::UnboundedSender<GamepadAction>,
        command_receiver: mpsc::Receiver<InputCommand>,
    ) -> Self {
        Self {
            devices: HashMap::new(),
            event_sender,
            command_receiver,
            last_guide_device: None,
            grabbed_device: None,
            visible: false,
        }
    }

    fn run(mut self) {
        loop {
            self.refresh_devices();
            for command in self.command_receiver.try_iter() {
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
        .is_some_and(|keys| keys.contains(KeyCode::BTN_GAMEPAD) && keys.contains(KeyCode::BTN_MODE))
}

fn action_for(event: EventSummary, visible: bool) -> Option<GamepadAction> {
    match event {
        EventSummary::Key(_, KeyCode::BTN_MODE, 1) => Some(GamepadAction::Toggle),
        EventSummary::Key(_, KeyCode::BTN_EAST, 1) if visible => Some(GamepadAction::Hide),
        EventSummary::Key(_, KeyCode::BTN_SOUTH, 1) if visible => Some(GamepadAction::Activate),
        EventSummary::Key(_, KeyCode::BTN_DPAD_LEFT | KeyCode::BTN_DPAD_RIGHT, 1) if visible => {
            Some(GamepadAction::ToggleSelection)
        }
        EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_HAT0X, value)
            if visible && value != 0 =>
        {
            Some(GamepadAction::ToggleSelection)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use evdev::{EventSummary, KeyCode, KeyEvent};

    use super::{GamepadAction, action_for};

    #[test]
    fn guide_toggles_from_any_state() {
        assert!(matches!(
            action_for(EventSummary::Key(KeyEvent, KeyCode::BTN_MODE, 1), false),
            Some(GamepadAction::Toggle)
        ));
    }

    #[test]
    fn overlay_controls_are_ignored_while_hidden() {
        assert!(action_for(EventSummary::Key(KeyEvent, KeyCode::BTN_SOUTH, 1), false).is_none());
        assert!(matches!(
            action_for(EventSummary::Key(KeyEvent, KeyCode::BTN_SOUTH, 1), true),
            Some(GamepadAction::Activate)
        ));
    }
}
