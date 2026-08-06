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

    if unit_name == HEROIC_UNIT_NAME {
        log_heroic_app_scopes().await;
        // The tracked hearthdeck-heroic.service and whatever escaped into
        // app-heroic-*.scope (see stop_leftover_heroic_scope's own docs) are
        // independent units - `systemctl stop` blocks until each unit's
        // processes actually exit (SIGTERM, then wait for graceful
        // shutdown), so stopping them one after another meant waiting out
        // that exit delay twice in a row for no reason. Stopping them
        // concurrently cuts the total wait to whichever one is slower,
        // instead of the sum of both - this is what the overlay's "Closing
        // app..." status waits on, so this directly shortens how long that
        // stays on screen after a Heroic game closes.
        let (primary, scope) = tokio::join!(stop_unit(unit_name), stop_unit("app-heroic-*.scope"));
        match scope {
            Ok(()) => {
                tracing::info!(
                    "stopped any leftover app-heroic-*.scope units after closing Heroic"
                );
            }
            Err(error) => tracing::warn!(
                %error,
                "could not stop leftover app-heroic-*.scope units after closing Heroic"
            ),
        }
        primary
    } else {
        stop_unit(unit_name).await
    }
}

#[cfg(any(target_os = "linux", test))]
async fn stop_unit(unit_name: &str) -> Result<()> {
    let status = tokio::process::Command::new("systemctl")
        .args(["--user", "stop", unit_name])
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("systemd user manager rejected stop of {unit_name}")
    }
    Ok(())
}

/// Best-effort diagnostic for a scope escaping `hearthdeck-heroic.service`'s
/// cgroup.
///
/// ponytail: confirmed twice on real hardware, via `/proc/<pid>/cgroup`, that
/// Heroic's own process (and whatever it launches) can end up in a
/// `app-heroic-<pid>.scope` under `app.slice` instead of the
/// `hearthdeck-heroic.service` cgroup `stop_unit` above targets - even after
/// switching Heroic's own launch to a direct `exec` (see the commit removing
/// `xdg-open`). The exact trigger inside Heroic/Electron/Wine/Proton for
/// this specific case was not pinned down (no strace access on the target
/// hardware) - most likely Electron's own GLib/D-Bus application
/// registration for tray/notification support triggers the same systemd
/// desktop-environment convention `xdg-open`/`gio launch` uses (see
/// https://systemd.io/DESKTOP_ENVIRONMENTS/), independent of how the process
/// was originally spawned. Rather than block on fully root-causing that,
/// `stop_application` targets the *observed, reproducible* symptom directly
/// by also stopping `app-heroic-*.scope`; this function only logs what it
/// found beforehand, for the next time this needs debugging. `systemctl`
/// unit-name globs silently match zero units when none exist, so this is a
/// no-op the rest of the time. Upgrade path if this class of bug recurs for
/// non-Heroic launches too: capture and stop `app-*-<pid>.scope` generically
/// instead of hardcoding "heroic".
#[cfg(any(target_os = "linux", test))]
async fn log_heroic_app_scopes() {
    match tokio::process::Command::new("systemctl")
        .args([
            "--user",
            "list-units",
            "--no-legend",
            "--no-pager",
            "--plain",
            "app-heroic-*",
        ])
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let listing = String::from_utf8_lossy(&output.stdout);
            tracing::info!(
                matched_units = %listing.trim(),
                "checked for leftover app-heroic-*.scope units after closing Heroic"
            );
        }
        Ok(output) => tracing::warn!(
            status = ?output.status,
            "could not list app-heroic-*.scope units before stopping them"
        ),
        Err(error) => tracing::warn!(
            %error,
            "could not run systemctl to list app-heroic-*.scope units"
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
