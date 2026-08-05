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
pub use linux::{
    HEROIC_UNIT_NAME, discover_applications, launch_application, launch_heroic_game,
    launch_retro_game,
};

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

    if unit_name == HEROIC_UNIT_NAME {
        stop_heroic_app_scopes().await;
    }
    Ok(())
}

/// Best-effort cleanup for a scope escaping `hearthdeck-heroic.service`'s
/// cgroup.
///
/// ponytail: confirmed twice on real hardware, via `/proc/<pid>/cgroup`, that
/// Heroic's own process (and whatever it launches) can end up in a
/// `app-heroic-<pid>.scope` under `app.slice` instead of the
/// `hearthdeck-heroic.service` cgroup `systemctl --user stop` above just
/// killed - even after switching Heroic's own launch to a direct `exec`
/// (see the commit removing `xdg-open`). The exact trigger inside
/// Heroic/Electron/Wine/Proton for this specific case was not pinned down
/// (no strace access on the target hardware) - most likely Electron's own
/// GLib/D-Bus application registration for tray/notification support
/// triggers the same systemd desktop-environment convention `xdg-open`/`gio
/// launch` uses (see https://systemd.io/DESKTOP_ENVIRONMENTS/), independent
/// of how the process was originally spawned. Rather than block on fully
/// root-causing that, this targets the *observed, reproducible* symptom
/// directly: stop any leftover `app-heroic-*.scope` unit after stopping the
/// tracked service, so whatever escaped it (Heroic itself, or the Wine/Proton
/// tree underneath it) still gets torn down. `systemctl` unit-name globs
/// silently match zero units when none exist, so this is a no-op the rest of
/// the time. Upgrade path if this class of bug recurs for non-Heroic
/// launches too: capture and stop `app-*-<pid>.scope` generically instead of
/// hardcoding "heroic".
#[cfg(any(target_os = "linux", test))]
async fn stop_heroic_app_scopes() {
    match tokio::process::Command::new("systemctl")
        .args(["--user", "stop", "app-heroic-*.scope"])
        .status()
        .await
    {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::warn!(
            ?status,
            "could not stop leftover app-heroic-*.scope units after closing Heroic"
        ),
        Err(error) => tracing::warn!(
            %error,
            "could not run systemctl to stop leftover app-heroic-*.scope units"
        ),
    }
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
