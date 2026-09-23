//! On-demand library categorization.
//!
//! A scan is not run in this process. The model is far too big to keep resident
//! beside the library and the UI: a scan wants most of a gigabyte and seconds of
//! CPU, and the daemon owns both the catalog and the API the UI is using. So the
//! daemon writes the library out as JSON, spawns the `hearthdeck-categorizer`
//! binary, waits for it, and reads the report back. The heavy state lives in a
//! process that exits.
//!
//! Two things this module deliberately does *not* do:
//!
//! * It never runs on its own. There is no scan at boot. A scan happens because
//!   the user turned smart categorization on, and the first one fetches the
//!   checkpoint as part of loading the engine — which is why the status has a
//!   "downloading" phase that the daemon, not the child, works out.
//! * It never decides *whether* categorization is wanted. The user's answer lives
//!   in the settings table with the rest of their preferences, and the API layer
//!   reads it; this module just runs the job it is asked to run.

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
use serde::{Deserialize, Serialize};
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
    /// Scratch directory for the handoff files: the library, the report, and the
    /// progress the child reports while it works.
    pub work_dir: PathBuf,
    /// Where a downloaded checkpoint is kept.
    ///
    /// Handed to the child as `HF_HUB_CACHE`, so the weights land somewhere this
    /// project owns. The default the hub would pick is the shared
    /// `~/.cache/huggingface`, which sits on whatever partition the desktop uses
    /// and is the first thing a cleanup tool or an image build throws away.
    pub cache_dir: PathBuf,
    pub engine: ScanEngine,
    pub model: ScanModel,
    /// A local checkpoint directory, so a scan never downloads one.
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
            cache_dir: get("HEARTHDECK_CATEGORIZER_CACHE_DIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| data_dir.join("models").join("hub")),
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

    /// Where the child writes what it is doing. One file, because the daemon and
    /// the child are separate processes and this is the only channel that says
    /// "still loading" as distinct from "deciding".
    pub fn phase_file(&self) -> PathBuf {
        self.work_dir.join("phase.json")
    }

    pub fn library_file(&self) -> PathBuf {
        self.work_dir.join("library.json")
    }

    pub fn report_file(&self) -> PathBuf {
        self.work_dir.join("report.json")
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
}

impl ProcessScanRunner {
    pub fn new(settings: CategorizerSettings) -> Self {
        Self { settings }
    }

    /// The exact command line the categorizer binary understands. Kept in one
    /// place so the two sides cannot drift apart silently.
    fn arguments(&self) -> Vec<OsString> {
        let mut arguments: Vec<OsString> = vec![
            "scan".into(),
            "--library".into(),
            self.settings.library_file().into_os_string(),
            "--output".into(),
            self.settings.report_file().into_os_string(),
            "--phase-file".into(),
            self.settings.phase_file().into_os_string(),
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
        let library_path = self.settings.library_file();
        tokio::fs::write(&library_path, library)
            .await
            .with_context(|| {
                format!(
                    "could not write the scan library {}",
                    library_path.display()
                )
            })?;
        // A report or a phase file left behind by an earlier run would make this
        // one look further along than it is — a failed scan could look completed,
        // and a status poll before the child has written anything could show the
        // previous run's progress. Both go before the child starts.
        for stale in [self.settings.report_file(), self.settings.phase_file()] {
            match tokio::fs::remove_file(&stale).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("could not clear the previous {}", stale.display())
                    });
                }
            }
        }

        let status = tokio::process::Command::new(&self.settings.binary)
            .args(self.arguments())
            // The hub would otherwise cache into the user's shared
            // `~/.cache/huggingface`, outside anything this project manages.
            .env("HF_HUB_CACHE", &self.settings.cache_dir)
            .status()
            .await
            .with_context(|| {
                format!(
                    "could not run the categorizer binary {}",
                    self.settings.binary.display()
                )
            })?;
        anyhow::ensure!(status.success(), "the categorizer exited with {status}");

        let report_path = self.settings.report_file();
        let report = tokio::fs::read(&report_path).await.with_context(|| {
            format!(
                "the categorizer wrote no report to {}",
                report_path.display()
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

/// Where a running scan has got to.
///
/// `Downloading` is the daemon's own refinement, not something the child reports:
/// fetching the checkpoint happens inside the engine load, and the daemon knows
/// whether a checkpoint was already on disk when it started the job. The child
/// only ever says "loading" or "scanning".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategorizationPhase {
    Idle,
    Downloading,
    Loading,
    Scanning,
}

/// Whether a checkpoint is available, and what it costs on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelStatus {
    /// Nothing to run yet. The first scan will fetch one.
    Absent,
    /// A checkpoint in this project's own cache.
    Downloaded { bytes: u64, path: PathBuf },
    /// A checkpoint the deployment pointed at directly, which is how an offline
    /// install sideloads one. Never deleted by Hearthdeck.
    External { path: PathBuf },
}

/// What the child last said it was doing.
#[derive(Debug, Deserialize)]
struct PhaseFile {
    phase: String,
    #[serde(default)]
    completed: Option<usize>,
    #[serde(default)]
    total: Option<usize>,
}

/// Read the child's phase file, tolerating every way it can be missing or
/// half-written: it is written by another process, and a status poll must never
/// fail because of it.
fn read_phase_file(path: &Path) -> Option<PhaseFile> {
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// How much checkpoint is on disk, and where.
///
/// `hf-hub`'s cache layout is its own business — a blob under `blobs/<etag>` with
/// a symlink to it under `snapshots/<revision>/` — so this looks for the file
/// name every Laya checkpoint has instead of assuming a path. The symlink is not
/// counted; the blob it points at is, because that is where the bytes are.
fn model_status(settings: &CategorizerSettings) -> ModelStatus {
    if let Some(path) = &settings.model_path {
        return if path.is_dir() {
            ModelStatus::External { path: path.clone() }
        } else {
            ModelStatus::Absent
        };
    }

    let mut bytes = 0;
    let mut checkpoint = None;
    scan_cache(&settings.cache_dir, 0, &mut bytes, &mut checkpoint);
    match checkpoint {
        Some(path) => ModelStatus::Downloaded { bytes, path },
        None => ModelStatus::Absent,
    }
}

const CHECKPOINT_WEIGHTS: &str = "model.safetensors";
/// The hub nests a few levels deep and nothing legitimate goes deeper, so this
/// only bounds a pathological tree.
const CACHE_SCAN_DEPTH: usize = 6;

fn scan_cache(dir: &Path, depth: usize, bytes: &mut u64, checkpoint: &mut Option<PathBuf>) {
    if depth > CACHE_SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            scan_cache(&path, depth + 1, bytes, checkpoint);
            continue;
        }
        if path
            .file_name()
            .is_some_and(|name| name == CHECKPOINT_WEIGHTS)
        {
            // The snapshot directory is what a checkpoint *is*, and what a
            // sideload would point `model_path` at.
            checkpoint.get_or_insert_with(|| {
                path.parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| path.clone())
            });
        }
        if file_type.is_file() {
            *bytes += entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        }
    }
}

