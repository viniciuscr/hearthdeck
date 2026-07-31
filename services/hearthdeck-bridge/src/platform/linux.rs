use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use hearthdeck_protocol::{DiscoveredApplication, HeroicRunner};
use tokio::process::Command;
use tracing::{debug, info, warn};

use super::{DESKTOP_APPS_SOURCE, LaunchedApplication};

pub async fn discover_applications(source_id: &str) -> Result<Vec<DiscoveredApplication>> {
    if source_id != DESKTOP_APPS_SOURCE {
        anyhow::bail!("unsupported application source")
    }
    let directories = desktop_entry_directories();
    info!(directories = ?directories, "application discovery scanning desktop-entry directories");

    let mut entries = HashMap::new();
    for source_directory in directories {
        let mut directory = match tokio::fs::read_dir(&source_directory).await {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                warn!(directory = %source_directory.display(), %error, "application discovery could not read desktop-entry directory");
                continue;
            }
        };
        let mut desktop_entry_count = 0;
        let mut accepted_entry_count = 0;
        loop {
            let file = match directory.next_entry().await {
                Ok(Some(file)) => file,
                Ok(None) => break,
                Err(error) => {
                    warn!(%error, "application discovery could not enumerate desktop-entry directory");
                    break;
                }
            };
            let path = file.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "desktop")
            {
                desktop_entry_count += 1;
                match parse_desktop_entry(&path).await {
                    Ok(entry) => {
                        accepted_entry_count += 1;
                        debug!(
                            directory = %source_directory.display(),
                            application_id = %entry.application_id,
                            title = %entry.name,
                            "application discovery accepted desktop entry"
                        );
                        entries.entry(entry.application_id.clone()).or_insert(entry);
                    }
                    Err(error) => {
                        debug!(
                            desktop_entry = %path.display(),
                            %error,
                            "application discovery skipped desktop entry"
                        );
                    }
                }
            }
        }
        info!(
            directory = %source_directory.display(),
            desktop_entry_count,
            accepted_entry_count,
            "application discovery scanned desktop-entry directory"
        );
    }
    let mut entries: Vec<_> = entries.into_values().collect();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}

pub async fn launch_application(
    source_id: &str,
    application_id: &str,
    session_id: &str,
) -> Result<LaunchedApplication> {
    let path = desktop_entry_path(source_id, application_id).await?;
    let launch = command_for_desktop_entry(&path).await?;
    launch_with_systemd(launch.command, launch.working_directory, session_id, true).await
}

/// Delegate game startup to Heroic so its configured Wine, Proton, UMU,
/// wrappers, and per-game launch options remain authoritative.
pub async fn launch_heroic_game(
    runner: HeroicRunner,
    application_id: &str,
    session_id: &str,
) -> Result<LaunchedApplication> {
    if is_kiosk_session() {
        anyhow::bail!("Heroic game launches are unavailable in the Hearthdeck Kiosk session")
    }
    if !valid_heroic_application_id(application_id) {
        anyhow::bail!("Heroic application identifier is invalid")
    }
    let runner = match runner {
        HeroicRunner::Legendary => "legendary",
        HeroicRunner::Gog => "gog",
    };
    let uri = format!("heroic://launch?appName={application_id}&runner={runner}&gui=false");
    launch_with_systemd(
        vec![OsString::from("xdg-open"), OsString::from(uri)],
        None,
        session_id,
        false,
    )
    .await
}

