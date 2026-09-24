//! Full-library scan: research, decide, aggregate.
//!
//! This is the unit a service drives. It is deliberately one-shot and owns
//! nothing long-lived: the caller builds a [`Categorizer`] (loading a checkpoint
//! is the expensive, memory-hungry part), runs a scan, then drops both.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Utc;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::decision::{AppCategorization, Categorizer};
use crate::error::{CategorizerError, Result};
use crate::model::AppProfile;
use crate::research::AppResearcher;
use crate::taxonomy::{Section, Taxonomy};

/// Progress notification, emitted once per application.
pub type ProgressCallback = Arc<dyn Fn(ScanProgress) + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanOptions {
    /// Ask the researcher about every application. Off by default: it is the
    /// only part of a scan that touches the network.
    pub research: bool,
    /// How many research lookups may be in flight at once.
    pub research_concurrency: usize,
    /// Fewest members a category needs before it is recommended as a tab.
    pub min_category_members: usize,
    /// Mean confidence a category needs before it is recommended as a tab.
    pub min_category_confidence: f64,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            research: false,
            research_concurrency: 4,
            min_category_members: 1,
            min_category_confidence: 0.4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanPhase {
    Researching,
    Deciding,
    Aggregating,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanProgress {
    pub phase: ScanPhase,
    pub completed: usize,
    pub total: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
}

/// One application the categorizer could not handle. The scan keeps going: a
/// single bad record must not cost the user their whole library, the same way
/// one failing discovery provider does not empty the catalog.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanFailure {
    pub app_id: String,
    pub error: String,
}

/// An application the model could not place: the signal that the taxonomy needs
/// a new entry, which is what a human (or a later pass) has to act on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnclassifiedApp {
    pub app_id: String,
    pub title: String,
}

/// A candidate tab and what the scan found for it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CategoryProposal {
    pub slug: String,
    pub name: String,
    pub section: Section,
    pub app_ids: Vec<String>,
    pub mean_confidence: f64,
    /// True when the category clears the membership and confidence bars, i.e.
    /// when it is worth creating a tab for.
    pub recommended: bool,
}

/// Everything one scan produced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanReport {
    pub generated_at: String,
    /// Id of the engine that produced the assignments.
    pub categorizer: String,
    /// Id of the researcher, when researching was enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub researcher: Option<String>,
    pub app_count: usize,
    pub assignments: Vec<AppCategorization>,
    pub failures: Vec<ScanFailure>,
    /// Candidate tabs, best first.
    pub categories: Vec<CategoryProposal>,
    pub unclassified: Vec<UnclassifiedApp>,
}

impl ScanReport {
    /// The apps this report put in the categories that share a dashboard rail.
    ///
    /// The one place that knows how a rail's members are worked out, so the thing
    /// that writes the rail and the thing that resolves it cannot disagree.
    pub fn rail_app_ids(&self, taxonomy: &Taxonomy, rail_id: &str) -> Vec<String> {
        let slugs: Vec<&str> = taxonomy
            .on_rail(rail_id)
            .map(|category| category.slug.as_str())
            .collect();
        let mut app_ids: Vec<String> = Vec::new();
        for category in &self.categories {
            if !slugs.contains(&category.slug.as_str()) {
                continue;
            }
            for app_id in &category.app_ids {
                // An app two categories of one rail both claim still belongs on
                // that rail once.
                if !app_ids.iter().any(|known| known == app_id) {
                    app_ids.push(app_id.clone());
                }
            }
        }
        app_ids
    }

    /// The categories worth creating a tab for, in recommendation order.
    pub fn recommended_categories(&self) -> impl Iterator<Item = &CategoryProposal> {
        self.categories
            .iter()
            .filter(|category| category.recommended)
    }

    pub fn categorizations_for(&self, app_id: &str) -> Option<&AppCategorization> {
        self.assignments
            .iter()
            .find(|assignment| assignment.app_id == app_id)
    }
}

/// Drives a scan over one engine and one taxonomy.
pub struct LibraryScanner {
    categorizer: Arc<dyn Categorizer>,
    researcher: Option<Arc<dyn AppResearcher>>,
    taxonomy: Taxonomy,
    options: ScanOptions,
    progress: Option<ProgressCallback>,
}

