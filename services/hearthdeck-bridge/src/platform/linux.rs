use std::{
    collections::{HashMap, HashSet},
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use hearthdeck_protocol::DiscoveredApplication;
use tokio::process::Command;
use tracing::{info, warn};

use super::DESKTOP_APPS_SOURCE;

pub async fn discover_applications(source_id: &str) -> Result<Vec<DiscoveredApplication>> {
    if source_id != DESKTOP_APPS_SOURCE {
        anyhow::bail!("unsupported application source")
    }
    let directories = desktop_entry_directories();
    info!(directories = ?directories, "application discovery scanning desktop-entry directories");

    let mut entries = HashMap::new();
    for directory in directories {
        let mut directory = match tokio::fs::read_dir(&directory).await {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                warn!(directory = %directory.display(), %error, "application discovery could not read desktop-entry directory");
                continue;
            }
        };
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
                && let Ok(entry) = parse_desktop_entry(&path).await
            {
                entries.entry(entry.application_id.clone()).or_insert(entry);
            }
        }
    }
    let mut entries: Vec<_> = entries.into_values().collect();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}

pub async fn launch_application(source_id: &str, application_id: &str) -> Result<()> {
    let entry = discover_applications(source_id)
        .await?
        .into_iter()
        .find(|entry| entry.application_id == application_id)
        .context("desktop entry is not registered")?;
    Command::new("gtk-launch")
        .arg(&entry.application_id)
        .spawn()
        .context("could not start gtk-launch")?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{desktop_entry_directories_for, parse_desktop_entry};

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
