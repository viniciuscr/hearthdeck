#[rustfmt::skip]
mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));
}

mod app;
mod app_group;
mod form_factor;
mod icon_cache;
mod input_ownership;
mod launch_state;
mod library_item;
mod localize;
mod motion;
mod providers;
mod screens;
mod style;
mod subscriptions;
mod system_status;
mod text_entry;
mod toplevel;
mod ui;
mod wallpaper;
mod widgets;

use config::{APP_ID, VERSION};

use localize::localize;

fn main() -> cosmic::iced::Result {
    init_logging();
    // Prepare i18n
    localize();

    app::run()
}

fn init_logging() {
    hearthdeck_observability::init("hearthdeck-frontend", "hearthdeck_frontend=info");
    tracing::info!(app_id = APP_ID, version = VERSION, "frontend starting");
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    #[test]
    fn frontend_logging_uses_shared_format_and_filter() {
        const CHILD: &str = "HEARTHDECK_LOGGING_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            super::init_logging();
            tracing::warn!(probe = true, "logging probe");
            return;
        }

        for (format, level) in [
            ("default", "default"),
            ("json", "info"),
            ("json", "warn"),
            ("pretty", "info"),
        ] {
            let mut command = Command::new(std::env::current_exe().expect("test executable"));
            command
                .args([
                    "--exact",
                    "tests::frontend_logging_uses_shared_format_and_filter",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env_remove("HEARTHDECK_LOG_FORMAT")
                .env_remove("RUST_LOG");
            if format != "default" {
                command
                    .env("HEARTHDECK_LOG_FORMAT", format)
                    .env("RUST_LOG", format!("hearthdeck_frontend={level}"));
            }
            let output = command.output().expect("run logging child");
            assert!(output.status.success(), "{format}/{level}: {output:?}");
            let stdout = String::from_utf8(output.stdout).expect("UTF-8 logs");
            if format == "pretty" {
                assert!(stdout.contains("frontend starting"), "{stdout}");
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
                .map(|line| serde_json::from_str(line).expect("JSON log event"))
                .collect();
            let probe = events
                .iter()
                .find(|event| event["message"] == "logging probe")
                .expect("structured probe event");
            assert_eq!(probe["level"], "WARN");
            assert_eq!(probe["probe"], true);
            assert_eq!(probe["target"], "hearthdeck_frontend::tests");
            let startup = events
                .iter()
                .find(|event| event["message"] == "frontend starting");
            if level != "warn" {
                let startup = startup.expect("startup event at default level");
                assert_eq!(startup["app_id"], super::APP_ID);
                assert_eq!(startup["version"], super::VERSION);
            } else {
                assert!(startup.is_none(), "RUST_LOG did not filter startup");
                assert_eq!(events.len(), 1);
            }
        }
    }
}
