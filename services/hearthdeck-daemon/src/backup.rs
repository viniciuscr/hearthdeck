//! Off-site backup: the destination a user picks and the timer that runs it.
//!
//! The runner, the units and the default job list are package-owned, so
//! `pacman -Syu` keeps them current. What belongs to the user is small, and this
//! module owns exactly that: `backup.env` (remote and destination root), the
//! timer drop-in that carries the schedule, and the enable/disable transition.
//!
//! Nothing here reads or writes a cloud credential. rclone owns those in
//! `~/.config/rclone/rclone.conf`; the UI selects a remote **by name**, the same
//! way the Stremio account is never held by the shell.
//!
//! `enabled` is not a stored setting: it is the timer's state. There is one
//! activation path, and it is the systemd timer the package installs disabled.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write as _},
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Days, Local, TimeZone as _, Utc};
use serde::{Deserialize, Serialize};

pub mod control;
pub mod providers;

use std::sync::Arc;

/// Everything the API needs for off-site backup: the user's files, the systemd
/// adapter, and the short status cache a polling UI reads through. One field in
/// `AppState`, so the routes never assemble these themselves.
#[derive(Clone)]
pub struct Backup {
    pub store: Store,
    pub runner: Arc<dyn control::CommandRunner>,
    cache: Arc<control::StatusCache>,
    providers: providers::Store,
    pcloud: Arc<dyn providers::PcloudApi>,
}

impl Backup {
    pub fn from_environment() -> Self {
        Self::with_runner(Store::from_environment(), Arc::new(control::ProcessRunner))
    }

    /// A caller that brings its own runner - the tests, and any future adapter
    /// that is not the process runner.
    pub fn with_runner(store: Store, runner: Arc<dyn control::CommandRunner>) -> Self {
        Self {
            providers: providers::Store::new(store.credentials_path()),
            store,
            runner,
            cache: Arc::new(control::StatusCache::new()),
            pcloud: Arc::new(providers::HttpPcloud::new()),
        }
    }

    pub fn providers(&self) -> providers::Providers<'_> {
        providers::Providers {
            store: &self.providers,
            api: self.pcloud.as_ref(),
        }
    }

    pub fn control(&self) -> control::Control<'_> {
        control::Control {
            store: &self.store,
            runner: self.runner.as_ref(),
            cache: &self.cache,
        }
    }
}

/// The units the package installs, disabled.
pub const TIMER_UNIT: &str = "hearthdeck-backup.timer";
pub const SERVICE_UNIT: &str = "hearthdeck-backup.service";
/// The drop-in that carries the schedule the user chose.
pub const SCHEDULE_DROP_IN: &str = "50-schedule.conf";
pub const DEFAULT_REMOTE: &str = "pcloud";
pub const DEFAULT_ROOT: &str = "hearthdeck-backups";
pub const DEFAULT_SCHEDULE: Schedule = Schedule {
    hour: 4,
    minute: 30,
};

/// The longest remote name accepted here, so a pasted line cannot become a file
/// full of noise.
const MAX_REMOTE: usize = 64;

/// Local hour and minute for the nightly run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub hour: u8,
    pub minute: u8,
}

/// What a user can change. `enabled` is carried through the same body so one
/// request expresses the whole decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSettings {
    pub enabled: bool,
    pub remote: String,
    pub root: String,
    pub schedule: Schedule,
}

