// Gamepad Guide/Mode-button watcher for hearthdeck-overlay.
//
// UNTESTED on real hardware. Two assumptions this needs confirming on the
// actual target device:
//   1. That the controller's Guide/Home button is reported by the kernel as
//      `KeyCode::BTN_MODE` at all - some controllers/drivers surface it
//      under a different code, or don't expose it as a distinct evdev key.
//      Confirm with `evtest` on the target hardware.
//   2. That reading `/dev/input/event*` works for this session's user
//      without extra setup - on stock Arch, systemd-logind's `uaccess`
//      udev tagging normally grants the active seat's logged-in user
//      access automatically, but this repo has never relied on that before
//      now, so it hasn't been verified here.
use std::time::Duration;

use cosmic::iced::Subscription;
use cosmic::iced::stream;
use evdev::{Device, EventSummary, KeyCode};
use futures_util::SinkExt;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy)]
pub struct GuideButtonPressed;

const RESCAN_DELAY: Duration = Duration::from_secs(5);

/// Emits `GuideButtonPressed` each time any currently-connected gamepad's
/// Guide/Mode button transitions to pressed.
///
/// KNOWN LIMITATION: devices are enumerated once per (re)scan loop, at
/// startup and after a device disappears; a controller plugged in while
/// otherwise idle is not picked up until the next rescan (every
/// `RESCAN_DELAY` while no matching device is found at all).
pub fn subscription() -> Subscription<GuideButtonPressed> {
    Subscription::run(|| {
        stream::channel(
            8,
            |mut output: futures_channel::mpsc::Sender<GuideButtonPressed>| async move {
                loop {
                    let devices = guide_button_devices();
                    if devices.is_empty() {
                        tracing::warn!(
                            "hearthdeck-overlay: no evdev device exposes BTN_MODE (Guide button); \
                         retrying in {RESCAN_DELAY:?}. Run `evtest` on this hardware to confirm \
                         the actual event code your controller sends."
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

                    while let Some(()) = rx.recv().await {
                        if output.send(GuideButtonPressed).await.is_err() {
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
/// sending `()` on the channel each time its Guide/Mode button is pressed.
///
/// Uses the async `next_event()` API (integrates with tokio's reactor, does
/// not block a worker thread) rather than the blocking `Device::fetch_events`
/// or the `Stream` trait impl (which needs evdev's extra `stream-trait`
/// feature on top of `tokio` - avoided here to keep dependencies smaller).
async fn watch_device(device: Device, tx: mpsc::Sender<()>) {
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
        match stream.next_event().await {
            Ok(event) => {
                if let EventSummary::Key(_, KeyCode::BTN_MODE, 1) = event.destructure()
                    && tx.send(()).await.is_err()
                {
                    return;
                }
            }
            Err(err) => {
                tracing::warn!(?err, "hearthdeck-overlay: evdev device read error");
                return;
            }
        }
    }
}

/// Opens every currently-connected evdev device that reports supporting
/// `KeyCode::BTN_MODE` (most gamepads' Guide/Home button).
fn guide_button_devices() -> Vec<Device> {
    evdev::enumerate()
        .map(|(_, device)| device)
        .filter(|device| {
            device
                .supported_keys()
                .is_some_and(|keys| keys.contains(KeyCode::BTN_MODE))
        })
        .collect()
}