impl LibraryScanner {
    /// A scanner with the baseline taxonomy and default options.
    pub fn new(categorizer: Arc<dyn Categorizer>) -> Self {
        Self {
            categorizer,
            researcher: None,
            taxonomy: Taxonomy::baseline(),
            options: ScanOptions::default(),
            progress: None,
        }
    }

    pub fn with_taxonomy(mut self, taxonomy: Taxonomy) -> Self {
        self.taxonomy = taxonomy;
        self
    }

    pub fn with_researcher(mut self, researcher: Arc<dyn AppResearcher>) -> Self {
        self.researcher = Some(researcher);
        self
    }

    pub fn with_options(mut self, options: ScanOptions) -> Self {
        self.options = options;
        self
    }

    /// Observe a scan in flight. The callback runs on the blocking decision
    /// worker, so it must not block in turn.
    pub fn with_progress(mut self, progress: ProgressCallback) -> Self {
        self.progress = Some(progress);
        self
    }

    pub async fn scan(&self, apps: Vec<AppProfile>) -> Result<ScanReport> {
        let started = std::time::Instant::now();
        let app_count = apps.len();
        let mut apps = apps;
        let mut failures = Vec::new();

        if self.options.research
            && let Some(researcher) = self.researcher.clone()
        {
            info!(
                researcher = researcher.id(),
                app_count, "categorizer scan: researching"
            );
            failures.extend(self.research_all(&researcher, &mut apps).await);
        }

        let categorizer = self.categorizer.clone();
        let taxonomy = self.taxonomy.clone();
        let progress = self.progress.clone();
        let (assignments, decision_failures) = tokio::task::spawn_blocking(move || {
            decide(categorizer.as_ref(), &taxonomy, &apps, progress.as_ref())
        })
        .await
        .map_err(|error| CategorizerError::Worker(error.to_string()))?;
        failures.extend(decision_failures);

        maybe_progress(
            self.progress.as_ref(),
            ScanProgress {
                phase: ScanPhase::Aggregating,
                completed: app_count,
                total: app_count,
                app_id: None,
            },
        );
        let categories = propose_categories(&assignments, &self.taxonomy, &self.options);
        let unclassified = assignments
            .iter()
            .filter(|assignment| assignment.needs_category)
            .map(|assignment| UnclassifiedApp {
                app_id: assignment.app_id.clone(),
                title: assignment.title.clone(),
            })
            .collect();

        let report = ScanReport {
            generated_at: Utc::now().to_rfc3339(),
            categorizer: self.categorizer.id().to_owned(),
            researcher: self
                .researcher
                .as_ref()
                .filter(|_| self.options.research)
                .map(|researcher| researcher.id().to_owned()),
            app_count,
            assignments,
            failures,
            categories,
            unclassified,
        };
        info!(
            categorizer = report.categorizer,
            app_count,
            failures = report.failures.len(),
            recommended = report.recommended_categories().count(),
            unclassified = report.unclassified.len(),
            duration_ms = started.elapsed().as_millis() as u64,
            "categorizer scan complete"
        );
        Ok(report)
    }

    async fn research_all(
        &self,
        researcher: &Arc<dyn AppResearcher>,
        apps: &mut [AppProfile],
    ) -> Vec<ScanFailure> {
        let total = apps.len();
        let concurrency = self.options.research_concurrency.max(1);
        let progress = self.progress.clone();
        let results: Vec<(usize, Result<Option<AppProfile>>)> =
            futures_util::stream::iter(apps.iter().enumerate().map(|(index, app)| {
                let researcher = researcher.clone();
                let progress = progress.clone();
                async move {
                    let outcome = researcher.research(app).await;
                    if let Some(progress) = progress.as_ref() {
                        progress(ScanProgress {
                            phase: ScanPhase::Researching,
                            completed: index + 1,
                            total,
                            app_id: Some(app.id.clone()),
                        });
                    }
                    (index, outcome)
                }
            }))
            .buffered(concurrency)
            .collect()
            .await;

        let mut failures = Vec::new();
        for (index, outcome) in results {
            match outcome {
                Ok(Some(researched)) => apps[index].merge_research(&researched),
                Ok(None) => {}
                Err(error) => {
                    debug!(app_id = apps[index].id, %error, "research lookup failed");
                    failures.push(ScanFailure {
                        app_id: apps[index].id.clone(),
                        error: error.to_string(),
                    });
                }
            }
        }
        failures
    }
}

