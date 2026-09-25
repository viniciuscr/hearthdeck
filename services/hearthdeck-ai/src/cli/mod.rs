//! The command-line front end.
//!
//! One subcommand per functionality. The daemon spawns exactly one command line
//! (`scan`); a human can run the same one by hand. Arguments are parsed by hand
//! rather than with a parser crate: the contract is a fixed, flat list of flags,
//! so a dependency would buy nothing.

mod scan;

use std::ffi::OsString;

/// Printed whenever the command line does not match the contract.
pub(crate) const USAGE: &str = "\
usage: hearthdeck-ai <command> [options]
commands:
  scan    categorize a library

scan options:
  --library <PATH>    JSON array of applications to scan (required)
  --output <PATH>     where to write the ScanReport (required)
  --engine <NAME>     heuristic|laya (default laya)
  --model <NAME>      english|multilingual (default english)
  --model-path <DIR>  a local checkpoint directory, so nothing is downloaded
  --dtype <DTYPE>     float32|float16|bfloat16 (default float16)
  --research          look applications up online before deciding
  --phase-file <PATH> report progress to this file for the parent process";

/// Dispatch to the functionality named by `argv`.
pub(crate) async fn run(args: &[OsString]) -> Result<(), String> {
    let Some(command) = args.get(1) else {
        return Err("missing command".to_owned());
    };
    match command.to_str() {
        Some("scan") => scan::run(args).await,
        Some(other) => Err(format!("unknown command `{other}`")),
        None => Err("command is not valid UTF-8".to_owned()),
    }
}
