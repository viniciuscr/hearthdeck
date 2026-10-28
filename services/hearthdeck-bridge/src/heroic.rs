//! Launching games through Heroic.
//!
//! Heroic itself - not a specific game - is the resource Hearthdeck manages
//! here, under one stable (not per-launch) unit name. Electron single-instance
//! locking means a second `heroic://launch` URI while Heroic is still running
//! is handled by that *same* already-running process, not a fresh one we'd
//! separately track; reusing one unit name (and checking whether it's already
//! active before deciding whether to start it) is what keeps every launch
//! attributable to a unit Hearthdeck can reliably stop later, regardless of
//! how many games have been launched through it. Heroic is left running
//! between games on purpose (faster subsequent launches); closing it (and
//! whatever game it's running) is `stop_application(Some(HEROIC_UNIT_NAME))`,
//! which is cgroup-based via `systemctl --user stop` - see `launch_heroic_game`
//! below for why Heroic must be exec'd directly (not via `xdg-open`) for that
//! stop to actually reach the running game, not just Heroic's own launcher.

use std::ffi::OsString;

use anyhow::{Context, Result};
use hearthdeck_protocol::HeroicRunner;
use tokio::process::Command;

use crate::systemd::{LaunchedApplication, application_is_running, launch_with_systemd};

/// The unit Heroic runs under - see the module docs for why it is one stable
/// name reused across game launches rather than one per launch.
pub(crate) const HEROIC_UNIT_NAME: &str = "hearthdeck-heroic.service";

/// Delegate game startup to Heroic so its configured Wine, Proton, UMU,
/// wrappers, save sync, and per-game launch options remain authoritative.
pub(crate) async fn launch_heroic_game(
    runner: HeroicRunner,
    application_id: &str,
) -> Result<LaunchedApplication> {
    if !valid_heroic_application_id(application_id) {
        anyhow::bail!("Heroic application identifier is invalid")
    }
    let runner = match runner {
        HeroicRunner::Legendary => "legendary",
        HeroicRunner::Gog => "gog",
    };
    let uri = format!("heroic://launch?appName={application_id}&runner={runner}&gui=false");

    if application_is_running(Some(HEROIC_UNIT_NAME)).await? {
        // Heroic is already up and holds Electron's single-instance lock: a
        // direct `heroic <uri>` invocation here is detected as a second
        // instance and forwards this argv to the running primary over
        // Electron's own (non-D-Bus) IPC, then exits - matching the
        // cold-start branch's fast-return semantics below. Spawn, don't
        // await: tokio reaps the exited relay process in the background, so
        // this does not leak zombies.
        Command::new("heroic")
            .arg("--no-gui")
            .arg(&uri)
            .spawn()
            .context("could not ask the running Heroic instance to launch the game")?;
        return Ok(LaunchedApplication {
            unit_name: Some(HEROIC_UNIT_NAME.to_owned()),
        });
    }

    // Cold start: exec the `heroic` binary directly - NOT `xdg-open`.
    // Confirmed on real hardware (see docs/arch-package.md): `xdg-open
    // heroic://...` resolves this custom URI scheme through `gio
    // open`/`gio launch` (GLib's desktop-file launcher), which has its own
    // systemd integration (https://systemd.io/DESKTOP_ENVIRONMENTS/) and
    // unconditionally starts registered .desktop apps in a *new*
    // `app-<name>-<pid>.scope` under app.slice - migrating Heroic, and
    // everything it spawns (wineserver, the game itself), out of whatever
    // cgroup launched it. That silently orphaned the whole process tree
    // from hearthdeck-heroic.service's cgroup: only `xdg-open` itself and a
    // couple of early zygote helpers stayed put, so stopping the unit never
    // reached the actual game. Heroic reads the launch URI straight from
    // argv (see its own src/backend/protocol.ts), so it never needed
    // xdg-open/gio at all - invoking it directly keeps the whole tree
    // inside the cgroup we track, the same way desktop apps and RetroArch
    // already work. `--no-gui` is Heroic's own confirmed-supported CLI flag
    // for suppressing its window (belt-and-suspenders with the URL's
    // `gui=false`, and it also makes Heroic exit itself once the game
    // exits, instead of only the `gui=false` query param's window-hide).
    let command = vec![
        OsString::from("heroic"),
        OsString::from("--no-gui"),
        OsString::from(uri),
    ];
    launch_with_systemd(HEROIC_UNIT_NAME, command, None).await
}

fn valid_heroic_application_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, b'.' | b'_' | b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::valid_heroic_application_id;

    #[test]
    fn validates_heroic_application_identifiers_before_uri_construction() {
        assert!(valid_heroic_application_id("Fortnite"));
        assert!(valid_heroic_application_id("1091500"));
        assert!(!valid_heroic_application_id("Fortnite&runner=gog"));
        assert!(!valid_heroic_application_id("../Fortnite"));
    }
}
