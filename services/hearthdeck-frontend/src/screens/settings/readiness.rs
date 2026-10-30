//! The emulator-readiness card on the Settings screen: which platforms can
//! play — core installed, BIOS in place — and the buttons that check again or
//! fetch the missing BIOS files from the user's own BIOS source. The daemon
//! owns the facts (`/v1/retro/readiness`); this module formats them and names
//! the requests.

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::theme;
use cosmic::widget::row;

use crate::app::Message;
use crate::fl;
use crate::providers::daemon::{RetroBiosSyncReport, RetroCoreInstallReport, RetroSystemReadiness};
use crate::style::{primary_action_button_class, standard_button_class};

use super::{action_button, body_line, button_row, caption_line, titled_section};

/// The card's state: the daemon's answer, whether a sync is in flight, and
/// what the last request did.
#[derive(Default)]
pub struct Flow {
    /// The readiness report, once it has landed. Until then the card says it
    /// is checking, like the categorization section beside it.
    pub readiness: Option<Vec<RetroSystemReadiness>>,
    /// A BIOS sync request is in flight; the buttons wait for it.
    pub syncing: bool,
    /// A core install request is in flight; the buttons wait for it.
    pub installing: bool,
    /// What the last sync did, for the card's status text.
    pub notice: Option<String>,
}

/// One line for the whole picture: how many platforms can play, and how many
/// BIOS files are still missing.
pub fn summary_line(readiness: &[RetroSystemReadiness]) -> String {
    let ready = readiness
        .iter()
        .filter(|entry| platform_missing(entry).is_empty())
        .count();
    let missing_bios = readiness
        .iter()
        .flat_map(|entry| &entry.bios)
        .filter(|file| !file.present)
        .count();
    let mut line = format!("{ready} of {} platforms ready", readiness.len());
    if missing_bios > 0 {
        line.push_str(&format!(" · {missing_bios} BIOS files missing"));
    }
    line
}

/// What one platform is still missing: its core package and any required BIOS
/// files. Empty means it can play.
pub fn platform_missing(entry: &RetroSystemReadiness) -> Vec<String> {
    let mut missing = Vec::new();
    if !entry.core.installed {
        // A core no distro package provides shows its file name: that is
        // what the sync fetches from the user's source.
        let name = if entry.core.package.is_empty() {
            entry.core.core_filename.as_str()
        } else {
            entry.core.package.as_str()
        };
        missing.push(format!("core {name}"));
    }
    for file in &entry.bios {
        if file.required && !file.present {
            missing.push(file.filename.clone());
        }
    }
    missing
}

/// The per-platform status line: what is missing, or that the platform is
/// ready — with present-but-unverified files called out either way.
pub fn platform_status(entry: &RetroSystemReadiness) -> String {
    let missing = platform_missing(entry);
    let unverified: Vec<&str> = entry
        .bios
        .iter()
        .filter(|file| file.present && !file.verified)
        .map(|file| file.filename.as_str())
        .collect();
    let mut line = if missing.is_empty() {
        "ready".to_owned()
    } else {
        format!("missing: {}", missing.join(", "))
    };
    if !unverified.is_empty() {
        line.push_str(&format!(" · not verified: {}", unverified.join(", ")));
    }
    line
}

/// What a sync did: which files it placed, and how many it could not.
/// The tail every notice gets when files failed: the count, and the first
/// failure's name and reason. A bare "1 failed" leaves the user as much in
/// the dark as no message at all.
fn failure_tail(what: &str, failures: &[(&str, &str)]) -> String {
    let Some(((name, reason), rest)) = failures.split_first() else {
        return String::new();
    };
    if rest.is_empty() {
        format!(" · 1 {what}: {name} — {reason}")
    } else {
        format!(
            " · {} {what}: {name} — {reason}, and {} more",
            failures.len(),
            rest.len()
        )
    }
}

pub fn sync_notice(report: &RetroBiosSyncReport) -> String {
    let mut line = if report.fetched.is_empty() {
        "nothing to fetch".to_owned()
    } else {
        format!("placed {}", report.fetched.join(", "))
    };
    let failures: Vec<(&str, &str)> = report
        .failed
        .iter()
        .map(|failure| (failure.filename.as_str(), failure.reason.as_str()))
        .collect();
    line.push_str(&failure_tail("could not be placed", &failures));
    line
}

