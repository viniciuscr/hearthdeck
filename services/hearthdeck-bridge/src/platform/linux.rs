use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use hearthdeck_protocol::DiscoveredApplication;
use tokio::process::Command;

use super::DESKTOP_APPS_SOURCE;

pub async fn discover_applications(source_id: &str) -> Result<Vec<DiscoveredApplication>> {
    if source_id != DESKTOP_APPS_SOURCE {
        anyhow::bail!("unsupported application source")
    }
    let mut entries = HashMap::new();
    for directory in desktop_entry_directories() {
        let mut directory = match tokio::fs::read_dir(&directory).await {
            Ok(directory) => directory,
            Err(_) => continue,
        };
        while let Some(file) = directory.next_entry().await? {
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
    let mut directories = Vec::new();
    if let Some(home) = env::var_os("XDG_DATA_HOME") {
        directories.push(PathBuf::from(home).join("applications"));
    } else if let Some(home) = env::var_os("HOME") {
        directories.push(PathBuf::from(home).join(".local/share/applications"));
    }
    let data_dirs =
        env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_owned());
    directories.extend(
        data_dirs
            .split(':')
            .map(|path| PathBuf::from(path).join("applications")),
    );
    directories
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
    use super::parse_desktop_entry;

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
}
