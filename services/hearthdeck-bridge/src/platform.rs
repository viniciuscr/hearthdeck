use anyhow::Result;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use hearthdeck_protocol::DiscoveredApplication;

#[cfg(target_os = "linux")]
pub const DESKTOP_APPS_SOURCE: &str = "desktop-apps";
#[cfg(target_os = "macos")]
pub const MACOS_APPS_SOURCE: &str = "macos-apps";

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{discover_applications, launch_application};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{discover_applications, launch_application};

pub struct LaunchedApplication {
    pub unit_name: Option<String>,
}

#[cfg(target_os = "linux")]
pub async fn stop_application(unit_name: Option<&str>) -> Result<()> {
    let Some(unit_name) = unit_name else {
        anyhow::bail!("application session cannot be stopped on this platform")
    };
    let status = tokio::process::Command::new("systemctl")
        .args(["--user", "stop", unit_name])
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("systemd user manager rejected application stop")
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub async fn stop_application(_unit_name: Option<&str>) -> Result<()> {
    anyhow::bail!("application session cannot be stopped on this platform")
}

#[cfg(target_os = "linux")]
pub async fn application_is_running(unit_name: Option<&str>) -> Result<bool> {
    let Some(unit_name) = unit_name else {
        return Ok(false);
    };
    Ok(tokio::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", unit_name])
        .status()
        .await?
        .success())
}

#[cfg(not(target_os = "linux"))]
pub async fn application_is_running(_unit_name: Option<&str>) -> Result<bool> {
    Ok(false)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub async fn discover_applications(_source_id: &str) -> Result<Vec<DiscoveredApplication>> {
    anyhow::bail!("application discovery is unsupported on this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub async fn launch_application(
    _source_id: &str,
    _application_id: &str,
    _session_id: &str,
) -> Result<LaunchedApplication> {
    anyhow::bail!("application launch is unsupported on this platform")
}
