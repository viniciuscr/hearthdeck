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
use tracing::{info, warn};

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
        let enabled = self.timer_active().await?;
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

        self.store.write(&requested)?;
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
        let timer_active = self.timer_active().await?;
        let service_active = self.unit_active(SERVICE_UNIT).await?;
        let result = self
            .show_property(SERVICE_UNIT, "Result")
            .await
            .unwrap_or(None);
        let failed = result
            .as_deref()
            .is_some_and(|result| !matches!(result, "success" | ""));
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
        let status = BackupStatus {
            timer_active,
            service_active,
            next_run,
            last_result: result.filter(|result| !result.is_empty()),
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
        self.systemctl(&["--user", "start", SERVICE_UNIT], "start the backup")
            .await?;
        info!("off-site backup run requested");
        self.cache.clear();
        Ok(())
    }

    async fn require_installed(&self) -> Result<(), ControlError> {
        let installed = self
            .runner
            .run(SYSTEMCTL, &strings(&["--user", "cat", SERVICE_UNIT]))
            .await
            .map(|output| output.success)
            .unwrap_or(false);
        if installed {
            Ok(())
        } else {
            Err(ControlError::NotInstalled)
        }
    }

    async fn timer_active(&self) -> Result<bool, ControlError> {
        self.unit_active(TIMER_UNIT).await
    }

    async fn unit_active(&self, unit: &str) -> Result<bool, ControlError> {
        let output = self
            .runner
            .run(SYSTEMCTL, &strings(&["--user", "is-active", unit]))
            .await
            .map_err(|error| ControlError::Systemd(error.to_string()))?;
        Ok(output.stdout.trim() == "active")
    }

    async fn show_property(
        &self,
        unit: &str,
        property: &str,
    ) -> Result<Option<String>, ControlError> {
        // `systemctl show --property=Result` answers `Result=success`: the name
        // is the key, not the whole argument.
        let argument = format!("--property={property}");
        let output = self
            .runner
            .run(SYSTEMCTL, &strings(&["--user", "show", unit, &argument]))
            .await
            .map_err(|error| ControlError::Systemd(error.to_string()))?;
        let prefix = format!("{property}=");
        Ok(output
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix(&prefix))
            .map(|value| value.trim().to_string()))
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
        let wanted = format!("{remote}:");
        Ok(output
            .stdout
            .lines()
            .any(|line| line.trim().eq_ignore_ascii_case(&wanted)))
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
            .map_err(|error| ControlError::Systemd(format!("{what}: {error}")))?;
        if !output.success {
            return Err(ControlError::Systemd(format!(
                "{what}: {}",
                output.stderr.trim()
            )));
        }
        Ok(output)
    }
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::{CommandOutput, CommandRunner, Control, ControlError, StatusCache};
    use crate::backup::{
        BackupSettings, DEFAULT_REMOTE, DEFAULT_ROOT, DEFAULT_SCHEDULE, Store, ValidationError,
    };
    use async_trait::async_trait;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    /// Answers from a table and records what was asked, so a test can assert both
    /// the answer and the order of the commands.
    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<String>>,
        remotes: Vec<&'static str>,
        installed: bool,
        /// What the service manager would answer, and what the enable and
        /// disable commands change - the timer's state is the setting here.
        timer_active: AtomicBool,
        service_active: AtomicBool,
        result: Option<&'static str>,
    }

    impl FakeRunner {
        fn installed() -> Self {
            Self {
                installed: true,
                remotes: vec!["pcloud"],
                timer_active: AtomicBool::new(false),
                service_active: AtomicBool::new(false),
                result: Some("success"),
                ..Self::default()
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("the call log").clone()
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

            let output = match program {
                "systemctl" => {
                    let joined = args.join(" ");
                    if joined.contains("cat ") && !self.installed {
                        CommandOutput {
                            success: false,
                            stdout: String::new(),
                            stderr: "No files found for hearthdeck-backup.service.".to_string(),
                        }
                    } else if joined.contains("enable --now") {
                        self.timer_active.store(true, Ordering::Relaxed);
                        CommandOutput {
                            success: true,
                            stdout: String::new(),
                            stderr: String::new(),
                        }
                    } else if joined.contains("disable --now") {
                        self.timer_active.store(false, Ordering::Relaxed);
                        CommandOutput {
                            success: true,
                            stdout: String::new(),
                            stderr: String::new(),
                        }
                    } else if joined.contains("is-active") {
                        let active = if joined.contains("timer") {
                            self.timer_active.load(Ordering::Relaxed)
                        } else {
                            self.service_active.load(Ordering::Relaxed)
                        };
                        let state = if active { "active" } else { "inactive" };
                        CommandOutput {
                            success: state == "active",
                            stdout: format!("{state}\n"),
                            stderr: String::new(),
                        }
                    } else if joined.contains("--property=Result") {
                        CommandOutput {
                            success: true,
                            stdout: format!("Result={}\n", self.result.unwrap_or("")),
                            stderr: String::new(),
                        }
                    } else {
                        CommandOutput {
                            success: true,
                            stdout: String::new(),
                            stderr: String::new(),
                        }
                    }
                }
                "rclone" => CommandOutput {
                    success: true,
                    stdout: self
                        .remotes
                        .iter()
                        .map(|remote| format!("{remote}:"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    stderr: String::new(),
                },
                _ => CommandOutput {
                    success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                },
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
