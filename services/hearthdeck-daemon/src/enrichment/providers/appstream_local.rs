use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use roxmltree::{Document, Node};

use crate::{catalog::EnrichmentRecord, enrichment::MetadataProvider};

pub struct AppStreamLocalProvider {
    directories: Vec<PathBuf>,
}

impl AppStreamLocalProvider {
    pub fn from_system() -> Self {
        Self {
            directories: appstream_directories(),
        }
    }
}

#[async_trait]
impl MetadataProvider for AppStreamLocalProvider {
    fn provider_id(&self) -> &'static str {
        "appstream-local"
    }

    fn refresh_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(60 * 60))
    }

    async fn enrich(&self) -> anyhow::Result<Vec<EnrichmentRecord>> {
        let mut records = Vec::new();
        for directory in &self.directories {
            let mut directory = match tokio::fs::read_dir(directory).await {
                Ok(directory) => directory,
                Err(_) => continue,
            };
            while let Some(entry) = directory.next_entry().await? {
                let path = entry.path();
                if is_metainfo_file(&path)
                    && let Ok(record) = parse_metainfo_file(&path).await
                {
                    records.push(record);
                }
            }
        }
        Ok(records)
    }
}

fn appstream_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(data_home) = env::var_os("XDG_DATA_HOME") {
        let data_home = PathBuf::from(data_home);
        directories.push(data_home.join("metainfo"));
        directories.push(data_home.join("appdata"));
    } else if let Some(home) = env::var_os("HOME") {
        let data_home = PathBuf::from(home).join(".local/share");
        directories.push(data_home.join("metainfo"));
        directories.push(data_home.join("appdata"));
    }
    let data_dirs =
        env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_owned());
    for directory in data_dirs.split(':') {
        let directory = PathBuf::from(directory);
        directories.push(directory.join("metainfo"));
        directories.push(directory.join("appdata"));
    }
    directories
}

fn is_metainfo_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".metainfo.xml") || name.ends_with(".appdata.xml"))
}

async fn parse_metainfo_file(path: &Path) -> anyhow::Result<EnrichmentRecord> {
    let xml = tokio::fs::read_to_string(path).await?;
    parse_component(&xml)
}

fn parse_component(xml: &str) -> anyhow::Result<EnrichmentRecord> {
    let document = Document::parse(xml)?;
    let component = document
        .descendants()
        .find(|node| node.has_tag_name("component"))
        .ok_or_else(|| anyhow::anyhow!("AppStream document has no component"))?;
    let primary_id = child_text(component, "id")
        .ok_or_else(|| anyhow::anyhow!("AppStream component has no id"))?;
    let mut application_ids = vec![primary_id];
    for node in component
        .descendants()
        .filter(|node| node.has_tag_name("id"))
    {
        if let Some(id) = node.text().map(str::trim).filter(|id| !id.is_empty())
            && !application_ids.contains(&id.to_owned())
        {
            application_ids.push(id.to_owned());
        }
    }
    for node in component
        .descendants()
        .filter(|node| node.has_tag_name("launchable"))
    {
        if let Some(id) = node.text().map(str::trim).filter(|id| !id.is_empty())
            && !application_ids.contains(&id.to_owned())
        {
            application_ids.push(id.to_owned());
        }
    }

    let urls = component
        .descendants()
        .filter(|node| node.has_tag_name("url"))
        .filter_map(|node| {
            Some((
                node.attribute("type")?.to_owned(),
                node.text()?.trim().to_owned(),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let categories = component
        .descendants()
        .filter(|node| node.has_tag_name("category"))
        .filter_map(|node| node.text().map(str::trim).filter(|value| !value.is_empty()))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let screenshots = component
        .descendants()
        .filter(|node| node.has_tag_name("image"))
        .filter(|node| node.attribute("type") == Some("source"))
        .filter_map(|node| node.text().map(str::trim).filter(|url| !url.is_empty()))
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    Ok(EnrichmentRecord {
        application_ids,
        priority: 100,
        payload: serde_json::json!({
            "summary": child_text(component, "summary"),
            "description": description_text(component),
            "developer": child_text(component, "developer_name"),
            "project_license": child_text(component, "project_license"),
            "categories": categories,
            "urls": urls,
            "icon": component.descendants()
                .find(|node| node.has_tag_name("icon"))
                .and_then(|node| node.text().map(str::trim).filter(|icon| !icon.is_empty()))
                .map(ToString::to_string),
            "screenshots": screenshots,
            "provenance": "appstream-local",
        }),
        updated_at: Utc::now().to_rfc3339(),
    })
}

fn child_text(component: Node<'_, '_>, tag_name: &str) -> Option<String> {
    component
        .children()
        .find(|node| node.has_tag_name(tag_name))
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

fn description_text(component: Node<'_, '_>) -> Option<String> {
    component
        .descendants()
        .find(|node| node.has_tag_name("description"))
        .and_then(|description| {
            description
                .descendants()
                .find(|node| node.has_tag_name("p"))
                .and_then(|paragraph| paragraph.text())
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::parse_component;

    #[test]
    fn parses_application_ids_and_rich_metadata() {
        let record = parse_component(
            r#"
            <component type="desktop-application">
              <id>org.example.App</id>
              <name>Example</name>
              <summary>A useful example</summary>
              <developer_name>Example Team</developer_name>
              <project_license>GPL-3.0-or-later</project_license>
              <provides><id type="desktop">org.example.App.desktop</id></provides>
              <launchable type="desktop-id">org.example.App.desktop</launchable>
              <categories><category>Utility</category></categories>
              <url type="homepage">https://example.org</url>
              <url type="vcs-browser">https://github.com/example/app</url>
              <icon type="stock">org.example.App</icon>
              <description><p>A longer useful description.</p></description>
              <screenshots><screenshot type="default"><image type="source">https://example.org/shot.png</image></screenshot></screenshots>
            </component>
            "#,
        )
        .unwrap();

        assert!(
            record
                .application_ids
                .contains(&"org.example.App".to_owned())
        );
        assert!(
            record
                .application_ids
                .contains(&"org.example.App.desktop".to_owned())
        );
        assert_eq!(record.payload["developer"], "Example Team");
        assert_eq!(record.payload["urls"]["homepage"], "https://example.org");
    }
}