/// Whether a scan is in flight, and how the last one ended.
#[derive(Default)]
struct ScanState {
    running: bool,
    /// The job began with no checkpoint on disk, so the load phase is a
    /// download. Fixed when the scan starts, because it is what the phase means
    /// for the whole of that phase.
    downloading: bool,
    last_error: Option<String>,
    last_completed_at: Option<String>,
}

/// What a settings page needs: is a scan running, how far along, is the model
/// installed, and how did the last one end.
#[derive(Clone, Debug, Serialize)]
pub struct CategorizationStatus {
    pub running: bool,
    pub phase: CategorizationPhase,
    /// Applications decided so far, while `phase` is `Scanning`.
    pub completed: usize,
    pub total: usize,
    pub model: ModelStatus,
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
    /// Kept here as well as in the runner: the status reports on where the
    /// checkpoint lives, which is true whether or not a scan is running.
    settings: CategorizerSettings,
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
            settings.clone(),
            catalog,
            pool,
            events,
            Arc::new(ProcessScanRunner::new(settings)),
        )
    }

    /// Build the service over an arbitrary runner.
    pub fn with_runner(
        settings: CategorizerSettings,
        catalog: CatalogStore,
        pool: SqlitePool,
        events: broadcast::Sender<ServerEvent>,
        runner: Arc<dyn ScanRunner>,
    ) -> Self {
        Self {
            inner: Arc::new(CategorizationInner {
                settings,
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
        let model = model_status(&self.inner.settings);
        let (phase, completed, total) = if state.running {
            phase_of(&self.inner.settings, state.downloading)
        } else {
            (CategorizationPhase::Idle, 0, 0)
        };
        CategorizationStatus {
            running: state.running,
            phase,
            completed,
            total,
            model,
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
    ///
    /// The first scan of an empty cache fetches the checkpoint as part of loading
    /// the engine, so this is also what turns a machine with no model into one
    /// with a model. Nothing else in the daemon does.
    pub async fn request_scan(&self) -> ScanRequest {
        // Answered before the slot is claimed, because it is what the phase means
        // for the whole of that phase: asked mid-run it would flip to "loading"
        // the moment the download finished.
        let downloading = model_status(&self.inner.settings) == ModelStatus::Absent;
        if !self.claim(downloading).await {
            return ScanRequest::AlreadyRunning;
        }
        let service = self.clone();
        tokio::spawn(async move { service.run_and_record().await });
        ScanRequest::Started
    }

    /// Removes a downloaded checkpoint, freeing what it costs on disk.
    ///
    /// Only this project's own cache is ever removed. A checkpoint the deployment
    /// pointed at belongs to the deployment, so this refuses instead of deleting
    /// a directory it does not own.
    pub async fn delete_model(&self) -> anyhow::Result<()> {
        let settings = &self.inner.settings;
        if settings.model_path.is_some() {
            anyhow::bail!("the checkpoint is a directory this deployment pointed at");
        }
        if self.inner.state.lock().await.running {
            anyhow::bail!("a scan is running and is reading the checkpoint");
        }
        match tokio::fs::remove_dir_all(&settings.cache_dir).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("could not remove the checkpoint cache"),
        }
        info!(
            cache_dir = %settings.cache_dir.display(),
            "categorization checkpoint removed"
        );
        Ok(())
    }

    /// Take the scan slot, if it is free.
    async fn claim(&self, downloading: bool) -> bool {
        let mut state = self.inner.state.lock().await;
        if state.running {
            return false;
        }
        state.running = true;
        state.downloading = downloading;
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

/// Which stretch of a scan is running, from what the child has written and what
/// the daemon knew when it started the job.
fn phase_of(
    settings: &CategorizerSettings,
    downloading: bool,
) -> (CategorizationPhase, usize, usize) {
    match read_phase_file(&settings.phase_file()) {
        Some(phase) if phase.phase == "scanning" => (
            CategorizationPhase::Scanning,
            phase.completed.unwrap_or(0),
            phase.total.unwrap_or(0),
        ),
        // A run that started without a checkpoint is fetching one while it loads.
        _ if downloading => (CategorizationPhase::Downloading, 0, 0),
        _ => (CategorizationPhase::Loading, 0, 0),
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
        CategorizationPhase, CategorizationService, CategorizerSettings, ModelStatus,
        ProcessScanRunner, ScanEngine, ScanModel, ScanRequest, ScanRunner, model_status,
        profile_from_item,
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

    /// Settings that point every path at a scratch directory, so a test can
    /// exercise the phases and the checkpoint bookkeeping without going anywhere
    /// near the real model cache.
    fn settings(root: &std::path::Path) -> CategorizerSettings {
        CategorizerSettings {
            binary: "hearthdeck-categorizer".into(),
            work_dir: root.join("work"),
            cache_dir: root.join("models"),
            engine: ScanEngine::Heuristic,
            model: ScanModel::English,
            model_path: None,
            dtype: "float16".to_owned(),
            research: false,
        }
    }

    struct Fixture {
        service: CategorizationService,
        settings: CategorizerSettings,
        // Held for its `Drop`: the pool opens connections lazily, so the database
        // file has to stay on disk for the whole test.
        _directory: tempfile::TempDir,
    }

    async fn fixture(runner: Arc<dyn ScanRunner>) -> Fixture {
        fixture_with(runner, |_| {}).await
    }

    /// The same, for a test that needs one setting tweaked.
    async fn fixture_with(
        runner: Arc<dyn ScanRunner>,
        adjust: impl FnOnce(&mut CategorizerSettings),
    ) -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let catalog = CatalogStore::new(database.pool().clone());
        let (events, _) = tokio::sync::broadcast::channel(8);
        let mut settings = settings(directory.path());
        adjust(&mut settings);
        let service = CategorizationService::with_runner(
            settings.clone(),
            catalog,
            database.pool().clone(),
            events,
            runner,
        );
        Fixture {
            service,
            settings,
            _directory: directory,
        }
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
            cache_dir: "/models/hub".into(),
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
                "--phase-file",
                "/work/phase.json",
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
        let Fixture { service, .. } = fixture(runner.clone()).await;
        let mut events = service.inner.events.subscribe();

        assert!(service.claim(false).await);
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
        assert_eq!(status.phase, CategorizationPhase::Idle);
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
        let Fixture { service, .. } = fixture(Arc::new(FixedRunner::failing())).await;

        assert!(service.claim(false).await);
        service.run_and_record().await;

        assert!(service.latest_report().await.unwrap().is_none());
        let status = service.status().await;
        assert!(!status.running, "a failure must not wedge the scan slot");
        assert_eq!(status.last_error.as_deref(), Some("checkpoint exploded"));
    }

    #[tokio::test]
    async fn a_second_request_coalesces_into_the_running_one() {
        let release = Arc::new(tokio::sync::Notify::new());
        let Fixture { service, .. } = fixture(Arc::new(HangingRunner {
            release: release.clone(),
        }))
        .await;

        assert_eq!(service.request_scan().await, ScanRequest::Started);
        assert_eq!(service.request_scan().await, ScanRequest::AlreadyRunning);
        release.notify_waiters();
    }

    #[tokio::test]
    async fn a_run_with_no_checkpoint_says_it_is_downloading_one() {
        let release = Arc::new(tokio::sync::Notify::new());
        let Fixture { service, .. } = fixture(Arc::new(HangingRunner {
            release: release.clone(),
        }))
        .await;

        assert_eq!(service.request_scan().await, ScanRequest::Started);
        let status = service.status().await;

        assert_eq!(
            status.model,
            ModelStatus::Absent,
            "the scratch cache starts empty"
        );
        assert_eq!(
            status.phase,
            CategorizationPhase::Downloading,
            "the load phase is a download when there is nothing to load"
        );
        release.notify_waiters();
    }

    #[tokio::test]
    async fn the_childs_phase_file_drives_the_scanning_progress() {
        let release = Arc::new(tokio::sync::Notify::new());
        let Fixture {
            service, settings, ..
        } = fixture(Arc::new(HangingRunner {
            release: release.clone(),
        }))
        .await;
        assert_eq!(service.request_scan().await, ScanRequest::Started);

        std::fs::create_dir_all(&settings.work_dir).unwrap();
        std::fs::write(
            settings.phase_file(),
            br#"{"phase":"scanning","completed":3,"total":10}"#,
        )
        .unwrap();

        let status = service.status().await;
        assert_eq!(status.phase, CategorizationPhase::Scanning);
        assert_eq!(status.completed, 3);
        assert_eq!(status.total, 10);
        release.notify_waiters();
    }

    #[tokio::test]
    async fn a_half_written_phase_file_is_not_an_error() {
        let release = Arc::new(tokio::sync::Notify::new());
        let Fixture {
            service, settings, ..
        } = fixture(Arc::new(HangingRunner {
            release: release.clone(),
        }))
        .await;
        assert_eq!(service.request_scan().await, ScanRequest::Started);

        std::fs::create_dir_all(&settings.work_dir).unwrap();
        // A status poll can land between the child's write and its rename, so a
        // truncated object is a thing that happens.
        let half = br#"{"phase":"scan"#;
        std::fs::write(settings.phase_file(), half).unwrap();

        let status = service.status().await;
        assert_eq!(status.phase, CategorizationPhase::Downloading);
        release.notify_waiters();
    }

    #[cfg(unix)]
    #[test]
    fn a_checkpoint_is_measured_once_and_reported_by_its_snapshot() {
        let directory = tempfile::tempdir().unwrap();

        let settings = settings(directory.path());
        let blob = settings
            .cache_dir
            .join("models--convaiinnovations--laya/blobs/abc123");
        let snapshot = settings
            .cache_dir
            .join("models--convaiinnovations--laya/snapshots/5e7b2b1b/model.safetensors");
        std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
        std::fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
        std::fs::write(&blob, vec![0u8; 4096]).unwrap();
        // The hub stores the snapshot as a link to the blob, so counting both
        // would report twice what is actually on disk.
        std::os::unix::fs::symlink(&blob, &snapshot).unwrap();

        assert_eq!(
            model_status(&settings),
            ModelStatus::Downloaded {
                bytes: 4096,
                path: snapshot.parent().unwrap().to_path_buf(),
            }
        );
    }

    #[tokio::test]
    async fn an_external_checkpoint_is_reported_and_never_deleted() {
        let external = tempfile::tempdir().unwrap();
        let path = external.path().join("sideloaded");
        std::fs::create_dir_all(&path).unwrap();

        let mut external_settings = settings(external.path());
        external_settings.model_path = Some(path.clone());
        assert_eq!(
            model_status(&external_settings),
            ModelStatus::External { path: path.clone() }
        );

        let sideloaded = path.clone();
        let runner = Arc::new(FixedRunner::succeeding());
        let Fixture { service, .. } = fixture_with(runner, |settings| {
            settings.model_path = Some(sideloaded);
        })
        .await;
        let error = service.delete_model().await.unwrap_err();

        let message = error.to_string();
        assert!(
            message.contains("pointed at"),
            "unexpected error: {message}"
        );
        assert!(path.is_dir(), "the deployment's directory must survive");
    }

    #[tokio::test]
    async fn deleting_a_checkpoint_frees_it_and_is_safe_to_repeat() {
        let Fixture {
            service, settings, ..
        } = fixture(Arc::new(FixedRunner::succeeding())).await;
        let checkpoint = settings.cache_dir.join("models--convaiinnovations--laya");
        std::fs::create_dir_all(&checkpoint).unwrap();
        std::fs::write(checkpoint.join("model.safetensors"), b"weights").unwrap();

        assert!(service.delete_model().await.is_ok());
        assert!(!settings.cache_dir.exists());
        assert!(service.delete_model().await.is_ok());
        assert_eq!(service.status().await.model, ModelStatus::Absent);
    }

    #[tokio::test]
    async fn a_running_scan_cannot_have_its_checkpoint_deleted() {
        let release = Arc::new(tokio::sync::Notify::new());
        let Fixture { service, .. } = fixture(Arc::new(HangingRunner {
            release: release.clone(),
        }))
        .await;
        assert_eq!(service.request_scan().await, ScanRequest::Started);

        let error = service.delete_model().await.unwrap_err();

        assert!(error.to_string().contains("scan is running"));
        release.notify_waiters();
    }
}
