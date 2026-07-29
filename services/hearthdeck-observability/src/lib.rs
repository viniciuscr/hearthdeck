//! Shared, production-safe logging setup for Hearthdeck service binaries.

use std::env;

use tracing::info;
use tracing_subscriber::EnvFilter;

/// Initializes structured logging once for a service process.
///
/// `HEARTHDECK_LOG_FORMAT=json` is the default for journald ingestion. Set
/// `HEARTHDECK_LOG_FORMAT=pretty` for readable local development logs. `RUST_LOG`
/// controls filtering without recompiling, for example:
/// `RUST_LOG=hearthdeck_daemon=debug,tower_http=info`.
pub fn init(component: &'static str, default_filter: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    let format = env::var("HEARTHDECK_LOG_FORMAT").unwrap_or_else(|_| "json".to_owned());

    match format.as_str() {
        "pretty" => {
            let _ = tracing_subscriber::fmt()
                .compact()
                .with_env_filter(filter)
                .with_target(true)
                .with_ansi(true)
                .try_init();
        }
        "json" => {
            let _ = tracing_subscriber::fmt()
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(true)
                .with_env_filter(filter)
                .with_target(true)
                .with_ansi(false)
                .try_init();
        }
        invalid => {
            let _ = tracing_subscriber::fmt()
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(true)
                .with_env_filter(filter)
                .with_target(true)
                .try_init();
            tracing::warn!(invalid_format = invalid, "invalid log format; using JSON");
        }
    }

    info!(component, log_format = %format, "logging initialized");
}
