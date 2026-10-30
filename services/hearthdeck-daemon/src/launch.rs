use hearthdeck_protocol::{
    ActivityEntry, BridgeRequest, CatalogItem, HeroicRunner, InputProfile, StremioMediaType,
    sources,
};

pub(crate) struct LaunchPlan {
    pub request: BridgeRequest,
    pub activity: ActivityEntry,
}

pub(crate) fn catalog(
    item: &CatalogItem,
    session_id: String,
    input_profile: InputProfile,
) -> Option<LaunchPlan> {
    let launch_id = item.launch_id.as_deref()?;
    let stremio_title = (item.source_id == sources::STREMIO
        || item.source_id == sources::STREMIO_ARRIVALS)
        .then(|| stremio_title_target(item))
        .flatten();
    let request = if item.source_id == sources::HEROIC {
        let (runner, application_id) = heroic_launch_target(launch_id)?;
        BridgeRequest::LaunchHeroicGame {
            runner,
            application_id,
            session_id,
            input_profile,
        }
    } else if let Some((video_id, media_type)) = stremio_title {
        BridgeRequest::LaunchStremioTitle {
            application_id: launch_id.to_owned(),
            video_id,
            media_type,
            session_id,
            input_profile,
        }
    } else {
        BridgeRequest::LaunchApplication {
            // Catalog providers name desktop entries; the bridge owns their launch.
            source_id: sources::DESKTOP_APPS.to_owned(),
            application_id: launch_id.to_owned(),
            session_id,
            input_profile,
        }
    };
    Some(LaunchPlan {
        request,
        activity: catalog_activity_entry(item),
    })
}

fn stremio_title_target(item: &CatalogItem) -> Option<(String, StremioMediaType)> {
    let discovery = item.metadata.get("discovery")?;
    let video_id = discovery
        .get("video_id")?
        .as_str()
        .map(str::trim)
        .filter(|video_id| !video_id.is_empty())?;
    let media_type = match discovery.get("type").and_then(serde_json::Value::as_str) {
        Some("series") => StremioMediaType::Series,
        // Records predating the type field were launched as films.
        _ => StremioMediaType::Movie,
    };
    Some((video_id.to_owned(), media_type))
}

fn catalog_activity_entry(item: &CatalogItem) -> ActivityEntry {
    let categories = item
        .metadata
        .get("categories")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned)
        .collect();
    ActivityEntry {
        id: format!("hearthdeck:{}", item.id),
        title: item.title.clone(),
        icon: item.icon.clone(),
        categories,
        kind: Some(item.kind.clone()),
        source: item.source_id.clone(),
        metadata: item.metadata.clone(),
    }
}

