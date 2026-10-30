//! Supervised launching and lifecycle for the transient systemd user units
//! every launch target runs under.
//!
//! The launch targets themselves live in their own modules (`desktop_apps`,
//! `stremio`, `heroic`, `retroarch`); what is shared is the unit machinery:
//! starting a command under `systemd-run`, and reliably stopping - or proving
//! still running - whatever a launch left behind.

use std::{collections::BTreeSet, env, ffi::OsString, path::PathBuf, process::Stdio};

use anyhow::{Context, Result};
use tokio::process::Command;
use tracing::info;

use crate::desktop_apps::current_desktops;

const TRACKED_UNIT_ENVIRONMENT_VARIABLE: &str = "HEARTHDECK_UNIT_NAME";

pub(crate) struct LaunchedApplication {
    pub(crate) unit_name: Option<String>,
}

pub(crate) async fn stop_application(unit_name: Option<&str>) -> Result<()> {
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
                info!(
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

async fn escaped_unit_for_process(pid: u32, unit_name: &str) -> Option<String> {
    let cgroup = tokio::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .await
        .ok()?;
    escaped_unit_from_cgroup(&cgroup, unit_name)
}

fn environment_contains_entry(environment: &[u8], entry: &[u8]) -> bool {
    environment
        .split(|byte| *byte == 0)
        .any(|value| value == entry)
}

/// Pure cgroup parsing used by `find_escaped_units`, split out so the logic is
/// unit-testable without needing a real systemd/proc environment.
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

pub(crate) async fn application_is_running(unit_name: Option<&str>) -> Result<bool> {
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

async fn is_unit_active(unit_name: &str) -> Result<bool> {
    Ok(tokio::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", unit_name])
        .status()
        .await?
        .success())
}

/// Starts `application_command` as a transient systemd user unit, so the
/// launch is supervised - and stoppable through the unit's own cgroup - the
/// same way every other session is.
pub(crate) async fn launch_with_systemd(
    unit_name: &str,
    application_command: Vec<OsString>,
    working_directory: Option<PathBuf>,
) -> Result<LaunchedApplication> {
    let mut command = Command::new("systemd-run");
    command
        .args([
            "--user",
            "--collect",
            "--quiet",
            "--service-type=exec",
            "--unit",
        ])
        .arg(unit_name)
        .arg("--working-directory")
        .arg(working_directory.unwrap_or_else(|| PathBuf::from("/")));
    if is_kiosk_session() {
        command.arg("--slice=hearthdeck-kiosk.slice");
    }
    command
        .arg("--setenv")
        .arg(format!("{}={unit_name}", TRACKED_UNIT_ENVIRONMENT_VARIABLE));
    // Forward the display/session environment the launched process needs to
    // connect to whatever session Hearthdeck itself is running in -
    // `systemd-run` does not inherit the caller's environment, and a
    // `systemd --user` unit's own default environment can be stale or
    // missing these entirely (Gamescope only ever assigns DISPLAY/
    // WAYLAND_DISPLAY to its own direct child - Hearthdeck itself - which is
    // why Hearthdeck's own startup imports them into systemd --user; see
    // linux/runner/my_application.cc). Reading them from the bridge's own
    // process environment and forwarding them explicitly is correct both
    // inside and outside the Kiosk session, so this no longer branches on
    // is_kiosk_session() the way it used to.
    for name in [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "GAMESCOPE_WAYLAND_DISPLAY",
        "XAUTHORITY",
        "GDK_BACKEND",
        "SDL_VIDEODRIVER",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "XDG_SESSION_TYPE",
        "DBUS_SESSION_BUS_ADDRESS",
    ] {
        if let Some(value) = env::var_os(name) {
            command
                .arg("--setenv")
                .arg(format!("{name}={}", value.to_string_lossy()));
        }
    }
    command.arg("--").args(application_command);
    let status = command
        .status()
        .await
        .context("could not start supervised application launch")?;
    if !status.success() {
        anyhow::bail!("systemd user manager rejected application launch")
    }
    Ok(LaunchedApplication {
        unit_name: Some(unit_name.to_owned()),
    })
}

fn is_kiosk_session() -> bool {
    current_desktops()
        .iter()
        .any(|desktop| desktop.eq_ignore_ascii_case("hearthdeck"))
}

/// The `systemd --user` unit [`restart_romm_service`] acts on. A named constant,
/// never built from request data: this is a narrowly scoped action on one
/// specific unit, not a generic "restart any unit" capability.
const ROMM_SERVICE_UNIT: &str = "romm.service";

/// The fixed argv that restarts it, kept as data so its scope is assertable in a
/// test without a systemd user session present - and without restarting a real
/// RomM stack as a side effect of running the test suite.
const ROMM_RESTART_ARGS: [&str; 3] = ["--user", "restart", ROMM_SERVICE_UNIT];

/// Maps a finished `systemctl restart` onto the caller's result. A missing unit
/// fails the same way `systemctl` itself reports it, surfaced to the caller
/// rather than silently ignored. `systemd` can also fail with nothing on stderr,
/// so that case still yields a message naming the unit that was rejected.
fn romm_restart_result(success: bool, stderr: &[u8]) -> Result<()> {
    if success {
        return Ok(());
    }
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    Err(if message.is_empty() {
        anyhow::anyhow!("systemd rejected the {ROMM_SERVICE_UNIT} restart")
    } else {
        anyhow::anyhow!("systemd rejected the {ROMM_SERVICE_UNIT} restart: {message}")
    })
}

/// Restarts the optional RomM systemd unit
/// (`deploy/systemd/romm.service`). The unit name is a fixed
/// constant, never caller-supplied: this is a narrowly scoped action on one
/// specific service, not a generic "restart any unit" capability. A missing
/// unit fails the same way `systemctl` itself reports it, surfaced to the
/// caller rather than silently ignored.
pub(crate) async fn restart_romm_service() -> Result<()> {
    let output = Command::new("systemctl")
        .args(ROMM_RESTART_ARGS)
        .stdin(Stdio::null())
        .output()
        .await?;
    romm_restart_result(output.status.success(), &output.stderr)
}

#[cfg(test)]
mod tests {
    use super::{environment_contains_entry, escaped_unit_from_cgroup};

    #[test]
    fn romm_restart_targets_the_one_fixed_unit() {
        // Locks in scope: this may only ever restart the single fixed unit, and
        // its name may not become a request parameter (the same discipline the
        // install-request boundary uses).
        assert_eq!(
            super::ROMM_RESTART_ARGS,
            ["--user", "restart", "romm.service"]
        );
    }

    #[test]
    fn romm_restart_surfaces_systemd_failures_instead_of_swallowing_them() {
        assert!(super::romm_restart_result(true, b"").is_ok());

        let reported = super::romm_restart_result(
            false,
            b"Failed to restart romm.service: Unit romm.service not found.\n",
        )
        .unwrap_err()
        .to_string();
        assert!(
            reported.contains("Unit romm.service not found"),
            "{reported}"
        );

        // systemd can fail with nothing useful on stderr; the message must still
        // name the unit that was rejected.
        let bare = super::romm_restart_result(false, b"  \n")
            .unwrap_err()
            .to_string();
        assert!(bare.contains("romm.service"), "{bare}");
    }

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
