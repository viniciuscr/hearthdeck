use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use hearthdeck_protocol::DiscoveredApplication;
use tokio::process::Command;

use super::MACOS_APPS_SOURCE;

pub async fn discover_applications(source_id: &str) -> Result<Vec<DiscoveredApplication>> {
    if source_id != MACOS_APPS_SOURCE {
        anyhow::bail!("unsupported application source")
    }

    let mut seen_bundle_ids = HashSet::new();
    let mut applications = Vec::new();
    for directory in application_directories() {
        let mut directory = match tokio::fs::read_dir(&directory).await {
            Ok(directory) => directory,
            Err(_) => continue,
        };
        while let Some(entry) = directory.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "app")
                && let Ok(application) = parse_application_bundle(&path).await
                && seen_bundle_ids.insert(application.application_id.clone())
            {
                applications.push(application);
            }
        }
    }
    applications.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(applications)
}

pub async fn launch_application(source_id: &str, application_id: &str) -> Result<()> {
    let application = discover_applications(source_id)
        .await?
        .into_iter()
        .find(|application| application.application_id == application_id)
        .context("application bundle is not registered")?;
    Command::new("open")
        .arg("-b")
        .arg(&application.application_id)
        .spawn()
        .context("could not invoke macOS LaunchServices")?;
    Ok(())
}

fn application_directories() -> Vec<PathBuf> {
    let mut directories = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        directories.push(PathBuf::from(home).join("Applications"));
    }
    directories
}

async fn parse_application_bundle(path: &Path) -> Result<DiscoveredApplication> {
    let plist_path = path.join("Contents/Info.plist");
    let output = Command::new("plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
        .arg(&plist_path)
        .output()
        .await
        .context("could not read application bundle identifier")?;
    if !output.status.success() {
        anyhow::bail!("application bundle has no readable identifier")
    }
    let application_id = String::from_utf8(output.stdout)
        .context("application identifier is not UTF-8")?
        .trim()
        .to_owned();
    if application_id.is_empty() {
        anyhow::bail!("application bundle identifier is empty")
    }

    let name = path
        .file_stem()
        .context("application bundle has no file name")?
        .to_string_lossy()
        .to_string();
    Ok(DiscoveredApplication {
        application_id,
        name,
        comment: None,
        icon: None,
        categories: vec!["Application".to_owned()],
    })
}