/// The synchronous decision pass, run on a blocking worker.
fn decide(
    categorizer: &dyn Categorizer,
    taxonomy: &Taxonomy,
    apps: &[AppProfile],
    progress: Option<&ProgressCallback>,
) -> (Vec<AppCategorization>, Vec<ScanFailure>) {
    let total = apps.len();
    let mut assignments = Vec::with_capacity(total);
    let mut failures = Vec::new();
    for (index, app) in apps.iter().enumerate() {
        match categorizer.categorize(app, taxonomy) {
            Ok(assignment) => assignments.push(assignment),
            Err(error) => failures.push(ScanFailure {
                app_id: app.id.clone(),
                error: error.to_string(),
            }),
        }
        maybe_progress(
            progress,
            ScanProgress {
                phase: ScanPhase::Deciding,
                completed: index + 1,
                total,
                app_id: Some(app.id.clone()),
            },
        );
    }
    (assignments, failures)
}

fn maybe_progress(progress: Option<&ProgressCallback>, update: ScanProgress) {
    if let Some(progress) = progress {
        progress(update);
    }
}

/// Fold per-application verdicts into the tabs worth creating.
///
/// Aggregation is where the "which categories should exist" decision actually
/// lands: the model only ever picks from the candidate list, so the counts and
/// mean confidences here are what turn a candidate into a recommendation.
fn propose_categories(
    assignments: &[AppCategorization],
    taxonomy: &Taxonomy,
    options: &ScanOptions,
) -> Vec<CategoryProposal> {
    let mut members: BTreeMap<&str, (Vec<String>, f64, usize)> = BTreeMap::new();
    for assignment in assignments {
        for category in &assignment.categories {
            let entry = members
                .entry(category.slug.as_str())
                .or_insert_with(|| (Vec::new(), 0.0, 0));
            entry.0.push(assignment.app_id.clone());
            entry.1 += category.confidence;
            entry.2 += 1;
        }
    }

    let mut proposals = Vec::new();
    for (slug, (app_ids, total_confidence, count)) in members {
        let Some(definition) = taxonomy.by_slug(slug) else {
            debug!(slug, "model chose a slug outside the taxonomy");
            continue;
        };
        let mean_confidence = total_confidence / count as f64;
        proposals.push(CategoryProposal {
            slug: definition.slug.clone(),
            name: definition.name.clone(),
            section: definition.section,
            recommended: app_ids.len() >= options.min_category_members
                && mean_confidence >= options.min_category_confidence,
            app_ids,
            mean_confidence,
        });
    }

    proposals.sort_by(|left, right| {
        right
            .recommended
            .cmp(&left.recommended)
            .then(right.app_ids.len().cmp(&left.app_ids.len()))
            .then(left.name.cmp(&right.name))
    });
    proposals
}

#[cfg(test)]
mod tests {
    use super::{LibraryScanner, ScanOptions, ScanPhase, ScanProgress};
    use crate::decision::Categorizer;
    use crate::heuristic::HeuristicCategorizer;
    use crate::model::AppProfile;
    use crate::research::AppResearcher;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    fn library() -> Vec<AppProfile> {
        vec![
            AppProfile {
                id: "netflix.desktop".to_owned(),
                title: "Netflix".to_owned(),
                exec: Some("chromium --app=https://netflix.com".to_owned()),
                ..AppProfile::default()
            },
            AppProfile {
                id: "org.videolan.VLC.desktop".to_owned(),
                title: "VLC media player".to_owned(),
                categories: vec!["AudioVideo".to_owned(), "Player".to_owned()],
                ..AppProfile::default()
            },
            AppProfile {
                id: "org.gnome.Builder.desktop".to_owned(),
                title: "Builder".to_owned(),
                categories: vec!["Development".to_owned()],
                ..AppProfile::default()
            },
            AppProfile {
                id: "com.example.Untagged.desktop".to_owned(),
                title: "Untagged".to_owned(),
                ..AppProfile::default()
            },
        ]
    }

