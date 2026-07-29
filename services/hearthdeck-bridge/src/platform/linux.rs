use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use hearthdeck_protocol::DiscoveredApplication;
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
    let command = command_for_desktop_entry(&path).await?;
    let unit_name = format!("hearthdeck-app-{session_id}.scope");
    let status = Command::new("systemd-run")
        .args([
            "--user",
            "--scope",
            "--collect",
            "--no-block",
            "--quiet",
            "--unit",
        ])
        .arg(&unit_name)
        .arg("--")
        .args(command)
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

async fn command_for_desktop_entry(path: &Path) -> Result<Vec<OsString>> {
    let content = tokio::fs::read_to_string(path).await?;
    let values = desktop_entry_values(&content);
    if values.get("Terminal") == Some(&"true") {
        anyhow::bail!("terminal desktop entries are not supported in console mode")
    }
    let exec = values.get("Exec").context("desktop entry has no Exec")?;
    let command = parse_exec(exec, path, values.get("Name").copied())?;
    if command.is_empty() {
        anyhow::bail!("desktop entry has an empty Exec command")
    }
    Ok(command.into_iter().map(OsString::from).collect())
}

fn parse_exec(
    value: &str,
    desktop_path: &Path,
    application_name: Option<&str>,
) -> Result<Vec<String>> {
    let mut arguments = Vec::new();
    let mut argument = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            argument.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character.is_whitespace() && !quoted {
            if !argument.is_empty() {
                arguments.push(std::mem::take(&mut argument));
            }
        } else {
            argument.push(character);
        }
    }
    if escaped || quoted {
        anyhow::bail!("desktop entry has an invalid Exec command")
    }
    if !argument.is_empty() {
        arguments.push(argument);
    }
    Ok(arguments
        .into_iter()
        .filter_map(|argument| expand_field_codes(&argument, desktop_path, application_name))
        .collect())
}

fn expand_field_codes(
    argument: &str,
    desktop_path: &Path,
    application_name: Option<&str>,
) -> Option<String> {
    let mut expanded = String::new();
    let mut characters = argument.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            expanded.push(character);
            continue;
        }
        match characters.next()? {
            '%' => expanded.push('%'),
            'c' => expanded.push_str(application_name.unwrap_or_default()),
            'k' => expanded.push_str(&desktop_path.to_string_lossy()),
            'f' | 'F' | 'u' | 'U' | 'i' => {}
            _ => return None,
        }
    }
    (!expanded.is_empty()).then_some(expanded)
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
    if values.get("Type") != Some(&"Application")
        || values.get("NoDisplay") == Some(&"true")
        || values.get("Hidden") == Some(&"true")
    {
        anyhow::bail!("not a visible application entry")
    }
    let name = values
        .get("Name")
        .context("desktop entry has no Name")?
        .to_string();
    values.get("Exec").context("desktop entry has no Exec")?;
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

    use super::{desktop_entry_directories_for, parse_desktop_entry, parse_exec};

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
    }

    #[test]
    fn parses_exec_without_forwarding_runtime_file_arguments() {
        let command = parse_exec(
            "example --title %c %U",
            std::path::Path::new("/tmp/example.desktop"),
            Some("Example"),
        )
        .unwrap();

        assert_eq!(command, ["example", "--title", "Example"]);
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
