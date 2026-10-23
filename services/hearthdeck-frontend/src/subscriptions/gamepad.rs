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
use gilrs::{Axis, Button, Event, EventType, Gamepad, GamepadId, GilrsBuilder};
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
    /// Open or close the filter drawer: the one surface over the grid that the
    /// pad reaches on its own, since it cannot be walked to.
    FilterPanel,
    /// Start+Select: summon the on-screen keyboard by putting the caret in a
    /// text field. The shell owns this chord rather than the keyboard's own
    /// process so it can clear the pad's other bindings for as long as the
    /// keyboard is up.
    Keyboard,
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
    armed: bool,
    dir: Option<Direction>,
    repeat_at: Option<Instant>,
    last_move_at: Option<Instant>,
}

impl NavState {
    fn poll(&mut self, gamepad: &Gamepad, tx: &UnboundedSender<GamepadEvent>) {
        let dir = direction_after_neutral(
            &mut self.armed,
            current_direction(gamepad, self.dir.is_some()),
        );
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

fn direction_after_neutral(armed: &mut bool, direction: Option<Direction>) -> Option<Direction> {
    if !*armed {
        *armed = direction.is_none();
        return None;
    }
    direction
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
        // The one press that waits: `Chords` opens the drawer on Select's
        // release so that Start+Select can be the keyboard instead.
        Button::Select => Some(GamepadEvent::FilterPanel),
        _ => None,
    }
}

/// Select's two jobs, told apart by Start.
///
/// Select opens the filter drawer, and Start+Select summons the keyboard. Both
/// keys have to keep working, so the filter cannot fire on the press: when
/// Start was pressed first the drawer would open under the keyboard, and when
/// Select was pressed first it would open before Start even arrived. The
/// filter therefore waits for Select's release, and a chord in the meantime
/// cancels it. Handling the release costs a tap's worth of latency and buys a
/// chord that works in either press order.
#[derive(Default)]
struct Chords {
    select_held: bool,
    /// A chord already fired for this pair of presses, so releasing Select must
    /// not also open the drawer and holding both must not fire twice.
    chord_fired: bool,
}

impl Chords {
    /// One button went down. `start_held` is the pad's own Start state at that
    /// moment, which is what makes Start-first and Select-first fold into the
    /// same state machine.
    fn pressed(&mut self, button: Button, start_held: bool) -> Option<GamepadEvent> {
        match button {
            Button::Select => {
                self.select_held = true;
                if start_held {
                    self.chord_fired = true;
                    return Some(GamepadEvent::Keyboard);
                }
                None
            }
            Button::Start => {
                if self.select_held && !self.chord_fired {
                    self.chord_fired = true;
                    return Some(GamepadEvent::Keyboard);
                }
                None
            }
            _ => map_button(button),
        }
    }

