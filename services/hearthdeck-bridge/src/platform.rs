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

    let escaped_unit = find_escaped_unit(unit_name).await;
    let stop_primary = stop_unit(unit_name);
    let Some(escaped_unit) = escaped_unit else {
        return stop_primary.await;
    };

    // Independent units - `systemctl stop` blocks until each unit's
    // processes actually exit (SIGTERM, then wait for graceful shutdown),
    // so stopping them one after another meant waiting out that exit delay
    // twice in a row for no reason. Stopping them concurrently cuts the
    // total wait to whichever one is slower, instead of the sum of both -
    // this is what the overlay's "Closing app..." status waits on, so this
    // directly shortens how long that stays on screen when a launch has
    // escaped (see `find_escaped_unit`'s own docs).
    let (primary, escaped) = tokio::join!(stop_primary, stop_unit(&escaped_unit));
    match escaped {
        Ok(()) => tracing::info!(
            unit = %escaped_unit,
            "stopped a unit the launched process had escaped into"
        ),
        Err(error) => tracing::warn!(
            unit = %escaped_unit,
            %error,
            "could not stop a unit the launched process had escaped into"
        ),
    }
    primary
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

/// Detects whether the process tracked as `unit_name`'s main process has
/// been moved into a *different* systemd unit's cgroup than the one
/// `systemd-run` originally placed it in, and returns that other unit's
/// name if so.
///
/// ponytail: confirmed on real hardware, via `/proc/<pid>/cgroup`, that a
/// launched process (Heroic specifically, so far) can end up in an
/// `app-<name>-<pid>.scope` under `app.slice` instead of the
/// `hearthdeck-*.service` cgroup `systemd-run` placed it in - even when
/// exec'd directly (see the commit removing `xdg-open` for Heroic). The
/// exact trigger wasn't pinned down (no strace access on the target
/// hardware) - most likely Electron's own GLib/D-Bus application
/// registration for tray/notification support triggers the same systemd
/// desktop-environment convention `xdg-open`/`gio launch` uses (see
/// https://systemd.io/DESKTOP_ENVIRONMENTS/), independent of how the
/// process was originally spawned.
///
/// Rather than special-case Heroic by name (and need a new branch for every
/// future app source that turns out to do the same thing - Flatpak apps are
/// a known example of something else that self-scopes this way), this asks
/// the kernel directly: read the tracked main PID's *actual* current cgroup
/// and compare it to the unit we expect it to still be in. Works
/// identically for any launched app, with no per-app-name branching.
#[cfg(any(target_os = "linux", test))]
async fn find_escaped_unit(unit_name: &str) -> Option<String> {
    let output = tokio::process::Command::new("systemctl")
        .args(["--user", "show", unit_name, "--property=MainPID", "--value"])
        .output()
        .await
        .ok()?;
    let pid: u32 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;
    if pid == 0 {
        // Unit has no main process (already exited, or never had one) -
        // nothing to have escaped.
        return None;
    }

    let cgroup = tokio::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .await
        .ok()?;
    escaped_unit_from_cgroup(&cgroup, unit_name)
}

/// Pure parsing half of `find_escaped_unit`, split out so the cgroup-parsing
/// logic is unit-testable without needing a real systemd/proc environment.
#[cfg(any(target_os = "linux", test))]
fn escaped_unit_from_cgroup(cgroup: &str, unit_name: &str) -> Option<String> {
    // cgroup v2 (the only kind systemd-managed user sessions use here) has
    // exactly one line: "0::/full/cgroup/path".
    let path = cgroup.trim().strip_prefix("0::")?;
    let current_unit = path.rsplit('/').next()?;

    let escaped = current_unit != unit_name
        && current_unit.ends_with(".scope")
        && path.contains("/app.slice/");
    escaped.then(|| current_unit.to_owned())
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

#[cfg(test)]
mod tests {
    use super::escaped_unit_from_cgroup;

    #[test]
    fn detects_a_process_moved_into_an_app_slice_scope() {
        let cgroup =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-heroic-5892.scope\n";
        assert_eq!(
            escaped_unit_from_cgroup(cgroup, "hearthdeck-heroic.service").as_deref(),
            Some("app-heroic-5892.scope")
        );
    }

    #[test]
    fn is_generic_to_any_tracked_unit_name_not_just_heroic() {
        let cgroup = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.example.Whatever-42.scope\n";
        assert_eq!(
            escaped_unit_from_cgroup(cgroup, "hearthdeck-app-some-session.service").as_deref(),
            Some("app-org.example.Whatever-42.scope")
        );
    }

    #[test]
    fn ignores_a_process_still_in_its_own_tracked_unit() {
        let cgroup = "0::/user.slice/user-1000.slice/user@1000.service/hearthdeck.slice/hearthdeck-app-abc.service\n";
        assert_eq!(
            escaped_unit_from_cgroup(cgroup, "hearthdeck-app-abc.service"),
            None
        );
    }

    #[test]
    fn ignores_units_outside_app_slice() {
        // A different cgroup path that happens to end in ".scope" but isn't
        // the desktop-environment app-launch convention this targets.
        let cgroup =
            "0::/user.slice/user-1000.slice/user@1000.service/session.slice/session-3.scope\n";
        assert_eq!(
            escaped_unit_from_cgroup(cgroup, "hearthdeck-app-abc.service"),
            None
        );
    }
}
