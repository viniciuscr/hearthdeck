use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use evdev::uinput::VirtualDevice;
use evdev::{
    AbsoluteAxisCode, AttributeSet, Device, EventType, InputEvent, KeyCode, RelativeAxisCode,
};
use tokio::net::UnixDatagram;

use crate::mapping::{Control, Mapper, OutputEvent, OutputKey};

const DEVICE_NAME: &str = "Hearthdeck Compatibility Input";

#[derive(Clone, Copy)]
pub(crate) struct AxisRange {
    minimum: i32,
    maximum: i32,
}

impl AxisRange {
    pub(crate) fn new(minimum: i32, maximum: i32) -> Self {
        Self { minimum, maximum }
    }
}

pub(crate) fn init_logging() {
    hearthdeck_observability::init("hearthdeck-input", "warn,hearthdeck_input=info");
}

pub(crate) fn bind_control_socket(path: &Path) -> Result<UnixDatagram> {
    if let Err(error) = std::fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(error).context("could not remove stale input control socket");
    }
    let socket = UnixDatagram::bind(path).context("could not bind input control socket")?;
    std::fs::set_permissions(path, Permissions::from_mode(0o600))?;
    Ok(socket)
}

pub(crate) fn create_virtual_device() -> Result<VirtualDevice> {
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

pub(crate) fn gamepad_devices() -> Vec<(PathBuf, Device)> {
    evdev::enumerate()
        .filter(|(_, device)| {
            device.name() != Some(DEVICE_NAME)
                && device
                    .supported_keys()
                    .is_some_and(|keys| keys.contains(KeyCode::BTN_SOUTH))
        })
        .collect()
}

pub(crate) fn map_key(
    mapper: &mut Mapper,
    device: u64,
    code: KeyCode,
    value: i32,
) -> Vec<OutputEvent> {
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

pub(crate) fn map_axis(
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

pub(crate) fn normalize_centered(value: i32, range: Option<AxisRange>) -> i32 {
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

pub(crate) fn normalize_trigger(value: i32, range: Option<AxisRange>) -> i32 {
    let Some(range) = range else {
        return i32::from(value > 0) * 1000;
    };
    let width = i64::from(range.maximum) - i64::from(range.minimum);
    if width <= 0 {
        return 0;
    }
    (((i64::from(value) - i64::from(range.minimum)) * 1000 / width).clamp(0, 1000)) as i32
}

pub(crate) fn emit(
    device: &mut VirtualDevice,
    events: impl IntoIterator<Item = OutputEvent>,
) -> Result<()> {
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
    fn shared_logging_preserves_filters_and_supports_both_formats() {
        const CHILD: &str = "HEARTHDECK_INPUT_LOGGING_TEST";
        if std::env::var_os(CHILD).is_some() {
            super::init_logging();
            tracing::error!(probe = true, "input error probe");
            tracing::info!("input info probe");
            return;
        }
        for (format, filter, info_visible) in [
            ("default", None, true),
            ("json", Some("hearthdeck_input=info"), true),
            ("pretty", Some("hearthdeck_input=info"), true),
        ] {
            let mut command =
                std::process::Command::new(std::env::current_exe().expect("test executable"));
            command
                .args([
                    "--exact",
                    "linux::tests::shared_logging_preserves_filters_and_supports_both_formats",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env_remove("RUST_LOG")
                .env_remove("HEARTHDECK_LOG_FORMAT");
            if format != "default" {
                command.env("HEARTHDECK_LOG_FORMAT", format);
            }
            if let Some(filter) = filter {
                command.env("RUST_LOG", filter);
            }
            let output = command.output().expect("logging child");
            assert!(output.status.success(), "{format}: {output:?}");
            let logs = String::from_utf8(output.stdout).expect("UTF-8 logs");
            assert!(logs.contains("input error probe"), "{format}: {logs}");
            assert_eq!(logs.contains("input info probe"), info_visible, "{logs}");
            if format == "pretty" {
                assert!(!logs.lines().any(|line| line.starts_with('{')), "{logs}");
            } else {
                let event = logs
                    .lines()
                    .find(|line| line.starts_with('{') && line.contains("input error probe"))
                    .expect("JSON probe");
                assert!(event.contains("\"probe\":true"), "{event}");
                assert!(event.contains("\"level\":\"ERROR\""), "{event}");
            }
        }
    }

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