async fn launch_with_systemd(
    application_command: Vec<OsString>,
    working_directory: Option<PathBuf>,
    session_id: &str,
    wrap_in_gamescope: bool,
) -> Result<LaunchedApplication> {
    let unit_name = format!("hearthdeck-app-{session_id}.service");
    let mut command = Command::new("systemd-run");
    command
        .args([
            "--user",
            "--collect",
            "--quiet",
            "--service-type=exec",
            "--unit",
        ])
        .arg(&unit_name)
        .arg("--working-directory")
        .arg(working_directory.unwrap_or_else(|| PathBuf::from("/")));
    let kiosk_session = is_kiosk_session();
    if kiosk_session {
        command.arg("--slice=hearthdeck-kiosk.slice");
    }
    let mut launch_environment = BTreeMap::new();
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
            launch_environment.insert(name, value);
        }
    }
    if let Some(wayland_display) = session_wayland_display() {
        launch_environment.insert("WAYLAND_DISPLAY", OsString::from(wayland_display));
    }
    for (name, value) in launch_environment {
        command
            .arg("--setenv")
            .arg(format!("{name}={}", value.to_string_lossy()));
    }
    command.arg("--");
    if kiosk_session && wrap_in_gamescope {
        command.args([
            "/usr/bin/gamescope",
            "--backend",
            "wayland",
            "--expose-wayland",
            "--force-windows-fullscreen",
            "-f",
            "--",
        ]);
    }
    let status = command
        .args(application_command)
        .status()
        .await
        .context("could not start supervised application launch")?;
    if !status.success() {
        anyhow::bail!("systemd user manager rejected application launch")
    }
    Ok(LaunchedApplication {
        unit_name: Some(unit_name),
    })
}

fn is_kiosk_session() -> bool {
    is_kiosk_session_for(&current_desktops())
}

fn is_kiosk_session_for(desktops: &[String]) -> bool {
    desktops
        .iter()
        .any(|desktop| desktop.eq_ignore_ascii_case("hearthdeck"))
}

fn session_wayland_display() -> Option<String> {
    let runtime_directory = env::var_os("XDG_RUNTIME_DIR")?;
    let value = std::fs::read_to_string(
        PathBuf::from(runtime_directory).join("hearthdeck/gamescope-wayland-display"),
    )
    .ok()?;
    let value = value.trim();
    valid_wayland_display(value).then(|| value.to_owned())
}

fn valid_wayland_display(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, b'.' | b'_' | b'-')
        })
}

fn valid_heroic_application_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, b'.' | b'_' | b'-')
        })
}

struct DesktopLaunch {
    command: Vec<OsString>,
    working_directory: Option<PathBuf>,
}

async fn desktop_entry_path(source_id: &str, application_id: &str) -> Result<PathBuf> {
    if source_id != DESKTOP_APPS_SOURCE {
        anyhow::bail!("unsupported application source")
    }
    for directory in desktop_entry_directories() {
        let path = directory.join(application_id);
        if path.is_file() && parse_desktop_entry(&path).await.is_ok() {
            return Ok(path);
        }
    }
    anyhow::bail!("desktop entry is not registered")
}

async fn command_for_desktop_entry(path: &Path) -> Result<DesktopLaunch> {
    let content = tokio::fs::read_to_string(path).await?;
    let values = desktop_entry_values(&content);
    validate_desktop_entry(&values)?;
    if desktop_entry_boolean(&values, "Terminal") {
        anyhow::bail!("terminal desktop entries are not supported in Kiosk mode")
    }
    if desktop_entry_boolean(&values, "DBusActivatable") {
        anyhow::bail!("D-Bus-activated desktop entries are not supported in a managed session")
    }
    let exec = values.get("Exec").context("desktop entry has no Exec")?;
    let command = parse_exec(
        exec,
        path,
        values.get("Name").copied(),
        values.get("Icon").copied(),
    )?;
    if command.is_empty() {
        anyhow::bail!("desktop entry has an empty Exec command")
    }
    if let Some(try_exec) = values.get("TryExec")
        && !executable_exists(try_exec)
    {
        anyhow::bail!("desktop entry TryExec is not executable")
    }
    Ok(DesktopLaunch {
        command: command.into_iter().map(OsString::from).collect(),
        working_directory: values
            .get("Path")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from),
    })
}