    #[tokio::test]
    async fn a_rail_is_the_categories_that_share_it() {
        let scanner = LibraryScanner::new(Arc::new(HeuristicCategorizer::new()));
        let report = scanner.scan(library()).await.unwrap();
        let taxonomy = crate::taxonomy::Taxonomy::baseline();

        // Netflix is Video & Streaming and VLC is Media Center: two tabs, one
        // rail, because "what is there to watch" is one question to a person.
        let mut on_rail = report.rail_app_ids(&taxonomy, "watch");
        on_rail.sort();
        assert_eq!(
            on_rail,
            vec![
                "netflix.desktop".to_owned(),
                "org.videolan.VLC.desktop".to_owned()
            ]
        );
        // The development category is on no rail, so its app is not dragged in:
        // a rail is not "everything the scan decided".
        assert!(!on_rail.contains(&"org.gnome.Builder.desktop".to_owned()));
        // A rail nobody declares has no members.
        assert!(report.rail_app_ids(&taxonomy, "listen").is_empty());
    }

    #[tokio::test]
    async fn a_scan_reports_tabs_unclassified_apps_and_progress() {
        let phases = Arc::new(Mutex::new(Vec::new()));
        let seen = phases.clone();
        let scanner = LibraryScanner::new(Arc::new(HeuristicCategorizer::new())).with_progress(
            Arc::new(move |progress: ScanProgress| {
                seen.lock()
                    .unwrap()
                    .push((progress.phase, progress.completed));
            }),
        );

        let report = scanner.scan(library()).await.unwrap();

        assert_eq!(report.app_count, 4);
        assert!(report.failures.is_empty());
        assert_eq!(report.categorizer, "heuristic");
        assert_eq!(report.unclassified.len(), 1);
        assert_eq!(
            report.unclassified[0].app_id,
            "com.example.Untagged.desktop"
        );

        let slugs: Vec<&str> = report
            .recommended_categories()
            .map(|category| category.slug.as_str())
            .collect();
        assert_eq!(
            slugs,
            vec!["development", "media_center", "video_streaming"]
        );
        assert_eq!(phases.lock().unwrap().len(), 5);
        assert_eq!(
            phases.lock().unwrap().last().unwrap().0,
            ScanPhase::Aggregating
        );
    }

    struct FakeResearcher;

    #[async_trait]
    impl AppResearcher for FakeResearcher {
        fn id(&self) -> &'static str {
            "fake"
        }

        async fn research(&self, app: &AppProfile) -> crate::error::Result<Option<AppProfile>> {
            if app.id != "com.example.Untagged.desktop" {
                return Ok(None);
            }
            Ok(Some(AppProfile {
                id: app.id.clone(),
                title: "Upstream name".to_owned(),
                summary: Some("A note-taking app".to_owned()),
                categories: vec!["Office".to_owned()],
                ..AppProfile::default()
            }))
        }
    }

    #[tokio::test]
    async fn research_can_rescue_an_untagged_application() {
        let scanner = LibraryScanner::new(Arc::new(HeuristicCategorizer::new()))
            .with_researcher(Arc::new(FakeResearcher))
            .with_options(ScanOptions {
                research: true,
                ..ScanOptions::default()
            });

        let report = scanner.scan(library()).await.unwrap();

        assert_eq!(report.researcher.as_deref(), Some("fake"));
        assert!(report.unclassified.is_empty());
        let rescued = report
            .categorizations_for("com.example.Untagged.desktop")
            .unwrap();
        assert_eq!(rescued.categories[0].slug, "productivity");
        // Research never rewrites installed truth.
        assert_eq!(rescued.title, "Untagged");
    }

    #[tokio::test]
    async fn a_failing_record_does_not_sink_the_scan() {
        struct Picky;

        impl Categorizer for Picky {
            fn id(&self) -> &'static str {
                "picky"
            }

            fn categorize(
                &self,
                app: &AppProfile,
                taxonomy: &crate::taxonomy::Taxonomy,
            ) -> crate::error::Result<crate::decision::AppCategorization> {
                if app.id.contains("VLC") {
                    return Err(crate::error::CategorizerError::Inference("boom".into()));
                }
                HeuristicCategorizer::new().categorize(app, taxonomy)
            }
        }

        let report = LibraryScanner::new(Arc::new(Picky))
            .scan(library())
            .await
            .unwrap();

        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].app_id, "org.videolan.VLC.desktop");
        assert_eq!(report.assignments.len(), 3);
    }
}