    /// One button came back up: Select's deferred action, if the chord did not
    /// take it.
    fn released(&mut self, button: Button) -> Option<GamepadEvent> {
        if button != Button::Select {
            return None;
        }
        self.select_held = false;
        let open_filter = !self.chord_fired;
        self.chord_fired = false;
        open_filter.then_some(GamepadEvent::FilterPanel)
    }
}

fn gilrs_loop(tx: UnboundedSender<GamepadEvent>) {
    // Force feedback is disabled on purpose: Hearthdeck never plays rumble
    // effects, and gilrs enables its force-feedback server by default. That
    // server re-asserts a (zero-strength) effect on every gamepad on every
    // tick, so once a pad's device node is unlinked (e.g. the input service
    // swapping its virtual device when a game closes) it logs
    // "Failed to modify effect ... ENODEV" forever instead of stopping.
    let mut gilrs = match GilrsBuilder::new().with_force_feedback(false).build() {
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
    // affect navigation coming from another pad. The chords are stateful for
    // the same reason - Select waits for its release - so each pad keeps its
    // own.
    let mut nav_states: HashMap<GamepadId, NavState> = HashMap::new();
    let mut chords: HashMap<GamepadId, Chords> = HashMap::new();

    while !tx.is_closed() {
        while let Some(Event { id, event, .. }) = gilrs.next_event() {
            let gamepad = gilrs.gamepad(id);
            if !gamepad.is_connected() {
                continue;
            }
            let chord = chords.entry(id).or_default();
            let event = match event {
                EventType::ButtonPressed(button, _) => {
                    chord.pressed(button, gamepad.is_pressed(Button::Start))
                }
                EventType::ButtonReleased(button, _) => chord.released(button),
                _ => None,
            };
            if let Some(event) = event
                && tx.unbounded_send(event).is_err()
            {
                return;
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

#[cfg(test)]
mod tests {
    use super::{Button, Chords, Direction, GamepadEvent, direction_after_neutral, map_button};

    /// The pad's vocabulary: which button means what. This is the whole contract
    /// between the controller and the shell - a change here changes how the app
    /// feels - so it is pinned rather than left to the `match` to drift.
    #[test]
    fn the_buttons_a_pad_offers_are_the_actions_the_shell_knows() {
        let cases: &[(Button, GamepadEvent)] = &[
            (Button::South, GamepadEvent::Confirm),
            (Button::East, GamepadEvent::Back),
            (Button::North, GamepadEvent::Search),
            (Button::West, GamepadEvent::ContextMenu),
            (Button::LeftTrigger, GamepadEvent::PrevGroup),
            (Button::RightTrigger, GamepadEvent::NextGroup),
            (Button::LeftTrigger2, GamepadEvent::PrevTab),
            (Button::RightTrigger2, GamepadEvent::NextTab),
            (Button::Select, GamepadEvent::FilterPanel),
        ];
        for (button, expected) in cases {
            assert_eq!(map_button(*button), Some(*expected), "button: {button:?}");
        }

        // The D-pad and the sticks are read as directions on every poll rather
        // than as buttons, and what nothing is bound to stays unbound.
        for button in [Button::DPadUp, Button::Start, Button::Mode, Button::C] {
            assert_eq!(map_button(button), None, "button: {button:?}");
        }
    }

    #[test]
    fn select_alone_opens_the_filter_when_it_is_released() {
        let mut chords = Chords::default();

        // Nothing on the press: the drawer waits to see whether the chord is
        // coming, so a Start+Select never opens it on the way past.
        assert_eq!(chords.pressed(Button::Select, false), None);
        assert_eq!(
            chords.released(Button::Select),
            Some(GamepadEvent::FilterPanel)
        );
    }

    /// Start+Select is the keyboard, in either press order, and neither press
    /// may also open the filter drawer.
    #[test]
    fn the_keyboard_chord_cancels_the_filter_in_both_press_orders() {
        // Start first: the Select press is already part of the chord.
        let mut chords = Chords::default();
        assert_eq!(chords.pressed(Button::Start, true), None);
        assert_eq!(
            chords.pressed(Button::Select, true),
            Some(GamepadEvent::Keyboard)
        );
        assert_eq!(chords.released(Button::Select), None);
        assert_eq!(chords.released(Button::Start), None);

        // Select first: the chord forms on Start and cancels Select's release.
        let mut chords = Chords::default();
        assert_eq!(chords.pressed(Button::Select, false), None);
        assert_eq!(
            chords.pressed(Button::Start, true),
            Some(GamepadEvent::Keyboard)
        );
        assert_eq!(chords.released(Button::Select), None);
        assert_eq!(chords.released(Button::Start), None);
    }

    #[test]
    fn holding_the_chord_fires_it_once() {
        let mut chords = Chords::default();
        assert_eq!(chords.pressed(Button::Select, false), None);
        assert_eq!(
            chords.pressed(Button::Start, true),
            Some(GamepadEvent::Keyboard)
        );

        // A second Start press while Select is still down is still the same
        // chord, not a second one - and it must not re-arm the filter either.
        assert_eq!(chords.released(Button::Start), None);
        assert_eq!(chords.pressed(Button::Start, true), None);
        assert_eq!(chords.released(Button::Select), None);
    }

    #[test]
    fn a_new_tap_of_select_opens_the_filter_after_a_chord() {
        let mut chords = Chords::default();
        assert_eq!(chords.pressed(Button::Select, false), None);
        assert_eq!(
            chords.pressed(Button::Start, true),
            Some(GamepadEvent::Keyboard)
        );
        assert_eq!(chords.released(Button::Select), None);
        assert_eq!(chords.released(Button::Start), None);

        assert_eq!(chords.pressed(Button::Select, false), None);
        assert_eq!(
            chords.released(Button::Select),
            Some(GamepadEvent::FilterPanel)
        );
    }

    #[test]
    fn resumed_subscription_waits_for_neutral_input() {
        let mut armed = false;

        assert_eq!(
            direction_after_neutral(&mut armed, Some(Direction::Down)),
            None
        );
        assert!(!armed);
        assert_eq!(direction_after_neutral(&mut armed, None), None);
        assert!(armed);
        assert_eq!(
            direction_after_neutral(&mut armed, Some(Direction::Down)),
            Some(Direction::Down)
        );
    }
}