/// What the settings route answers: the settings, the fact that makes them
/// actionable, and when they last changed.
#[derive(Clone, Debug, Serialize)]
pub struct SettingsView {
    pub enabled: bool,
    pub remote: String,
    pub root: String,
    pub schedule: Schedule,
    /// Whether `rclone listremotes` knows the selected remote. The UI disables
    /// Enable with an explanation when it does not.
    pub remote_available: bool,
    /// Whether `rclone` runs at all. A missing package is not a missing remote,
    /// and the page has to be able to say which of the two it is.
    pub rclone_installed: bool,
    /// Every remote `rclone listremotes` knows, so the page lists what exists
    /// instead of asking the user to type a name blind.
    pub remotes: Vec<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// The last run, and what is scheduled next.
#[derive(Clone, Debug, Default, Serialize)]
pub struct BackupStatus {
    pub timer_active: bool,
    pub service_active: bool,
    pub next_run: Option<DateTime<Utc>>,
    pub last_result: Option<String>,
    pub last_finished_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

/// Why a write was refused. `control` turns each of these into a status code.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error(
        "a remote name is 1 to {MAX_REMOTE} characters of letters, digits, dot, dash or underscore"
    )]
    Remote,
    #[error(
        "a destination root is a relative path of letters, digits, dot, dash, underscore or slash"
    )]
    Root,
    #[error("the schedule hour must be 0 to 23 and the minute 0 to 59")]
    Schedule,
    #[error("add the remote in rclone first")]
    RemoteMissing,
}

/// Checks a request against the rules the contract states. `remote_available` is
/// passed in because only the caller can ask rclone.
pub fn validate(settings: &BackupSettings, remote_available: bool) -> Result<(), ValidationError> {
    let remote = &settings.remote;
    let remote_ok = !remote.is_empty()
        && remote.len() <= MAX_REMOTE
        && remote
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !remote_ok {
        return Err(ValidationError::Remote);
    }

    let root = &settings.root;
    let root_ok = !root.is_empty()
        && !root.starts_with('/')
        && !root
            .split('/')
            .any(|part| part == ".." || part.is_empty() && root != part)
        && root
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'));
    if !root_ok {
        return Err(ValidationError::Root);
    }

    if settings.schedule.hour > 23 || settings.schedule.minute > 59 {
        return Err(ValidationError::Schedule);
    }

    // Enabling a timer with no remote would only produce nightly failures, so the
    // write is refused instead of stored and forgotten.
    if settings.enabled && !remote_available {
        return Err(ValidationError::RemoteMissing);
    }
    Ok(())
}

/// What the files on disk say. `enabled` is absent on purpose: it is the timer's
/// state, which the systemd adapter reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    pub remote: String,
    pub root: String,
    pub schedule: Schedule,
}

/// The two directories this module writes to: the user's backup configuration,
/// and the systemd user unit drop-ins.
#[derive(Clone, Debug)]
pub struct Store {
    config_root: PathBuf,
    systemd_root: PathBuf,
}

