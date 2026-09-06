// Gamepad watcher for hearthdeck-overlay: the Guide/Mode button toggles the
// overlay, and (while it's open) D-pad up/down moves the menu selection and
// the South/A button activates it. While the overlay is visible, this also
// grabs the gamepad exclusively (see `subscription`'s own docs) so the app
// underneath stops receiving any controller input at all until the overlay
// closes.
//
// UNTESTED on real hardware. Assumptions this needs confirming on the actual
// target device:
//   1. That the controller's Guide/Home button is reported by the kernel as
//      `KeyCode::BTN_MODE` at all - some controllers/drivers surface it
//      under a different code, or don't expose it as a distinct evdev key.
//      Confirm with `evtest` on the target hardware.
//   2. That reading `/dev/input/event*` works for this session's user
//      without extra setup - on stock Arch, systemd-logind's `uaccess`
//      udev tagging normally grants the active seat's logged-in user
//      access automatically, but this repo has never relied on that before
//      now, so it hasn't been verified here.
//   3. D-pad reporting is genuinely ambiguous across controllers/drivers:
//      some report discrete `BTN_DPAD_UP`/`BTN_DPAD_DOWN` key events, older
//      ones report the D-pad as the `ABS_HAT0Y` hat axis instead (value -1
//      up, 1 down, 0 released) - this crate's own history includes a prior
//      bug from assuming the wrong one (see docs/kiosk-session.md). Both are
//      read here so either kind of controller can navigate the menu;
//      confirm on real hardware which (if either) actually fires, with
//      `evtest`.
use std::future::Future;
use std::time::Duration;

use cosmic::iced::Subscription;
use cosmic::iced::stream;
use evdev::{AbsoluteAxisCode, Device, EventSummary, KeyCode};
use futures_util::SinkExt;
use tokio::sync::mpsc;

/// A single interpreted gamepad input relevant to the overlay: opening/
/// closing it, and - while it's open - moving/activating the menu selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamepadEvent {
    ToggleOverlay,
    NavigateUp,
    NavigateDown,
    Activate,
}

const RESCAN_DELAY: Duration = Duration::from_secs(5);

#[derive(Default)]
struct WatchTasks(Vec<tokio::task::JoinHandle<()>>);

impl WatchTasks {
    fn spawn(&mut self, future: impl Future<Output = ()> + Send + 'static) {
        self.0.push(tokio::spawn(future));
    }
}

impl Drop for WatchTasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

/// Emits a `GamepadEvent` each time any currently-connected gamepad reports
/// one of the button/axis transitions the overlay cares about.
///
/// `visible` controls whether every currently-connected gamepad is opened
/// with an exclusive `EVIOCGRAB` (via `evdev`'s `Device::grab`) before being
/// watched: reading a device's `/dev/input/eventN` node does not, by itself,
/// stop any other process (the game/app underneath) from reading the exact
/// same events at the same time - both are just independent listeners on the
/// same device node. `EVIOCGRAB` is the actual kernel mechanism for
/// exclusive input: while one file descriptor holds the grab, the kernel
/// stops delivering that device's events to every other open descriptor -
/// this app's own un-grabbed reads included - until the grab is released
/// (which happens automatically when the grabbing descriptor is closed, no
/// explicit ungrab needed). This is why the caller must pass `visible`
/// (not just grab unconditionally at startup): grabbing is only meant to
/// apply while the overlay's menu is actually on screen, not permanently
/// steal the controller from whatever's running the rest of the time.
///
/// `visible` is threaded through as this `Subscription`'s identifying data
/// (`Subscription::run_with`): a change in the value iced sees between calls
/// to `Overlay::subscription` is what causes iced to tear down the previous
/// stream and spawn a fresh one with the new grab state - not a channel/signal
/// into an already-running stream. Its child watcher tasks are owned by an
/// abort-on-drop collection, so tearing down the stream closes every `Device`
/// and releases any grab. The brief gap while devices are re-enumerated and
/// reopened across that transition is an accepted tradeoff for reusing iced's
/// state-driven subscription mechanism instead of hand-rolling a persistent
/// task with its own control channel.
///
/// KNOWN LIMITATION: devices are enumerated once per (re)scan loop, at
/// startup and after a device disappears; a controller plugged in while
/// otherwise idle is not picked up until the next rescan (every
/// `RESCAN_DELAY` while no matching device is found at all).
pub fn subscription(visible: bool) -> Subscription<GamepadEvent> {
    Subscription::run_with(visible, |&visible| {
        stream::channel(
            8,
            move |mut output: futures_channel::mpsc::Sender<GamepadEvent>| async move {
                loop {
                    let devices = gamepad_devices();
                    if devices.is_empty() {
                        tracing::warn!(
                            "hearthdeck-overlay: no evdev device exposes BTN_MODE (Guide button); \
                         retrying in {RESCAN_DELAY:?}. Run `evtest` on this hardware to confirm \
                         the actual event codes your controller sends."
                        );
                        tokio::time::sleep(RESCAN_DELAY).await;
                        continue;
                    }

                    // One task per device, all feeding the same channel, rather
                    // than a merged Stream: evdev's Stream impl needs its
                    // `stream-trait` feature on top of `tokio`, and a plain
                    // per-device `next_event().await` loop needs neither,
                    // keeping this crate's dependency footprint smaller.
                    let (tx, mut rx) = mpsc::channel(8);
                    let mut tasks = WatchTasks::default();
                    for device in devices {
                        let tx = tx.clone();
                        tasks.spawn(watch_device(device, tx, visible));
                    }
                    drop(tx);

                    while let Some(event) = rx.recv().await {
                        if output.send(event).await.is_err() {
                            return;
                        }
                    }

                    tracing::warn!(
                        "hearthdeck-overlay: all evdev devices disconnected, rescanning"
                    );
                }
            },
        )
    })
}

