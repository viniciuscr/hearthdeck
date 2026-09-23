//! On-demand library categorization.
//!
//! A scan is not run in this process. The model is far too big to keep resident
//! beside the library and the UI: a scan wants hundreds of megabytes and seconds
//! of CPU, and the daemon owns both the catalog and the API the UI is using. So
//! the daemon writes the library out as JSON, spawns the `hearthdeck-categorizer`
//! binary, waits for it, and reads the report back. The heavy state lives in a
//! process that exits.
//!
//! That also answers "not while nobody asked": nothing is loaded until a scan is
//! requested, at most one runs at a time, and the child is gone when it is done.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use anyhow::Context;
use async_trait::async_trait;
use chrono::Utc;
use hearthdeck_categorizer::{AppKind, AppProfile, ScanReport};
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use tokio::sync::{Mutex, broadcast};
use tracing::{error, info};

use crate::{
    catalog::{CatalogItem, CatalogStore},
    state::ServerEvent,
};

/// Which engine the child runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanEngine {
    /// The deterministic tables, no checkpoint.
    Heuristic,
    /// The Laya checkpoint.
    Laya,
}

impl ScanEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::Laya => "laya",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "heuristic" => Some(Self::Heuristic),
            "laya" => Some(Self::Laya),
            _ => None,
        }
    }
}

/// Which Laya checkpoint to load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanModel {
    English,
    Multilingual,
}

impl ScanModel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::English => "english",
            Self::Multilingual => "multilingual",
        }
    }
}

/// How to run a scan.
#[derive(Clone, Debug)]
pub struct CategorizerSettings {
    /// The sibling `hearthdeck-categorizer` binary. A bare name is looked up on
    /// `PATH`, which is how the packaged unit finds it in `~/.local/bin`.
    pub binary: PathBuf,
    /// Scratch directory for the library and report handoff files.
    pub work_dir: PathBuf,
    pub engine: ScanEngine,
    pub model: ScanModel,
    /// A local checkpoint directory, so a scan does not download one.
    pub model_path: Option<PathBuf>,
    pub dtype: String,
    pub research: bool,
}

impl CategorizerSettings {
    /// Read the daemon's environment, or `None` when categorization is off.
    ///
    /// Off is the default on purpose: enabling it is what opts a machine into
    /// downloading a checkpoint on its first scan.
    pub fn load(database_path: &Path) -> Option<Self> {
        Self::from_lookup(database_path, &|name| std::env::var(name).ok())
    }

    /// The same read, with the environment injected, so it can be tested.
    fn from_lookup(database_path: &Path, get: &dyn Fn(&str) -> Option<String>) -> Option<Self> {
        if !flag(get("HEARTHDECK_CATEGORIZER_ENABLED")) {
            return None;
        }
        // The scratch files sit beside the database rather than in a directory
        // of their own: they are derived from the library the database holds,
        // and a deployment that moves one wants the other to follow.
        let data_dir = database_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        Some(Self {
            binary: get("HEARTHDECK_CATEGORIZER_BIN")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("hearthdeck-categorizer")),
            work_dir: get("HEARTHDECK_CATEGORIZATION_DIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| data_dir.join("categorization")),
            engine: get("HEARTHDECK_CATEGORIZER_ENGINE")
                .as_deref()
                .and_then(ScanEngine::parse)
                .unwrap_or(ScanEngine::Laya),
            model: match get("HEARTHDECK_CATEGORIZER_MODEL").as_deref() {
                Some("multilingual") => ScanModel::Multilingual,
                _ => ScanModel::English,
            },
            model_path: get("HEARTHDECK_CATEGORIZER_MODEL_PATH")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            dtype: get("HEARTHDECK_CATEGORIZER_DTYPE")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "float16".to_owned()),
            research: flag(get("HEARTHDECK_CATEGORIZER_RESEARCH")),
        })
    }
}

