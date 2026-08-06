// Gamepad watcher for hearthdeck-overlay: the Guide/Mode button toggles the
// overlay, and (while it's open) D-pad up/down moves the menu selection and
// the South/A button activates it.
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

/// Emits a `GamepadEvent` each time any currently-connected gamepad reports
/// one of the button/axis transitions the overlay cares about.
///
/// KNOWN LIMITATION: devices are enumerated once per (re)scan loop, at
/// startup and after a device disappears; a controller plugged in while
/// otherwise idle is not picked up until the next rescan (every
/// `RESCAN_DELAY` while no matching device is found at all).
pub fn subscription() -> Subscription<GamepadEvent> {
    Subscription::run(|| {
        stream::channel(
            8,
            |mut output: futures_channel::mpsc::Sender<GamepadEvent>| async move {
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
                    let mut tasks = Vec::new();
                    for device in devices {
                        let tx = tx.clone();
                        tasks.push(tokio::spawn(watch_device(device, tx)));
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
                    for task in tasks {
                        task.abort();
                    }
                }
            },
        )
    })
}

/// Reads one device's events until it errors out (typically: unplugged),
/// sending the corresponding `GamepadEvent` for each button/axis transition
/// this module cares about.
///
/// Uses the async `next_event()` API (integrates with tokio's reactor, does
/// not block a worker thread) rather than the blocking `Device::fetch_events`
/// or the `Stream` trait impl (which needs evdev's extra `stream-trait`
/// feature on top of `tokio` - avoided here to keep dependencies smaller).
async fn watch_device(device: Device, tx: mpsc::Sender<GamepadEvent>) {
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
            EventSummary::Key(_, KeyCode::BTN_MODE, 1) => Some(GamepadEvent::ToggleOverlay),
            EventSummary::Key(_, KeyCode::BTN_DPAD_UP, 1) => Some(GamepadEvent::NavigateUp),
            EventSummary::Key(_, KeyCode::BTN_DPAD_DOWN, 1) => Some(GamepadEvent::NavigateDown),
            EventSummary::Key(_, KeyCode::BTN_SOUTH, 1) => Some(GamepadEvent::Activate),
            // Hat-axis D-pad, for controllers that don't report BTN_DPAD_*
            // as discrete keys - see this module's own top-of-file docs.
            EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_HAT0Y, -1) => {
                Some(GamepadEvent::NavigateUp)
            }
            EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_HAT0Y, 1) => {
                Some(GamepadEvent::NavigateDown)
            }
            _ => None,
        };

        if let Some(mapped) = mapped
            && tx.send(mapped).await.is_err()
        {
            return;
        }
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
