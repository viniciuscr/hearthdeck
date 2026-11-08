//! The systemd side of off-site backup.
//!
//! Every command is a fixed argv built here; nothing from the API reaches a unit
//! name or an argument. That is the same rule the diagnostics and session
//! adapters follow, and it is why `CommandRunner` takes a program from this
//! module's own constants rather than from a caller.

use std::{
    process::Stdio,
    sync::Mutex,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone as _, Utc};
use tracing::{debug, info, warn};

use super::{
    BackupSettings, BackupStatus, SERVICE_UNIT, SettingsView, Store, TIMER_UNIT, ValidationError,
    next_run, validate,
};

/// The only programs this module runs.
const SYSTEMCTL: &str = "systemctl";
const JOURNALCTL: &str = "journalctl";
const RCLONE: &str = "rclone";

/// How long a status answer is reused. A polling UI must not spawn a
/// `systemctl` per frame, and status is not worth more than a few seconds of
/// freshness.
const STATUS_TTL: Duration = Duration::from_secs(5);

/// Why a backup operation failed. The API maps each to a status code: a build
/// without the units is a 503, a refused write is a 409 or a 400, and a broken
/// service manager is upstream's problem.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ControlError {
    #[error("this Hearthdeck build does not include off-site backup")]
    NotInstalled,
    #[error("{0}")]
    Refused(#[from] ValidationError),
    #[error("the user service manager could not {0}")]
    Systemd(String),
    #[error("backup configuration could not be written: {0}")]
    Storage(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// What the control needs from the machine. A trait so tests can answer without
/// a real service manager, exactly as `ScanRunner` does for the categorizer.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, program: &'static str, args: &[String]) -> std::io::Result<CommandOutput>;
}

/// The real runner: the system's own commands, with no shell in between.
pub struct ProcessRunner;

#[async_trait]
impl CommandRunner for ProcessRunner {
    async fn run(&self, program: &'static str, args: &[String]) -> std::io::Result<CommandOutput> {
        let output = tokio::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .await?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

/// The last status answer and when it was taken.
#[derive(Default)]
pub struct StatusCache {
    last: Mutex<Option<(Instant, BackupStatus)>>,
}

impl StatusCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn get(&self) -> Option<BackupStatus> {
        let cached = self.last.lock().ok()?;
        let (taken, status) = cached.as_ref()?;
        (taken.elapsed() < STATUS_TTL).then(|| status.clone())
    }

    fn put(&self, status: &BackupStatus) {
        if let Ok(mut cached) = self.last.lock() {
            *cached = Some((Instant::now(), status.clone()));
        }
    }

    /// A write changes what status would say, so the answer is dropped rather
    /// than aged.
    pub fn clear(&self) {
        if let Ok(mut cached) = self.last.lock() {
            *cached = None;
        }
    }
}

pub(crate) struct Control<'a> {
    pub store: &'a Store,
    pub runner: &'a dyn CommandRunner,
    pub cache: &'a StatusCache,
}

impl Control<'_> {
    /// The current settings, whether the timer runs, and whether rclone knows the
    /// remote.
    pub async fn settings(&self) -> Result<SettingsView, ControlError> {
        self.require_installed().await?;
        let stored = self.store.read();
        let timer = self.show_properties(TIMER_UNIT, &["ActiveState"]).await?;
        let enabled = is_running(&timer);
        let remote_available = self.remote_available(&stored.remote).await?;
        Ok(SettingsView {
            enabled,
            remote: stored.remote,
            root: stored.root,
            schedule: stored.schedule,
            remote_available,
            updated_at: self.store.updated_at(),
        })
    }

    /// Validates, writes both user-owned files, reloads the units and moves the
    /// timer. The order matters: the configuration is on disk before the timer
    /// that reads it is enabled.
    pub async fn update(&self, requested: BackupSettings) -> Result<SettingsView, ControlError> {
        self.require_installed().await?;
        let remote_available = self.remote_available(&requested.remote).await?;
        validate(&requested, remote_available)?;

        if let Err(error) = self.store.write(&requested) {
            warn!(
                %error,
                remote = %requested.remote,
                root = %requested.root,
                "the backup configuration could not be written"
            );
            return Err(error.into());
        }
        self.systemctl(&["--user", "daemon-reload"], "reload its units")
            .await?;
        if requested.enabled {
            self.systemctl(
                &["--user", "enable", "--now", TIMER_UNIT],
                "enable the timer",
            )
            .await?;
        } else {
            self.systemctl(
                &["--user", "disable", "--now", TIMER_UNIT],
                "disable the timer",
            )
            .await?;
        }
        info!(
            remote = %requested.remote,
            root = %requested.root,
            enabled = requested.enabled,
            hour = requested.schedule.hour,
            minute = requested.schedule.minute,
            "off-site backup settings written"
        );
        self.cache.clear();
        self.settings().await
    }

    /// The last run and the next one. Cached briefly, because the UI polls it.
    pub async fn status(&self) -> Result<BackupStatus, ControlError> {
        if let Some(cached) = self.cache.get() {
            return Ok(cached);
        }
        self.require_installed().await?;
        let stored = self.store.read();
        let timer = self.show_properties(TIMER_UNIT, &["ActiveState"]).await?;
        let service = self
            .show_properties(SERVICE_UNIT, &["ActiveState", "Result", "ExecMainStatus"])
            .await?;
        // `ActiveState`, not `is-active`: a `Type=oneshot` unit is `activating`
        // for the whole run, and an `is-active` equality check would call that
        // "not running" - which is the state the UI most wants to show.
        let timer_active = is_running(&timer);
        let service_active = is_running(&service);
        let result = service.get("Result").cloned().unwrap_or_default();
        let exit_status = service
            .get("ExecMainStatus")
            .and_then(|status| status.parse::<i32>().ok())
            .unwrap_or(0);
        let failed = !matches!(result.as_str(), "success" | "");
        let next_run = if timer_active {
            next_run(stored.schedule, chrono::Local::now())
        } else {
            None
        };
        let last_error = if failed {
            self.last_error().await
        } else {
            None
        };
        // The service's own answer: `success`, or what systemd says went wrong
        // and the exit status that goes with it.
        let last_result = match result.as_str() {
            "" => None,
            "success" => Some("success".to_string()),
            other if exit_status != 0 => Some(format!("{other} (exit {exit_status})")),
            other => Some(other.to_string()),
        };
        let status = BackupStatus {
            timer_active,
            service_active,
            next_run,
            last_result,
            last_finished_at: self.last_finished().await,
            last_error,
        };
        self.cache.put(&status);
        Ok(status)
    }

    /// Starts one run. It deliberately does not wait: a backup can take minutes,
    /// and the caller polls `status` instead.
    pub async fn run_now(&self) -> Result<(), ControlError> {
        self.require_installed().await?;
        // `--no-block` is the point, not a shortcut: the unit is `Type=oneshot`
        // with a six-hour start timeout, and the route promises 202 immediately.
        self.systemctl(
            &["--user", "start", "--no-block", SERVICE_UNIT],
            "start the backup",
        )
        .await?;
        info!("off-site backup run requested");
        self.cache.clear();
        Ok(())
    }

    async fn require_installed(&self) -> Result<(), ControlError> {
        // Both units, not just the service: a machine with the service but no
        // timer cannot back anything up on a schedule, and the contract calls a
        // missing unit a 503 rather than a broken manager.
        for unit in [SERVICE_UNIT, TIMER_UNIT] {
            let installed = self
                .runner
                .run(SYSTEMCTL, &strings(&["--user", "cat", unit]))
                .await
                .map(|output| output.success)
                .unwrap_or(false);
            if !installed {
                return Err(ControlError::NotInstalled);
            }
        }
        Ok(())
    }

    /// One `systemctl show` per unit, several properties: the status read must
    /// not spawn the manager once per field it wants.
    ///
    /// `systemctl show --property=Result` answers `Result=success`: the name is
    /// the key, not the whole argument.
    async fn show_properties(
        &self,
        unit: &str,
        properties: &[&str],
    ) -> Result<std::collections::HashMap<String, String>, ControlError> {
        let mut args = vec!["--user".to_string(), "show".to_string(), unit.to_string()];
        args.extend(
            properties
                .iter()
                .map(|property| format!("--property={property}")),
        );
        let output = self.runner.run(SYSTEMCTL, &args).await.map_err(|error| {
            warn!(%error, unit, "the user service manager could not be asked about the backup");
            ControlError::Systemd(error.to_string())
        })?;
        if !output.success {
            warn!(
                unit,
                stderr = %output.stderr.trim(),
                "the user service manager did not answer for the backup unit"
            );
            // An empty answer is not the same as an idle unit: a manager that
            // cannot answer must surface as a 502, not as "off" with a 200.
            return Err(ControlError::Systemd(format!(
                "read {unit}: {}",
                output.stderr.trim()
            )));
        }
        let wanted = properties;
        Ok(output
            .stdout
            .lines()
            .filter_map(|line| line.split_once('='))
            .filter(|(key, _)| wanted.contains(key))
            .map(|(key, value)| (key.to_string(), value.trim().to_string()))
            .collect())
    }

    /// Whether `rclone listremotes` knows the remote. A remote that rclone has
    /// never heard of cannot be enabled, and its credentials never reach us.
    async fn remote_available(&self, remote: &str) -> Result<bool, ControlError> {
        let output = self.runner.run(RCLONE, &strings(&["listremotes"])).await;
        let Ok(output) = output else {
            warn!("rclone could not be run; treating the remote as absent");
            return Ok(false);
        };
        if !output.success {
            warn!("rclone listremotes failed; treating the remote as absent");
            return Ok(false);
        }
        // rclone names are case-sensitive, and a case-insensitive match here
        // would enable a remote that `rclone` itself then refuses.
        let wanted = format!("{remote}:");
        Ok(output.stdout.lines().any(|line| line.trim() == wanted))
    }

    /// When the last run finished, from the journal: the service's own timestamp
    /// property is a localized string, while the journal carries epoch
    /// microseconds.
    async fn last_finished(&self) -> Option<DateTime<Utc>> {
        let output = self
            .runner
            .run(
                JOURNALCTL,
                &strings(&[
                    "--user",
                    "-u",
                    SERVICE_UNIT,
                    "-n",
                    "1",
                    "--no-pager",
                    "--output=json",
                ]),
            )
            .await
            .map_err(|error| debug!(%error, "the last finish time is unavailable"))
            .ok()?;
        if !output.success {
            return None;
        }
        let entry: serde_json::Value = serde_json::from_str(output.stdout.trim()).ok()?;
        let micros: i64 = entry
            .get("__REALTIME_TIMESTAMP")?
            .as_str()?
            .trim()
            .parse()
            .ok()?;
        Utc.timestamp_micros(micros).single()
    }

    async fn last_error(&self) -> Option<String> {
        let output = self
            .runner
            .run(
                JOURNALCTL,
                &strings(&[
                    "--user",
                    "-u",
                    SERVICE_UNIT,
                    "-n",
                    "20",
                    "--no-pager",
                    "--output=cat",
                ]),
            )
            .await
            .map_err(|error| debug!(%error, "the last error line is unavailable"))
            .ok()?;
        if !output.success {
            return None;
        }
        output
            .stdout
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_string())
    }

    async fn systemctl(&self, args: &[&str], what: &str) -> Result<CommandOutput, ControlError> {
        let output = self
            .runner
            .run(SYSTEMCTL, &strings(args))
            .await
            .map_err(|error| {
                warn!(%error, command = ?args, "the user service manager could not be run");
                ControlError::Systemd(format!("{what}: {error}"))
            })?;
        if !output.success {
            warn!(
                command = ?args,
                stderr = %output.stderr.trim(),
                "the user service manager refused a backup command"
            );
            return Err(ControlError::Systemd(format!(
                "{what}: {}",
                output.stderr.trim()
            )));
        }
        Ok(output)
    }
}

