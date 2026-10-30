//! Installing libretro core packages through the polkit-scoped helper.
//!
//! This module is the whole privileged path on the bridge side: package names
//! are validated against one narrow shape (the same one the daemon computes
//! from its core table and the helper re-checks), and the only program pkexec
//! is asked to run is the allowlisted helper below — polkit action
//! `org.hearthdeck.core.install` names exactly this path, so no other program
//! inherits the rule.

use anyhow::{Context, Result};
use hearthdeck_protocol::{RetroCoreInstallFailure, RetroCoreInstallReport};

/// The helper pkexec may run: the polkit policy's
/// `org.freedesktop.policykit.exec.path` annotation names this exact path,
/// and `the_polkit_policy_names_the_helper_the_bridge_runs` keeps the two in
/// sync.
const INSTALL_HELPER: &str = "/usr/lib/hearthdeck/hearthdeck-install-cores";

/// The one shape of package name the helper may install: `libretro-` and a
/// lowercase alphanumeric tail. The daemon names packages from its own
/// platform-to-core table; this re-check is the bridge's own boundary, and
/// the helper checks again before pacman sees anything.
pub(crate) fn validate_install_packages(packages: &[String]) -> Result<Vec<String>> {
    anyhow::ensure!(!packages.is_empty(), "no core packages to install");
    for package in packages {
        let name = package.strip_prefix("libretro-").unwrap_or_default();
        anyhow::ensure!(
            !name.is_empty()
                && name.len() <= 64
                && name.chars().all(|character| character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || character == '-'),
            "{package} is not an installable core package name"
        );
    }
    Ok(packages.to_vec())
}

/// Runs the helper and turns its line protocol into a per-package report.
/// Thin on purpose: everything decided here is decided in
/// `validate_install_packages` and `parse_helper_report`.
pub(crate) async fn install_cores(packages: &[String]) -> Result<RetroCoreInstallReport> {
    let output = tokio::process::Command::new("pkexec")
        .arg(INSTALL_HELPER)
        .args(packages)
        .output()
        .await
        .context("could not run the core install helper")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // A refused pkexec (no authorization, helper missing) prints nothing on
    // stdout; a helper that ran reports one line per package even when some
    // failed. Only the empty case is "this never ran".
    anyhow::ensure!(
        output.status.success() || !stdout.trim().is_empty(),
        "the core install helper was refused or did not run"
    );
    Ok(parse_helper_report(&stdout, packages))
}

/// The helper's line protocol: `ok <package>` or `fail <package> <reason>`,
/// one line per package it tried. A package with no line of its own is
/// reported as a failure rather than silently dropped.
pub(crate) fn parse_helper_report(output: &str, requested: &[String]) -> RetroCoreInstallReport {
    let mut installed = Vec::new();
    let mut failed = Vec::new();
    for package in requested {
        let outcome = output.lines().find_map(|line| {
            if let Some(rest) = line.strip_prefix("ok ")
                && rest.trim() == package
            {
                return Some(Ok(()));
            }
            if let Some(rest) = line.strip_prefix("fail ")
                && let Some((name, reason)) = rest.trim().split_once(' ')
                && name == package
            {
                return Some(Err(reason.trim().to_owned()));
            }
            None
        });
        match outcome {
            Some(Ok(())) => installed.push(package.clone()),
            Some(Err(reason)) => failed.push(RetroCoreInstallFailure {
                package: package.clone(),
                reason,
            }),
            None => failed.push(RetroCoreInstallFailure {
                package: package.clone(),
                reason: "the helper reported nothing for this package".to_owned(),
            }),
        }
    }
    RetroCoreInstallReport { installed, failed }
}

#[cfg(test)]
mod tests {
    use super::{INSTALL_HELPER, parse_helper_report, validate_install_packages};

    #[test]
    fn only_libretro_package_names_pass_the_bridge() {
        for name in [
            "libretro-snes9x",
            "libretro-beetle-psx",
            "libretro-mupen64plus-next",
        ] {
            assert!(
                validate_install_packages(&[name.to_owned()]).is_ok(),
                "{name} should pass"
            );
        }
        for name in [
            "bash",
            "libretro-",
            "libretro-FOO",
            "libretro-a/b",
            "libretro-a b",
            "libretro-..",
            "--help",
            "pacman",
        ] {
            assert!(
                validate_install_packages(&[name.to_owned()]).is_err(),
                "{name} should be refused"
            );
        }
        assert!(validate_install_packages(&[]).is_err());
    }

    #[test]
    fn helper_lines_become_a_per_package_report() {
        let requested = [
            "libretro-snes9x".to_owned(),
            "libretro-mame".to_owned(),
            "libretro-flycast".to_owned(),
        ];
        let report = parse_helper_report(
            "ok libretro-snes9x\nfail libretro-mame pacman-failed\n",
            &requested,
        );
        assert_eq!(report.installed, vec!["libretro-snes9x".to_owned()]);
        assert_eq!(report.failed.len(), 2);
        assert_eq!(report.failed[0].package, "libretro-mame");
        assert_eq!(report.failed[0].reason, "pacman-failed");
        assert_eq!(report.failed[1].package, "libretro-flycast");
        assert_eq!(
            report.failed[1].reason,
            "the helper reported nothing for this package"
        );
    }

    /// The polkit policy must name exactly the helper path the bridge asks
    /// pkexec to run, and the PKGBUILD must ship both — otherwise the
    /// authorization silently covers a different program, or no program.
    #[test]
    fn the_polkit_policy_names_the_helper_the_bridge_runs() {
        let policy = include_str!("../../../packaging/arch/org.hearthdeck.core.install.policy");
        assert!(policy.contains(INSTALL_HELPER));

        let pkgbuild = include_str!("../../../packaging/arch/PKGBUILD");
        assert!(pkgbuild.contains("scripts/hearthdeck-install-cores"));
        assert!(pkgbuild.contains("org.hearthdeck.core.install.policy"));
        assert!(pkgbuild.contains("10-hearthdeck-cores.rules"));
    }
}
