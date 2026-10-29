//! On-demand library categorization.
//!
//! A scan is not run in this process. The model is far too big to keep resident
//! beside the library and the UI: a scan wants most of a gigabyte and seconds of
//! CPU, and the daemon owns both the catalog and the API the UI is using. So the
//! daemon writes the library out as JSON, spawns the `hearthdeck-ai` binary,
//! waits for it, and reads the report back. The heavy state lives in a
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
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use anyhow::Context;
use async_trait::async_trait;
use chrono::Utc;
use hearthdeck_ai::categorization::{
    AppKind, AppProfile, HeuristicCategorizer, LibraryScanner, ScanReport, Taxonomy,
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::{Mutex, broadcast};
use tracing::{error, info, warn};

use crate::{
    catalog::{CatalogItem, CatalogStore},
    collections::{self, CollectionStore, OwnedCollection},
    state::ServerEvent,
    traits::{self, FormFactorVerdict, TraitStore},
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
    /// The sibling `hearthdeck-ai` binary. A bare name is looked up on `PATH`,
    /// which is how the packaged unit finds it in `~/.local/bin`.
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
    /// How long the child may run before the daemon kills it.
    ///
    /// A deadlock breaker, not a performance budget: the first scan on a fresh
    /// host downloads the checkpoint, so the default is deliberately generous.
    /// Its job is to guarantee the scan slot is always released — without it a
    /// child stalled on a download would leave `running` claimed forever, and
    /// because scans coalesce on `running`, every later scan would silently
    /// no-op until the daemon restarted.
    pub timeout: Duration,
}

impl CategorizerSettings {
    /// Read the daemon's environment.
    ///
    /// Always answers: whether a scan ever runs is the user's preference, held
    /// in their settings, not something a deployment has to be configured for.
    /// The environment variables below are overrides for the unusual machine
    /// (a sideloaded checkpoint, a second model), never a switch to turn the
    /// feature on — there is exactly one switch, and it is in the UI.
    ///
    /// Nothing here loads a checkpoint or touches the network; the first scan
    /// is what downloads one, and only a user can ask for that.
    pub fn load(database_path: &Path) -> Self {
        Self::from_lookup(
            database_path,
            &|name| std::env::var(name).ok(),
            default_binary(),
        )
    }

    /// The same read, with the environment injected, so it can be tested.
    fn from_lookup(
        database_path: &Path,
        get: &dyn Fn(&str) -> Option<String>,
        fallback_binary: PathBuf,
    ) -> Self {
        // The scratch files sit beside the database rather than in a directory
        // of their own: they are derived from the library the database holds,
        // and a deployment that moves one wants the other to follow.
        let data_dir = database_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        Self {
            binary: get("HEARTHDECK_AI_BIN")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or(fallback_binary),
            work_dir: get("HEARTHDECK_CATEGORIZATION_DIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| data_dir.join("categorization")),
            cache_dir: get("HEARTHDECK_AI_CACHE_DIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| data_dir.join("models").join("hub")),
            engine: get("HEARTHDECK_AI_ENGINE")
                .as_deref()
                .and_then(ScanEngine::parse)
                .unwrap_or(ScanEngine::Laya),
            model: match get("HEARTHDECK_AI_MODEL").as_deref() {
                Some("multilingual") => ScanModel::Multilingual,
                _ => ScanModel::English,
            },
            model_path: get("HEARTHDECK_AI_MODEL_PATH")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            dtype: get("HEARTHDECK_AI_DTYPE")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "float16".to_owned()),
            timeout: get("HEARTHDECK_AI_TIMEOUT_SECS")
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|seconds| *seconds > 0)
                .map(Duration::from_secs)
                .unwrap_or_else(default_scan_timeout),
        }
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

/// Ceiling on one scan, in the absence of an override. An hour is far more than a
/// scan needs once the checkpoint is local, which is the point: it only ever
/// fires on a run that has genuinely wedged.
fn default_scan_timeout() -> Duration {
    Duration::from_secs(60 * 60)
}

/// The name of the program that does the scan.
const AI_BINARY: &str = "hearthdeck-ai";

/// The AI worker that ships beside the daemon, or the bare name to look for on
/// `PATH` when there is none.
///
/// Both binaries land in the same directory however Hearthdeck was installed —
/// `~/.local/bin` from the install task, `target/debug` in a development build —
/// so the daemon finds its own worker without anybody setting an environment
/// variable for it.
fn sibling_of(daemon: &Path) -> PathBuf {
    let Some(directory) = daemon.parent() else {
        return PathBuf::from(AI_BINARY);
    };
    let candidate = directory.join(AI_BINARY);
    if candidate.is_file() {
        candidate
    } else {
        PathBuf::from(AI_BINARY)
    }
}

fn default_binary() -> PathBuf {
    std::env::current_exe()
        .ok()
        .map(|daemon| sibling_of(&daemon))
        .unwrap_or_else(|| PathBuf::from(AI_BINARY))
}

/// The collections a scan implies: one per dashboard rail the taxonomy declares,
/// for the rails this scan put something on.
///
/// The members are not written — the report is already stored, and the rails are
/// resolved from it on read. So this is a list of *definitions*, which is what
/// lets a rail disappear on its own: nothing refers to a rail that no longer
/// applies, so nothing has to delete it.
fn rail_collections(report: &ScanReport) -> Vec<OwnedCollection> {
    let taxonomy = Taxonomy::baseline();
    taxonomy
        .rails()
        .into_iter()
        .filter(|rail| !report.rail_app_ids(&taxonomy, &rail.id).is_empty())
        .map(|rail| {
            // The slug and the rule are the same string: a rail is addressed by
            // what it is, and re-creating one under a new slug would orphan the
            // old one rather than replacing it.
            let rule = collections::owned_rule(collections::OWNER_LAYA, &rail.id);
            OwnedCollection {
                slug: rule.clone(),
                name: Some(rail.name.clone()),
                rule,
            }
        })
        .collect()
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

    /// The exact command line the hearthdeck-ai binary understands. Kept in one
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
        // The checkpoint cache too, so the first scan writes into a directory that
        // already exists whatever the downloader would have done with the path.
        // Nothing else creates it, and a scan is the only thing that ever will.
        tokio::fs::create_dir_all(&self.settings.cache_dir)
            .await
            .with_context(|| {
                format!(
                    "could not create the checkpoint cache {}",
                    self.settings.cache_dir.display()
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

        let mut command = tokio::process::Command::new(&self.settings.binary);
        command
            .args(self.arguments())
            // The hub would otherwise cache into the user's shared
            // `~/.cache/huggingface`, outside anything this project manages.
            .env("HF_HUB_CACHE", &self.settings.cache_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        info!(
            binary = %self.settings.binary.display(),
            arguments = ?self.arguments(),
            "starting the categorizer"
        );
        let mut child = command.spawn().with_context(|| {
            format!(
                "could not run the categorizer binary {}",
                self.settings.binary.display()
            )
        })?;
        // The child is a separate process, and its own tracing goes to its
        // stdout. Relaying both streams here is what puts its work into the
        // daemon's log - and so into `~/hearthdeck.log` - instead of leaving it
        // somewhere only a journal query can find. Without this a failing scan
        // reports nothing but "exited with status 1".
        let stdout = child.stdout.take().expect("stdout piped above");
        let stderr = child.stderr.take().expect("stderr piped above");
        let stdout = tokio::spawn(relay(stdout, false));
        let stderr = tokio::spawn(relay(stderr, true));
        let status = match tokio::time::timeout(self.settings.timeout, child.wait()).await {
            Ok(status) => status.context("could not wait for the categorizer")?,
            Err(_) => {
                // The child hung. Kill it and reap so the pipes close and the relay
                // tasks end, then fail: `run_scan`'s caller releases the scan slot on
                // any error, which is the whole point of bounding the wait.
                let _ = child.kill().await;
                let _ = child.wait().await;
                anyhow::bail!(
                    "the categorizer did not finish within {} seconds",
                    self.settings.timeout.as_secs()
                );
            }
        };
        let (stdout_lines, stderr_lines) = (stdout.await, stderr.await);
        info!(
            exit_status = %status,
            stdout_lines = stdout_lines.as_ref().map_or(0, |lines| *lines),
            stderr_lines = stderr_lines.as_ref().map_or(0, |lines| *lines),
            "the categorizer exited"
        );
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

/// Copies one of the child's streams into the daemon's own log, line by line.
///
/// The categorizer is where the model's work actually happens, so its output is
/// the only place a scan's real progress and failures are visible. Relaying it
/// through the daemon's tracing is what gets it into `~/hearthdeck.log`, which is
/// filtered by `SyslogIdentifier` and would otherwise only catch the daemon's own
/// lines.
async fn relay<R: AsyncRead + Unpin>(reader: R, is_error: bool) -> usize {
    let mut lines = BufReader::new(reader).lines();
    let mut count = 0usize;
    while let Ok(Some(line)) = lines.next_line().await {
        if is_error {
            warn!(target: "hearthdeck_daemon::categorizer", "{line}");
        } else {
            info!(target: "hearthdeck_daemon::categorizer", "{line}");
        }
        count += 1;
    }
    count
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
    /// The feature was turned off or its rails removed while the job ran: what
    /// it decides is thrown away instead of written.
    discard: bool,
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
    /// Where a finished scan writes the rails it implies. This service is a
    /// writer of collections, not a special case inside them: it owns what it
    /// writes, and the collections it writes are rows like any other.
    collections: CollectionStore,
    events: broadcast::Sender<ServerEvent>,
    runner: Arc<dyn ScanRunner>,
    /// What scans decided about single items, laid over the catalog when served.
    traits: TraitStore,
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
                store: CategorizationStore::new(pool.clone()),
                traits: TraitStore::new(pool.clone()),
                collections: CollectionStore::new(pool),
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

    /// Ask for the dashboard rails. Returns as soon as the job is accepted;
    /// completion is announced on the event stream and is visible in
    /// [`CategorizationService::status`].
    pub async fn request_scan(&self) -> ScanRequest {
        if !self.claim(false).await {
            return ScanRequest::AlreadyRunning;
        }
        let service = self.clone();
        tokio::spawn(async move { service.run_and_record().await });
        ScanRequest::Started
    }

    /// Fetches the checkpoint and classifies nothing: what turning the feature
    /// on does. A machine that already has a model has nothing to fetch.
    pub async fn request_download(&self) -> ScanRequest {
        if model_status(&self.inner.settings) != ModelStatus::Absent {
            return ScanRequest::Started;
        }
        if !self.claim(true).await {
            return ScanRequest::AlreadyRunning;
        }
        let service = self.clone();
        tokio::spawn(async move { service.download().await });
        ScanRequest::Started
    }

    /// Loads the engine over an empty library, which downloads the checkpoint,
    /// and releases the slot. No report is stored and no rail is written.
    async fn download(&self) {
        let outcome = self.inner.runner.run(Vec::new()).await;
        self.release(outcome.as_ref().err().map(ToString::to_string), false)
            .await;
        match outcome {
            Ok(_) => info!("categorization model ready"),
            Err(error) => error!(%error, "categorization model download failed"),
        }
        self.announce(0, 0);
    }

    /// Take the scan slot, if it is free.
    async fn claim(&self, downloading: bool) -> bool {
        let mut state = self.inner.state.lock().await;
        if state.running {
            return false;
        }
        state.running = true;
        state.downloading = downloading;
        state.discard = false;
        state.last_error = None;
        true
    }

    /// Frees the slot and answers whether the job's result must be thrown away.
    /// A failure must leave `running` false, or every later request would be
    /// swallowed forever.
    async fn release(&self, error: Option<String>, completes_a_run: bool) -> bool {
        let mut state = self.inner.state.lock().await;
        state.running = false;
        match error {
            Some(error) => state.last_error = Some(error),
            None if completes_a_run => state.last_completed_at = Some(Utc::now().to_rfc3339()),
            None => {}
        }
        state.discard
    }

    /// Throws away what the running job, if any, would still write.
    pub async fn discard_running(&self) {
        let mut state = self.inner.state.lock().await;
        if state.running {
            state.discard = true;
        }
    }

    /// Tells clients a job ended, so a settings page re-reads the status.
    fn announce(&self, app_count: usize, categories: usize) {
        let _ = self.inner.events.send(ServerEvent::CategorizationChanged {
            categorizer: self.inner.settings.engine.as_str().to_owned(),
            app_count,
            categories,
        });
    }

    async fn run_and_record(&self) {
        let outcome = self.run_scan().await.map_err(|error| error.to_string());
        let discarded = self.release(outcome.as_ref().err().cloned(), true).await;
        let report = match outcome {
            Ok(report) if !discarded => report,
            Ok(_) => {
                info!("rails discarded: removed while publishing");
                return self.announce(0, 0);
            }
            Err(error) => {
                error!(%error, "categorization scan failed");
                return self.announce(0, 0);
            }
        };
        if let Err(error) = self.write_rail_collections(&report).await {
            warn!(%error, "could not write the collections this scan implies");
        }
        self.announce(report.app_count, report.recommended_categories().count());
    }

    /// Replaces this feature's rails with the ones this report implies.
    ///
    /// Replacement, not merging: a rail the scan no longer has anything for is
    /// gone on the next scan, without anybody deleting it by hand.
    async fn write_rail_collections(&self, report: &ScanReport) -> anyhow::Result<usize> {
        let collections = rail_collections(report);
        let count = collections.len();
        let slugs: Vec<&str> = collections.iter().map(|rail| rail.slug.as_str()).collect();
        self.inner
            .collections
            .replace_owned(collections::OWNER_LAYA, &collections)
            .await
            .context("could not write the collections this report implies")?;
        info!(count, ?slugs, "scan rails written");
        Ok(count)
    }

    /// Stores what a report decided about each application's form factor, and
    /// tells clients the catalog they hold is out of date.
    async fn record_form_factors(
        &self,
        report: &ScanReport,
        hashes: &HashMap<String, String>,
    ) -> anyhow::Result<()> {
        let verdicts: Vec<FormFactorVerdict> = report
            .assignments
            .iter()
            .filter_map(|assignment| {
                let probability = assignment.traits.couch?;
                Some(FormFactorVerdict {
                    item_id: assignment.app_id.clone(),
                    value: traits::form_factor_for(probability),
                    confidence: probability,
                    input_hash: hashes.get(&assignment.app_id)?.clone(),
                })
            })
            .collect();
        if verdicts.is_empty() {
            return Ok(());
        }
        self.inner
            .traits
            .write_form_factors(&traits::stamp(&report.categorizer), &verdicts)
            .await?;
        self.inner.traits.prune().await?;
        let _ = self.inner.events.send(ServerEvent::LibraryChanged {
            source_id: TRAITS_SOURCE.to_owned(),
            record_count: verdicts.len(),
        });
        Ok(())
    }

    /// Classifies the applications nobody has judged under the current model:
    /// new ones, and ones whose record has changed since.
    ///
    /// This is the couch-detection button. It never starts a download - turning
    /// the feature on does that - and it takes the same one slot a scan does, so
    /// the two cannot run at once.
    pub async fn classify_pending(&self) -> anyhow::Result<TraitRun> {
        let settings = &self.inner.settings;
        if settings.engine != ScanEngine::Laya || model_status(settings) == ModelStatus::Absent {
            return Ok(TraitRun::UpToDate);
        }
        let stamp = traits::stamp(settings.engine.as_str());
        let items = self.inner.catalog.list().await?;
        let current = self.inner.traits.current(&stamp).await?;
        let hashes = application_hashes(&items);
        let apps: Vec<AppProfile> = items
            .iter()
            .filter(|item| {
                hashes
                    .get(&item.id)
                    .is_some_and(|hash| current.get(&item.id) != Some(hash))
            })
            .map(profile_from_item)
            .collect();
        if apps.is_empty() {
            return Ok(TraitRun::UpToDate);
        }
        if !self.claim(false).await {
            return Ok(TraitRun::Busy);
        }
        let count = apps.len();
        info!(count, "classifying new and changed applications");
        let outcome = self.inner.runner.run(apps).await;
        let discarded = self
            .release(outcome.as_ref().err().map(ToString::to_string), false)
            .await;
        self.announce(count, 0);
        let report = outcome?;
        if discarded {
            info!(count, "form factors discarded: the feature was turned off");
            return Ok(TraitRun::Classified(0));
        }
        self.record_form_factors(&report, &hashes).await?;
        Ok(TraitRun::Classified(count))
    }

    /// Forgets what the model decided, which is what turning it off does.
    pub async fn clear_traits(&self) -> anyhow::Result<u64> {
        let removed = self.inner.traits.clear().await?;
        if removed > 0 {
            let _ = self.inner.events.send(ServerEvent::LibraryChanged {
                source_id: TRAITS_SOURCE.to_owned(),
                record_count: 0,
            });
        }
        Ok(removed)
    }

    /// Files the library by rules, in process: the rails need no model.
    async fn run_scan(&self) -> anyhow::Result<ScanReport> {
        let items = self.inner.catalog.list().await?;
        let apps: Vec<AppProfile> = items.iter().map(profile_from_item).collect();
        let scanner = LibraryScanner::new(Arc::new(HeuristicCategorizer::new()));
        // `scan`'s future is not provably `Send`, so it cannot live inside a spawned task.
        let report = tokio::task::spawn_blocking(move || {
            tokio::runtime::Handle::current().block_on(scanner.scan(apps))
        })
        .await??;
        self.inner.store.save(&report).await?;
        info!(
            app_count = report.app_count,
            unclassified = report.unclassified.len(),
            "dashboard rails decided"
        );
        Ok(report)
    }
}

/// The source id a change to what the model decided is announced under.
const TRAITS_SOURCE: &str = "categorization";

/// How a background pass ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraitRun {
    /// This many applications were classified.
    Classified(usize),
    /// Every application already has a verdict made under the current model,
    /// or there is no model to ask.
    UpToDate,
    /// A scan is running; try again later.
    Busy,
}

/// The input hash of every application in the catalog, by item id. Only
/// applications: a game or a film has no couch-or-desktop question to answer.
fn application_hashes(items: &[CatalogItem]) -> HashMap<String, String> {
    items
        .iter()
        .filter(|item| item.kind == crate::catalog::APPLICATION)
        .map(|item| {
            (
                item.id.clone(),
                traits::input_hash(&profile_from_item(item)),
            )
        })
        .collect()
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
        ProcessScanRunner, ScanEngine, ScanModel, ScanRequest, ScanRunner, TraitRun, model_status,
        profile_from_item, rail_collections, sibling_of,
    };
    use crate::{
        catalog::{CatalogItem, CatalogRecord, CatalogStore},
        database::Database,
        state::ServerEvent,
    };
    use async_trait::async_trait;
    use hearthdeck_ai::categorization::{AppKind, AppProfile, ScanReport};
    use hearthdeck_ai::categorization::{CategoryProposal, Section};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn report() -> ScanReport {
        ScanReport {
            generated_at: "2026-09-23T10:00:00Z".to_owned(),
            categorizer: "fake".to_owned(),
            app_count: 2,
            assignments: Vec::new(),
            failures: Vec::new(),
            categories: Vec::new(),
            unclassified: Vec::new(),
        }
    }

    fn proposal(slug: &str, app_ids: &[&str]) -> CategoryProposal {
        CategoryProposal {
            slug: slug.to_owned(),
            name: slug.to_owned(),
            section: Section::Applications,
            app_ids: app_ids.iter().map(|id| (*id).to_owned()).collect(),
            mean_confidence: 0.9,
            recommended: true,
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
            binary: "hearthdeck-ai".into(),
            work_dir: root.join("work"),
            cache_dir: root.join("models"),
            engine: ScanEngine::Heuristic,
            model: ScanModel::English,
            model_path: None,
            dtype: "float16".to_owned(),
            timeout: std::time::Duration::from_secs(60 * 60),
        }
    }

    /// The fixture's directory is bound in every pattern below rather than
    /// elided with `..`, because a `..` in a struct pattern drops the fields it
    /// does not name — at the end of that `let`, not at the end of the test.
    /// Dropping the guard deletes the database file while the pool is still
    /// opening connections lazily into it, which fails a test at random with
    /// "unable to open database file".
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
    fn settings_are_always_produced_and_the_environment_only_overrides_them() {
        // Nothing in the environment: the defaults stand. Enabling is the user's
        // setting in the UI, so a deployment that configures nothing still has a
        // working categorizer.
        let dir = std::path::Path::new("/data");
        let settings = CategorizerSettings::from_lookup(dir, &|_| None, bare_binary());
        assert_eq!(settings.engine, ScanEngine::Laya);
        assert_eq!(settings.model, ScanModel::English);
        assert_eq!(settings.binary, std::path::PathBuf::from("hearthdeck-ai"));
        assert_eq!(settings.timeout, std::time::Duration::from_secs(60 * 60));
    }

    #[test]
    fn a_scan_timeout_can_be_overridden_and_a_bad_value_is_ignored() {
        let dir = std::path::Path::new("/data");
        let seconds = |value: &str| {
            let value = value.to_owned();
            CategorizerSettings::from_lookup(
                dir,
                &move |name| (name == "HEARTHDECK_AI_TIMEOUT_SECS").then(|| value.clone()),
                bare_binary(),
            )
            .timeout
        };

        assert_eq!(seconds("120"), std::time::Duration::from_secs(120));
        assert_eq!(seconds("0"), std::time::Duration::from_secs(60 * 60));
        assert_eq!(seconds("nonsense"), std::time::Duration::from_secs(60 * 60));
    }

    #[test]
    fn settings_come_from_the_environment_with_laya_defaults() {
        // The scratch directory is derived from the database location, so this
        // doubles as a check that a moved database moves its scans with it.
        let database = std::path::Path::new("/data/hearthdeck.db");
        let settings = CategorizerSettings::from_lookup(
            database,
            &|name| match name {
                "HEARTHDECK_AI_MODEL" => Some("multilingual".to_owned()),
                _ => None,
            },
            bare_binary(),
        );

        assert_eq!(settings.engine, ScanEngine::Laya);
        assert_eq!(settings.model, ScanModel::Multilingual);
        assert_eq!(settings.dtype, "float16");
        assert!(settings.model_path.is_none());
        assert_eq!(
            settings.work_dir,
            std::path::PathBuf::from("/data/categorization")
        );
        assert_eq!(settings.binary, std::path::PathBuf::from("hearthdeck-ai"));
    }

    #[test]
    fn the_categorizer_is_looked_for_beside_the_daemon_before_the_path() {
        let directory = tempfile::tempdir().unwrap();
        let daemon = directory.path().join("hearthdeck-daemon");

        // Neither binary is installed: the bare name, left to `PATH`.
        assert_eq!(
            sibling_of(&daemon),
            std::path::PathBuf::from("hearthdeck-ai")
        );

        // Installed side by side, which is how both the package and `just
        // install-services` lay them out: found without a variable to say so.
        let worker = directory.path().join("hearthdeck-ai");
        std::fs::write(&worker, b"").unwrap();
        assert_eq!(sibling_of(&daemon), worker);
    }

    fn bare_binary() -> std::path::PathBuf {
        std::path::PathBuf::from("hearthdeck-ai")
    }

    #[test]
    fn a_scan_implies_one_rail_per_rail_the_taxonomy_declares() {
        let mut report = report();
        report.categories = vec![
            proposal("video_streaming", &["desktop:netflix.desktop"]),
            // A category on no rail contributes a tab and nothing else.
            proposal("development", &["desktop:builder.desktop"]),
        ];

        let rails = rail_collections(&report);
        assert_eq!(rails.len(), 1);
        assert_eq!(rails[0].slug, "laya:watch");
        assert_eq!(rails[0].name.as_deref(), Some("Watch"));
        // The rule is what the resolver reads, and it is the slug: a rail is
        // addressed by what it is.
        assert_eq!(rails[0].rule, rails[0].slug);
    }

    #[test]
    fn a_scan_with_nothing_on_a_rail_implies_no_rails() {
        let mut report = report();
        report.categories = vec![proposal("development", &["desktop:builder.desktop"])];

        assert!(rail_collections(&report).is_empty());
    }

    #[test]
    fn the_argv_matches_what_the_binary_parses() {
        let settings = CategorizerSettings {
            binary: "hearthdeck-ai".into(),
            work_dir: "/work".into(),
            cache_dir: "/models/hub".into(),
            engine: ScanEngine::Heuristic,
            model: ScanModel::Multilingual,
            model_path: Some("/models/laya".into()),
            dtype: "float32".to_owned(),
            timeout: std::time::Duration::from_secs(60 * 60),
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

    /// Decides every application it is given is made for the couch, and counts
    /// how often it was asked.
    struct CouchRunner {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ScanRunner for CouchRunner {
        async fn run(&self, apps: Vec<AppProfile>) -> anyhow::Result<ScanReport> {
            use hearthdeck_ai::categorization::{AppCategorization, AppTraits, DecisionProvenance};
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut report = report();
            report.categorizer = "laya".to_owned();
            report.app_count = apps.len();
            report.assignments = apps
                .iter()
                .map(|app| AppCategorization {
                    app_id: app.id.clone(),
                    title: app.title.clone(),
                    section: Section::Applications,
                    section_confidence: 0.9,
                    categories: Vec::new(),
                    needs_category: false,
                    traits: AppTraits {
                        couch: Some(0.9),
                        ..AppTraits::default()
                    },
                    provenance: DecisionProvenance::Laya,
                    rationale: None,
                })
                .collect();
            Ok(report)
        }
    }

    /// A [`CouchRunner`] that waits for the test to let it answer.
    struct GatedCouchRunner {
        gate: Arc<tokio::sync::Notify>,
        inner: CouchRunner,
    }

    #[async_trait]
    impl ScanRunner for GatedCouchRunner {
        async fn run(&self, apps: Vec<AppProfile>) -> anyhow::Result<ScanReport> {
            self.gate.notified().await;
            self.inner.run(apps).await
        }
    }

    fn application(description: &str) -> CatalogRecord {
        CatalogRecord {
            id: "kodi".into(),
            title: "Kodi".into(),
            kind: "application".into(),
            launch_id: None,
            icon: None,
            metadata: serde_json::json!({ "description": description }),
            updated_at: "2026-10-19T00:00:00Z".into(),
        }
    }

    #[tokio::test]
    async fn an_application_is_classified_once_and_asked_about_again_only_when_it_changes() {
        let runner = Arc::new(CouchRunner {
            calls: AtomicUsize::new(0),
        });
        let Fixture {
            service,
            _directory,
            ..
        } = fixture_with(runner.clone(), |settings| {
            settings.engine = ScanEngine::Laya;
            // A checkpoint that is "on disk", so the pass has a model to ask.
            settings.model_path = settings.work_dir.parent().map(ToOwned::to_owned);
        })
        .await;
        let catalog = service.inner.catalog.clone();
        catalog
            .replace_source("desktop", vec![application("A media centre")])
            .await
            .unwrap();

        assert_eq!(
            service.classify_pending().await.unwrap(),
            TraitRun::Classified(1)
        );
        let served = catalog.get("kodi").await.unwrap().unwrap();
        assert_eq!(served.metadata["form_factor"], "couch");

        assert_eq!(
            service.classify_pending().await.unwrap(),
            TraitRun::UpToDate
        );
        assert_eq!(runner.calls.load(Ordering::SeqCst), 1);

        catalog
            .replace_source("desktop", vec![application("A media centre with add-ons")])
            .await
            .unwrap();
        assert_eq!(
            service.classify_pending().await.unwrap(),
            TraitRun::Classified(1)
        );
        assert_eq!(runner.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn the_background_pass_never_downloads_a_model_nobody_asked_for() {
        let runner = Arc::new(CouchRunner {
            calls: AtomicUsize::new(0),
        });
        let Fixture {
            service,
            _directory,
            ..
        } = fixture_with(runner.clone(), |settings| {
            settings.engine = ScanEngine::Laya
        })
        .await;
        service
            .inner
            .catalog
            .replace_source("desktop", vec![application("A media centre")])
            .await
            .unwrap();

        assert_eq!(
            service.classify_pending().await.unwrap(),
            TraitRun::UpToDate
        );
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn turning_the_model_off_forgets_what_it_decided() {
        let runner = Arc::new(CouchRunner {
            calls: AtomicUsize::new(0),
        });
        let Fixture {
            service,
            _directory,
            ..
        } = fixture_with(runner, |settings| {
            settings.engine = ScanEngine::Laya;
            settings.model_path = settings.work_dir.parent().map(ToOwned::to_owned);
        })
        .await;
        let catalog = service.inner.catalog.clone();
        catalog
            .replace_source("desktop", vec![application("A media centre")])
            .await
            .unwrap();
        service.classify_pending().await.unwrap();

        assert_eq!(service.clear_traits().await.unwrap(), 1);

        let served = catalog.get("kodi").await.unwrap().unwrap();
        assert!(served.metadata.get("form_factor").is_none());
    }

    #[tokio::test]
    async fn publishing_rails_files_streaming_apps_by_rules_without_the_model() {
        let runner = Arc::new(FixedRunner::succeeding());
        let Fixture {
            service,
            _directory,
            ..
        } = fixture(runner.clone()).await;
        let mut netflix = application("Watch films");
        netflix.id = "netflix".into();
        netflix.title = "Netflix".into();
        service
            .inner
            .catalog
            .replace_source("desktop", vec![netflix])
            .await
            .unwrap();
        let mut events = service.inner.events.subscribe();

        assert!(service.claim(false).await);
        service.run_and_record().await;

        assert_eq!(runner.calls.load(Ordering::SeqCst), 0, "no model is asked");
        let stored = service
            .latest_report()
            .await
            .unwrap()
            .expect("stored report");
        assert_eq!(stored.categorizer, "heuristic");
        assert_eq!(stored.app_count, 1);
        let rows = service.inner.collections.list().await.unwrap();
        assert!(rows.iter().any(|row| row.slug == "laya:watch"));
        let status = service.status().await;
        assert!(!status.running);
        assert!(status.last_completed_at.is_some());
        assert!(matches!(
            events.try_recv(),
            Ok(ServerEvent::CategorizationChanged { app_count: 1, .. })
        ));
    }

    #[tokio::test]
    async fn rails_removed_while_publishing_are_not_written_back() {
        let Fixture {
            service,
            _directory,
            ..
        } = fixture(Arc::new(FixedRunner::succeeding())).await;
        let mut netflix = application("Watch films");
        netflix.id = "netflix".into();
        service
            .inner
            .catalog
            .replace_source("desktop", vec![netflix])
            .await
            .unwrap();

        assert!(service.claim(false).await);
        service.discard_running().await;
        service.run_and_record().await;

        let rows = service.inner.collections.list().await.unwrap();
        assert!(
            rows.iter()
                .all(|row| row.owner != crate::collections::OWNER_LAYA)
        );
    }

    #[tokio::test]
    async fn couch_verdicts_are_dropped_when_the_feature_is_turned_off_mid_run() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let runner = Arc::new(GatedCouchRunner {
            gate: gate.clone(),
            inner: CouchRunner {
                calls: AtomicUsize::new(0),
            },
        });
        let Fixture {
            service,
            _directory,
            ..
        } = fixture_with(runner, |settings| {
            settings.engine = ScanEngine::Laya;
            settings.model_path = settings.work_dir.parent().map(ToOwned::to_owned);
        })
        .await;
        let catalog = service.inner.catalog.clone();
        catalog
            .replace_source("desktop", vec![application("A media centre")])
            .await
            .unwrap();

        let job = tokio::spawn({
            let service = service.clone();
            async move { service.classify_pending().await }
        });
        while !service.status().await.running {
            tokio::task::yield_now().await;
        }
        service.discard_running().await;
        gate.notify_one();
        job.await.unwrap().unwrap();

        let served = catalog.get("kodi").await.unwrap().unwrap();
        assert!(served.metadata.get("form_factor").is_none());
    }

    #[tokio::test]
    async fn a_download_fetches_the_model_and_neither_stores_nor_publishes() {
        let runner = Arc::new(FixedRunner::succeeding());
        let Fixture {
            service,
            _directory,
            ..
        } = fixture(runner.clone()).await;

        assert!(service.claim(true).await);
        service.download().await;

        assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
        assert!(service.latest_report().await.unwrap().is_none());
        assert!(
            service
                .inner
                .collections
                .list()
                .await
                .unwrap()
                .iter()
                .all(|row| row.owner != crate::collections::OWNER_LAYA)
        );
        let status = service.status().await;
        assert!(!status.running);
        assert!(
            status.last_completed_at.is_none(),
            "nothing was categorized"
        );
    }

    #[tokio::test]
    async fn turning_it_on_with_a_model_present_runs_nothing() {
        let runner = Arc::new(FixedRunner::succeeding());
        let Fixture {
            service,
            _directory,
            ..
        } = fixture_with(runner.clone(), |settings| {
            settings.model_path = settings.work_dir.parent().map(ToOwned::to_owned);
        })
        .await;

        assert_eq!(service.request_download().await, ScanRequest::Started);
        assert!(!service.status().await.running);
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_failed_download_releases_the_slot() {
        let Fixture {
            service,
            _directory,
            ..
        } = fixture(Arc::new(FixedRunner::failing())).await;

        assert!(service.claim(true).await);
        service.download().await;

        let status = service.status().await;
        assert!(!status.running, "a failure must not wedge the scan slot");
        assert_eq!(status.last_error.as_deref(), Some("checkpoint exploded"));
    }

    #[tokio::test]
    async fn a_second_request_coalesces_into_the_running_one() {
        let release = Arc::new(tokio::sync::Notify::new());
        let Fixture {
            service,
            _directory,
            ..
        } = fixture(Arc::new(HangingRunner {
            release: release.clone(),
        }))
        .await;

        assert_eq!(service.request_download().await, ScanRequest::Started);
        assert_eq!(service.request_scan().await, ScanRequest::AlreadyRunning);
        release.notify_waiters();
    }

    #[tokio::test]
    async fn a_run_with_no_checkpoint_says_it_is_downloading_one() {
        let release = Arc::new(tokio::sync::Notify::new());
        let Fixture {
            service,
            _directory,
            ..
        } = fixture(Arc::new(HangingRunner {
            release: release.clone(),
        }))
        .await;

        assert_eq!(service.request_download().await, ScanRequest::Started);
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
            service,
            settings,
            _directory,
        } = fixture(Arc::new(HangingRunner {
            release: release.clone(),
        }))
        .await;
        assert_eq!(service.request_download().await, ScanRequest::Started);

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
            service,
            settings,
            _directory,
        } = fixture(Arc::new(HangingRunner {
            release: release.clone(),
        }))
        .await;
        assert_eq!(service.request_download().await, ScanRequest::Started);

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

    #[test]
    fn an_external_checkpoint_is_reported_as_external() {
        let external = tempfile::tempdir().unwrap();
        let path = external.path().join("sideloaded");
        std::fs::create_dir_all(&path).unwrap();

        let mut external_settings = settings(external.path());
        external_settings.model_path = Some(path.clone());
        assert_eq!(
            model_status(&external_settings),
            ModelStatus::External { path }
        );
    }
}
