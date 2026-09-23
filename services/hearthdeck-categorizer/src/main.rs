//! One-shot scan CLI.
//!
//! The daemon spawns this as a child process to categorize a library: it reads a
//! JSON array of [`AppProfile`]s from `--library`, runs one scan, and writes the
//! [`ScanReport`] as pretty JSON to `--output`. There is no interactive
//! behaviour, and the report never goes to stdout — the observability helper
//! does, but stdout is the journal's to read.
//!
//! Arguments are parsed by hand rather than with a parser crate: the contract is
//! a fixed, flat list of flags and the daemon spawns one exact command line, so
//! a dependency would buy nothing.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use hearthdeck_categorizer::{
    AppProfile, Categorizer, HeuristicCategorizer, LibraryScanner, NoopResearcher, ScanOptions,
    ScanReport,
};
#[cfg(feature = "laya")]
use hearthdeck_categorizer::{LayaCategorizer, LayaConfig, LayaModel};
use tracing::info;

/// Printed to stderr whenever the command line does not match the contract.
const USAGE: &str = "usage: hearthdeck-categorizer scan --library <PATH> --output <PATH> \
                     [--engine heuristic|laya] [--model english|multilingual] \
                     [--model-path <DIR>] [--dtype <DTYPE>] [--research]";

/// Checkpoint precision used when `--dtype` is not given.
const DEFAULT_DTYPE: &str = "float16";

#[tokio::main]
async fn main() -> ExitCode {
    hearthdeck_observability::init("hearthdeck-categorizer", "hearthdeck_categorizer=info");

    let args: Vec<OsString> = std::env::args_os().collect();
    let request = match Request::parse(&args) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("hearthdeck-categorizer: {message}");
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match run(request).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("hearthdeck-categorizer: {message}");
            ExitCode::FAILURE
        }
    }
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
}

impl Request {
    /// Parse `argv`, or return a message explaining what was wrong with it.
    fn parse(args: &[OsString]) -> Result<Self, String> {
        let mut args = args.iter().skip(1);

        let command = args
            .next()
            .ok_or_else(|| "missing subcommand (expected `scan`)".to_owned())?;
        if command.to_str() != Some("scan") {
            return Err(format!(
                "unknown subcommand `{}` (expected `scan`)",
                command.to_string_lossy()
            ));
        }

        // Index-based rather than an iterator so the value helpers can borrow the
        // same slice while the cursor advances.
        let args: Vec<OsString> = args.cloned().collect();
        let mut index = 0;
        let mut library = None;
        let mut output = None;
        let mut engine = Engine::Laya;
        let mut model = ModelChoice::English;
        let mut model_path = None;
        let mut dtype = DEFAULT_DTYPE.to_owned();
        let mut research = false;

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

async fn run(request: Request) -> Result<(), String> {
    info!(
        library = %request.library.display(),
        output = %request.output.display(),
        engine = request.engine.as_str(),
        model = request.model.as_str(),
        dtype = %request.dtype,
        model_path = ?request.model_path,
        research = request.research,
        "categorizer scan requested"
    );

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
