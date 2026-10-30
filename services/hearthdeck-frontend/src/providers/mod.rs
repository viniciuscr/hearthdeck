pub mod daemon;
pub mod filter;

use std::path::PathBuf;

use crate::app_group::SectionUi;
use crate::form_factor::FormFactor;
use crate::icon_cache::{self, IconReference};
use crate::library_item::LibraryItem;
use cosmic::desktop::fde;
use hearthdeck_protocol::CatalogItem;

pub(super) fn library_icon(icon: Option<&str>, fallback: &str) -> fde::IconSource {
    match icon.and_then(icon_cache::classify) {
        Some(IconReference::File(path)) => fde::IconSource::Path(PathBuf::from(path)),
        Some(IconReference::Name(name)) => fde::IconSource::Name(name.to_owned()),
        Some(IconReference::Remote(_) | IconReference::RommAsset(_)) | None => {
            fde::IconSource::Name(fallback.to_owned())
        }
    }
}

pub(super) fn catalog_item_to_library_item(item: CatalogItem) -> LibraryItem {
    let metadata = &item.metadata;
    let mut categories: Vec<String> = metadata
        .get("categories")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    categories.retain(|category| !category.eq_ignore_ascii_case("game"));
    if item.kind.eq_ignore_ascii_case("game") {
        categories.push("Game".to_owned());
    }
    let store = metadata.get("store").and_then(|value| value.as_str());
    if let Some(store) = store
        && !categories
            .iter()
            .any(|category| category.eq_ignore_ascii_case(store))
    {
        categories.push(store.to_owned());
    }
    let is_game = categories
        .iter()
        .any(|category| category.eq_ignore_ascii_case("game"));
    if is_game && let Some(store) = store {
        categories.retain(|category| !category.eq_ignore_ascii_case(store));
    }

    let fallback_icon = if is_game {
        "applications-games"
    } else {
        "application-x-executable"
    };

    let mut entry = LibraryItem {
        id: format!("{}{}", daemon::CATALOG_ENTRY_PREFIX, item.id),
        name: item.title,
        exec: item.launch_id,
        icon: library_icon(item.icon.as_deref(), fallback_icon),
        categories,
        store: if is_game {
            store.map(str::to_owned)
        } else {
            None
        },
        ..LibraryItem::default()
    };
    crate::app_group::set_sources(
        &mut entry,
        metadata.get("source").and_then(|value| value.as_str()),
    );
    if crate::app_group::Section::Applications.matches(&entry) {
        crate::form_factor::tag(&mut entry);
        // Model verdicts may promote an app but cannot demote a known couch app.
        if metadata.get("form_factor").and_then(|value| value.as_str()) == Some("couch") {
            crate::form_factor::set(&mut entry, FormFactor::Couch);
        }
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::{FormFactor, catalog_item_to_library_item};
    use cosmic::desktop::fde::IconSource;
    use hearthdeck_protocol::CatalogItem;

    fn record(icon: Option<&str>) -> CatalogItem {
        CatalogItem {
            id: "test".into(),
            source_id: "custom".into(),
            title: "Test".into(),
            kind: "application".into(),
            launch_id: None,
            icon: icon.map(str::to_owned),
            metadata: serde_json::json!({"categories": ["Utility"]}),
        }
    }

    #[test]
    fn library_item_preserves_identity_launch_target_and_filter_values() {
        let mut game = record(Some("/usr/share/icons/game.png"));
        game.id = "heroic:example".to_owned();
        game.title = "Example".to_owned();
        game.launch_id = Some("legendary:example".to_owned());
        game.kind = "game".to_owned();
        game.metadata = serde_json::json!({"categories": ["Game", "Epic"], "store": "Epic"});

        let item = catalog_item_to_library_item(game);

        assert_eq!(item.id, "hearthdeck:heroic:example");
        assert_eq!(item.name, "Example");
        assert_eq!(item.exec.as_deref(), Some("legendary:example"));
        assert_eq!(item.categories, ["Game"]);
        assert_eq!(item.store.as_deref(), Some("Epic"));
        assert!(item.matches_category("hearthdeck-store:Epic"));
        assert!(item.wm_class.is_none());
        assert!(matches!(
            item.icon,
            IconSource::Path(path) if path == std::path::Path::new("/usr/share/icons/game.png")
        ));
    }

    #[test]
    fn resolves_icon_names_and_absolute_paths() {
        assert!(matches!(
            catalog_item_to_library_item(record(Some("org.example.App"))).icon,
            IconSource::Name(name) if name == "org.example.App"
        ));
        assert!(matches!(
            catalog_item_to_library_item(record(Some("/usr/share/icons/example.png"))).icon,
            IconSource::Path(path) if path == std::path::Path::new("/usr/share/icons/example.png")
        ));
    }

    #[test]
    fn missing_application_icon_uses_fallback() {
        assert!(matches!(
            catalog_item_to_library_item(record(None)).icon,
            IconSource::Name(name) if name == "application-x-executable"
        ));
    }

    /// A launcher may write its artwork as a `file://` URL; the scheme off, it is
    /// the same file, and a renderer cannot open it as a name.
    #[test]
    fn a_file_url_is_read_as_the_file_it_names() {
        assert!(matches!(
            catalog_item_to_library_item(record(Some("file:///home/user/.local/share/art/cover.png"))).icon,
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
            catalog_item_to_library_item(record(Some(romm))).icon,
            IconSource::Name(name) if name == "application-x-executable"
        ));

        let mut game = record(Some(romm));
        game.kind = "game".to_owned();
        assert!(matches!(
            catalog_item_to_library_item(game).icon,
            IconSource::Name(name) if name == "applications-games"
        ));
    }

    #[test]
    fn game_store_is_typed_without_a_category_tag() {
        let mut game = record(None);
        game.kind = "game".to_owned();
        game.metadata =
            serde_json::json!({"categories": ["Game", "Epic Games"], "store": "Epic Games"});

        let item = catalog_item_to_library_item(game);
        assert_eq!(item.categories, ["Game"]);
        assert_eq!(item.store.as_deref(), Some("Epic Games"));
    }

    #[test]
    fn an_application_carries_where_it_was_installed() {
        let mut application = record(None);
        application.metadata["source"] = serde_json::json!("flatpak");

        let item = catalog_item_to_library_item(application);
        assert_eq!(item.categories, ["Utility"]);
        assert_eq!(item.sources, ["flatpak"]);
    }

    /// The model can say an application suits the couch, and only that: its
    /// doubt about one the rules already know is no reason to demote it.
    #[test]
    fn a_model_verdict_can_promote_an_application_but_never_demote_one() {
        let resolved_form = |form: FormFactor, id: &str| {
            let mut application = record(None);
            application.id = id.into();
            application.metadata["form_factor"] = serde_json::json!(match form {
                FormFactor::Couch => "couch",
                FormFactor::Desktop => "desktop",
            });
            catalog_item_to_library_item(application).form_factor
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
        client.id = "desktop:net.stremio.Stremio.desktop".into();
        client.metadata = serde_json::json!({"categories": ["Video"], "source": "flatpak"});

        let item = catalog_item_to_library_item(client);
        assert_eq!(item.categories, ["Video"]);
        assert_eq!(item.sources, ["flatpak", "streaming"]);
    }

    #[test]
    fn missing_or_empty_origin_does_not_add_a_source() {
        for origin in [None, Some(String::new())] {
            let mut application = record(None);
            application.metadata["source"] = serde_json::json!(origin);
            let item = catalog_item_to_library_item(application);
            assert!(item.sources.is_empty());
            assert_eq!(item.categories, ["Utility"]);
        }
    }
}
