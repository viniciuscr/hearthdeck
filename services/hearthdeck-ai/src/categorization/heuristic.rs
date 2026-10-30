//! A deterministic scan engine using the catalog's shared identity rules.
//!
//! It exists for three reasons: it is the fallback when no checkpoint can be
//! loaded, it is the baseline a Laya scan has to beat before the switch is worth
//! making, and it is what the tests run against, since loading a checkpoint in
//! CI is neither fast nor hermetic.
//!

use super::decision::{
    AppCategorization, AppTraits, Categorizer, CategoryMatch, DecisionProvenance,
};
use super::error::Result;
use super::model::AppProfile;
use super::rules;
use super::taxonomy::{Section, Taxonomy, section_from_traits};

/// Probability assigned to a trait a hard rule matched.
const HARD: f64 = 1.0;
/// Confidence recorded for a category derived from a freedesktop category. The
/// mapping is lossy, so it is deliberately below certainty.
const MAPPED: f64 = 0.6;

/// Namespace the daemon uses to tag a record with its RomM console.
const CONSOLE_PREFIX: &str = "hearthdeck-console:";

/// Freedesktop category to taxonomy slug, checked in order: the first category
/// on a record that has a mapping wins.
const FREEDESKTOP_TO_SLUG: &[(&str, &str)] = &[
    ("audio", "music_audio"),
    ("music", "music_audio"),
    ("video", "photo_video"),
    ("graphics", "photo_video"),
    ("development", "development"),
    ("education", "education"),
    ("science", "education"),
    ("office", "productivity"),
    ("network", "internet"),
    ("chat", "internet"),
    ("webbrowser", "internet"),
    ("email", "internet"),
    ("settings", "system"),
    ("system", "system"),
    ("utility", "system"),
    ("filetools", "system"),
    ("player", "media_center"),
    ("audiovideo", "media_center"),
];

/// The deterministic categorizer.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeuristicCategorizer;

impl HeuristicCategorizer {
    pub fn new() -> Self {
        Self
    }
}

impl Categorizer for HeuristicCategorizer {
    fn id(&self) -> &'static str {
        "heuristic"
    }

    fn categorize(&self, app: &AppProfile, taxonomy: &Taxonomy) -> Result<AppCategorization> {
        let signal = app.signal_terms();
        let is_game = app.has_category("game");
        let emulator = app
            .categories
            .iter()
            .any(|category| category.starts_with(CONSOLE_PREFIX))
            || rules::emulator(&signal);
        // A game is never a streaming-service client, however its exec line is
        // spelled — the same guard the catalog applies.
        let watch = !is_game && rules::streaming(&signal);

        let game_probability = if is_game { HARD } else { 0.0 };
        let emulator_probability = if emulator { HARD } else { 0.0 };
        let (section, section_confidence) =
            section_from_traits(Some(game_probability), Some(emulator_probability), 0.5);

        let mut categories = Vec::new();
        let mut needs_category = false;
        let mut rationale = Vec::new();
        if is_game {
            rationale.push("freedesktop Game category".to_owned());
        }
        if emulator {
            rationale.push("matched an emulator name or console tag".to_owned());
        }
        if watch {
            rationale.push("matched the streaming-service list".to_owned());
        }

        // Only an application can be filed under a candidate category. A game or
        // an emulator is grouped by store or console in the UI, and reporting it
        // as "needs a category" would ask the taxonomy for a tab that the
        // sections it belongs to do not draw from. The lookup still runs, so a
        // taxonomy that does define game-section categories can use them.
        let candidate = if watch {
            Some("video_streaming")
        } else {
            app.categories
                .iter()
                .find_map(|category| slug_for(category))
        };
        match candidate
            .and_then(|slug| taxonomy.by_slug(slug))
            .filter(|category| category.section == section)
        {
            Some(category) => {
                categories.push(CategoryMatch {
                    slug: category.slug.clone(),
                    name: category.name.clone(),
                    confidence: MAPPED,
                });
                if !watch {
                    rationale.push(format!(
                        "mapped from a freedesktop category to {}",
                        category.slug
                    ));
                }
            }
            None if section == Section::Applications => {
                needs_category = true;
                rationale.push("no freedesktop category mapped to a tab".to_owned());
            }
            None => {}
        }

        Ok(AppCategorization {
            app_id: app.id.clone(),
            title: app.title.clone(),
            section,
            section_confidence,
            categories,
            needs_category,
            traits: AppTraits {
                game: Some(game_probability),
                watch_service: Some(if watch { HARD } else { 0.0 }),
                emulator: Some(emulator_probability),
                couch: None,
            },
            provenance: DecisionProvenance::Heuristic,
            rationale: (!rationale.is_empty()).then(|| rationale.join("; ")),
        })
    }
}

fn slug_for(category: &str) -> Option<&'static str> {
    let category = category.to_lowercase();
    FREEDESKTOP_TO_SLUG
        .iter()
        .find(|(name, _)| *name == category)
        .map(|(_, slug)| *slug)
}

#[cfg(test)]
mod tests {
    use super::HeuristicCategorizer;
    use crate::categorization::decision::Categorizer;
    use crate::categorization::model::{AppKind, AppProfile};
    use crate::categorization::taxonomy::{Section, Taxonomy};

    fn categorize(app: &AppProfile) -> crate::categorization::decision::AppCategorization {
        HeuristicCategorizer::new()
            .categorize(app, &Taxonomy::baseline())
            .unwrap()
    }

    #[test]
    fn a_steam_game_is_a_pc_game() {
        let app = AppProfile {
            id: "steam:440".to_owned(),
            title: "Team Fortress 2".to_owned(),
            kind: Some(AppKind::Game),
            categories: vec!["Game".to_owned()],
            ..AppProfile::default()
        };
        let result = categorize(&app);
        assert_eq!(result.section, Section::PcGames);
        assert!(result.categories.is_empty());
        assert!(!result.needs_category);
    }

    #[test]
    fn retroarch_is_a_console_game() {
        let app = AppProfile {
            id: "org.libretro.RetroArch.desktop".to_owned(),
            title: "RetroArch".to_owned(),
            exec: Some("/usr/bin/retroarch".to_owned()),
            ..AppProfile::default()
        };
        let result = categorize(&app);

        assert_eq!(result.section, Section::ConsoleGames);
        // Console games are grouped by console, so an emulator with no
        // freedesktop categories must not be reported as needing one.
        assert!(result.categories.is_empty());
        assert!(!result.needs_category);
    }

    #[test]
    fn a_browser_window_for_netflix_is_a_streaming_app() {
        let app = AppProfile {
            id: "netflix.desktop".to_owned(),
            title: "Netflix".to_owned(),
            kind: Some(AppKind::Application),
            exec: Some("chromium --app=https://netflix.com".to_owned()),
            categories: vec!["AudioVideo".to_owned()],
            ..AppProfile::default()
        };
        let result = categorize(&app);
        assert_eq!(result.section, Section::Applications);
        assert_eq!(result.categories[0].slug, "video_streaming");
        assert!(result.traits.is_watch_service(0.5));
    }

    #[test]
    fn an_unmappable_application_asks_for_a_category() {
        let app = AppProfile {
            id: "com.example.Thing.desktop".to_owned(),
            title: "Thing".to_owned(),
            ..AppProfile::default()
        };
        let result = categorize(&app);
        assert_eq!(result.section, Section::Applications);
        assert!(result.needs_category);
        assert!(result.categories.is_empty());
    }
}
