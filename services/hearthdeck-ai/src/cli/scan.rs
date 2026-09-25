//! The `scan` command: one-shot categorization of a library.
//!
//! The daemon spawns this as a child process to categorize a library: it reads a
//! JSON array of [`AppProfile`]s from `--library`, runs one scan, and writes the
//! [`ScanReport`] as pretty JSON to `--output`. There is no interactive
//! behaviour, and the report never goes to stdout — the observability helper
//! does, but stdout is the journal's to read.
//!
//! A second, optional channel exists for the parent process: `--phase-file`
//! names a JSON file this process overwrites as it works, because the daemon can
//! poll a file but cannot see inside its child. It is a data channel exactly like
//! `--output`, not a log, and it is best effort — a scan succeeds with or without
//! it.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "laya")]
use hearthdeck_ai::ai::{LayaConfig, LayaModel};
#[cfg(feature = "laya")]
use hearthdeck_ai::categorization::LayaCategorizer;
use hearthdeck_ai::categorization::{
    AppProfile, Categorizer, HeuristicCategorizer, LibraryScanner, NoopResearcher, ScanOptions,
    ScanPhase, ScanProgress, ScanReport,
};
use serde::Serialize;
use tracing::{debug, info, warn};

/// Checkpoint precision used when `--dtype` is not given.
const DEFAULT_DTYPE: &str = "float16";

pub(super) async fn run(args: &[OsString]) -> Result<(), String> {
    let request = Request::parse(args)?;
    execute(request).await
}

/// Which decision engine to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Engine {
    Heuristic,
    Laya,
}

impl Engine {
    fn as_str(self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::Laya => "laya",
        }
    }
}

/// Which Laya checkpoint to load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelChoice {
    English,
    Multilingual,
}

impl ModelChoice {
    fn as_str(self) -> &'static str {
        match self {
            Self::English => "english",
            Self::Multilingual => "multilingual",
        }
    }
}

/// A parsed, validated command line.
struct Request {
    library: PathBuf,
    output: PathBuf,
    engine: Engine,
    model: ModelChoice,
    model_path: Option<PathBuf>,
    dtype: String,
    research: bool,
    /// Where to publish scan progress for the parent process. Absent when a
    /// human runs the CLI by hand, in which case nothing is written.
    phase_file: Option<PathBuf>,
}

impl Request {
    /// Parse `argv` (program and command included), or return a message
    /// explaining what was wrong with it.
    fn parse(args: &[OsString]) -> Result<Self, String> {
        // Skip the program name and the `scan` command the dispatcher matched.
        let args: Vec<OsString> = args.iter().skip(2).cloned().collect();
        let mut index = 0;
        let mut library = None;
        let mut output = None;
        let mut engine = Engine::Laya;
        let mut model = ModelChoice::English;
        let mut model_path = None;
        let mut dtype = DEFAULT_DTYPE.to_owned();
        let mut research = false;
        let mut phase_file = None;

        while index < args.len() {
            let flag = args[index].to_str().ok_or_else(|| {
                format!(
                    "argument is not valid UTF-8: {}",
                    args[index].to_string_lossy()
                )
            })?;
            index += 1;
            match flag {
                "--library" => library = Some(PathBuf::from(value(&args, &mut index, flag)?)),
                "--output" => output = Some(PathBuf::from(value(&args, &mut index, flag)?)),
                "--engine" => engine = parse_engine(&value_text(&args, &mut index, flag)?)?,
                "--model" => model = parse_model(&value_text(&args, &mut index, flag)?)?,
                "--model-path" => {
                    model_path = Some(PathBuf::from(value(&args, &mut index, flag)?));
                }
                "--dtype" => dtype = value_text(&args, &mut index, flag)?,
                "--research" => research = true,
                "--phase-file" => {
                    phase_file = Some(PathBuf::from(value(&args, &mut index, flag)?));
                }
                other => return Err(format!("unknown argument `{other}`")),
            }
        }

        Ok(Self {
            library: library.ok_or_else(|| "missing required `--library <PATH>`".to_owned())?,
            output: output.ok_or_else(|| "missing required `--output <PATH>`".to_owned())?,
            engine,
            model,
            model_path,
            dtype,
            research,
            phase_file,
        })
    }
}

