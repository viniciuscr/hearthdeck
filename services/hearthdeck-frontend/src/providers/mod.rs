pub mod daemon;
pub mod filter;

use std::path::PathBuf;

use async_trait::async_trait;
use cosmic::desktop::{DesktopEntryData, fde};
use serde::{Deserialize, Serialize};

use crate::icon_cache::{self, IconReference};

/// A provider-agnostic record representing a discovered application or game.
///
/// Providers produce `Vec<GameRecord>`. The app converts these into
/// `DesktopEntryData` for display alongside XDG `.desktop` entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameRecord {
    /// Unique identifier, prefixed by source (e.g. `"heroic:epic:Fortnite"`).
    pub id: String,
    pub name: String,
    pub exec: Option<String>,
    pub icon: Option<String>,
    pub path: Option<PathBuf>,
    pub categories: Vec<String>,
    pub terminal: bool,
    pub prefers_dgpu: bool,
    /// Which provider discovered this record.
    pub source: String,
    /// Provider-specific metadata (store name, runner, version, etc.).
    pub metadata: serde_json::Value,
}

impl GameRecord {
    /// Convert into a `DesktopEntryData` for integration with the existing UI.
    pub fn into_desktop_entry(mut self) -> DesktopEntryData {
        let is_game = self
            .categories
            .iter()
            .any(|category| category.eq_ignore_ascii_case("game"));
        if is_game
            && let Some(store) = self
                .metadata
                .get("store")
                .and_then(serde_json::Value::as_str)
        {
            self.categories
                .retain(|category| !category.eq_ignore_ascii_case(store));
            self.categories.push(format!("hearthdeck-store:{store}"));
        }

        let fallback_icon = if is_game {
            "applications-games"
        } else {
            "application-x-executable"
        };
        // Read through the same rule the provider fetches by, so a reference
        // means one thing here and there. A poster URL or a RomM path is the
        // provider's to turn into a file *before* a record is built; one that got
        // here un-fetched is not art, and a name the theme cannot read draws
        // nothing at all, so the record wears the mark for its kind instead.
        let icon_source = match self.icon.as_deref().and_then(icon_cache::classify) {
            Some(IconReference::File(path)) => fde::IconSource::Path(PathBuf::from(path)),
            Some(IconReference::Name(name)) => fde::IconSource::Name(name.to_owned()),
            Some(IconReference::Remote(_) | IconReference::RommAsset(_)) | None => {
                fde::IconSource::Name(fallback_icon.to_owned())
            }
        };

        let mut entry = DesktopEntryData {
            id: self.id,
            name: self.name,
            wm_class: None,
            exec: self.exec,
            icon: icon_source,
            path: self.path,
            categories: self.categories,
            desktop_actions: Vec::new(),
            mime_types: Vec::new(),
            prefers_dgpu: self.prefers_dgpu,
            terminal: self.terminal,
        };
        let modeled_couch = self
            .metadata
            .get("form_factor")
            .and_then(serde_json::Value::as_str)
            == Some("couch");
        // Where the library got this record, for the section that filters by it.
        // Read here rather than inside the tag, because the streaming tag is
        // derived from the finished entry.
        let source = self
            .metadata
            .get("source")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        crate::app_group::tag_source(&mut entry, source.as_deref());
        if crate::app_group::Section::Applications.matches(&entry) {
            crate::form_factor::tag(&mut entry);
            // A model can only promote: what it calls desktop-only is what the
            // rules already said, and its doubt must not demote a streaming client.
            if modeled_couch {
                crate::form_factor::set(&mut entry, crate::form_factor::FormFactor::Couch);
            }
        }
        entry
    }
}

/// A game provider discovers installed games or applications from a specific
/// source (Heroic Games Launcher, Steam, Lutris, etc.).
///
/// # Implementation notes
///
/// - `discover` should be idempotent and return the full current state.
#[async_trait]
pub trait GameProvider: Send + Sync {
    /// Discover all items from this provider.
    async fn discover(&self) -> anyhow::Result<Vec<GameRecord>>;
}

/// A game provider discovers installed games or applications from a specific
#[cfg(test)]
mod tests {
    use super::GameRecord;
    use cosmic::desktop::fde::IconSource;