fn parse_exec(
    value: &str,
    desktop_path: &Path,
    application_name: Option<&str>,
    icon: Option<&str>,
) -> Result<Vec<String>> {
    let mut arguments = Vec::new();
    let mut argument = String::new();
    let mut quoted = false;
    let mut argument_was_quoted = false;
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if quoted {
            if character == '"' {
                quoted = false;
            } else if character == '\\' {
                let escaped = characters
                    .next()
                    .context("desktop entry has an invalid quoted Exec command")?;
                if !matches!(escaped, '"' | '`' | '$' | '\\') {
                    anyhow::bail!("desktop entry has an invalid quoted Exec escape")
                }
                argument.push(escaped);
            } else {
                argument.push(character);
            }
        } else if character == '\\' {
            let escaped = characters
                .next()
                .context("desktop entry has an invalid Exec command")?;
            argument.push(escaped);
        } else if character == '"' {
            quoted = true;
            argument_was_quoted = true;
        } else if character.is_whitespace() && !quoted {
            if !argument.is_empty() {
                arguments.push((std::mem::take(&mut argument), argument_was_quoted));
                argument_was_quoted = false;
            }
        } else {
            argument.push(character);
        }
    }
    if quoted {
        anyhow::bail!("desktop entry has an invalid Exec command")
    }
    if !argument.is_empty() {
        arguments.push((argument, argument_was_quoted));
    }
    let mut expanded = Vec::new();
    for (argument, was_quoted) in arguments {
        expand_field_codes(
            &mut expanded,
            &argument,
            was_quoted,
            desktop_path,
            application_name,
            icon,
        )?;
    }
    Ok(expanded)
}

fn expand_field_codes(
    arguments: &mut Vec<String>,
    argument: &str,
    was_quoted: bool,
    desktop_path: &Path,
    application_name: Option<&str>,
    icon: Option<&str>,
) -> Result<()> {
    if matches!(argument, "%f" | "%F" | "%u" | "%U") {
        if was_quoted {
            anyhow::bail!("desktop entry has a quoted file field code")
        }
        return Ok(());
    }
    if argument == "%i" {
        if was_quoted {
            anyhow::bail!("desktop entry has a quoted icon field code")
        }
        if let Some(icon) = icon {
            arguments.push("--icon".to_owned());
            arguments.push(icon.to_owned());
        }
        return Ok(());
    }
    let mut expanded = String::new();
    let mut characters = argument.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            expanded.push(character);
            continue;
        }
        if was_quoted {
            anyhow::bail!("desktop entry has a field code inside quoted text")
        }
        match characters
            .next()
            .context("desktop entry has an incomplete field code")?
        {
            '%' => expanded.push('%'),
            'c' => expanded.push_str(application_name.unwrap_or_default()),
            'k' => expanded.push_str(&desktop_path.to_string_lossy()),
            'f' | 'F' | 'u' | 'U' | 'i' => {
                anyhow::bail!("desktop entry has a field code in an invalid position")
            }
            _ => anyhow::bail!("desktop entry has an unknown field code"),
        }
    }
    if !expanded.is_empty() {
        arguments.push(expanded);
    }
    Ok(())
}

fn executable_exists(command: &str) -> bool {
    let path = Path::new(command);
    if path.is_absolute() || command.contains('/') {
        return path.is_file()
            && fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0);
    }
    env::var_os("PATH")
        .as_deref()
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .map(|directory| directory.join(command))
        .any(|candidate| {
            candidate.is_file()
                && fs::metadata(candidate)
                    .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
        })
}

fn validate_desktop_entry(values: &HashMap<&str, &str>) -> Result<()> {
    if values.get("Type") != Some(&"Application") {
        anyhow::bail!("not an application desktop entry")
    }
    if desktop_entry_boolean(values, "NoDisplay") || desktop_entry_boolean(values, "Hidden") {
        anyhow::bail!("not a visible application entry")
    }
    let desktops = current_desktops();
    if let Some(only_show_in) = values.get("OnlyShowIn")
        && !visible_in_desktop(only_show_in, &desktops)
    {
        anyhow::bail!("desktop entry is hidden in the current desktop")
    }
    if let Some(not_show_in) = values.get("NotShowIn")
        && visible_in_desktop(not_show_in, &desktops)
    {
        anyhow::bail!("desktop entry is hidden in the current desktop")
    }
    Ok(())
}

