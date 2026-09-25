//! Binary entry point: dispatch a subcommand to the functionality it names.
//!
//! The library ([`hearthdeck_ai`]) is where the functionalities live; this file
//! only wires up the command line and the process's exit code.

mod cli;

use std::ffi::OsString;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    hearthdeck_observability::init("hearthdeck-ai", "hearthdeck_ai=info");

    let args: Vec<OsString> = std::env::args_os().collect();
    match cli::run(&args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("hearthdeck-ai: {message}");
            eprintln!("{}", cli::USAGE);
            ExitCode::FAILURE
        }
    }
}
