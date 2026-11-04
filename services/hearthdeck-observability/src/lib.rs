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

    let active_format = match format.as_str() {
        "pretty" => "pretty",
        _ => "json",
    };
    let result = match active_format {
        "pretty" => tracing_subscriber::fmt()
            .compact()
            .with_env_filter(filter)
            .with_target(true)
            .with_ansi(true)
            .try_init(),
        _ => tracing_subscriber::fmt()
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(true)
            .with_env_filter(filter)
            .with_target(true)
            .with_ansi(false)
            .try_init(),
    };
    if let Err(error) = result {
        tracing::error!(component, %error, "logging initialization failed");
        return;
    }

    if !matches!(format.as_str(), "pretty" | "json") {
        tracing::warn!(invalid_format = %format, "invalid log format; using JSON");
    }
    info!(component, log_format = active_format, "logging initialized");
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    #[test]
    fn initialization_reports_the_active_format_and_rejects_reinitialization() {
        const CHILD: &str = "HEARTHDECK_OBSERVABILITY_TEST_CHILD";
        if let Some(mode) = std::env::var_os(CHILD) {
            super::init("probe", "info");
            if mode == "duplicate" {
                super::init("duplicate", "info");
            }
            tracing::warn!(probe = true, "logging probe");
            return;
        }

        for format in ["default", "json", "pretty", "invalid", "duplicate"] {
            let mut command = Command::new(std::env::current_exe().expect("test executable"));
            command
                .args([
                    "--exact",
                    "tests::initialization_reports_the_active_format_and_rejects_reinitialization",
                    "--nocapture",
                ])
                .env(CHILD, format)
                .env_remove("HEARTHDECK_LOG_FORMAT")
                .env_remove("RUST_LOG");
            if !matches!(format, "default" | "duplicate") {
                command.env("HEARTHDECK_LOG_FORMAT", format);
            }
            let output = command.output().expect("run logging child");
            assert!(output.status.success(), "{format}: {output:?}");
            let stdout = String::from_utf8(output.stdout).expect("UTF-8 logs");
            if format == "pretty" {
                assert!(stdout.contains("logging initialized"), "{stdout}");
                assert!(stdout.contains("logging probe"), "{stdout}");
                assert!(
                    !stdout.lines().any(|line| line.starts_with('{')),
                    "{stdout}"
                );
                continue;
            }
            let events: Vec<serde_json::Value> = stdout
                .lines()
                .filter(|line| line.starts_with('{'))
                .map(|line| serde_json::from_str(line).expect("JSON event"))
                .collect();
            let startup: Vec<_> = events
                .iter()
                .filter(|event| event["message"] == "logging initialized")
                .collect();
            assert_eq!(startup.len(), 1, "{format}: {stdout}");
            assert_eq!(startup[0]["log_format"], "json", "{format}: {stdout}");
            assert_eq!(startup[0]["component"], "probe", "{format}: {stdout}");
            assert!(
                events.iter().any(|event| event["probe"] == true),
                "{stdout}"
            );
            if format == "invalid" {
                assert!(
                    events.iter().any(|event| {
                        event["message"] == "invalid log format; using JSON"
                            && event["invalid_format"] == "invalid"
                    }),
                    "{stdout}"
                );
            }
            if format == "duplicate" {
                assert!(
                    events.iter().any(|event| {
                        event["message"] == "logging initialization failed"
                            && event["component"] == "duplicate"
                            && event["level"] == "ERROR"
                    }),
                    "{stdout}"
                );
            }
        }
    }
}
