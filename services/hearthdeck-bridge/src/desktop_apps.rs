//! Desktop applications: discovery of `.desktop` entries and launching one
//! as a supervised unit.
//!
//! This is the generic launch target - the `desktop-apps` source - and the
//! entry machinery the target-specific modules (`stremio`) build on. Only
//! `Type=Application` entries that validate can be discovered or launched;
//! the entry id is always a bare file name inside the discovery directories,
//! never a path.

use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use hearthdeck_protocol::{ApplicationSource, DiscoveredApplication};
use tracing::{debug, info, warn};

use crate::systemd::{LaunchedApplication, launch_with_systemd};

pub(crate) use hearthdeck_protocol::sources::DESKTOP_APPS as DESKTOP_APPS_SOURCE;

pub(crate) async fn discover_applications(source_id: &str) -> Result<Vec<DiscoveredApplication>> {
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

pub(crate) async fn launch_application(
    source_id: &str,
    application_id: &str,
    session_id: &str,
) -> Result<LaunchedApplication> {
    let path = desktop_entry_path(source_id, application_id).await?;
    let launch = command_for_desktop_entry(&path).await?;
    let unit_name = format!("hearthdeck-app-{session_id}.service");
    launch_with_systemd(&unit_name, launch.command, launch.working_directory).await
}

pub(crate) async fn desktop_entry_path(source_id: &str, application_id: &str) -> Result<PathBuf> {
    if source_id != DESKTOP_APPS_SOURCE {
        anyhow::bail!("unsupported application source")
    }
    // `application_id` is client-supplied. Discovery is safe because it reads the
    // file's own name, but this launch path joins the raw string onto a discovery
    // root, so a value containing `..` or a path separator could point at any
    // `.desktop` file on the host. Require a single normal component.
    if !is_bare_file_name(application_id) {
        anyhow::bail!("invalid desktop entry id")
    }
    for directory in desktop_entry_directories() {
        let path = directory.join(application_id);
        if path.is_file() && parse_desktop_entry(&path).await.is_ok() {
            return Ok(path);
        }
    }
    anyhow::bail!("desktop entry is not registered")
}

/// Whether `value` is a single file-name component: not empty, not `.` or `..`,
/// and free of separators. The single-component check is what rejects `./x`,
/// `../x`, `/x`, and `dir/x`; the explicit `.`/`..` check catches what
/// `components()` would otherwise fold into one.
fn is_bare_file_name(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && Path::new(value).components().count() == 1
}

/// The command a validated desktop entry runs under, with its field codes
/// already expanded the way the entry's `Exec` line spells them.
pub(crate) struct DesktopLaunch {
    pub(crate) command: Vec<OsString>,
    pub(crate) working_directory: Option<PathBuf>,
}

pub(crate) async fn command_for_desktop_entry(path: &Path) -> Result<DesktopLaunch> {
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
    let mut quote = None;
    let mut argument_was_quoted = false;
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else if delimiter == '"' && character == '\\' {
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
        } else if matches!(character, '"' | '\'') {
            quote = Some(character);
            argument_was_quoted = true;
        } else if character.is_whitespace() {
            if !argument.is_empty() {
                arguments.push((std::mem::take(&mut argument), argument_was_quoted));
                argument_was_quoted = false;
            }
        } else {
            argument.push(character);
        }
    }
    if quote.is_some() {
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

/// The desktops this session reports itself as, from `XDG_CURRENT_DESKTOP`.
/// `systemd`'s kiosk-slice decision reads the same list.
pub(crate) fn current_desktops() -> Vec<String> {
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

/// The user's XDG data home, as the running environment says it is.
fn xdg_data_home() -> Option<PathBuf> {
    xdg_data_home_from(
        env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    )
}

/// `$XDG_DATA_HOME`, or `$HOME/.local/share` when it is unset or empty - the
/// fallback the specification defines, and the one the directory list below and
/// the classification of what is found there have to agree on.
fn xdg_data_home_from(data_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    data_home
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| home.map(|path| path.join(".local/share")))
}

fn desktop_entry_directories_for(
    data_home: Option<PathBuf>,
    home: Option<PathBuf>,
    data_dirs: Option<String>,
) -> Vec<PathBuf> {
    let data_home = xdg_data_home_from(data_home, home);
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

/// Where the desktop entry at `path` came from.
///
/// The directory list is what finds the entries, but the path is what classifies
/// them: XDG's data directories hold whatever the distribution put in them -
/// flatpak's and snapd's export directories are added to that same list - so the
/// origin is read off the location rather than assumed from the list entry that
/// found it.
///
/// An unrecognised location reads as the distribution's, which is what a data
/// directory nobody listed in practice is.
fn origin_of(path: &Path, data_home: Option<&Path>) -> ApplicationSource {
    let location = path.to_string_lossy();
    // Flatpak first: a per-user export lives *under* the data home, so asking
    // about the data home first would classify it as the user's own.
    if location.contains("/flatpak/") {
        return ApplicationSource::Flatpak;
    }
    if location.starts_with("/var/lib/snapd") {
        return ApplicationSource::Snap;
    }
    if location.starts_with("/nix/store")
        || location.starts_with("/nix/var")
        || location.starts_with("/run/current-system")
        || location.starts_with("/run/system-manager")
        || location.starts_with("/etc/profiles/per-user")
    {
        return ApplicationSource::Nix;
    }
    if data_home.is_some_and(|home| path.starts_with(home)) {
        return ApplicationSource::Local;
    }
    ApplicationSource::System
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
        source: origin_of(path, xdg_data_home().as_deref()),
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
    use std::path::{Path, PathBuf};

    use super::{
        ApplicationSource, desktop_entry_directories_for, is_bare_file_name, origin_of,
        parse_desktop_entry, parse_exec, visible_in_desktop,
    };

    /// A launcher's origin is read off where it was installed, not off the file
    /// itself, so the same entry name is a different answer in each directory.
    #[test]
    fn an_entry_is_classified_by_where_its_launcher_was_installed() {
        let data_home = Path::new("/home/user/.local/share");
        let cases: &[(&str, ApplicationSource)] = &[
            (
                "/usr/share/applications/example.desktop",
                ApplicationSource::System,
            ),
            (
                "/usr/local/share/applications/example.desktop",
                ApplicationSource::System,
            ),
            (
                "/home/user/.local/share/applications/example.desktop",
                ApplicationSource::Local,
            ),
            (
                "/home/user/.local/share/flatpak/exports/share/applications/org.example.App.desktop",
                ApplicationSource::Flatpak,
            ),
            (
                "/var/lib/flatpak/exports/share/applications/org.example.App.desktop",
                ApplicationSource::Flatpak,
            ),
            (
                "/var/lib/snapd/desktop/applications/example_example.desktop",
                ApplicationSource::Snap,
            ),
            (
                "/nix/store/hash-example/share/applications/example.desktop",
                ApplicationSource::Nix,
            ),
            (
                "/run/current-system/sw/share/applications/example.desktop",
                ApplicationSource::Nix,
            ),
        ];
        for (path, expected) in cases {
            assert_eq!(
                origin_of(Path::new(path), Some(data_home)),
                *expected,
                "path: {path}"
            );
        }

        // An unrecognised location reads as the distribution's; without a data
        // home to compare against, even the user's own directory is one.
        assert_eq!(
            origin_of(
                Path::new("/home/user/.local/share/applications/example.desktop"),
                None
            ),
            ApplicationSource::System,
        );
    }

    #[test]
    fn a_desktop_entry_id_must_be_a_single_file_name() {
        // Valid desktop-entry ids: reversed-DNS names and plain file names.
        assert!(is_bare_file_name("org.example.App.desktop"));
        assert!(is_bare_file_name("firefox.desktop"));

        // Everything a client could use to walk out of a discovery root.
        assert!(!is_bare_file_name(""));
        assert!(!is_bare_file_name("."));
        assert!(!is_bare_file_name(".."));
        assert!(!is_bare_file_name("../evil.desktop"));
        assert!(!is_bare_file_name("/etc/evil.desktop"));
        assert!(!is_bare_file_name("sub/evil.desktop"));
    }

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
    fn preserves_single_quoted_web_app_commands() {
        let command = parse_exec(
            "sh -c 'XAPP_FORCE_GTKWINDOW_ICON=webapp firefox --class WebApp --app=https://example.com'",
            std::path::Path::new("/tmp/webapp.desktop"),
            Some("Web App"),
            None,
        )
        .unwrap();

        assert_eq!(
            command,
            [
                "sh",
                "-c",
                "XAPP_FORCE_GTKWINDOW_ICON=webapp firefox --class WebApp --app=https://example.com"
            ]
        );
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
}