impl Store {
    /// `HEARTHDECK_BACKUP_CONFIG_DIR` and `HEARTHDECK_SYSTEMD_USER_DIR`, else the
    /// XDG locations. The overrides exist for tests and for a hand-run daemon.
    pub fn from_environment() -> Self {
        let config_root = std::env::var_os("HEARTHDECK_BACKUP_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| xdg_config().map(|dir| dir.join("hearthdeck-backup")))
            .unwrap_or_else(|| PathBuf::from("hearthdeck-backup"));
        let systemd_root = std::env::var_os("HEARTHDECK_SYSTEMD_USER_DIR")
            .map(PathBuf::from)
            .or_else(|| xdg_config().map(|dir| dir.join("systemd/user")))
            .unwrap_or_else(|| PathBuf::from("systemd/user"));
        Self::new(config_root, systemd_root)
    }

    pub fn new(config_root: PathBuf, systemd_root: PathBuf) -> Self {
        Self {
            config_root,
            systemd_root,
        }
    }

    pub fn env_path(&self) -> PathBuf {
        self.config_root.join("backup.env")
    }

    /// Where provider credentials live. A separate file from `backup.env` so the
    /// unit can read it with `EnvironmentFile=-` and a run inherits it.
    pub(crate) fn credentials_path(&self) -> PathBuf {
        self.config_root.join("credentials.env")
    }

    pub fn drop_in_path(&self) -> PathBuf {
        self.systemd_root
            .join(format!("{TIMER_UNIT}.d"))
            .join(SCHEDULE_DROP_IN)
    }

    /// The files, with the documented defaults for anything absent or
    /// unreadable. A missing configuration is a fresh install, not an error.
    pub fn read(&self) -> Stored {
        let env = fs::read_to_string(self.env_path()).unwrap_or_default();
        let (remote, root) = parse_env(&env);
        let drop_in = fs::read_to_string(self.drop_in_path()).unwrap_or_default();
        Stored {
            remote: remote.unwrap_or_else(|| DEFAULT_REMOTE.to_string()),
            root: root.unwrap_or_else(|| DEFAULT_ROOT.to_string()),
            schedule: parse_schedule(&drop_in).unwrap_or(DEFAULT_SCHEDULE),
        }
    }

    /// Writes both user-owned files: the environment file the unit reads, and the
    /// schedule drop-in. Atomic and private - the file names a cloud remote, and
    /// nothing else on the machine needs it.
    pub fn write(&self, settings: &BackupSettings) -> io::Result<()> {
        let env = format!(
            "HEARTHDECK_BACKUP_REMOTE={}\nHEARTHDECK_BACKUP_ROOT={}\n",
            settings.remote, settings.root
        );
        write_private(&self.env_path(), env.as_bytes())?;
        let drop_in = format!(
            "[Timer]\nOnCalendar=\nOnCalendar=*-*-* {:02}:{:02}:00\n",
            settings.schedule.hour, settings.schedule.minute
        );
        write_private(&self.drop_in_path(), drop_in.as_bytes())
    }

    /// When the settings last changed, for the "updated" line in the UI.
    pub fn updated_at(&self) -> Option<DateTime<Utc>> {
        let modified = fs::metadata(self.env_path()).ok()?.modified().ok()?;
        Some(DateTime::<Utc>::from(modified))
    }
}

fn xdg_config() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|home| Path::new(&home).join(".config"))
}

/// The two keys the unit reads through `EnvironmentFile=`. Anything else in the
/// file is left alone: it belongs to the runner, not to this module.
fn parse_env(text: &str) -> (Option<String>, Option<String>) {
    let mut remote = None;
    let mut root = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("HEARTHDECK_BACKUP_REMOTE=") {
            remote = Some(unquote(value));
        } else if let Some(value) = line.strip_prefix("HEARTHDECK_BACKUP_ROOT=") {
            root = Some(unquote(value));
        }
    }
    (remote, root)
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .to_string()
}

/// Reads the hour and minute back out of the drop-in the daemon itself wrote.
/// A file it did not write reads as absent rather than as a guess.
fn parse_schedule(drop_in: &str) -> Option<Schedule> {
    // The writer emits `OnCalendar=` (a reset) before the real entry, and a
    // hand-edited file may hold several. The last assignment wins, and a reset
    // clears what came before it - so the parser tracks both.
    let mut schedule = None;
    for line in drop_in.lines() {
        let Some(value) = line.trim().strip_prefix("OnCalendar=") else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            schedule = None;
            continue;
        }
        let Some((_, time)) = value.rsplit_once(' ') else {
            continue;
        };
        let mut parts = time.split(':');
        let (Some(hour), Some(minute)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (Ok(hour), Ok(minute)) = (hour.parse::<u8>(), minute.parse::<u8>()) else {
            continue;
        };
        if hour <= 23 && minute <= 59 {
            schedule = Some(Schedule { hour, minute });
        }
    }
    schedule
}

/// The next time the schedule fires, in UTC.
///
/// The timer's `OnCalendar` is local time, so the answer has to start there and
/// be converted. Computed rather than read back from systemd: the timer's own
/// next-elapse property is a localized string, and this is the value the daemon
/// just wrote.
pub fn next_run(schedule: Schedule, now: DateTime<Local>) -> Option<DateTime<Utc>> {
    for offset in 0..=1u64 {
        let date = now.date_naive().checked_add_days(Days::new(offset))?;
        let naive = date.and_hms_opt(u32::from(schedule.hour), u32::from(schedule.minute), 0)?;
        if let Some(candidate) = Local.from_local_datetime(&naive).earliest()
            && candidate > now
        {
            return Some(candidate.with_timezone(&Utc));
        }
    }
    None
}