fn heroic_launch_target(launch_id: &str) -> Option<(HeroicRunner, String)> {
    let (runner, application_id) = launch_id.split_once(':')?;
    if application_id.is_empty() {
        return None;
    }
    let runner = match runner {
        "legendary" => HeroicRunner::Legendary,
        "gog" => HeroicRunner::Gog,
        _ => return None,
    };
    Some((runner, application_id.to_owned()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn item(source: &str, launch_id: Option<&str>, metadata: serde_json::Value) -> CatalogItem {
        CatalogItem {
            id: "test:item".to_owned(),
            source_id: source.to_owned(),
            title: "Test title".to_owned(),
            kind: "game".to_owned(),
            launch_id: launch_id.map(str::to_owned),
            icon: Some("test-icon".to_owned()),
            metadata,
            classification: Default::default(),
        }
    }

    #[test]
    fn heroic_plans_accept_only_supported_runners_with_an_application_id() {
        for (launch_id, runner, id) in [
            ("legendary:Fortnite", HeroicRunner::Legendary, "Fortnite"),
            ("gog:1091500", HeroicRunner::Gog, "1091500"),
        ] {
            let plan = catalog(
                &item(sources::HEROIC, Some(launch_id), json!({})),
                "session".to_owned(),
                InputProfile::Desktop,
            )
            .unwrap();
            assert!(
                matches!(plan.request, BridgeRequest::LaunchHeroicGame {
                runner: actual, application_id, session_id, input_profile: InputProfile::Desktop,
            } if matches!((&actual, &runner),
                (HeroicRunner::Legendary, HeroicRunner::Legendary) |
                (HeroicRunner::Gog, HeroicRunner::Gog))
                && application_id == id && session_id == "session"),
                "{launch_id}"
            );
        }
        for launch_id in [None, Some("steam:570"), Some("legendary:"), Some("invalid")] {
            assert!(
                catalog(
                    &item(sources::HEROIC, launch_id, json!({})),
                    "session".to_owned(),
                    InputProfile::Desktop
                )
                .is_none(),
                "{launch_id:?}"
            );
        }
    }

    #[test]
    fn both_stremio_sources_plan_titles_from_discovery_metadata() {
        for source in [sources::STREMIO, sources::STREMIO_ARRIVALS] {
            for (kind, expected) in [
                (Some("series"), StremioMediaType::Series),
                (Some("movie"), StremioMediaType::Movie),
                (Some("unknown"), StremioMediaType::Movie),
                (None, StremioMediaType::Movie),
            ] {
                let plan = catalog(
                    &item(
                        source,
                        Some("stremio.desktop"),
                        json!({
                            "discovery": {"video_id": " tt123:4:7 ", "type": kind}
                        }),
                    ),
                    "session".to_owned(),
                    InputProfile::Desktop,
                )
                .unwrap();
                assert!(
                    matches!(plan.request, BridgeRequest::LaunchStremioTitle {
                    application_id, video_id, media_type, session_id,
                    input_profile: InputProfile::Desktop,
                } if application_id == "stremio.desktop" && video_id == "tt123:4:7"
                    && media_type == expected && session_id == "session"),
                    "{source}, {kind:?}"
                );
            }
        }
    }

    #[test]
    fn records_without_stremio_targets_keep_their_desktop_launch() {
        for (source, metadata) in [
            (sources::STREMIO, json!({})),
            (
                sources::STREMIO_ARRIVALS,
                json!({"discovery": {"video_id": "  "}}),
            ),
            (sources::STREMIO, json!({"discovery": {"video_id": 42}})),
            (
                "other-provider",
                json!({"discovery": {"video_id": "tt123"}}),
            ),
        ] {
            let plan = catalog(
                &item(source, Some("app.desktop"), metadata),
                "session".to_owned(),
                InputProfile::Desktop,
            )
            .unwrap();
            assert!(
                matches!(plan.request, BridgeRequest::LaunchApplication {
                source_id, application_id, session_id, input_profile: InputProfile::Desktop,
            } if source_id == sources::DESKTOP_APPS && application_id == "app.desktop"
                && session_id == "session"),
                "{source}"
            );
        }
        assert!(
            catalog(
                &item("other-provider", None, json!({})),
                "session".to_owned(),
                InputProfile::Desktop
            )
            .is_none()
        );
    }

    #[test]
    fn activity_keeps_the_catalog_identity_not_the_desktop_launch_identity() {
        let metadata =
            json!({"categories": ["Game", 42, "Favorites"], "discovery": {"video_id": "tt123"}});
        let plan = catalog(
            &item(sources::STREMIO, Some("app.desktop"), metadata.clone()),
            "session".to_owned(),
            InputProfile::Desktop,
        )
        .unwrap();
        let entry = plan.activity;
        assert_eq!(entry.id, "hearthdeck:test:item");
        assert_eq!(entry.title, "Test title");
        assert_eq!(entry.icon.as_deref(), Some("test-icon"));
        assert_eq!(entry.kind.as_deref(), Some("game"));
        assert_eq!(entry.source, sources::STREMIO);
        assert_eq!(entry.categories, ["Game", "Favorites"]);
        assert_eq!(entry.metadata, metadata);
    }
}
