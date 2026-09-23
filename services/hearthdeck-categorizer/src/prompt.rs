//! The prompt contract: how an application becomes Laya state and questions, and
//! how Laya's answers become an [`AppCategorization`].
//!
//! Everything here is plain `serde_json`, with no dependency on the inference
//! runtime. That keeps the interesting half of the engine — what we ask and how
//! we read the reply — unit-testable without a checkpoint, and lets the
//! deterministic engine share the same interpretation rules.

use serde_json::{Map, Value, json};

use crate::decision::{AppCategorization, AppTraits, CategoryMatch, DecisionProvenance};
use crate::model::AppProfile;
use crate::taxonomy::{Section, Taxonomy};

/// Question ids. Stable because the prompt cache in Laya keys off them.
pub const Q_GAME: &str = "is_game";
pub const Q_EMULATOR: &str = "is_emulator";
pub const Q_WATCH: &str = "is_watch_service";
pub const Q_CATEGORY: &str = "category";

/// The escape hatch appended to the category choices. An app that lands here is
/// what tells the aggregate pass that the taxonomy is missing an entry.
pub const CATEGORY_NONE: &str = "None of these - the app does not fit any of the listed categories";

/// How much of a description is fed to the model. The state shares the token
/// budget with the question head, and the head is where the option markers live,
/// so an unbounded blurb would eat the answer.
const DESCRIPTION_BUDGET: usize = 600;

/// Render the application as the model's "state".
///
/// A string, not JSON: `serialize_state` passes strings through, which lets us
/// order the fields ourselves. Order matters because the state is truncated from
/// the right, so what identifies the app best has to come first.
pub fn build_state(app: &AppProfile) -> String {
    let mut lines = Vec::with_capacity(10);
    lines.push(format!("title: {}", app.title.trim()));
    if let Some(kind) = app.kind {
        lines.push(format!("kind: {kind:?}").to_lowercase());
    }
    if let Some(summary) = app.summary.as_deref() {
        lines.push(format!("summary: {}", summary.trim()));
    }
    if let Some(developer) = app.developer.as_deref() {
        lines.push(format!("developer: {}", developer.trim()));
    }
    if !app.categories.is_empty() {
        lines.push(format!("categories: {}", app.categories.join("; ")));
    }
    if let Some(store) = app.store.as_deref() {
        lines.push(format!("store: {}", store.trim()));
    }
    if let Some(platform) = app.platform.as_deref() {
        lines.push(format!("platform: {}", platform.trim()));
    }
    if let Some(exec) = app.exec.as_deref() {
        lines.push(format!("launch: {}", exec.trim()));
    }
    if !app.keywords.is_empty() {
        lines.push(format!("keywords: {}", app.keywords.join(", ")));
    }
    if let Some(description) = app.description.as_deref() {
        lines.push(format!(
            "description: {}",
            truncate(description.trim(), DESCRIPTION_BUDGET)
        ));
    }
    lines.push(format!("id: {}", app.id));
    lines.join("\n")
}