/// Writes a file only this user can read: a private directory, a temp file in the
/// same directory, then a rename - so a crash mid-write leaves the old file
/// whole.
pub(crate) fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    let Some(directory) = path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the file has no parent directory",
        ));
    };
    fs::create_dir_all(directory)?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;

    let temporary = directory.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("backup")
    ));
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::{
        BackupSettings, DEFAULT_REMOTE, DEFAULT_ROOT, DEFAULT_SCHEDULE, Schedule, Store, Stored,
        ValidationError, next_run, parse_env, parse_schedule, validate,
    };
    use chrono::{Local, TimeZone as _, Utc};
    use std::os::unix::fs::PermissionsExt as _;

    fn settings() -> BackupSettings {
        BackupSettings {
            enabled: true,
            remote: DEFAULT_REMOTE.to_string(),
            root: DEFAULT_ROOT.to_string(),
            schedule: DEFAULT_SCHEDULE,
        }
    }

    fn store() -> (Store, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "hearthdeck-backup-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let store = Store::new(root.join("backup"), root.join("systemd"));
        (store, root)
    }

    #[test]
    fn a_remote_name_is_a_short_safe_word() {
        let mut settings = settings();
        for name in ["pcloud", "gdrive-1", "aws.glacier_2"] {
            settings.remote = name.to_string();
            assert!(validate(&settings, true).is_ok(), "{name} is accepted");
        }
        for name in ["", "two words", "semi;colon", "slash/name"] {
            settings.remote = name.to_string();
            assert_eq!(
                validate(&settings, true),
                Err(ValidationError::Remote),
                "{name:?} is refused"
            );
        }
        settings.remote = "r".repeat(65);
        assert_eq!(validate(&settings, true), Err(ValidationError::Remote));
    }

    #[test]
    fn a_root_is_relative_and_never_escapes() {
        let mut settings = settings();
        for root in ["hearthdeck-backups", "hearthdeck/saves", "a.b_c"] {
            settings.root = root.to_string();
            assert!(validate(&settings, true).is_ok(), "{root} is accepted");
        }
        for root in ["", "/absolute", "../up", "a/../b", "a b"] {
            settings.root = root.to_string();
            assert_eq!(
                validate(&settings, true),
                Err(ValidationError::Root),
                "{root:?} is refused"
            );
        }
    }

    #[test]
    fn a_schedule_is_a_real_clock_time() {
        let mut settings = settings();
        settings.schedule = Schedule {
            hour: 24,
            minute: 0,
        };
        assert_eq!(validate(&settings, true), Err(ValidationError::Schedule));
        settings.schedule = Schedule {
            hour: 23,
            minute: 60,
        };
        assert_eq!(validate(&settings, true), Err(ValidationError::Schedule));
        settings.schedule = Schedule {
            hour: 23,
            minute: 59,
        };
        assert!(validate(&settings, true).is_ok());
    }

    #[test]
    fn enabling_without_the_remote_is_refused_but_disabling_is_not() {
        let mut settings = settings();
        settings.enabled = true;
        assert_eq!(
            validate(&settings, false),
            Err(ValidationError::RemoteMissing),
            "a timer with no remote would only fail nightly"
        );
        settings.enabled = false;
        assert!(
            validate(&settings, false).is_ok(),
            "turning it off never needs the remote"
        );
    }

    #[test]
    fn the_user_files_round_trip_and_only_this_user_can_read_them() {
        let (store, root) = store();
        let settings = BackupSettings {
            enabled: false,
            remote: "gdrive".to_string(),
            root: "hearthdeck/saves".to_string(),
            schedule: Schedule { hour: 2, minute: 5 },
        };
        store.write(&settings).expect("a write");

        let env = std::fs::read_to_string(root.join("backup/backup.env")).expect("the env file");
        assert!(env.contains("HEARTHDECK_BACKUP_REMOTE=gdrive"));
        assert!(env.contains("HEARTHDECK_BACKUP_ROOT=hearthdeck/saves"));

        let drop_in = std::fs::read_to_string(
            root.join("systemd/hearthdeck-backup.timer.d/50-schedule.conf"),
        )
        .expect("the drop-in");
        assert!(
            drop_in.contains("OnCalendar=\nOnCalendar=*-*-* 02:05:00"),
            "the reset comes before the value, or systemd keeps the packaged \
             04:30 as well: {drop_in}"
        );

        let mode = std::fs::metadata(root.join("backup/backup.env"))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the file is private");

        assert_eq!(
            store.read(),
            Stored {
                remote: "gdrive".to_string(),
                root: "hearthdeck/saves".to_string(),
                schedule: Schedule { hour: 2, minute: 5 },
            }
        );
        assert!(store.updated_at().is_some(), "the write is dated");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_configuration_reads_as_the_documented_defaults() {
        let (store, root) = store();
        assert_eq!(
            store.read(),
            Stored {
                remote: DEFAULT_REMOTE.to_string(),
                root: DEFAULT_ROOT.to_string(),
                schedule: DEFAULT_SCHEDULE,
            },
            "a fresh install has the documented defaults"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_schedule_is_read_back_from_the_drop_in_the_daemon_wrote() {
        assert_eq!(
            parse_schedule("[Timer]\nOnCalendar=\nOnCalendar=*-*-* 04:30:00\n"),
            Some(Schedule {
                hour: 4,
                minute: 30
            }),
            "the reset line is skipped"
        );
        assert_eq!(
            parse_schedule("[Timer]\nOnCalendar=*-*-* 04:30:00\nOnCalendar=*-*-* 02:05:00\n"),
            Some(Schedule { hour: 2, minute: 5 }),
            "the last entry is the one in force"
        );
        assert_eq!(
            parse_schedule("[Timer]\nOnCalendar=*-*-* 04:30:00\nOnCalendar=\n"),
            None,
            "a trailing reset leaves no schedule"
        );
        assert_eq!(parse_schedule("[Timer]\nOnCalendar=*-*-* 99:30:00\n"), None);
        assert_eq!(parse_schedule("garbage"), None);
        assert_eq!(parse_schedule(""), None);
    }

    #[test]
    fn the_env_file_is_read_by_key_not_by_position() {
        let (remote, root) = parse_env(
            "\nHEARTHDECK_BACKUP_ROOT=\"saves\"\nOTHER=1\nHEARTHDECK_BACKUP_REMOTE='pcloud'\n",
        );
        assert_eq!(remote.as_deref(), Some("pcloud"));
        assert_eq!(root.as_deref(), Some("saves"));
        assert_eq!(parse_env(""), (None, None));
    }

    #[test]
    fn the_next_run_is_the_next_local_occurrence() {
        let schedule = DEFAULT_SCHEDULE;

        let before = Local
            .with_ymd_and_hms(2026, 11, 8, 3, 0, 0)
            .single()
            .expect("a local time");
        let next = next_run(schedule, before).expect("a next run");
        assert_eq!(
            next.with_timezone(&Local).format("%H:%M").to_string(),
            "04:30",
            "the same day, when it is still ahead"
        );

        let after = Local
            .with_ymd_and_hms(2026, 11, 8, 5, 0, 0)
            .single()
            .expect("a local time");
        let next = next_run(schedule, after).expect("a next run");
        assert_eq!(
            next.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            "2026-11-09 04:30",
            "the following day, when it has passed"
        );
        assert!(next > Utc::now() - chrono::Duration::days(2));
    }
}