fn flag(value: Option<String>) -> bool {
    value.is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

/// Runs one scan over a library.
///
/// A trait because a scan is the one part of the daemon that cannot be tested
/// without a checkpoint: tests inject a runner that answers immediately.
#[async_trait]
pub trait ScanRunner: Send + Sync {
    async fn run(&self, apps: Vec<AppProfile>) -> anyhow::Result<ScanReport>;
}

/// The real runner: hand the library to the categorizer binary.
pub struct ProcessScanRunner {
    settings: CategorizerSettings,
    library_path: PathBuf,
    report_path: PathBuf,
}

impl ProcessScanRunner {
    pub fn new(settings: CategorizerSettings) -> Self {
        Self {
            library_path: settings.work_dir.join("library.json"),
            report_path: settings.work_dir.join("report.json"),
            settings,
        }
    }

    /// The exact command line the categorizer binary understands. Kept in one
    /// place so the two sides cannot drift apart silently.
    fn arguments(&self) -> Vec<OsString> {
        let mut arguments: Vec<OsString> = vec![
            "scan".into(),
            "--library".into(),
            self.library_path.clone().into_os_string(),
            "--output".into(),
            self.report_path.clone().into_os_string(),
            "--engine".into(),
            self.settings.engine.as_str().into(),
            "--model".into(),
            self.settings.model.as_str().into(),
            "--dtype".into(),
            self.settings.dtype.clone().into(),
        ];
        if let Some(path) = &self.settings.model_path {
            arguments.push("--model-path".into());
            arguments.push(path.clone().into_os_string());
        }
        if self.settings.research {
            arguments.push("--research".into());
        }
        arguments
    }
}

#[async_trait]
impl ScanRunner for ProcessScanRunner {
    async fn run(&self, apps: Vec<AppProfile>) -> anyhow::Result<ScanReport> {
        tokio::fs::create_dir_all(&self.settings.work_dir)
            .await
            .with_context(|| {
                format!(
                    "could not create the categorization directory {}",
                    self.settings.work_dir.display()
                )
            })?;
        let library =
            serde_json::to_vec(&apps).context("could not serialize the library for a scan")?;
        tokio::fs::write(&self.library_path, library)
            .await
            .with_context(|| {
                format!(
                    "could not write the scan library {}",
                    self.library_path.display()
                )
            })?;
        // A report left behind by an earlier run would make a failed scan look
        // like a successful one, so it goes before the child starts.
        match tokio::fs::remove_file(&self.report_path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "could not clear the previous report {}",
                        self.report_path.display()
                    )
                });
            }
        }

        let status = tokio::process::Command::new(&self.settings.binary)
            .args(self.arguments())
            .status()
            .await
            .with_context(|| {
                format!(
                    "could not run the categorizer binary {}",
                    self.settings.binary.display()
                )
            })?;
        anyhow::ensure!(status.success(), "the categorizer exited with {status}");

        let report = tokio::fs::read(&self.report_path).await.with_context(|| {
            format!(
                "the categorizer wrote no report to {}",
                self.report_path.display()
            )
        })?;
        serde_json::from_slice(&report).context("the categorizer wrote an unreadable report")
    }
}

/// The latest report, and nothing else: the assignments are read back whole and
/// projected for the UI, so there is no reason to shred them across tables.
/// Keeping only the newest row is deliberate — a report describes one library at
/// one moment, and a stale one is worse than none.
#[derive(Clone)]
struct CategorizationStore {
    pool: SqlitePool,
}