fn current_desktops() -> Vec<String> {
    env::var("XDG_CURRENT_DESKTOP")
        .ok()
        .map(|desktop| desktop.split(':').map(ToOwned::to_owned).collect())
        .unwrap_or_default()
}

fn visible_in_desktop(desktops: &str, current_desktops: &[String]) -> bool {
    desktops
        .split(';')
        .filter(|desktop| !desktop.is_empty())
        .any(|desktop| current_desktops.iter().any(|current| current == desktop))
}

fn desktop_entry_boolean(values: &HashMap<&str, &str>, key: &str) -> bool {
    values.get(key).is_some_and(|value| *value == "true")
}

fn desktop_entry_directories() -> Vec<PathBuf> {
    desktop_entry_directories_for(
        env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
        env::var("XDG_DATA_DIRS").ok(),
    )
}

fn desktop_entry_directories_for(
    data_home: Option<PathBuf>,
    home: Option<PathBuf>,
    data_dirs: Option<String>,
) -> Vec<PathBuf> {
    let data_home = data_home
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| home.map(|path| path.join(".local/share")));
    let data_dirs = data_dirs
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());

    let mut directories = Vec::new();
    let mut seen = HashSet::new();
    if let Some(data_home) = data_home {
        push_unique(&mut directories, &mut seen, data_home.join("applications"));
        // Flatpak exports launchers outside the normal XDG data root.
        push_unique(
            &mut directories,
            &mut seen,
            data_home.join("flatpak/exports/share/applications"),
        );
    }
    for data_dir in data_dirs
        .split(':')
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        push_unique(&mut directories, &mut seen, data_dir.join("applications"));
    }
    push_unique(
        &mut directories,
        &mut seen,
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
    );
    directories
}

fn push_unique(directories: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, path: PathBuf) {
    if seen.insert(path.clone()) {
        directories.push(path);
    }
}