/// The core packages still missing across the report, sorted and deduped:
/// what the install button would ask for.
pub fn missing_core_packages(readiness: &[RetroSystemReadiness]) -> Vec<String> {
    let mut packages: Vec<String> = readiness
        .iter()
        .filter(|entry| !entry.core.installed)
        .map(|entry| entry.core.package.clone())
        .filter(|package| !package.is_empty())
        .collect();
    packages.sort();
    packages.dedup();
    packages
}

/// What an install run did: which packages landed, and how many could not.
pub fn install_notice(report: &RetroCoreInstallReport) -> String {
    let mut line = if report.installed.is_empty() {
        "no cores to install".to_owned()
    } else {
        format!("installed {}", report.installed.join(", "))
    };
    let failures: Vec<(&str, &str)> = report
        .failed
        .iter()
        .map(|failure| (failure.package.as_str(), failure.reason.as_str()))
        .collect();
    line.push_str(&failure_tail("could not be installed", &failures));
    line
}

/// What the card says while a request is in flight. The press must always
/// leave something on screen: a slow or dead network behind a silent row of
/// disabled buttons reads as a broken button.
pub fn progress_line(syncing: bool, installing: bool) -> Option<String> {
    match (syncing, installing) {
        (true, _) => Some("Fetching from your source…".to_owned()),
        (_, true) => Some("Installing cores…".to_owned()),
        (false, false) => None,
    }
}

/// The card: the summary, one line per platform, and the check/fetch buttons.
pub fn view<'a>(state: &Flow) -> Element<'a, Message> {
    let Spacing { space_s, .. } = theme::spacing();
    let section = titled_section(fl!("readiness-heading"));

    let Some(readiness) = &state.readiness else {
        return section.add(body_line(fl!("readiness-checking"))).into();
    };

    let mut section = section
        .add(body_line(summary_line(readiness)))
        .add(caption_line(fl!("readiness-intro")));
    for entry in readiness {
        section = section.add(body_line(format!(
            "{} — {}",
            entry.display_name,
            platform_status(entry)
        )));
    }
    if let Some(progress) = progress_line(state.syncing, state.installing) {
        section = section.add(caption_line(progress));
    } else if let Some(notice) = &state.notice {
        section = section.add(caption_line(notice.clone()));
    }
    let missing_packages = missing_core_packages(readiness);
    let buttons = row![
        action_button(
            fl!("readiness-check"),
            Message::RefreshRetroReadiness,
            standard_button_class(),
            !state.syncing && !state.installing,
        ),
        action_button(
            fl!("readiness-install"),
            Message::InstallRetroCores,
            primary_action_button_class(),
            !state.syncing && !state.installing && !missing_packages.is_empty(),
        ),
        action_button(
            fl!("readiness-sync"),
            Message::SyncRetroBios,
            primary_action_button_class(),
            !state.syncing && !state.installing,
        ),
    ]
    .spacing(space_s);
    section.add(button_row(buttons)).into()
}

#[cfg(test)]
mod tests {
    use super::{
        install_notice, missing_core_packages, platform_missing, platform_status, progress_line,
        summary_line, sync_notice,
    };
    use hearthdeck_protocol::{
        RetroBiosFile, RetroBiosSyncFailure, RetroBiosSyncReport, RetroCoreInstallFailure,
        RetroCoreInstallReport, RetroCoreReadiness, RetroSystemReadiness,
    };

    /// `bios` rows are `(filename, required, present, verified)`.
    fn entry(installed: bool, bios: &[(&str, bool, bool, bool)]) -> RetroSystemReadiness {
        RetroSystemReadiness {
            platform: "ps2".to_owned(),
            display_name: "PlayStation 2".to_owned(),
            core: RetroCoreReadiness {
                core_filename: "pcsx2_libretro.so".to_owned(),
                package: String::new(),
                installed,
            },
            bios: bios
                .iter()
                .map(|(filename, required, present, verified)| RetroBiosFile {
                    system: "Sony/PlayStation 2".to_owned(),
                    filename: (*filename).to_owned(),
                    required: *required,
                    present: *present,
                    verified: *verified,
                    target: format!("pcsx2/bios/{filename}"),
                })
                .collect(),
        }
    }

    #[test]
    fn a_platform_with_its_core_and_verified_bios_is_ready() {
        let ready = entry(true, &[("SCPH-70004.BIN", true, true, true)]);
        assert!(platform_missing(&ready).is_empty());
        assert_eq!(platform_status(&ready), "ready");
    }

