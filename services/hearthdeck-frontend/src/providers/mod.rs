pub mod daemon;
pub mod filter;

use std::path::PathBuf;

use crate::app_group::SectionUi;
use crate::form_factor::FormFactor;
use crate::icon_cache::{self, IconReference};
use crate::library_item::LibraryItem;
use cosmic::desktop::fde;
use hearthdeck_protocol::RetroRomVersion;

/// A provider-agnostic record representing a discovered application or game.
///
/// The daemon adapter builds these before the app converts them into
/// `LibraryItem` for display.
#[derive(Debug, Clone)]
pub struct GameRecord {
    /// Unique identifier, prefixed by source (e.g. `"heroic:epic:Fortnite"`).
    pub id: String,
    pub name: String,
    pub exec: Option<String>,
    pub icon: Option<String>,
    pub categories: Vec<String>,
    pub store: Option<String>,
    pub install_source: Option<String>,
    pub modeled_form: Option<FormFactor>,
    pub sibling_roms: Vec<RetroRomVersion>,
    pub version_label: Option<String>,
}

impl GameRecord {
    /// Convert into a `LibraryItem` for integration with the existing UI.
    pub fn into_library_item(mut self) -> LibraryItem {
        let is_game = self
            .categories
            .iter()
            .any(|category| category.eq_ignore_ascii_case("game"));
        if is_game && let Some(store) = &self.store {
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

        let mut entry = LibraryItem {
            id: self.id,
            name: self.name,
            wm_class: None,
            exec: self.exec,
            icon: icon_source,
            categories: self.categories,
            form_factor: None,
        };
        crate::app_group::tag_source(&mut entry, self.install_source.as_deref());
        if crate::app_group::Section::Applications.matches(&entry) {
            crate::form_factor::tag(&mut entry);
            // A model can only promote: what it calls desktop-only is what the
            // rules already said, and its doubt must not demote a streaming client.
            if self.modeled_form == Some(FormFactor::Couch) {
                crate::form_factor::set(&mut entry, FormFactor::Couch);
            }
        }
        entry
    }
}

#[cfg(test)]
mod tests {
    use super::{FormFactor, GameRecord};
    use cosmic::desktop::fde::IconSource;

    fn record(icon: Option<&str>) -> GameRecord {
        GameRecord {
            id: "test".into(),
            name: "Test".into(),
            exec: None,
            icon: icon.map(str::to_owned),
            categories: vec!["Utility".into()],
            store: None,
            install_source: None,
            modeled_form: None,
            sibling_roms: Vec::new(),
            version_label: None,
        }
    }

    #[test]
    fn library_item_preserves_identity_launch_target_and_filter_values() {
        let mut game = record(Some("/usr/share/icons/game.png"));
        game.id = "hearthdeck:heroic:example".to_owned();
        game.name = "Example".to_owned();
        game.exec = Some("legendary:example".to_owned());
        game.categories = vec!["Game".to_owned(), "Epic".to_owned()];
        game.store = Some("Epic".to_owned());

        let item = game.into_library_item();

        assert_eq!(item.id, "hearthdeck:heroic:example");
        assert_eq!(item.name, "Example");
        assert_eq!(item.exec.as_deref(), Some("legendary:example"));
        assert_eq!(item.categories, ["Game", "hearthdeck-store:Epic"]);
        assert!(item.wm_class.is_none());
        assert!(matches!(
            item.icon,
            IconSource::Path(path) if path == std::path::Path::new("/usr/share/icons/game.png")
        ));
    }

    #[test]
    fn resolves_icon_names_and_absolute_paths() {
        assert!(matches!(
            record(Some("org.example.App")).into_library_item().icon,
            IconSource::Name(name) if name == "org.example.App"
        ));
        assert!(matches!(
            record(Some("/usr/share/icons/example.png"))
                .into_library_item()
                .icon,
            IconSource::Path(path) if path == std::path::Path::new("/usr/share/icons/example.png")
        ));
    }

    #[test]
    fn missing_application_icon_uses_fallback() {
        assert!(matches!(
            record(None).into_library_item().icon,
            IconSource::Name(name) if name == "application-x-executable"
        ));
    }

    /// A launcher may write its artwork as a `file://` URL; the scheme off, it is
    /// the same file, and a renderer cannot open it as a name.
    #[test]
    fn a_file_url_is_read_as_the_file_it_names() {
        assert!(matches!(
            record(Some("file:///home/user/.local/share/art/cover.png"))
                .into_library_item()
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
            record(Some(romm)).into_library_item().icon,
            IconSource::Name(name) if name == "application-x-executable"
        ));

        let mut game = record(Some(romm));
        game.categories = vec!["Game".into()];
        assert!(matches!(
            game.into_library_item().icon,
            IconSource::Name(name) if name == "applications-games"
        ));
    }

    #[test]
    fn game_store_becomes_a_tab_category() {
        let mut game = record(None);
        game.categories = vec!["Game".into(), "Epic Games".into()];
        game.store = Some("Epic Games".into());

        assert_eq!(
            game.into_library_item().categories,
            vec!["Game", "hearthdeck-store:Epic Games"]
        );
    }

    #[test]
    fn an_application_carries_where_it_was_installed() {
        let mut application = record(None);
        application.install_source = Some("flatpak".into());

        assert_eq!(
            application.into_library_item().categories,
            ["Utility", "hearthdeck-source:flatpak"]
        );
    }

    /// The model can say an application suits the couch, and only that: its
    /// doubt about one the rules already know is no reason to demote it.
    #[test]
    fn a_model_verdict_can_promote_an_application_but_never_demote_one() {
        let resolved_form = |form: FormFactor, id: &str| {
            let mut application = record(None);
            application.id = id.into();
            application.modeled_form = Some(form);
            application.into_library_item().form_factor
        };

        assert_eq!(
            resolved_form(FormFactor::Couch, "org.gimp.GIMP.desktop"),
            Some(FormFactor::Couch)
        );
        assert_eq!(
            resolved_form(FormFactor::Desktop, "net.stremio.Stremio.desktop"),
            Some(FormFactor::Couch)
        );
    }

    /// A streaming client is both a place and a kind of application, so it
    /// carries both tags: a selection on either one finds it.
    #[test]
    fn a_streaming_client_is_tagged_beside_its_origin() {
        let mut client = record(None);
        client.id = "hearthdeck:desktop:net.stremio.Stremio.desktop".into();
        client.categories = vec!["Video".into()];
        client.install_source = Some("flatpak".into());

        assert_eq!(
            client.into_library_item().categories,
            [
                "Video",
                "hearthdeck-source:flatpak",
                "hearthdeck-source:streaming"
            ]
        );
    }
}