    fn record(icon: Option<&str>) -> GameRecord {
        GameRecord {
            id: "test".into(),
            name: "Test".into(),
            exec: None,
            icon: icon.map(str::to_owned),
            path: None,
            categories: vec!["Utility".into()],
            terminal: false,
            prefers_dgpu: false,
            source: "test".into(),
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn resolves_icon_names_and_absolute_paths() {
        assert!(matches!(
            record(Some("org.example.App")).into_desktop_entry().icon,
            IconSource::Name(name) if name == "org.example.App"
        ));
        assert!(matches!(
            record(Some("/usr/share/icons/example.png"))
                .into_desktop_entry()
                .icon,
            IconSource::Path(path) if path == std::path::Path::new("/usr/share/icons/example.png")
        ));
    }

    #[test]
    fn missing_application_icon_uses_fallback() {
        assert!(matches!(
            record(None).into_desktop_entry().icon,
            IconSource::Name(name) if name == "application-x-executable"
        ));
    }

    /// A launcher may write its artwork as a `file://` URL; the scheme off, it is
    /// the same file, and a renderer cannot open it as a name.
    #[test]
    fn a_file_url_is_read_as_the_file_it_names() {
        assert!(matches!(
            record(Some("file:///home/user/.local/share/art/cover.png"))
                .into_desktop_entry()
                .icon,
            IconSource::Path(path) if path == std::path::Path::new("/home/user/.local/share/art/cover.png")
        ));
    }

    /// RomM hands out paths in its own URL space, so an unproxied one is not a
    /// file: the record wears the mark for its kind instead of a path the
    /// renderer would try and fail to open.
    #[test]
    fn romm_artwork_that_was_never_fetched_is_not_a_path() {
        let romm = "/assets/romm/resources/roms/9/4014/cover/small.png";
        assert!(matches!(
            record(Some(romm)).into_desktop_entry().icon,
            IconSource::Name(name) if name == "application-x-executable"
        ));

        let mut game = record(Some(romm));
        game.categories = vec!["Game".into()];
        assert!(matches!(
            game.into_desktop_entry().icon,
            IconSource::Name(name) if name == "applications-games"
        ));
    }

    #[test]
    fn game_store_becomes_a_tab_category() {
        let mut game = record(None);
        game.categories = vec!["Game".into(), "Epic Games".into()];
        game.metadata = serde_json::json!({"store": "Epic Games"});

        assert_eq!(
            game.into_desktop_entry().categories,
            vec!["Game", "hearthdeck-store:Epic Games"]
        );
    }

    #[test]
    fn an_application_carries_where_it_was_installed() {
        let mut application = record(None);
        application.metadata = serde_json::json!({"source": "flatpak"});

        assert_eq!(
            application.into_desktop_entry().categories,
            [
                "Utility",
                "hearthdeck-source:flatpak",
                "hearthdeck-form:desktop"
            ]
        );
    }

    /// The model can say an application suits the couch, and only that: its
    /// doubt about one the rules already know is no reason to demote it.
    #[test]
    fn a_model_verdict_can_promote_an_application_but_never_demote_one() {
        let categories = |metadata: serde_json::Value, id: &str| {
            let mut application = record(None);
            application.id = id.into();
            application.metadata = metadata;
            application.into_desktop_entry().categories
        };

        assert!(
            categories(
                serde_json::json!({"form_factor": "couch"}),
                "org.gimp.GIMP.desktop"
            )
            .contains(&"hearthdeck-form:couch".to_owned())
        );
        assert!(
            categories(
                serde_json::json!({"form_factor": "desktop"}),
                "net.stremio.Stremio.desktop"
            )
            .contains(&"hearthdeck-form:couch".to_owned())
        );
    }

    /// A streaming client is both a place and a kind of application, so it
    /// carries both tags: a selection on either one finds it.
    #[test]
    fn a_streaming_client_is_tagged_beside_its_origin() {
        let mut client = record(None);
        client.id = "hearthdeck:desktop:net.stremio.Stremio.desktop".into();
        client.categories = vec!["Video".into()];
        client.metadata = serde_json::json!({"source": "flatpak"});

        assert_eq!(
            client.into_desktop_entry().categories,
            [
                "Video",
                "hearthdeck-source:flatpak",
                "hearthdeck-source:streaming",
                "hearthdeck-form:couch"
            ]
        );
    }
}
