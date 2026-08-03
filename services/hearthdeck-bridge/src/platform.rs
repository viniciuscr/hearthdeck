use anyhow::Result;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use hearthdeck_protocol::DiscoveredApplication;
#[cfg(all(not(target_os = "linux"), not(test)))]
use hearthdeck_protocol::HeroicRunner;

#[cfg(any(target_os = "linux", test))]
pub const DESKTOP_APPS_SOURCE: &str = "desktop-apps";
#[cfg(all(target_os = "macos", not(test)))]
pub const MACOS_APPS_SOURCE: &str = "macos-apps";

#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(any(target_os = "linux", test))]
pub use linux::{discover_applications, launch_application, launch_heroic_game, launch_retro_game};

#[cfg(all(target_os = "macos", not(test)))]
mod macos;
#[cfg(all(target_os = "macos", not(test)))]
pub use macos::{discover_applications, launch_application};

pub struct LaunchedApplication {
    pub unit_name: Option<String>,
}

#[cfg(any(target_os = "linux", test))]
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

#[cfg(all(not(target_os = "linux"), not(test)))]
pub async fn stop_application(_unit_name: Option<&str>) -> Result<()> {
    anyhow::bail!("application session cannot be stopped on this platform")
}

#[cfg(any(target_os = "linux", test))]
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

#[cfg(all(not(target_os = "linux"), not(test)))]
pub async fn application_is_running(_unit_name: Option<&str>) -> Result<bool> {
    Ok(false)
}

#[cfg(all(not(target_os = "linux"), not(test)))]
pub async fn launch_heroic_game(
    _runner: HeroicRunner,
    _application_id: &str,
) -> Result<LaunchedApplication> {
    anyhow::bail!("Heroic game launch is unsupported on this platform")
}

#[cfg(all(not(target_os = "linux"), not(test)))]
pub async fn launch_retro_game(
    _core_path: &str,
    _rom_path: &str,
    _session_id: &str,
) -> Result<LaunchedApplication> {
    anyhow::bail!("RetroArch game launch is unsupported on this platform")
}

#[cfg(all(not(any(target_os = "linux", target_os = "macos")), not(test)))]
pub async fn discover_applications(_source_id: &str) -> Result<Vec<DiscoveredApplication>> {
    anyhow::bail!("application discovery is unsupported on this platform")
}

#[cfg(all(not(any(target_os = "linux", target_os = "macos")), not(test)))]
pub async fn launch_application(
    _source_id: &str,
    _application_id: &str,
    _session_id: &str,
) -> Result<LaunchedApplication> {
    anyhow::bail!("application launch is unsupported on this platform")
}
