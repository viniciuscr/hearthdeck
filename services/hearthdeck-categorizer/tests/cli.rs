//! End-to-end tests over the real binary.
//!
//! They spawn the same command line the daemon will and then read the report the
//! process wrote to `--output`, so the argument contract and the JSON on disk are
//! both covered. Everything here stays on `--engine heuristic` so the suite runs
//! with `--no-default-features` too, where no `laya` feature (and no checkpoint)
//! exists.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use hearthdeck_categorizer::{ScanReport, Section};

/// One freedesktop `Game`, one app with a `Development` category, one emulator
/// that declares nothing, and one record that says nothing the categorizer can
/// use at all.
const LIBRARY: &str = r#"[
    {
        "id": "steam:440",
        "title": "Team Fortress 2",
        "kind": "game",
        "categories": ["Game"]
    },
    {
        "id": "org.kde.kdevelop.desktop",
        "title": "KDevelop",
        "categories": ["Development"]
    },
    {
        "id": "org.libretro.RetroArch.desktop",
        "title": "RetroArch",
        "exec": "org.libretro.RetroArch.desktop"
    },
    {
        "id": "com.example.Mystery.desktop",
        "title": "Mystery"
    }
]"#;

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hearthdeck-categorizer"))
        .args(args)
        .output()
        .expect("failed to spawn hearthdeck-categorizer")
}

fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("failed to write scratch file");
    path
}

fn text(path: &Path) -> String {
    std::fs::read_to_string(path).expect("failed to read scratch file")
}

#[test]
fn a_heuristic_scan_writes_a_report() {
    let dir = tempfile::tempdir().expect("tempdir");
    let library = write(dir.path(), "library.json", LIBRARY);
    let output = dir.path().join("report.json");

    let result = run(&[
        "scan",
        "--library",
        library.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--engine",
        "heuristic",
    ]);

    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let report: ScanReport = serde_json::from_str(&text(&output)).expect("report is valid JSON");

    assert_eq!(report.categorizer, "heuristic");
    assert_eq!(report.researcher, None);
    assert_eq!(report.app_count, 4);
    assert_eq!(report.assignments.len(), 4);

    let game = report.categorizations_for("steam:440").expect("game");
    assert_eq!(game.section, Section::PcGames);
    assert!(game.categories.is_empty());

    let developer = report
        .categorizations_for("org.kde.kdevelop.desktop")
        .expect("developer");
    assert_eq!(developer.section, Section::Applications);
    assert_eq!(developer.categories.len(), 1);
    assert_eq!(developer.categories[0].slug, "development");

    // An emulator is a console game with no category of its own, which is not
    // the same thing as an app the taxonomy has no answer for.
    let emulator = report
        .categorizations_for("org.libretro.RetroArch.desktop")
        .expect("emulator");
    assert_eq!(emulator.section, Section::ConsoleGames);
    assert!(emulator.categories.is_empty());
    assert!(!emulator.needs_category);

    assert_eq!(report.unclassified.len(), 1);
    assert_eq!(report.unclassified[0].app_id, "com.example.Mystery.desktop");
    assert_eq!(report.unclassified[0].title, "Mystery");

    let recommended: Vec<&str> = report
        .recommended_categories()
        .map(|category| category.slug.as_str())
        .collect();
    assert_eq!(recommended, ["development"]);
}

#[test]
fn a_scan_without_a_library_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = dir.path().join("report.json");

    let result = run(&["scan", "--output", output.to_str().unwrap()]);

    assert!(!result.status.success());
    assert!(!output.exists(), "no report should be written");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("--library"), "stderr: {stderr}");
    assert!(stderr.contains("usage:"), "stderr: {stderr}");
}

#[test]
fn a_command_without_the_scan_subcommand_is_a_usage_error() {
    let result = run(&[]);

    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("usage:"), "stderr: {stderr}");
}

#[test]
fn research_installs_the_noop_researcher() {
    let dir = tempfile::tempdir().expect("tempdir");
    let library = write(dir.path(), "library.json", LIBRARY);
    let output = dir.path().join("report.json");

    let result = run(&[
        "scan",
        "--library",
        library.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--engine",
        "heuristic",
        "--research",
    ]);

    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let json: serde_json::Value =
        serde_json::from_str(&text(&output)).expect("report is valid JSON");
    // `NoopResearcher::id` is "none" — research is wired up but answers nothing.
    assert_eq!(json["researcher"], "none");

    let report: ScanReport = serde_json::from_value(json).expect("report is a ScanReport");
    assert_eq!(report.researcher.as_deref(), Some("none"));
    assert_eq!(report.assignments.len(), 4);
}
