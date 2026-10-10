use chrono::{DateTime, Local};

#[derive(Clone, Debug, Default)]
pub struct SystemStatus {
    pub wifi: Option<WifiStatus>,
    pub bluetooth: Option<BluetoothStatus>,
}

#[derive(Clone, Debug)]
pub struct WifiStatus {
    pub enabled: bool,
    pub connected: bool,
    pub strength: u8,
}

#[derive(Clone, Debug)]
pub struct BluetoothStatus {
    pub enabled: bool,
    pub connected: bool,
}

impl WifiStatus {
    pub fn icon_name(&self) -> &'static str {
        if !self.enabled {
            "network-wireless-signal-none-symbolic"
        } else if !self.connected {
            "network-wireless-disconnected-symbolic"
        } else if self.strength < 25 {
            "network-wireless-signal-weak-symbolic"
        } else if self.strength < 50 {
            "network-wireless-signal-ok-symbolic"
        } else if self.strength < 75 {
            "network-wireless-signal-good-symbolic"
        } else {
            "network-wireless-signal-excellent-symbolic"
        }
    }

    pub fn label(&self) -> &'static str {
        if !self.enabled {
            "Wi-Fi off"
        } else if self.connected {
            "Wi-Fi connected"
        } else {
            "Wi-Fi disconnected"
        }
    }
}

impl BluetoothStatus {
    pub fn icon_name(&self) -> &'static str {
        if !self.enabled {
            "bluetooth-disabled-symbolic"
        } else if self.connected {
            "bluetooth-active-symbolic"
        } else {
            "bluetooth-symbolic"
        }
    }

    pub fn label(&self) -> &'static str {
        if !self.enabled {
            "Bluetooth off"
        } else if self.connected {
            "Bluetooth connected"
        } else {
            "Bluetooth on"
        }
    }
}

impl SystemStatus {
    pub async fn load() -> Self {
        let (wifi, bluetooth) = tokio::join!(wifi_status(), bluetooth_status());
        Self { wifi, bluetooth }
    }
}

pub fn current_time() -> String {
    DateTime::<Local>::from(std::time::SystemTime::now())
        .format("%-I:%M %p")
        .to_string()
}

/// Available bytes on the filesystem containing the given path.
fn available_disk_bytes(path: &str) -> u64 {
    let Ok(cstr) = std::ffi::CString::new(path) else {
        return 0;
    };
    nix::sys::statvfs::statvfs(cstr.as_c_str())
        .map(|vfs| vfs.blocks_available() * vfs.fragment_size())
        .unwrap_or(0)
}

/// Free space on the home filesystem, formatted for the sidebar footer.
///
/// Reads the filesystem, so it is called from an effect - at startup and again
/// on the periodic poll - and never from a view.
pub(crate) fn disk_free_label() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    human_size(available_disk_bytes(&home))
}

/// Formats a byte count for humans, e.g. "128.4 GB".
pub(crate) fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{:.0} {}", value, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

#[cfg(target_os = "linux")]
async fn wifi_status() -> Option<WifiStatus> {
    use nmrs::{ActiveConnection, NetworkManager};

    let snapshot = NetworkManager::new().await.ok()?.snapshot().await.ok()?;
    let active = snapshot
        .active_connections
        .iter()
        .find_map(|connection| match connection {
            ActiveConnection::Wifi(wifi) => Some(wifi.strength.unwrap_or_default()),
            _ => None,
        });
    Some(WifiStatus {
        enabled: snapshot.wifi.enabled,
        connected: active.is_some(),
        strength: active.unwrap_or_default(),
    })
}

#[cfg(not(target_os = "linux"))]
async fn wifi_status() -> Option<WifiStatus> {
    None
}

#[cfg(target_os = "linux")]
async fn bluetooth_status() -> Option<BluetoothStatus> {
    let session = bluer::Session::new().await.ok()?;
    let adapter = session.default_adapter().await.ok()?;
    let enabled = adapter.is_powered().await.unwrap_or_default();
    let mut connected = false;
    for address in adapter.device_addresses().await.unwrap_or_default() {
        let Ok(device) = adapter.device(address) else {
            continue;
        };
        if device.is_connected().await.unwrap_or_default() {
            connected = true;
            break;
        }
    }
    Some(BluetoothStatus { enabled, connected })
}

#[cfg(not(target_os = "linux"))]
async fn bluetooth_status() -> Option<BluetoothStatus> {
    None
}

#[cfg(test)]
mod tests {
    use super::{BluetoothStatus, WifiStatus, current_time, disk_free_label, human_size};

    #[test]
    fn current_time_is_displayable() {
        assert!(!current_time().is_empty());
    }

    /// The sidebar footer reads a bare byte count out as a size a person can
    /// use. The unit steps at each 1024, and only bytes are shown without a
    /// decimal, because "0.0 B" reads like a measurement.
    #[test]
    fn a_byte_count_reads_as_a_size_at_the_right_unit() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(1024 * 1024), "1.0 MB");
        assert_eq!(human_size(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(human_size(1024_u64.pow(4)), "1.0 TB");
        // The last unit absorbs everything past it rather than overflowing the
        // table.
        assert_eq!(human_size(1024_u64.pow(5)), "1024.0 TB");
    }

    /// The label is read off the filesystem, so this only holds that the call
    /// resolves and formats: an unreadable `$HOME` falls back rather than
    /// panicking, and the result is never blank.
    #[test]
    fn the_free_space_label_is_displayable() {
        assert!(!disk_free_label().is_empty());
    }

    #[test]
    fn wifi_icon_tracks_connection_strength() {
        let mut status = WifiStatus {
            enabled: true,
            connected: true,
            strength: 80,
        };
        assert_eq!(
            status.icon_name(),
            "network-wireless-signal-excellent-symbolic"
        );

        status.connected = false;
        assert_eq!(status.icon_name(), "network-wireless-disconnected-symbolic");

        status.enabled = false;
        assert_eq!(status.icon_name(), "network-wireless-signal-none-symbolic");
    }

    #[test]
    fn bluetooth_icon_tracks_power_and_connection() {
        let mut status = BluetoothStatus {
            enabled: false,
            connected: false,
        };
        assert_eq!(status.icon_name(), "bluetooth-disabled-symbolic");

        status.enabled = true;
        assert_eq!(status.icon_name(), "bluetooth-symbolic");

        status.connected = true;
        assert_eq!(status.icon_name(), "bluetooth-active-symbolic");
    }
}