/// Take the value that follows a flag.
fn value<'a>(args: &'a [OsString], index: &mut usize, flag: &str) -> Result<&'a OsString, String> {
    let value = args
        .get(*index)
        .ok_or_else(|| format!("`{flag}` requires a value"))?;
    *index += 1;
    Ok(value)
}

/// Take the value that follows a flag, requiring it to be UTF-8 (a path may not
/// be, which is why [`value`] keeps it as an `OsString`).
fn value_text(args: &[OsString], index: &mut usize, flag: &str) -> Result<String, String> {
    value(args, index, flag)?
        .to_str()
        .map(ToString::to_string)
        .ok_or_else(|| format!("`{flag}` must be valid UTF-8"))
}

fn parse_engine(value: &str) -> Result<Engine, String> {
    match value {
        "heuristic" => Ok(Engine::Heuristic),
        "laya" => Ok(Engine::Laya),
        other => Err(format!(
            "unknown engine `{other}` (expected `heuristic` or `laya`)"
        )),
    }
}

fn parse_model(value: &str) -> Result<ModelChoice, String> {
    match value {
        "english" => Ok(ModelChoice::English),
        "multilingual" => Ok(ModelChoice::Multilingual),
        other => Err(format!(
            "unknown model `{other}` (expected `english` or `multilingual`)"
        )),
    }
}

async fn execute(request: Request) -> Result<(), String> {
    info!(
        library = %request.library.display(),
        output = %request.output.display(),
        engine = request.engine.as_str(),
        model = request.model.as_str(),
        dtype = %request.dtype,
        model_path = ?request.model_path,
        research = request.research,
        phase_file = ?request.phase_file,
        "categorizer scan requested"
    );

    // The phase file is the daemon's only window into this process, so it is
    // announced before the library is read and long before the engine is built.
    let phase = request
        .phase_file
        .clone()
        .map(|path| Arc::new(PhaseWriter::new(path)));
    if let Some(phase) = &phase {
        phase.loading();
    }

    let apps = load_library(&request.library)?;
    let categorizer = build_categorizer(&request)?;
    let options = ScanOptions {
        research: request.research,
        ..ScanOptions::default()
    };

    let mut scanner = LibraryScanner::new(categorizer).with_options(options);
    if request.research {
        scanner = scanner.with_researcher(Arc::new(NoopResearcher));
    }
    if let Some(phase) = phase {
        scanner = scanner.with_progress(Arc::new(move |progress: ScanProgress| {
            // `Researching` and `Aggregating` are not progress the daemon has a
            // place for; only per-application decisions are reported.
            if progress.phase == ScanPhase::Deciding {
                phase.scanning(progress.completed, progress.total);
            }
        }));
    }

    let report = scanner
        .scan(apps)
        .await
        .map_err(|error| format!("scan failed: {error}"))?;
    write_report(&request.output, &report)?;

    info!(
        assignments = report.assignments.len(),
        unclassified = report.unclassified.len(),
        recommended = report.recommended_categories().count(),
        "categorizer scan written"
    );
    Ok(())
}

/// One entry of the phase file. Serialized field order is the order the daemon
/// reads, and `completed`/`total` are omitted entirely unless there is progress
/// to report — the file's shape is a contract, so no extra keys appear.
#[derive(Serialize)]
struct PhaseUpdate<'a> {
    phase: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<usize>,
}