/// Whether a unit's `ActiveState` means "doing work right now". `activating` is
/// the whole run of a `Type=oneshot` unit, so it belongs with `active`.
fn is_running(properties: &std::collections::HashMap<String, String>) -> bool {
    matches!(
        properties.get("ActiveState").map(String::as_str),
        Some("active" | "activating" | "reloading")
    )
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::{CommandOutput, CommandRunner, Control, ControlError, StatusCache};
    use crate::backup::{
        BackupSettings, DEFAULT_REMOTE, DEFAULT_ROOT, DEFAULT_SCHEDULE, Store, TIMER_UNIT,
        ValidationError,
    };
    use async_trait::async_trait;
    use std::fmt::Write as _;
    use std::sync::Mutex;

    /// Answers from a table and records what was asked, so a test can assert both
    /// the answer and the order of the commands.
    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<String>>,
        remotes: Vec<&'static str>,
        installed: bool,
        /// A manager that cannot answer at all: `systemctl show` fails.
        manager_down: bool,
        /// What the manager would report, and what enable/disable changes: the
        /// timer's state is the setting, and a running oneshot is `activating`.
        timer: Mutex<&'static str>,
        service: Mutex<&'static str>,
        result: Option<&'static str>,
        exit_status: u32,
    }

    impl FakeRunner {
        fn installed() -> Self {
            Self {
                installed: true,
                manager_down: false,
                remotes: vec!["pcloud"],
                timer: Mutex::new("inactive"),
                service: Mutex::new("inactive"),
                result: Some("success"),
                exit_status: 0,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("the call log").clone()
        }

        fn with_service(self, state: &'static str) -> Self {
            *self.service.lock().expect("the service state") = state;
            self
        }

        fn with_result(mut self, result: &'static str, exit_status: u32) -> Self {
            self.result = Some(result);
            self.exit_status = exit_status;
            self
        }
    }

    #[async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(
            &self,
            program: &'static str,
            args: &[String],
        ) -> std::io::Result<CommandOutput> {
            self.calls
                .lock()
                .expect("the call log")
                .push(format!("{program} {}", args.join(" ")));

            let ok = |stdout: String| CommandOutput {
                success: true,
                stdout,
                stderr: String::new(),
            };
            let output = match program {
                "systemctl" => {
                    let joined = format!(" {} ", args.join(" "));
                    if joined.contains(" cat ") && !self.installed {
                        CommandOutput {
                            success: false,
                            stdout: String::new(),
                            stderr: "No files found for hearthdeck-backup.service.".to_string(),
                        }
                    } else if joined.contains(" enable --now ") {
                        *self.timer.lock().expect("the timer state") = "active";
                        ok(String::new())
                    } else if joined.contains(" disable --now ") {
                        *self.timer.lock().expect("the timer state") = "inactive";
                        ok(String::new())
                    } else if joined.contains(" show ") && self.manager_down {
                        CommandOutput {
                            success: false,
                            stdout: String::new(),
                            stderr: "Failed to connect to bus: No such file or directory"
                                .to_string(),
                        }
                    } else if joined.contains(" show ") {
                        let state = if joined.contains(TIMER_UNIT) {
                            *self.timer.lock().expect("the timer state")
                        } else {
                            *self.service.lock().expect("the service state")
                        };
                        let mut stdout = String::new();
                        for property in args
                            .iter()
                            .filter_map(|arg| arg.strip_prefix("--property="))
                        {
                            match property {
                                "ActiveState" => {
                                    let _ = writeln!(stdout, "ActiveState={state}");
                                }
                                "Result" => {
                                    let _ =
                                        writeln!(stdout, "Result={}", self.result.unwrap_or(""));
                                }
                                "ExecMainStatus" => {
                                    let _ = writeln!(stdout, "ExecMainStatus={}", self.exit_status);
                                }
                                _ => {}
                            }
                        }
                        ok(stdout)
                    } else {
                        ok(String::new())
                    }
                }
                // The journal is where the runner's own words land; a failure
                // test needs one line to offer.
                "journalctl" => ok("hearthdeck-backup: rclone upload failed\n".to_string()),
                "rclone" => ok(self
                    .remotes
                    .iter()
                    .map(|remote| format!("{remote}:"))
                    .collect::<Vec<_>>()
                    .join("\n")),
                _ => ok(String::new()),
            };
            Ok(output)
        }
    }

    fn store() -> (Store, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "hearthdeck-backup-control-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        (Store::new(root.join("backup"), root.join("systemd")), root)
    }

    fn requested(enabled: bool, remote: &str) -> BackupSettings {
        BackupSettings {
            enabled,
            remote: remote.to_string(),
            root: DEFAULT_ROOT.to_string(),
            schedule: DEFAULT_SCHEDULE,
        }
    }

    #[tokio::test]
    async fn enabling_writes_the_files_before_it_starts_the_timer() {
        let (store, root) = store();
        let runner = FakeRunner::installed();
        let cache = StatusCache::new();
        let control = Control {
            store: &store,
            runner: &runner,
            cache: &cache,
        };

        let view = control
            .update(requested(true, DEFAULT_REMOTE))
            .await
            .expect("the write succeeds");

        assert!(view.enabled, "the timer is on after the write");
        assert!(view.remote_available);
        let calls = runner.calls();
        let reload = calls
            .iter()
            .position(|call| call.contains("daemon-reload"))
            .expect("a reload");
        let enable = calls
            .iter()
            .position(|call| call.contains("enable --now"))
            .expect("an enable");
        assert!(reload < enable, "the units reload before the timer moves");
        assert!(
            root.join("backup/backup.env").exists(),
            "the environment file is on disk"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn enabling_a_remote_rclone_does_not_know_is_refused_and_writes_nothing() {
        let (store, root) = store();
        let runner = FakeRunner::installed();
        let cache = StatusCache::new();
        let control = Control {
            store: &store,
            runner: &runner,
            cache: &cache,
        };

        let error = control
            .update(requested(true, "nowhere"))
            .await
            .expect_err("a missing remote is refused");
        assert!(matches!(
            error,
            ControlError::Refused(ValidationError::RemoteMissing)
        ));
        assert!(
            !root.join("backup/backup.env").exists(),
            "a refused write leaves the old configuration alone"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The route promises 202 immediately, and the unit is a oneshot with a
    /// six-hour timeout: a blocking start would hold the request for the run.
    #[tokio::test]
    async fn run_now_asks_for_the_start_without_waiting() {
        let (store, root) = store();
        let runner = FakeRunner::installed();
        let cache = StatusCache::new();
        let control = Control {
            store: &store,
            runner: &runner,
            cache: &cache,
        };

        control.run_now().await.expect("accepted");

        let calls = runner.calls();
        assert!(
            calls
                .iter()
                .any(|call| call.contains("start --no-block hearthdeck-backup.service")),
            "the start must not block: {calls:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A `Type=oneshot` unit is `activating` for the whole run; treating only
    /// `active` as running would make the Running state unreachable.
    #[tokio::test]
    async fn a_running_oneshot_counts_as_running() {
        let (store, root) = store();
        let runner = FakeRunner::installed().with_service("activating");
        let cache = StatusCache::new();
        let control = Control {
            store: &store,
            runner: &runner,
            cache: &cache,
        };

        let status = control.status().await.expect("a status");
        assert!(status.service_active, "activating is work in progress");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The service's own answer plus its exit status, so a failed run says what
    /// failed rather than just "failed".
    #[tokio::test]
    async fn a_failed_run_reports_the_result_and_the_exit_status() {
        let (store, root) = store();
        let runner = FakeRunner::installed().with_result("failed", 1);
        let cache = StatusCache::new();
        let control = Control {
            store: &store,
            runner: &runner,
            cache: &cache,
        };

        let status = control.status().await.expect("a status");
        assert_eq!(status.last_result.as_deref(), Some("failed (exit 1)"));
        assert_eq!(
            status.last_error.as_deref(),
            Some("hearthdeck-backup: rclone upload failed"),
            "the journal's last line is what the UI offers"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A manager that cannot answer must not be read as "off": the routes
    /// promise a 502, and an empty answer would silently claim the timer is idle.
    #[tokio::test]
    async fn a_manager_that_cannot_answer_is_an_error_not_an_idle_timer() {
        let (store, root) = store();
        let runner = FakeRunner {
            manager_down: true,
            ..FakeRunner::installed()
        };
        let cache = StatusCache::new();
        let control = Control {
            store: &store,
            runner: &runner,
            cache: &cache,
        };

        assert!(matches!(
            control.settings().await,
            Err(ControlError::Systemd(_))
        ));
        assert!(matches!(
            control.status().await,
            Err(ControlError::Systemd(_))
        ));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_service_the_package_did_not_install_is_not_an_error_page() {
        let (store, root) = store();
        let runner = FakeRunner::default();
        let cache = StatusCache::new();
        let control = Control {
            store: &store,
            runner: &runner,
            cache: &cache,
        };

        assert!(matches!(
            control.settings().await,
            Err(ControlError::NotInstalled)
        ));
        assert!(matches!(
            control.run_now().await,
            Err(ControlError::NotInstalled)
        ));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn status_is_answered_from_the_cache_within_its_window() {
        let (store, root) = store();
        let runner = FakeRunner::installed();
        let cache = StatusCache::new();
        let control = Control {
            store: &store,
            runner: &runner,
            cache: &cache,
        };

        let first = control.status().await.expect("a status");
        assert_eq!(first.last_result.as_deref(), Some("success"));
        let after_first = runner.calls().len();

        let second = control.status().await.expect("a cached status");
        assert_eq!(first.timer_active, second.timer_active);
        assert_eq!(
            runner.calls().len(),
            after_first,
            "a polling UI does not spawn systemctl each time"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