/// Reads one device's events until it errors out (typically: unplugged),
/// sending the corresponding `GamepadEvent` for each button/axis transition
/// this module cares about. Grabs the device first (see `subscription`'s own
/// docs) when `grab` is set, logging a warning and continuing ungrabbed if
/// the grab itself fails (e.g. no permission, or another process already
/// holds it) rather than not watching the device at all.
///
/// Uses the async `next_event()` API (integrates with tokio's reactor, does
/// not block a worker thread) rather than the blocking `Device::fetch_events`
/// or the `Stream` trait impl (which needs evdev's extra `stream-trait`
/// feature on top of `tokio` - avoided here to keep dependencies smaller).
async fn watch_device(mut device: Device, tx: mpsc::Sender<GamepadEvent>, grab: bool) {
    if grab && let Err(err) = device.grab() {
        tracing::warn!(
            ?err,
            "hearthdeck-overlay: failed to grab gamepad exclusively; \
             input will also reach whatever is running underneath the overlay"
        );
    }

    let mut stream = match device.into_event_stream() {
        Ok(stream) => stream,
        Err(err) => {
            tracing::warn!(
                ?err,
                "hearthdeck-overlay: failed to open evdev event stream"
            );
            return;
        }
    };

    loop {
        let event = match stream.next_event().await {
            Ok(event) => event,
            Err(err) => {
                tracing::warn!(?err, "hearthdeck-overlay: evdev device read error");
                return;
            }
        };

        let mapped = match event.destructure() {
            EventSummary::Key(_, code, value) => map_key_event(code, value),
            // Hat-axis D-pad, for controllers that don't report BTN_DPAD_*
            // as discrete keys - see this module's own top-of-file docs.
            EventSummary::AbsoluteAxis(_, axis, value) => map_axis_event(axis, value),
            _ => None,
        };

        if let Some(mapped) = mapped
            && tx.send(mapped).await.is_err()
        {
            return;
        }
    }
}

fn map_key_event(code: KeyCode, value: i32) -> Option<GamepadEvent> {
    match (code, value) {
        (KeyCode::BTN_MODE, 1) => Some(GamepadEvent::ToggleOverlay),
        (KeyCode::BTN_DPAD_UP, 1) => Some(GamepadEvent::NavigateUp),
        (KeyCode::BTN_DPAD_DOWN, 1) => Some(GamepadEvent::NavigateDown),
        (KeyCode::BTN_SOUTH, 1) => Some(GamepadEvent::Activate),
        _ => None,
    }
}

fn map_axis_event(axis: AbsoluteAxisCode, value: i32) -> Option<GamepadEvent> {
    match (axis, value) {
        (AbsoluteAxisCode::ABS_HAT0Y, -1) => Some(GamepadEvent::NavigateUp),
        (AbsoluteAxisCode::ABS_HAT0Y, 1) => Some(GamepadEvent::NavigateDown),
        _ => None,
    }
}

/// Opens every currently-connected evdev device that reports supporting
/// `KeyCode::BTN_MODE` (most gamepads' Guide/Home button) - used as the
/// signal that a device is a gamepad worth watching for the other events
/// too, not just the toggle button.
fn gamepad_devices() -> Vec<Device> {
    evdev::enumerate()
        .map(|(_, device)| device)
        .filter(|device| {
            device
                .supported_keys()
                .is_some_and(|keys| keys.contains(KeyCode::BTN_MODE))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::{GamepadEvent, WatchTasks, map_axis_event, map_key_event};
    use evdev::{AbsoluteAxisCode, KeyCode};

    struct DropSignal(Arc<AtomicBool>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn dropping_watch_tasks_cancels_device_watchers() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let dropped = Arc::new(AtomicBool::new(false));
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let signal = DropSignal(Arc::clone(&dropped));
            let mut tasks = WatchTasks::default();

            tasks.spawn(async move {
                let _signal = signal;
                let _ = started_tx.send(());
                pending::<()>().await;
            });
            started_rx.await.unwrap();
            drop(tasks);
            tokio::task::yield_now().await;

            assert!(dropped.load(Ordering::SeqCst));
        });
    }

    #[test]
    fn guide_dpad_and_south_button_map_on_press_only() {
        assert_eq!(
            map_key_event(KeyCode::BTN_MODE, 1),
            Some(GamepadEvent::ToggleOverlay)
        );
        assert_eq!(
            map_key_event(KeyCode::BTN_DPAD_UP, 1),
            Some(GamepadEvent::NavigateUp)
        );
        assert_eq!(
            map_key_event(KeyCode::BTN_DPAD_DOWN, 1),
            Some(GamepadEvent::NavigateDown)
        );
        assert_eq!(
            map_key_event(KeyCode::BTN_SOUTH, 1),
            Some(GamepadEvent::Activate)
        );
        assert_eq!(map_key_event(KeyCode::BTN_SOUTH, 0), None);
        assert_eq!(map_key_event(KeyCode::BTN_SOUTH, 2), None);
    }

    #[test]
    fn hat_axis_maps_directions_and_ignores_release() {
        assert_eq!(
            map_axis_event(AbsoluteAxisCode::ABS_HAT0Y, -1),
            Some(GamepadEvent::NavigateUp)
        );
        assert_eq!(
            map_axis_event(AbsoluteAxisCode::ABS_HAT0Y, 1),
            Some(GamepadEvent::NavigateDown)
        );
        assert_eq!(map_axis_event(AbsoluteAxisCode::ABS_HAT0Y, 0), None);
        assert_eq!(map_axis_event(AbsoluteAxisCode::ABS_HAT0X, 1), None);
    }
}