fn truncate(text: &str, budget: usize) -> String {
    if text.chars().count() <= budget {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(budget).collect();
    out.push('…');
    out
}

/// Build the question map for a taxonomy.
///
/// Three `noul` propositions carry the section split; one `choice` carries the
/// category. The category list is a JSON array rather than a map on purpose:
/// Laya preserves array order for `choice` criteria but re-sorts object keys.
pub fn build_questions(taxonomy: &Taxonomy) -> Map<String, Value> {
    let mut categories: Vec<String> = taxonomy
        .categories()
        .iter()
        .map(|category| category.label())
        .collect();
    categories.push(CATEGORY_NONE.to_owned());

    let mut questions = Map::new();
    questions.insert(
        Q_GAME.to_owned(),
        json!({
            "type": "noul",
            "instructions": "The following application is a video game, i.e. software whose primary purpose is playing games. Emulators, game launchers and retro-game frontends count as games; media players, browsers and utilities do not.",
            "criteria": {
                "false": "no, this is a general-purpose application, tool or media player",
                "true": "yes, this is a video game or a game launcher"
            }
        }),
    );
    questions.insert(
        Q_EMULATOR.to_owned(),
        json!({
            "type": "noul",
            "instructions": "The following application is a console or arcade emulator, or a retro-game frontend, i.e. software that runs ROM images of console and arcade games.",
            "criteria": {
                "false": "no, this does not run console or arcade ROMs",
                "true": "yes, this is a console or arcade emulator or a retro-game frontend"
            }
        }),
    );
    questions.insert(
        Q_WATCH.to_owned(),
        json!({
            "type": "noul",
            "instructions": "The following application exists so that the user watches streaming, broadcast or on-demand video, such as Netflix, Disney+, Prime Video, Max, Crunchyroll, Hulu or a live-TV service. Local media players, games and video editors do not count.",
            "criteria": {
                "false": "no, this is not a streaming or broadcast video service",
                "true": "yes, this is a service for watching streaming or broadcast video"
            }
        }),
    );
    questions.insert(
        Q_CATEGORY.to_owned(),
        json!({
            "type": "choice",
            "instructions": "Pick the single category that best describes this application.",
            "criteria": categories
        }),
    );
    questions
}

/// Turn one prediction's answers into a verdict.
///
/// The category answer is accepted only when it agrees with the section the
/// trait questions implied, so a contradictory answer degrades to
/// `needs_category` instead of silently misfiling the app.
pub fn interpret(
    app: &AppProfile,
    answers: &Map<String, Value>,
    taxonomy: &Taxonomy,
    threshold: f64,
) -> AppCategorization {
    let game = noul(answers.get(Q_GAME));
    let emulator = noul(answers.get(Q_EMULATOR));
    let watch_service = noul(answers.get(Q_WATCH));
    // A reply that answered neither trait question says nothing about where the
    // app belongs, so it is not allowed to look like a confident "Applications".
    let (section, section_confidence) = if game.is_none() && emulator.is_none() {
        (Section::Applications, 0.0)
    } else {
        Section::from_traits(game, emulator, threshold)
    };

    let answer = answers.get(Q_CATEGORY);
    let chosen = choice(answer).and_then(|label| taxonomy.by_label(label));
    let mut categories = Vec::new();
    let mut needs_category = false;
    let mut rationale = None;

    match chosen {
        Some(category) if category.section == section => categories.push(CategoryMatch {
            slug: category.slug.clone(),
            name: category.name.clone(),
            confidence: probability(answer, &category.label())
                .unwrap_or_else(|| confidence(answer)),
        }),
        _ if section == Section::Applications => {
            needs_category = true;
            rationale = Some(match choice(answer) {
                Some(label) => format!("category answer {label:?} does not name a candidate"),
                None => "the model returned no category answer".to_owned(),
            });
        }
        _ => {}
    }

    AppCategorization {
        app_id: app.id.clone(),
        title: app.title.clone(),
        section,
        section_confidence,
        categories,
        needs_category,
        traits: AppTraits {
            game,
            watch_service,
            emulator,
        },
        provenance: DecisionProvenance::Laya,
        rationale,
    }
}

fn noul(answer: Option<&Value>) -> Option<f64> {
    answer?.get("noul")?.as_f64()
}

fn choice(answer: Option<&Value>) -> Option<&str> {
    answer?.get("choice")?.as_str()
}

fn probability(answer: Option<&Value>, label: &str) -> Option<f64> {
    answer?.get("probabilities")?.get(label)?.as_f64()
}

fn confidence(answer: Option<&Value>) -> f64 {
    answer
        .and_then(|answer| answer.get("confidence"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::{
        CATEGORY_NONE, Q_CATEGORY, Q_EMULATOR, Q_GAME, Q_WATCH, build_questions, build_state,
        interpret,
    };
    use crate::model::{AppKind, AppProfile};
    use crate::taxonomy::{Section, Taxonomy};
    use serde_json::{Value, json};

    fn app() -> AppProfile {
        AppProfile {
            id: "netflix.desktop".to_owned(),
            title: "Netflix".to_owned(),
            kind: Some(AppKind::Application),
            exec: Some("chromium --app=https://netflix.com".to_owned()),
            ..AppProfile::default()
        }
    }

    fn answers(pairs: &[(&str, Value)]) -> serde_json::Map<String, Value> {
        pairs
            .iter()
            .map(|(id, value)| ((*id).to_owned(), value.clone()))
            .collect()
    }

    #[test]
    fn state_leads_with_the_title() {
        let state = build_state(&app());
        assert!(state.starts_with("title: Netflix\n"));
        assert!(state.contains("launch: chromium --app=https://netflix.com"));
    }

    #[test]
    fn questions_carry_an_ordered_category_list_with_an_escape_hatch() {
        let taxonomy = Taxonomy::baseline();
        let questions = build_questions(&taxonomy);
        let criteria = questions[Q_CATEGORY]["criteria"].as_array().unwrap();

        assert_eq!(criteria.len(), taxonomy.categories().len() + 1);
        assert_eq!(criteria[0], json!(taxonomy.categories()[0].label()));
        assert_eq!(criteria.last().unwrap(), &json!(CATEGORY_NONE));
        assert!(questions[Q_GAME]["criteria"]["true"].is_string());
        assert_eq!(questions[Q_WATCH]["type"], "noul");
        assert_eq!(questions[Q_EMULATOR]["type"], "noul");
    }

    #[test]
    fn a_confident_streaming_service_becomes_video_streaming() {
        let taxonomy = Taxonomy::baseline();
        let label = taxonomy.by_slug("video_streaming").unwrap().label();
        let result = interpret(
            &app(),
            &answers(&[
                (Q_GAME, json!({"noul": 0.02})),
                (Q_EMULATOR, json!({"noul": 0.01})),
                (Q_WATCH, json!({"noul": 0.97})),
                (
                    Q_CATEGORY,
                    json!({"choice": label, "probabilities": {label.clone(): 0.91}, "confidence": 0.8}),
                ),
            ]),
            &taxonomy,
            0.5,
        );

        assert_eq!(result.section, Section::Applications);
        assert!(!result.needs_category);
        assert_eq!(result.categories.len(), 1);
        assert_eq!(result.categories[0].slug, "video_streaming");
        assert_eq!(result.categories[0].confidence, 0.91);
        assert!(result.traits.is_watch_service(0.5));
    }

    #[test]
    fn emulators_land_in_console_games_and_keep_no_category() {
        let result = interpret(
            &app(),
            &answers(&[
                (Q_GAME, json!({"noul": 0.93})),
                (Q_EMULATOR, json!({"noul": 0.88})),
                (
                    Q_CATEGORY,
                    json!({"choice": CATEGORY_NONE, "probabilities": {}, "confidence": 0.2}),
                ),
            ]),
            &Taxonomy::baseline(),
            0.5,
        );

        assert_eq!(result.section, Section::ConsoleGames);
        assert!(result.categories.is_empty());
        assert!(!result.needs_category);
    }

    #[test]
    fn a_reply_without_trait_answers_is_left_unconfident_and_unfiled() {
        let result = interpret(
            &app(),
            &answers(&[(Q_CATEGORY, json!({"choice": CATEGORY_NONE}))]),
            &Taxonomy::baseline(),
            0.5,
        );

        assert_eq!(result.section, Section::Applications);
        assert_eq!(result.section_confidence, 0.0);
        assert!(result.needs_category);
    }

    #[test]
    fn an_unmatched_category_on_an_application_asks_for_a_new_one() {
        let result = interpret(
            &app(),
            &answers(&[
                (Q_GAME, json!({"noul": 0.05})),
                (Q_EMULATOR, json!({"noul": 0.02})),
                (
                    Q_CATEGORY,
                    json!({"choice": CATEGORY_NONE, "confidence": 0.4}),
                ),
            ]),
            &Taxonomy::baseline(),
            0.5,
        );

        assert_eq!(result.section, Section::Applications);
        assert!(result.needs_category);
        assert!(result.rationale.is_some());
    }
}