async fn parse_desktop_entry(path: &Path) -> Result<DiscoveredApplication> {
    let content = tokio::fs::read_to_string(path).await?;
    let values = desktop_entry_values(&content);
    validate_desktop_entry(&values)?;
    command_for_desktop_entry(path).await?;
    let name = values
        .get("Name")
        .context("desktop entry has no Name")?
        .to_string();
    let application_id = path
        .file_name()
        .context("desktop entry has no file name")?
        .to_string_lossy()
        .to_string();
    Ok(DiscoveredApplication {
        application_id,
        name,
        comment: values.get("Comment").map(ToString::to_string),
        icon: values.get("Icon").map(ToString::to_string),
        categories: values
            .get("Categories")
            .map(|value| {
                value
                    .split(';')
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        launch_scheme: values
            .get("Exec")
            .and_then(|exec| exec.contains("heroic://").then_some("heroic".to_owned())),
    })
}

fn desktop_entry_values(content: &str) -> HashMap<&str, &str> {
    let mut values = HashMap::new();
    let mut in_desktop_entry = false;
    for line in content.lines().map(str::trim) {
        if line.starts_with('[') {
            if in_desktop_entry {
                break;
            }
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if in_desktop_entry
            && !line.starts_with('#')
            && let Some((key, value)) = line.split_once('=')
        {
            values.insert(key.trim(), value.trim());
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        desktop_entry_directories_for, is_kiosk_session_for, parse_desktop_entry, parse_exec,
        valid_heroic_application_id, valid_wayland_display, visible_in_desktop,
    };

    #[tokio::test]
    async fn parses_only_the_desktop_entry_group() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("example.desktop");
        tokio::fs::write(
            &path,
            "[Desktop Entry]\nType=Application\nName=Example\nExec=example --open\nCategories=Utility;\n[Desktop Action other]\nName=Wrong name\n",
        )
        .await
        .unwrap();

        let entry = parse_desktop_entry(&path).await.unwrap();

        assert_eq!(entry.name, "Example");
        assert_eq!(entry.categories, ["Utility"]);
        assert_eq!(entry.launch_scheme, None);
    }

    #[tokio::test]
    async fn identifies_heroic_shortcuts_for_dedicated_game_discovery() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("heroic-game.desktop");
        tokio::fs::write(
            &path,
            "[Desktop Entry]\nType=Application\nName=Example\nExec=xdg-open heroic://launch?appName=Example&runner=legendary\nCategories=Game;\n",
        )
        .await
        .unwrap();

        let entry = parse_desktop_entry(&path).await.unwrap();

        assert_eq!(entry.launch_scheme.as_deref(), Some("heroic"));
    }

    #[test]
    fn parses_exec_without_forwarding_runtime_file_arguments() {
        let command = parse_exec(
            "example --title %c %U",
            std::path::Path::new("/tmp/example.desktop"),
            Some("Example"),
            None,
        )
        .unwrap();

        assert_eq!(command, ["example", "--title", "Example"]);
    }

    #[test]
    fn expands_icon_field_code_as_two_arguments() {
        let command = parse_exec(
            "example %i",
            std::path::Path::new("/tmp/example.desktop"),
            Some("Example"),
            Some("example-icon"),
        )
        .unwrap();

        assert_eq!(command, ["example", "--icon", "example-icon"]);
    }

    #[test]
    fn rejects_field_codes_inside_quoted_arguments() {
        let result = parse_exec(
            "example \"%c\"",
            std::path::Path::new("/tmp/example.desktop"),
            Some("Example"),
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn recognizes_hearthdeck_cosmic_kiosk_desktop_names() {
        let current_desktops = vec!["hearthdeck".to_owned(), "COSMIC".to_owned()];

        assert!(visible_in_desktop("hearthdeck;", &current_desktops));
        assert!(visible_in_desktop("COSMIC;", &current_desktops));
        assert!(!visible_in_desktop("gamescope;", &current_desktops));
        assert!(!visible_in_desktop("KDE;", &current_desktops));
    }

    #[test]
    fn kiosk_mode_is_detected_only_from_the_hearthdeck_desktop_name() {
        assert!(is_kiosk_session_for(&["hearthdeck".to_owned()]));
        assert!(!is_kiosk_session_for(&["COSMIC".to_owned()]));
    }

    #[test]
    fn validates_the_runtime_gamescope_wayland_display_name() {
        assert!(valid_wayland_display("gamescope-0"));
        assert!(valid_wayland_display("gamescope.overlay_1"));
        assert!(!valid_wayland_display("../socket"));
        assert!(!valid_wayland_display("gamescope-0\nOTHER=value"));
    }

    #[test]
    fn uses_xdg_defaults_and_flatpak_exports_when_environment_is_empty() {
        let directories = desktop_entry_directories_for(
            Some(PathBuf::new()),
            Some(PathBuf::from("/home/tester")),
            Some(String::new()),
        );

        assert_eq!(
            directories,
            vec![
                PathBuf::from("/home/tester/.local/share/applications"),
                PathBuf::from("/home/tester/.local/share/flatpak/exports/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
            ],
        );
    }

    #[test]
    fn ignores_relative_xdg_data_directories() {
        let directories = desktop_entry_directories_for(
            None,
            Some(PathBuf::from("/home/tester")),
            Some("relative:/opt/share:/usr/share".to_owned()),
        );

        assert!(!directories.contains(&PathBuf::from("relative/applications")));
        assert!(directories.contains(&PathBuf::from("/opt/share/applications")));
        assert_eq!(
            directories
                .iter()
                .filter(|path| path == &&PathBuf::from("/usr/share/applications"))
                .count(),
            1,
        );
    }

    #[test]
    fn validates_heroic_application_identifiers_before_uri_construction() {
        assert!(valid_heroic_application_id("Fortnite"));
        assert!(valid_heroic_application_id("1091500"));
        assert!(!valid_heroic_application_id("Fortnite&runner=gog"));
        assert!(!valid_heroic_application_id("../Fortnite"));
    }
}