/// Publishes scan progress for the parent process as a JSON file.
///
/// Every write here is best effort by design: the daemon's progress line is not
/// worth a failed categorization run, so a missing directory, a read-only mount
/// or a permission problem is logged and then ignored.
struct PhaseWriter {
    path: PathBuf,
    /// Whether a failure has already been logged. A scan writes this file once
    /// per application, so only the first problem is worth a warning.
    reported: AtomicBool,
}

impl PhaseWriter {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            reported: AtomicBool::new(false),
        }
    }

    /// The engine is being built. From the daemon's point of view this whole
    /// stretch is "getting the model ready": a checkpoint is downloaded here when
    /// it is not cached, and the daemon tells a download from a load itself by
    /// watching the model directory.
    fn loading(&self) {
        self.write(&PhaseUpdate {
            phase: "loading",
            completed: None,
            total: None,
        });
    }

    /// `completed` applications out of `total` have been decided.
    fn scanning(&self, completed: usize, total: usize) {
        self.write(&PhaseUpdate {
            phase: "scanning",
            completed: Some(completed),
            total: Some(total),
        });
    }

    fn write(&self, update: &PhaseUpdate<'_>) {
        let json = match serde_json::to_string(update) {
            Ok(json) => json,
            Err(error) => return self.report(&error),
        };

        // Written beside the target and renamed over it: the daemon polls every
        // few seconds, and a rename inside one directory is atomic, so it can
        // never read half an object.
        let temporary = self.temporary_path();
        if let Err(error) =
            fs::write(&temporary, json).and_then(|()| fs::rename(&temporary, &self.path))
        {
            self.report(&error);
        }
    }

    fn temporary_path(&self) -> PathBuf {
        let mut temporary = self.path.clone().into_os_string();
        temporary.push(".tmp");
        PathBuf::from(temporary)
    }

    /// Note a phase-file problem and carry on. This method is the only place
    /// that decides such a problem is survivable, so the scan never sees it.
    fn report(&self, error: &dyn std::fmt::Display) {
        if self.reported.swap(true, Ordering::Relaxed) {
            debug!(path = %self.path.display(), %error, "phase file still not writable");
        } else {
            warn!(
                path = %self.path.display(),
                %error,
                "phase file not writable; scan continues without progress"
            );
        }
    }
}

fn build_categorizer(request: &Request) -> Result<Arc<dyn Categorizer>, String> {
    match request.engine {
        Engine::Heuristic => Ok(Arc::new(HeuristicCategorizer::new())),
        Engine::Laya => build_laya_categorizer(request),
    }
}

/// Build the Laya engine. Without the `laya` feature there is no checkpoint code
/// in the binary at all, so the engine is reported as unavailable rather than
/// silently falling back to the heuristic.
#[cfg(feature = "laya")]
fn build_laya_categorizer(request: &Request) -> Result<Arc<dyn Categorizer>, String> {
    let model = match request.model {
        ModelChoice::English => LayaModel::English,
        ModelChoice::Multilingual => LayaModel::Multilingual,
    };
    let config = LayaConfig {
        model,
        model_path: request.model_path.clone(),
        dtype: request.dtype.clone(),
        ..LayaConfig::default()
    };

    let categorizer = LayaCategorizer::load(&config)
        .map_err(|error| format!("laya checkpoint failed: {error}"))?;
    info!(model = categorizer.model(), "laya checkpoint loaded");
    Ok(Arc::new(categorizer))
}

#[cfg(not(feature = "laya"))]
fn build_laya_categorizer(_request: &Request) -> Result<Arc<dyn Categorizer>, String> {
    Err("this build has no `laya` feature; rerun with --engine heuristic".to_owned())
}

fn load_library(path: &Path) -> Result<Vec<AppProfile>, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read library {}: {error}", path.display()))?;
    serde_json::from_str(&contents)
        .map_err(|error| format!("failed to parse library {}: {error}", path.display()))
}

fn write_report(path: &Path, report: &ScanReport) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed to serialize report: {error}"))?;
    fs::write(path, json)
        .map_err(|error| format!("failed to write report {}: {error}", path.display()))
}