impl CategorizationStore {
    fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn save(&self, report: &ScanReport) -> anyhow::Result<()> {
        let payload = serde_json::to_string(report).context("could not serialize the report")?;
        sqlx::query(
            r#"
            INSERT INTO categorization_reports (id, generated_at, categorizer, payload_json, updated_at)
            VALUES (1, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
              generated_at = excluded.generated_at,
              categorizer = excluded.categorizer,
              payload_json = excluded.payload_json,
              updated_at = excluded.updated_at
            "#,
        )
        .bind(&report.generated_at)
        .bind(&report.categorizer)
        .bind(payload)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn latest(&self) -> anyhow::Result<Option<ScanReport>> {
        let row = sqlx::query("SELECT payload_json FROM categorization_reports WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let payload: String = row.get("payload_json");
        Ok(serde_json::from_str(&payload).ok())
    }
}

/// Whether a scan is in flight, and how the last one ended.
#[derive(Default)]
struct ScanState {
    running: bool,
    last_error: Option<String>,
    last_completed_at: Option<String>,
}

/// What the UI needs to show a spinner, and to explain a failure.
#[derive(Clone, Debug, Serialize)]
pub struct CategorizationStatus {
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_completed_at: Option<String>,
}

/// Outcome of asking for a scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanRequest {
    Started,
    /// A scan is already in flight. Coalescing rather than queueing means a
    /// user leaning on the button cannot pile up scans, and the answer is the
    /// same either way: a fresh report is coming.
    AlreadyRunning,
}

/// Owns the one-at-a-time scan slot and the stored report.
#[derive(Clone)]
pub struct CategorizationService {
    inner: Arc<CategorizationInner>,
}

struct CategorizationInner {
    catalog: CatalogStore,
    store: CategorizationStore,
    events: broadcast::Sender<ServerEvent>,
    runner: Arc<dyn ScanRunner>,
    state: Mutex<ScanState>,
}

impl CategorizationService {
    /// Build the service that spawns the categorizer binary.
    pub fn start(
        settings: CategorizerSettings,
        catalog: CatalogStore,
        pool: SqlitePool,
        events: broadcast::Sender<ServerEvent>,
    ) -> Self {
        Self::with_runner(
            catalog,
            pool,
            events,
            Arc::new(ProcessScanRunner::new(settings)),
        )
    }

    /// Build the service over an arbitrary runner.
    pub fn with_runner(
        catalog: CatalogStore,
        pool: SqlitePool,
        events: broadcast::Sender<ServerEvent>,
        runner: Arc<dyn ScanRunner>,
    ) -> Self {
        Self {
            inner: Arc::new(CategorizationInner {
                catalog,
                store: CategorizationStore::new(pool),
                events,
                runner,
                state: Mutex::new(ScanState::default()),
            }),
        }
    }

    pub async fn status(&self) -> CategorizationStatus {
        let state = self.inner.state.lock().await;
        CategorizationStatus {
            running: state.running,
            last_error: state.last_error.clone(),
            last_completed_at: state.last_completed_at.clone(),
        }
    }

    pub async fn latest_report(&self) -> anyhow::Result<Option<ScanReport>> {
        self.inner.store.latest().await
    }

    /// Ask for a scan. Returns as soon as the scan is accepted, not when it is
    /// finished; completion is announced on the event stream and is visible in
    /// [`CategorizationService::status`].
    pub async fn request_scan(&self) -> ScanRequest {
        if !self.claim().await {
            return ScanRequest::AlreadyRunning;
        }
        let service = self.clone();
        tokio::spawn(async move { service.run_and_record().await });
        ScanRequest::Started
    }

    /// Run the first scan, but only for a library that has never been
    /// categorized. This is the "create the first version" pass: later boots
    /// read the stored report instead of paying for the model again.
    pub async fn scan_if_missing(&self) {
        match self.inner.store.latest().await {
            Ok(Some(_)) => info!("categorization report present; skipping the startup scan"),
            Ok(None) => {
                info!("no categorization report yet; starting the first scan");
                let _ = self.request_scan().await;
            }
            Err(error) => error!(%error, "could not read the stored categorization report"),
        }
    }

    /// Take the scan slot, if it is free.
    async fn claim(&self) -> bool {
        let mut state = self.inner.state.lock().await;
        if state.running {
            return false;
        }
        state.running = true;
        state.last_error = None;
        true
    }

    /// Run a scan and release the slot, whatever the outcome. A failure must
    /// leave `running` false, or every later request would be swallowed forever.
    async fn run_and_record(&self) {
        let outcome = self.run_scan().await;
        {
            let mut state = self.inner.state.lock().await;
            state.running = false;
            match &outcome {
                Ok(_) => state.last_completed_at = Some(Utc::now().to_rfc3339()),
                Err(error) => state.last_error = Some(error.to_string()),
            }
        }
        match outcome {
            Ok(report) => {
                let _ = self.inner.events.send(ServerEvent::CategorizationChanged {
                    categorizer: report.categorizer.clone(),
                    app_count: report.app_count,
                    categories: report.recommended_categories().count(),
                });
            }
            Err(error) => error!(%error, "categorization scan failed"),
        }
    }

    async fn run_scan(&self) -> anyhow::Result<ScanReport> {
        let items = self.inner.catalog.list().await?;
        let apps: Vec<AppProfile> = items.iter().map(profile_from_item).collect();
        info!(app_count = apps.len(), "categorization scan starting");
        let started = Instant::now();
        let report = self.inner.runner.run(apps).await?;
        self.inner.store.save(&report).await?;
        info!(
            app_count = report.app_count,
            failures = report.failures.len(),
            unclassified = report.unclassified.len(),
            duration_ms = started.elapsed().as_millis() as u64,
            "categorization scan stored"
        );
        Ok(report)
    }
}

/// Flatten a catalog item into what a decision may use. The item id is what the
/// report is keyed by, so a verdict can be joined back to the library.
fn profile_from_item(item: &CatalogItem) -> AppProfile {
    let mut profile = AppProfile::from_metadata(
        item.id.as_str(),
        item.title.as_str(),
        Some(AppKind::parse(&item.kind)),
        &item.metadata,
    );
    // The launch target is not part of the metadata blob, and it is the strongest
    // identity signal a record has: `netflix.desktop` for a desktop entry, or
    // `legendary:Fortnite` for a launcher game.
    if let Some(launch_id) = &item.launch_id {
        profile.exec = Some(launch_id.clone());
    }
    profile
}

#[cfg(test)]
mod tests {
    use super::{
        CategorizationService, CategorizerSettings, ProcessScanRunner, ScanEngine, ScanModel,
        ScanRequest, ScanRunner, profile_from_item,
    };
    use crate::{
        catalog::{CatalogItem, CatalogStore},
        database::Database,
        state::ServerEvent,
    };
    use async_trait::async_trait;
    use hearthdeck_categorizer::{AppKind, AppProfile, ScanReport};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn report() -> ScanReport {
        ScanReport {
            generated_at: "2026-09-23T10:00:00Z".to_owned(),
            categorizer: "fake".to_owned(),
            researcher: None,
            app_count: 2,
            assignments: Vec::new(),
            failures: Vec::new(),
            categories: Vec::new(),
            unclassified: Vec::new(),
        }
    }

    struct FixedRunner {
        calls: AtomicUsize,
        fail: bool,
    }

    impl FixedRunner {
        fn succeeding() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                fail: true,
            }
        }
    }