    #[test]
    fn a_missing_core_and_required_bios_are_named() {
        let not_ready = entry(false, &[("SCPH-*.bin", true, false, false)]);
        assert_eq!(
            platform_missing(&not_ready),
            vec!["core pcsx2_libretro.so".to_owned(), "SCPH-*.bin".to_owned()]
        );
        assert_eq!(
            platform_status(&not_ready),
            "missing: core pcsx2_libretro.so, SCPH-*.bin"
        );
    }

    #[test]
    fn optional_bios_stays_ready_and_unverified_copies_are_called_out() {
        let optional_absent = entry(true, &[("scph5501.bin", false, false, false)]);
        assert_eq!(platform_status(&optional_absent), "ready");

        let unverified = entry(true, &[("scph5501.bin", false, true, false)]);
        assert_eq!(
            platform_status(&unverified),
            "ready · not verified: scph5501.bin"
        );
    }

    #[test]
    fn the_install_button_offers_only_packaged_cores() {
        // `entry()` names a source-only core (no package): it belongs to the
        // sync, not the install button.
        let report = vec![entry(true, &[]), entry(false, &[]), entry(false, &[])];
        assert!(missing_core_packages(&report).is_empty());

        let packaged = RetroSystemReadiness {
            core: RetroCoreReadiness {
                core_filename: "snes9x_libretro.so".to_owned(),
                package: "libretro-snes9x".to_owned(),
                installed: false,
            },
            ..entry(false, &[])
        };
        assert_eq!(
            missing_core_packages(&[packaged]),
            vec!["libretro-snes9x".to_owned()]
        );
    }

    #[test]
    fn the_install_notice_names_what_landed_and_why_the_rest_failed() {
        let report = RetroCoreInstallReport {
            installed: vec!["libretro-snes9x".to_owned()],
            failed: vec![RetroCoreInstallFailure {
                package: "libretro-mame".to_owned(),
                reason: "pacman-failed".to_owned(),
            }],
        };
        assert_eq!(
            install_notice(&report),
            "installed libretro-snes9x · 1 could not be installed: libretro-mame — pacman-failed"
        );

        let empty = RetroCoreInstallReport {
            installed: Vec::new(),
            failed: Vec::new(),
        };
        assert_eq!(install_notice(&empty), "no cores to install");
    }

    #[test]
    fn the_summary_counts_ready_platforms_and_missing_bios() {
        let report = vec![
            entry(true, &[("a.bin", true, true, true)]),
            entry(false, &[("b.bin", true, false, false)]),
            entry(true, &[("c.bin", false, false, false)]),
        ];
        // Two files are absent (one required, one optional) and the sync
        // fetches both, so the summary counts both.
        assert_eq!(
            summary_line(&report),
            "2 of 3 platforms ready · 2 BIOS files missing"
        );
        assert_eq!(summary_line(&[entry(true, &[])]), "1 of 1 platforms ready");
    }

    #[test]
    fn the_card_says_it_is_working_while_a_request_is_in_flight() {
        assert_eq!(
            progress_line(true, false).as_deref(),
            Some("Fetching from your source…")
        );
        assert_eq!(
            progress_line(false, true).as_deref(),
            Some("Installing cores…")
        );
        assert_eq!(progress_line(false, false), None);
    }

    #[test]
    fn the_sync_notice_names_what_was_placed_and_why_the_rest_failed() {
        let report = RetroBiosSyncReport {
            fetched: vec!["syscard3.pce".to_owned()],
            failed: vec![RetroBiosSyncFailure {
                filename: "neogeo.zip".to_owned(),
                reason: "no matching file in the source".to_owned(),
            }],
            readiness: Vec::new(),
        };
        assert_eq!(
            sync_notice(&report),
            "placed syscard3.pce · 1 could not be placed: neogeo.zip — no matching file in the source"
        );

        // Several failures still name the first one and count the rest.
        let two = RetroBiosSyncReport {
            fetched: Vec::new(),
            failed: vec![
                RetroBiosSyncFailure {
                    filename: "neogeo.zip".to_owned(),
                    reason: "no matching file in the source".to_owned(),
                },
                RetroBiosSyncFailure {
                    filename: "pcsx2_libretro.so".to_owned(),
                    reason: "no file in the source's cores/ folder".to_owned(),
                },
            ],
            readiness: Vec::new(),
        };
        assert_eq!(
            sync_notice(&two),
            "nothing to fetch · 2 could not be placed: neogeo.zip — no matching file in the source, and 1 more"
        );

        let empty = RetroBiosSyncReport {
            fetched: Vec::new(),
            failed: Vec::new(),
            readiness: Vec::new(),
        };
        assert_eq!(sync_notice(&empty), "nothing to fetch");
    }
}
