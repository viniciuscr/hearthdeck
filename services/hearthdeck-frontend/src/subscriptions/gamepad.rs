//! Gamepad input support via `gilrs`.
//!
//! A background thread owns the `gilrs` instance (it is not `Send`) and
//! reports semantic navigation events to the application through an
//! unbounded channel. If no gamepad can be opened (e.g. missing udev
//! permissions), the thread exits silently and the subscription simply
//! never produces any events.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use futures::channel::mpsc::{self, UnboundedSender};
use gilrs::{Axis, Button, Event, EventType, Gamepad, GamepadId, Gilrs};
use log::{info, warn};

use cosmic::iced::Subscription;

/// A semantic gamepad action, mapped to the launcher's UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GamepadEvent {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    Confirm,
    Back,
    Search,
    ContextMenu,
    PrevGroup,
    NextGroup,
    PrevTab,
    NextTab,
}

/// Subscribe to gamepad navigation events.
pub fn gamepad_events() -> Subscription<GamepadEvent> {
    Subscription::run(|| {
        let (tx, rx) = mpsc::unbounded::<GamepadEvent>();
        std::thread::spawn(move || gilrs_loop(tx));
        rx
    })
}

/// How often the gamepad state is polled.
const POLL_INTERVAL: Duration = Duration::from_millis(33);
/// Delay before a held direction starts repeating.
const INITIAL_REPEAT_DELAY: Duration = Duration::from_millis(400);
/// Interval between repeated moves while a direction is held.
const REPEAT_INTERVAL: Duration = Duration::from_millis(180);
/// Analog stick must be pushed past this before it counts as a directional
/// press.
const STICK_PRESS_THRESHOLD: f32 = 0.55;
/// Once engaged, the stick must drop below this before the direction is
/// released. Being lower than the press threshold adds hysteresis, so a
/// stick that jitters around the press threshold cannot re-trigger moves.
const STICK_RELEASE_THRESHOLD: f32 = 0.30;
/// Minimum time between two emitted moves. Guards against button/stick
/// bounce producing several moves from a single tap.
const MOVE_DEBOUNCE: Duration = Duration::from_millis(60);
/// Set to `true` if a controller reports up as a positive `LeftStickY`.
///
/// The SDL gamepad convention is up = negative; this controller's gilrs
/// mapping reports up as positive instead.
const STICK_Y_UP_POSITIVE: bool = true;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn into_event(self) -> GamepadEvent {
        match self {
            Direction::Up => GamepadEvent::MoveUp,
            Direction::Down => GamepadEvent::MoveDown,
            Direction::Left => GamepadEvent::MoveLeft,
            Direction::Right => GamepadEvent::MoveRight,
        }
    }
}

#[derive(Default)]
struct NavState {
    dir: Option<Direction>,
    repeat_at: Option<Instant>,
    last_move_at: Option<Instant>,
}

impl NavState {
    fn poll(&mut self, gamepad: &Gamepad, tx: &UnboundedSender<GamepadEvent>) {
        let dir = current_direction(gamepad, self.dir.is_some());
        match (self.dir, dir) {
            (Some(prev), Some(next)) if prev == next => {
                if let Some(repeat_at) = self.repeat_at
                    && Instant::now() >= repeat_at
                    && self.emit(tx, next.into_event())
                {
                    self.repeat_at = Some(Instant::now() + REPEAT_INTERVAL);
                }
            }
            (_, Some(next)) => {
                if self.emit(tx, next.into_event()) {
                    self.dir = Some(next);
                    self.repeat_at = Some(Instant::now() + INITIAL_REPEAT_DELAY);
                }
            }
            (_, None) => {
                self.dir = None;
                self.repeat_at = None;
            }
        }
    }

    /// Sends a move event, unless one was sent very recently (debounce).
    fn emit(&mut self, tx: &UnboundedSender<GamepadEvent>, ev: GamepadEvent) -> bool {
        if self
            .last_move_at
            .is_some_and(|t| Instant::now() - t < MOVE_DEBOUNCE)
        {
            return false;
        }
        let _ = tx.unbounded_send(ev);
        self.last_move_at = Some(Instant::now());
        true
    }
}

/// Determines which direction is currently pressed, from the D-pad or the
/// left analog stick. The analog stick uses hysteresis: while a direction is
/// already engaged (`engaged == true`) a lower release threshold applies, so
/// small jitters near the press threshold do not produce extra moves.
fn current_direction(gamepad: &Gamepad, engaged: bool) -> Option<Direction> {
    if gamepad.is_pressed(Button::DPadUp) {
        return Some(Direction::Up);
    }
    if gamepad.is_pressed(Button::DPadDown) {
        return Some(Direction::Down);
    }
    if gamepad.is_pressed(Button::DPadLeft) {
        return Some(Direction::Left);
    }
    if gamepad.is_pressed(Button::DPadRight) {
        return Some(Direction::Right);
    }

    let threshold = if engaged {
        STICK_RELEASE_THRESHOLD
    } else {
        STICK_PRESS_THRESHOLD
    };
    let x = gamepad.value(Axis::LeftStickX);
    let y = gamepad.value(Axis::LeftStickY);
    if x.abs() >= y.abs() {
        if x > threshold {
            Some(Direction::Right)
        } else if x < -threshold {
            Some(Direction::Left)
        } else {
            None
        }
    } else if y.abs() >= threshold {
        // SDL convention: up = negative Y (unless STICK_Y_UP_POSITIVE).
        if (y > 0.0) == STICK_Y_UP_POSITIVE {
            Some(Direction::Up)
        } else {
            Some(Direction::Down)
        }
    } else {
        None
    }
}

fn map_button(button: Button) -> Option<GamepadEvent> {
    match button {
        Button::South => Some(GamepadEvent::Confirm),
        Button::East => Some(GamepadEvent::Back),
        Button::North => Some(GamepadEvent::Search),
        Button::West => Some(GamepadEvent::ContextMenu),
        Button::LeftTrigger => Some(GamepadEvent::PrevGroup),
        Button::RightTrigger => Some(GamepadEvent::NextGroup),
        Button::LeftTrigger2 => Some(GamepadEvent::PrevTab),
        Button::RightTrigger2 => Some(GamepadEvent::NextTab),
        _ => None,
    }
}

fn gilrs_loop(tx: UnboundedSender<GamepadEvent>) {
    let mut gilrs = match Gilrs::new() {
        Ok(gilrs) => {
            info!("Gamepad support enabled");
            gilrs
        }
        Err(err) => {
            warn!("Gamepad support disabled: {err}");
            return;
        }
    };

    // Drain the initial batch of events (device discovery, etc.).
    while gilrs.next_event().is_some() {}

    // Per-gamepad navigation state: a stuck button on one pad must not
    // affect navigation coming from another pad.
    let mut nav_states: HashMap<GamepadId, NavState> = HashMap::new();

    loop {
        while let Some(Event { id, event, .. }) = gilrs.next_event() {
            let gamepad = gilrs.gamepad(id);
            if !gamepad.is_connected() {
                continue;
            }
            let EventType::ButtonPressed(button, _) = event else {
                continue;
            };
            if let Some(ev) = map_button(button) {
                let _ = tx.unbounded_send(ev);
            }
        }

        for (id, _) in gilrs.gamepads() {
            let gamepad = gilrs.gamepad(id);
            if !gamepad.is_connected() {
                continue;
            }
            nav_states.entry(id).or_default().poll(&gamepad, &tx);
        }

        gilrs.inc();
        std::thread::sleep(POLL_INTERVAL);
    }
}