    #[async_trait]
    impl ScanRunner for FixedRunner {
        async fn run(&self, _apps: Vec<AppProfile>) -> anyhow::Result<ScanReport> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                anyhow::bail!("checkpoint exploded");
            }
            Ok(report())
        }
    }

    async fn service(runner: Arc<dyn ScanRunner>) -> (CategorizationService, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let catalog = CatalogStore::new(database.pool().clone());
        let (events, _) = tokio::sync::broadcast::channel(8);
        (
            CategorizationService::with_runner(catalog, database.pool().clone(), events, runner),
            // The pool opens connections lazily, so the database file has to stay
            // on disk for the whole test.
            directory,
        )
    }

    /// Blocks inside `run`, so the scan slot stays claimed while a test inspects
    /// it. With an instant runner the scan finishes before the second request
    /// arrives, and the test proves nothing.
    struct HangingRunner {
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl ScanRunner for HangingRunner {
        async fn run(&self, _apps: Vec<AppProfile>) -> anyhow::Result<ScanReport> {
            self.release.notified().await;
            Ok(report())
        }
    }

    #[test]
    fn categorization_is_off_unless_it_is_asked_for() {
        let dir = std::path::Path::new("/data");
        assert!(CategorizerSettings::from_lookup(dir, &|_| None).is_none());
        assert!(
            CategorizerSettings::from_lookup(dir, &|name| Some(format!("false-for-{name}")))
                .is_none()
        );
    }

    #[test]
    fn settings_come_from_the_environment_with_laya_defaults() {
        // The scratch directory is derived from the database location, so this
        // doubles as a check that a moved database moves its scans with it.
        let database = std::path::Path::new("/data/hearthdeck.db");
        let settings = CategorizerSettings::from_lookup(database, &|name| match name {
            "HEARTHDECK_CATEGORIZER_ENABLED" => Some("true".to_owned()),
            "HEARTHDECK_CATEGORIZER_MODEL" => Some("multilingual".to_owned()),
            _ => None,
        })
        .expect("enabled settings");

        assert_eq!(settings.engine, ScanEngine::Laya);
        assert_eq!(settings.model, ScanModel::Multilingual);
        assert_eq!(settings.dtype, "float16");
        assert!(!settings.research);
        assert!(settings.model_path.is_none());
        assert_eq!(
            settings.work_dir,
            std::path::PathBuf::from("/data/categorization")
        );
        assert_eq!(
            settings.binary,
            std::path::PathBuf::from("hearthdeck-categorizer")
        );
    }

    #[test]
    fn the_argv_matches_what_the_binary_parses() {
        let settings = CategorizerSettings {
            binary: "hearthdeck-categorizer".into(),
            work_dir: "/work".into(),
            engine: ScanEngine::Heuristic,
            model: ScanModel::Multilingual,
            model_path: Some("/models/laya".into()),
            dtype: "float32".to_owned(),
            research: true,
        };
        let runner = ProcessScanRunner::new(settings);
        let arguments: Vec<String> = runner
            .arguments()
            .into_iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();

        assert_eq!(
            arguments,
            vec![
                "scan",
                "--library",
                "/work/library.json",
                "--output",
                "/work/report.json",
                "--engine",
                "heuristic",
                "--model",
                "multilingual",
                "--dtype",
                "float32",
                "--model-path",
                "/models/laya",
                "--research",
            ]
        );
    }

    #[test]
    fn a_catalog_item_keeps_its_id_and_launch_target() {
        let item = CatalogItem {
            id: "desktop:netflix.desktop".to_owned(),
            source_id: "desktop-apps".to_owned(),
            title: "Netflix".to_owned(),
            kind: "application".to_owned(),
            launch_id: Some("netflix.desktop".to_owned()),
            icon: None,
            metadata: serde_json::json!({
                "comment": "Watch TV shows and movies",
                "categories": ["AudioVideo"],
            }),
        };
        let profile = profile_from_item(&item);

        assert_eq!(profile.id, "desktop:netflix.desktop");
        assert_eq!(profile.exec.as_deref(), Some("netflix.desktop"));
        assert_eq!(profile.kind, Some(AppKind::Application));
        assert_eq!(
            profile.summary.as_deref(),
            Some("Watch TV shows and movies")
        );
        // The watch heuristic reads the id and launch target together.
        assert!(profile.signal_terms().contains("netflix"));
    }

    #[tokio::test]
    async fn a_successful_scan_is_stored_and_announced() {
        let runner = Arc::new(FixedRunner::succeeding());
        let (service, _directory) = service(runner.clone()).await;
        let mut events = service.inner.events.subscribe();

        assert!(service.claim().await);
        service.run_and_record().await;

        let stored = service
            .latest_report()
            .await
            .unwrap()
            .expect("stored report");
        assert_eq!(stored.categorizer, "fake");
        assert_eq!(stored.app_count, 2);
        let status = service.status().await;
        assert!(!status.running);
        assert!(status.last_error.is_none());
        assert!(status.last_completed_at.is_some());
        assert!(matches!(
            events.try_recv(),
            Ok(ServerEvent::CategorizationChanged { app_count: 2, .. })
        ));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_failed_scan_releases_the_slot_and_stores_nothing() {
        let (service, _directory) = service(Arc::new(FixedRunner::failing())).await;

        assert!(service.claim().await);
        service.run_and_record().await;

        assert!(service.latest_report().await.unwrap().is_none());
        let status = service.status().await;
        assert!(!status.running, "a failure must not wedge the scan slot");
        assert_eq!(status.last_error.as_deref(), Some("checkpoint exploded"));
    }

    #[tokio::test]
    async fn a_second_request_coalesces_into_the_running_one() {
        let release = Arc::new(tokio::sync::Notify::new());
        let (service, _directory) = service(Arc::new(HangingRunner {
            release: release.clone(),
        }))
        .await;

        assert_eq!(service.request_scan().await, ScanRequest::Started);
        assert_eq!(service.request_scan().await, ScanRequest::AlreadyRunning);
        release.notify_waiters();
    }

    #[tokio::test]
    async fn the_startup_scan_runs_once_and_never_again() {
        let runner = Arc::new(FixedRunner::succeeding());
        let (service, _directory) = service(runner.clone()).await;

        service.scan_if_missing().await;
        // The request spawns; wait for the slot to be free before checking again.
        for _ in 0..100 {
            if !service.status().await.running {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(service.latest_report().await.unwrap().is_some());

        service.scan_if_missing().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            runner.calls.load(Ordering::SeqCst),
            1,
            "a stored report must suppress the startup scan"
        );
    }
}
