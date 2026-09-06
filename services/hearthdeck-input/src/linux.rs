use std::collections::{HashMap, HashSet};
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use evdev::uinput::VirtualDevice;
use evdev::{
    AbsoluteAxisCode, AttributeSet, Device, EventSummary, EventType, InputEvent, KeyCode,
    RelativeAxisCode,
};
use tokio::net::UnixDatagram;
use tokio::sync::mpsc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::mapping::{Control, Mapper, OutputEvent, OutputKey};

const DEVICE_NAME: &str = "Hearthdeck Compatibility Input";
const SOCKET_NAME: &str = "hearthdeck-input.sock";
const SCAN_INTERVAL: Duration = Duration::from_secs(2);
const MOUSE_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Clone, Copy)]
struct AxisRange {
    minimum: i32,
    maximum: i32,
}

enum DeviceMessage {
    Key {
        device: u64,
        code: KeyCode,
        value: i32,
    },
    Axis {
        device: u64,
        code: AbsoluteAxisCode,
        value: i32,
        range: Option<AxisRange>,
    },
    Disconnected {
        device: u64,
        path: PathBuf,
    },
}

pub async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let socket_path = socket_path()?;
    let socket = bind_control_socket(&socket_path)?;
    let mut output = create_virtual_device().context("could not create virtual input device")?;
    let (device_tx, mut device_rx) = mpsc::channel(128);
    let mut mapper = Mapper::default();
    let mut active_paths = HashSet::new();
    let mut path_devices = HashMap::new();
    let mut next_device = 1_u64;
    let mut scan = tokio::time::interval(SCAN_INTERVAL);
    let mut mouse = tokio::time::interval(MOUSE_INTERVAL);
    let mut command = [0_u8; 32];

    info!(path = ?socket_path, "input compatibility broker ready");

    loop {
        tokio::select! {
            _ = scan.tick() => {
                for (path, device) in gamepad_devices() {
                    if !active_paths.insert(path.clone()) {
                        continue;
                    }
                    let device_id = next_device;
                    next_device += 1;
                    path_devices.insert(path.clone(), device_id);
                    tokio::spawn(watch_device(path, device_id, device, device_tx.clone()));
                }
            }
            Some(message) = device_rx.recv() => {
                let events = match message {
                    DeviceMessage::Key { device, code, value } => {
                        map_key(&mut mapper, device, code, value)
                    }
                    DeviceMessage::Axis { device, code, value, range } => {
                        map_axis(&mut mapper, device, code, value, range)
                    }
                    DeviceMessage::Disconnected { device, path } => {
                        if path_devices.get(&path) == Some(&device) {
                            active_paths.remove(&path);
                            path_devices.remove(&path);
                        }
                        mapper.remove_device(device)
                    }
                };
                emit(&mut output, events)?;
            }
            received = socket.recv(&mut command) => {
                let length = received.context("input control socket read failed")?;
                let events = match &command[..length] {
                    b"desktop" => {
                        info!("desktop input compatibility enabled");
                        mapper.set_active(true)
                    }
                    b"native" => {
                        info!("desktop input compatibility disabled");
                        mapper.set_active(false)
                    }
                    _ => {
                        warn!("ignored invalid input profile command");
                        Vec::new()
                    }
                };
                emit(&mut output, events)?;
            }
            _ = mouse.tick() => {
                if let Some(event) = mapper.mouse_tick() {
                    emit(&mut output, [event])?;
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    emit(&mut output, mapper.set_active(false))?;
    let _ = std::fs::remove_file(socket_path);
    Ok(())
}

fn socket_path() -> Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    Ok(PathBuf::from(runtime).join(SOCKET_NAME))
}

fn bind_control_socket(path: &Path) -> Result<UnixDatagram> {
    if let Err(error) = std::fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(error).context("could not remove stale input control socket");
    }
    let socket = UnixDatagram::bind(path).context("could not bind input control socket")?;
    std::fs::set_permissions(path, Permissions::from_mode(0o600))?;
    Ok(socket)
}

fn create_virtual_device() -> Result<VirtualDevice> {
    let keys = [
        KeyCode::KEY_ENTER,
        KeyCode::KEY_ESC,
        KeyCode::KEY_SPACE,
        KeyCode::KEY_TAB,
        KeyCode::KEY_PAGEUP,
        KeyCode::KEY_PAGEDOWN,
        KeyCode::KEY_LEFT,
        KeyCode::KEY_RIGHT,
        KeyCode::KEY_UP,
        KeyCode::KEY_DOWN,
        KeyCode::BTN_LEFT,
        KeyCode::BTN_RIGHT,
    ]
    .into_iter()
    .collect::<AttributeSet<_>>();
    let relative_axes = [RelativeAxisCode::REL_X, RelativeAxisCode::REL_Y]
        .into_iter()
        .collect::<AttributeSet<_>>();

    Ok(VirtualDevice::builder()?
        .name(DEVICE_NAME)
        .with_keys(&keys)?
        .with_relative_axes(&relative_axes)?
        .build()?)
}

fn gamepad_devices() -> Vec<(PathBuf, Device)> {
    evdev::enumerate()
        .filter(|(_, device)| {
            device.name() != Some(DEVICE_NAME)
                && device
                    .supported_keys()
                    .is_some_and(|keys| keys.contains(KeyCode::BTN_SOUTH))
        })
        .collect()
}

async fn watch_device(
    path: PathBuf,
    device_id: u64,
    device: Device,
    tx: mpsc::Sender<DeviceMessage>,
) {
    let ranges = device
        .get_absinfo()
        .map(|axes| {
            axes.map(|(code, info)| {
                (
                    code,
                    AxisRange {
                        minimum: info.minimum(),
                        maximum: info.maximum(),
                    },
                )
            })
            .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let name = device.name().unwrap_or("unknown gamepad").to_owned();
    let mut stream = match device.into_event_stream() {
        Ok(stream) => stream,
        Err(error) => {
            warn!(?path, %error, "could not open gamepad event stream");
            let _ = tx
                .send(DeviceMessage::Disconnected {
                    device: device_id,
                    path,
                })
                .await;
            return;
        }
    };
    info!(?path, %name, "watching gamepad");

    loop {
        let message = match stream.next_event().await {
            Ok(event) => match event.destructure() {
                EventSummary::Key(_, code, value) => Some(DeviceMessage::Key {
                    device: device_id,
                    code,
                    value,
                }),
                EventSummary::AbsoluteAxis(_, code, value) => Some(DeviceMessage::Axis {
                    device: device_id,
                    code,
                    value,
                    range: ranges.get(&code).copied(),
                }),
                _ => None,
            },
            Err(error) => {
                warn!(?path, %error, "gamepad event stream ended");
                break;
            }
        };
        if let Some(message) = message
            && tx.send(message).await.is_err()
        {
            return;
        }
    }

    let _ = tx
        .send(DeviceMessage::Disconnected {
            device: device_id,
            path,
        })
        .await;
}

fn map_key(mapper: &mut Mapper, device: u64, code: KeyCode, value: i32) -> Vec<OutputEvent> {
    let control = match code {
        KeyCode::BTN_SOUTH => Control::South,
        KeyCode::BTN_EAST => Control::East,
        KeyCode::BTN_WEST => Control::West,
        KeyCode::BTN_NORTH => Control::North,
        KeyCode::BTN_TL => Control::LeftBumper,
        KeyCode::BTN_TR => Control::RightBumper,
        KeyCode::BTN_START => Control::Start,
        KeyCode::BTN_SELECT => Control::Select,
        KeyCode::BTN_DPAD_LEFT => Control::DpadLeft,
        KeyCode::BTN_DPAD_RIGHT => Control::DpadRight,
        KeyCode::BTN_DPAD_UP => Control::DpadUp,
        KeyCode::BTN_DPAD_DOWN => Control::DpadDown,
        KeyCode::BTN_TL2 => Control::LeftTrigger,
        KeyCode::BTN_TR2 => Control::RightTrigger,
        _ => return Vec::new(),
    };
    mapper.button(device, control, value != 0)
}

fn map_axis(
    mapper: &mut Mapper,
    device: u64,
    code: AbsoluteAxisCode,
    value: i32,
    range: Option<AxisRange>,
) -> Vec<OutputEvent> {
    match code {
        AbsoluteAxisCode::ABS_HAT0X => mapper.directional_axis(
            device,
            Control::DpadLeft,
            Control::DpadRight,
            normalize_centered(value, range),
        ),
        AbsoluteAxisCode::ABS_HAT0Y => mapper.directional_axis(
            device,
            Control::DpadUp,
            Control::DpadDown,
            normalize_centered(value, range),
        ),
        AbsoluteAxisCode::ABS_X => mapper.directional_axis(
            device,
            Control::StickLeft,
            Control::StickRight,
            normalize_centered(value, range),
        ),
        AbsoluteAxisCode::ABS_Y => mapper.directional_axis(
            device,
            Control::StickUp,
            Control::StickDown,
            normalize_centered(value, range),
        ),
        AbsoluteAxisCode::ABS_RX => {
            mapper.right_stick_axis(device, true, normalize_centered(value, range));
            Vec::new()
        }
        AbsoluteAxisCode::ABS_RY => {
            mapper.right_stick_axis(device, false, normalize_centered(value, range));
            Vec::new()
        }
        AbsoluteAxisCode::ABS_Z => mapper.button(
            device,
            Control::LeftTrigger,
            normalize_trigger(value, range) > 500,
        ),
        AbsoluteAxisCode::ABS_RZ => mapper.button(
            device,
            Control::RightTrigger,
            normalize_trigger(value, range) > 500,
        ),
        _ => Vec::new(),
    }
}

fn normalize_centered(value: i32, range: Option<AxisRange>) -> i32 {
    let Some(range) = range else {
        return value.clamp(-1, 1) * 1000;
    };
    let center = (i64::from(range.minimum) + i64::from(range.maximum)) / 2;
    let half_range = (i64::from(range.maximum) - i64::from(range.minimum)) / 2;
    if half_range <= 0 {
        return 0;
    }
    (((i64::from(value) - center) * 1000 / half_range).clamp(-1000, 1000)) as i32
}

fn normalize_trigger(value: i32, range: Option<AxisRange>) -> i32 {
    let Some(range) = range else {
        return i32::from(value > 0) * 1000;
    };
    let width = i64::from(range.maximum) - i64::from(range.minimum);
    if width <= 0 {
        return 0;
    }
    (((i64::from(value) - i64::from(range.minimum)) * 1000 / width).clamp(0, 1000)) as i32
}

fn emit(device: &mut VirtualDevice, events: impl IntoIterator<Item = OutputEvent>) -> Result<()> {
    let events = events
        .into_iter()
        .flat_map(|event| match event {
            OutputEvent::Key { key, pressed } => vec![InputEvent::new(
                EventType::KEY.0,
                output_key_code(key).code(),
                i32::from(pressed),
            )],
            OutputEvent::MouseMove { x, y } => vec![
                InputEvent::new(EventType::RELATIVE.0, RelativeAxisCode::REL_X.0, x),
                InputEvent::new(EventType::RELATIVE.0, RelativeAxisCode::REL_Y.0, y),
            ],
        })
        .collect::<Vec<_>>();
    if !events.is_empty() {
        device
            .emit(&events)
            .context("could not emit virtual input")?;
    }
    Ok(())
}

fn output_key_code(key: OutputKey) -> KeyCode {
    match key {
        OutputKey::Enter => KeyCode::KEY_ENTER,
        OutputKey::Escape => KeyCode::KEY_ESC,
        OutputKey::Space => KeyCode::KEY_SPACE,
        OutputKey::Tab => KeyCode::KEY_TAB,
        OutputKey::PageUp => KeyCode::KEY_PAGEUP,
        OutputKey::PageDown => KeyCode::KEY_PAGEDOWN,
        OutputKey::Left => KeyCode::KEY_LEFT,
        OutputKey::Right => KeyCode::KEY_RIGHT,
        OutputKey::Up => KeyCode::KEY_UP,
        OutputKey::Down => KeyCode::KEY_DOWN,
        OutputKey::MouseLeft => KeyCode::BTN_LEFT,
        OutputKey::MouseRight => KeyCode::BTN_RIGHT,
    }
}

#[cfg(test)]
mod tests {
    use super::{AxisRange, normalize_centered, normalize_trigger};

    #[test]
    fn centered_axes_use_device_range() {
        let range = Some(AxisRange {
            minimum: 0,
            maximum: 65_535,
        });
        assert_eq!(normalize_centered(0, range), -1000);
        assert_eq!(normalize_centered(32_767, range), 0);
        assert_eq!(normalize_centered(65_535, range), 1000);
    }

    #[test]
    fn triggers_use_device_range() {
        let range = Some(AxisRange {
            minimum: 0,
            maximum: 255,
        });
        assert_eq!(normalize_trigger(0, range), 0);
        assert_eq!(normalize_trigger(128, range), 501);
        assert_eq!(normalize_trigger(255, range), 1000);
    }

    #[test]
    fn missing_hat_range_uses_discrete_values() {
        assert_eq!(normalize_centered(-1, None), -1000);
        assert_eq!(normalize_centered(0, None), 0);
        assert_eq!(normalize_centered(1, None), 1000);
    }
}
