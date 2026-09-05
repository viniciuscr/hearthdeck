use anyhow::{Context, Result};
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use hearthdeck_protocol::DiscoveredApplication;
#[cfg(all(not(target_os = "linux"), not(test)))]
use hearthdeck_protocol::HeroicRunner;
#[cfg(any(target_os = "linux", test))]
use std::collections::BTreeSet;

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
const TRACKED_UNIT_ENVIRONMENT_VARIABLE: &str = "HEARTHDECK_UNIT_NAME";

#[cfg(any(target_os = "linux", test))]
pub async fn stop_application(unit_name: Option<&str>) -> Result<()> {
    let Some(unit_name) = unit_name else {
        anyhow::bail!("application session cannot be stopped on this platform")
    };

    let escaped_units = find_escaped_units(unit_name).await;
    let stop_primary = async {
        if is_unit_active(unit_name).await? {
            stop_unit(unit_name).await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    let stop_escaped = async {
        for escaped_unit in &escaped_units {
            if is_unit_active(escaped_unit).await? {
                stop_unit(escaped_unit).await.with_context(|| {
                    format!("could not stop escaped application unit {escaped_unit}")
                })?;
                tracing::info!(
                    unit = %escaped_unit,
                    "stopped a unit the launched process had escaped into"
                );
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    let (primary, escaped) = tokio::join!(stop_primary, stop_escaped);
    primary?;
    escaped?;
    anyhow::ensure!(
        !application_is_running(Some(unit_name)).await?,
        "application processes are still running after stop"
    );
    Ok(())
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

/// Finds app scopes containing processes launched for `unit_name`.
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
/// Every supervised launch inherits `HEARTHDECK_UNIT_NAME`, so scanning
/// same-user `/proc` entries still attributes the process tree after systemd
/// has cleared the original service's `MainPID`. The `MainPID` lookup stays
/// as a compatibility path for applications launched before that marker was
/// added.
#[cfg(any(target_os = "linux", test))]
async fn find_escaped_units(unit_name: &str) -> Vec<String> {
    let mut escaped_units = BTreeSet::new();

    if let Some(pid) = unit_main_pid(unit_name).await
        && let Some(escaped_unit) = escaped_unit_for_process(pid, unit_name).await
    {
        escaped_units.insert(escaped_unit);
    }

    let marker = format!("{TRACKED_UNIT_ENVIRONMENT_VARIABLE}={unit_name}");
    let Ok(mut processes) = tokio::fs::read_dir("/proc").await else {
        return escaped_units.into_iter().collect();
    };
    while let Ok(Some(process)) = processes.next_entry().await {
        let Some(pid) = process
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(environment) = tokio::fs::read(process.path().join("environ")).await else {
            continue;
        };
        if environment_contains_entry(&environment, marker.as_bytes())
            && let Some(escaped_unit) = escaped_unit_for_process(pid, unit_name).await
        {
            escaped_units.insert(escaped_unit);
        }
    }

    escaped_units.into_iter().collect()
}

#[cfg(any(target_os = "linux", test))]
async fn unit_main_pid(unit_name: &str) -> Option<u32> {
    let output = tokio::process::Command::new("systemctl")
        .args(["--user", "show", unit_name, "--property=MainPID", "--value"])
        .output()
        .await
        .ok()?;
    let pid: u32 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;
    (pid != 0).then_some(pid)
}

#[cfg(any(target_os = "linux", test))]
async fn escaped_unit_for_process(pid: u32, unit_name: &str) -> Option<String> {
    let cgroup = tokio::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .await
        .ok()?;
    escaped_unit_from_cgroup(&cgroup, unit_name)
}

#[cfg(any(target_os = "linux", test))]
fn environment_contains_entry(environment: &[u8], entry: &[u8]) -> bool {
    environment
        .split(|byte| *byte == 0)
        .any(|value| value == entry)
}

/// Pure cgroup parsing used by `find_escaped_units`, split out so the logic is
/// unit-testable without needing a real systemd/proc environment.
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
    if is_unit_active(unit_name).await? {
        return Ok(true);
    }
    // A launched process can migrate into a different `app-<name>-<pid>.scope`
    // under app.slice than the `systemd-run` unit that started it (see
    // `find_escaped_unit`'s own docs). When that happens the original unit
    // reports inactive even though the app is very much still running, which
    // would make `active_managed_session` prune the session -- so the
    // overlay's "Close App" sees "no active session" and does nothing, and
    // launching a second app can't see the first one to stop it. Mirror the
    // stop path and treat an escaped, still-active scope as running too.
    for escaped_unit in find_escaped_units(unit_name).await {
        if is_unit_active(&escaped_unit).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(any(target_os = "linux", test))]
async fn is_unit_active(unit_name: &str) -> Result<bool> {
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
    use super::{environment_contains_entry, escaped_unit_from_cgroup};

    #[test]
    fn matches_only_the_exact_nul_delimited_launch_marker() {
        let environment = b"PATH=/usr/bin\0HEARTHDECK_UNIT_NAME=hearthdeck-app-abc.service\0";

        assert!(environment_contains_entry(
            environment,
            b"HEARTHDECK_UNIT_NAME=hearthdeck-app-abc.service"
        ));
        assert!(!environment_contains_entry(
            environment,
            b"HEARTHDECK_UNIT_NAME=hearthdeck-app-ab.service"
        ));
    }

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
