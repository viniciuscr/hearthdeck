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
        if !self.enabled || !self.connected {
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
    use super::{WifiStatus, current_time};

    #[test]
    fn current_time_is_displayable() {
        assert!(!current_time().is_empty());
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
    }
}
