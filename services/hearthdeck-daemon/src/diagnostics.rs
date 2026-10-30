use std::{
    collections::VecDeque,
    process::Stdio,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use tokio::process::Command;

use crate::romm;
use crate::settings::SettingsRepository;

const LOG_LINE_LIMIT: usize = 200;
const LOG_MESSAGE_LIMIT: usize = 600;
const ROMM_LOG_LIMIT: usize = 40;

#[derive(Serialize)]
pub struct DiagnosticsSnapshot {
    pub generated_at: String,
    pub services: Vec<ServiceStatus>,
    pub romm: RommDiagnostic,
    pub logs: LogTail,
}

#[derive(Clone, Serialize)]
pub struct ServiceStatus {
    pub id: &'static str,
    pub unit: &'static str,
    pub state: String,
    pub detail: String,
}

#[derive(Serialize)]
pub struct RommDiagnostic {
    pub configured: bool,
    pub status: &'static str,
    pub base_url: Option<String>,
    pub console_count: Option<usize>,
    pub checked_at: String,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct LogTail {
    pub available: bool,
    pub error: Option<String>,
    pub entries: Vec<LogEntry>,
}

/// Stable identifier for where a log line came from. The COSMIC frontend
/// groups the diagnostics log tail into tabs keyed on this value: `daemon`
/// and `bridge` come from their respective systemd journals, `api` is the
/// daemon's own per-request access log (normally noisy, so it's split out
/// of the general `daemon` tab instead of being dropped), and `romm` is
/// synthesized locally from the periodic RomM connectivity check since RomM
/// itself is an external server with no journal we can tail.
#[derive(Clone, Serialize)]
pub struct LogEntry {
    pub timestamp: Option<String>,
    pub source: String,
    pub level: String,
    pub message: String,
}

/// How long the subprocess-derived half of a snapshot is reused.
///
/// `service_statuses` forks five `systemctl` processes and `recent_logs` one
/// `journalctl`, and the frontend polls diagnostics while its screen is open. A
/// few seconds of staleness is invisible on a status screen and keeps that poll
/// from being a process-spawn storm.
const SNAPSHOT_CACHE_TTL: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct SubprocessSnapshot {
    at: Instant,
    services: Vec<ServiceStatus>,
    logs: LogTail,
}

/// The service statuses and log tail, reused for [`SNAPSHOT_CACHE_TTL`].
///
/// The RomM half of the snapshot is deliberately *not* cached: it is a live
/// connectivity check, and its whole point is to answer whether RomM responds now.
async fn subprocess_snapshot() -> (Vec<ServiceStatus>, LogTail) {
    static CACHE: OnceLock<Mutex<Option<SubprocessSnapshot>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    // Clone out rather than hold the guard across the forks below.
    let cached = cache
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    if let Some(entry) = cached
        && entry.at.elapsed() < SNAPSHOT_CACHE_TTL
    {
        return (entry.services, entry.logs);
    }

    let (services, logs) = tokio::join!(service_statuses(), recent_logs());
    let fresh = SubprocessSnapshot {
        at: Instant::now(),
        services: services.clone(),
        logs: logs.clone(),
    };
    *cache.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(fresh);
    (services, logs)
}

pub async fn snapshot(settings: &SettingsRepository) -> DiagnosticsSnapshot {
    let ((services, logs), romm) = tokio::join!(subprocess_snapshot(), romm_diagnostic(settings));
    DiagnosticsSnapshot {
        generated_at: Utc::now().to_rfc3339(),
        services,
        romm,
        logs,
    }
}

async fn romm_diagnostic(settings: &SettingsRepository) -> RommDiagnostic {
    let checked_at = Utc::now().to_rfc3339();
    let configured = match settings.romm().await {
        Ok(configured) => configured,
        Err(error) => {
            let message = format!("Could not read RomM settings: {error}");
            push_romm_log("error", message.clone());
            return RommDiagnostic {
                configured: false,
                status: "degraded",
                base_url: None,
                console_count: None,
                checked_at,
                error: Some(message),
            };
        }
    };
    let Some(configured) = configured else {
        push_romm_log("info", "RomM is not configured.".to_owned());
        return RommDiagnostic {
            configured: false,
            status: "not_configured",
            base_url: None,
            console_count: None,
            checked_at,
            error: None,
        };
    };
    match romm::platforms(settings).await {
        Ok(platforms) => {
            push_romm_log(
                "info",
                format!(
                    "Connected to {} ({} consoles available)",
                    configured.base_url,
                    platforms.len()
                ),
            );
            RommDiagnostic {
                configured: true,
                status: "ready",
                base_url: Some(configured.base_url),
                console_count: Some(platforms.len()),
                checked_at,
                error: None,
            }
        }
        Err(error) => {
            let message = match error {
                romm::RommQueryError::NotConfigured => "RomM is not configured".to_owned(),
                romm::RommQueryError::Failed(error) => error.to_string(),
            };
            push_romm_log(
                "error",
                format!("Could not reach {}: {message}", configured.base_url),
            );
            RommDiagnostic {
                configured: true,
                status: "degraded",
                base_url: Some(configured.base_url),
                console_count: None,
                checked_at,
                error: Some(message),
            }
        }
    }
}

fn romm_log_buffer() -> &'static Mutex<VecDeque<LogEntry>> {
    static BUFFER: OnceLock<Mutex<VecDeque<LogEntry>>> = OnceLock::new();
    BUFFER.get_or_init(|| Mutex::new(VecDeque::with_capacity(ROMM_LOG_LIMIT)))
}

/// Records an event from the periodic RomM connectivity check so the
/// diagnostics log tail has real content for the "RomM" tab, even though
/// RomM runs as an external server with no local journal to tail. Repeated
/// identical messages (e.g. "still connected" on every 5s poll) collapse
/// into the existing entry instead of flooding the tab with duplicates.
fn push_romm_log(level: &str, message: String) {
    let Ok(mut buffer) = romm_log_buffer().lock() else {
        return;
    };
    if buffer.back().is_some_and(|entry| entry.message == message) {
        return;
    }
    if buffer.len() >= ROMM_LOG_LIMIT {
        buffer.pop_front();
    }
    buffer.push_back(LogEntry {
        timestamp: Some(Utc::now().to_rfc3339()),
        source: "romm".to_owned(),
        level: level.to_owned(),
        message,
    });
}

fn recent_romm_logs() -> Vec<LogEntry> {
    let Ok(buffer) = romm_log_buffer().lock() else {
        return Vec::new();
    };
    buffer
        .iter()
        .rev()
        .map(|entry| LogEntry {
            timestamp: entry.timestamp.clone(),
            source: entry.source.clone(),
            level: entry.level.clone(),
            message: entry.message.clone(),
        })
        .collect()
}

async fn service_statuses() -> Vec<ServiceStatus> {
    let (session, daemon, bridge_socket, bridge, romm_container) = tokio::join!(
        service_status("session", "hearthdeck.target", false),
        service_status("daemon", "hearthdeck-daemon.service", false),
        service_status("bridge_socket", "hearthdeck-bridge.socket", false),
        service_status("bridge", "hearthdeck-bridge.service", true),
        // Optional: the packaged unit is skipped when its compose file is
        // absent. This is safe to query unconditionally.
        service_status("romm_container", "romm.service", false),
    );
    vec![session, daemon, bridge_socket, bridge, romm_container]
}

async fn service_status(id: &'static str, unit: &'static str, on_demand: bool) -> ServiceStatus {
    let output = Command::new("systemctl")
        .args([
            "--user",
            "show",
            unit,
            "--property=ActiveState",
            "--property=SubState",
            "--value",
        ])
        .stdin(Stdio::null())
        .output()
        .await;
    let Ok(output) = output else {
        return ServiceStatus {
            id,
            unit,
            state: "unavailable".to_owned(),
            detail: "Could not query the user service manager.".to_owned(),
        };
    };
    if !output.status.success() {
        return ServiceStatus {
            id,
            unit,
            state: "unavailable".to_owned(),
            detail: "The user service manager did not return this unit.".to_owned(),
        };
    }
    let output = String::from_utf8_lossy(&output.stdout);
    let states = output
        .lines()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let active_state = states.first().copied().unwrap_or("unknown");
    let sub_state = states.get(1).copied().unwrap_or("unknown");
    ServiceStatus {
        id,
        unit,
        state: active_state.to_owned(),
        detail: if on_demand && active_state == "inactive" {
            "On demand; starts when the daemon needs a host request.".to_owned()
        } else {
            format!("{active_state} ({sub_state})")
        },
    }
}

async fn recent_logs() -> LogTail {
    let romm_logs = recent_romm_logs();
    let output = Command::new("journalctl")
        .args([
            "--user",
            "--no-pager",
            "--output=json",
            "--reverse",
            "--lines=400",
            "--unit=hearthdeck-daemon.service",
            "--unit=hearthdeck-bridge.service",
            // Optional: the unit may be skipped and have no journal entries.
            "--unit=romm.service",
        ])
        .stdin(Stdio::null())
        .output()
        .await;
    let Ok(output) = output else {
        // The journal itself is unavailable, but RomM connectivity events are
        // synthesized locally and don't depend on it, so they still show up
        // in their own tab.
        return LogTail {
            available: false,
            error: Some("Could not start journalctl for Hearthdeck services.".to_owned()),
            entries: romm_logs,
        };
    };
    if !output.status.success() {
        return LogTail {
            available: false,
            error: Some("The user journal is unavailable for Hearthdeck services.".to_owned()),
            entries: romm_logs,
        };
    }
    let mut entries: Vec<LogEntry> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_journal_entry)
        .take(LOG_LINE_LIMIT)
        .collect();
    entries.extend(romm_logs);
    LogTail {
        available: true,
        error: None,
        entries,
    }
}

fn parse_journal_entry(line: &str) -> Option<LogEntry> {
    let record = serde_json::from_str::<Value>(line).ok()?;
    let raw_message = record.get("MESSAGE")?.as_str()?;
    let unit = record
        .get("_SYSTEMD_UNIT")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source = log_source(unit, raw_message);
    // display_message's redaction is tuned for Hearthdeck's own structured
    // JSON log lines, where a stray local path or bearer token in an error
    // string would be an accidental leak. romm.service's lines are plain
    // Podman Compose output: paths, image references, and container
    // names are the entire point of reading them, not secrets, and that
    // process never has access to Hearthdeck's own tokens. Redacting them
    // the same way would turn the one tab meant to show a real
    // WorkingDirectory/image-pull error into a wall of "[path]".
    let message = if source == "romm" {
        truncate_plain(raw_message)
    } else {
        display_message(raw_message)
    };
    let timestamp = record
        .get("__REALTIME_TIMESTAMP")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(DateTime::from_timestamp_micros)
        .map(|timestamp| timestamp.to_rfc3339());
    let level = record
        .get("PRIORITY")
        .and_then(Value::as_str)
        .map(priority_label)
        .unwrap_or_else(|| "info".to_owned());
    Some(LogEntry {
        timestamp,
        source,
        level,
        message,
    })
}

fn truncate_plain(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() > LOG_MESSAGE_LIMIT {
        let mut truncated = trimmed.chars().take(LOG_MESSAGE_LIMIT).collect::<String>();
        truncated.push_str("...");
        truncated
    } else {
        trimmed.to_owned()
    }
}

fn display_message(raw: &str) -> String {
    let Ok(Value::Object(event)) = serde_json::from_str::<Value>(raw) else {
        return truncate_and_redact(raw);
    };
    let message = event
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("structured service event");
    let details = [
        "source_id",
        "provider_id",
        "record_count",
        "console_count",
        "installed_game_count",
        "status_code",
        "item_id",
        "base_url",
        "error",
    ]
    .into_iter()
    .filter_map(|key| event.get(key).map(|value| (key, value)))
    .map(|(key, value)| format!("{key}={}", value_to_text(value)))
    .collect::<Vec<_>>();
    let rendered = if details.is_empty() {
        message.to_owned()
    } else {
        format!("{message} ({})", details.join(", "))
    };
    truncate_and_redact(&rendered)
}

fn value_to_text(value: &Value) -> String {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| value.to_string())
}

/// Assigns a stable source id per journal line. The daemon's own per-request
/// access log (`"request completed"`, previously dropped entirely as noise)
/// is split into its own `api` source instead of being merged with general
/// `daemon` events, so the two can be viewed as separate log tabs. Lines
/// from the optional `romm.service` unit (deploy/systemd/romm.service,
/// Podman Compose's own start/stop output) join the same `romm` tab the
/// synthesized RomM connectivity-check messages already use (`push_romm_log`),
/// rather than getting a separate tab, since both are "what's going on with
/// RomM" from the user's point of view.
fn log_source(unit: &str, raw_message: &str) -> String {
    match unit {
        "hearthdeck-bridge.service" => "bridge",
        "romm.service" => "romm",
        _ if structured_message(raw_message).as_deref() == Some("request completed") => "api",
        _ => "daemon",
    }
    .to_owned()
}

fn structured_message(raw: &str) -> Option<String> {
    let Value::Object(event) = serde_json::from_str::<Value>(raw).ok()? else {
        return None;
    };
    event
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn priority_label(priority: &str) -> String {
    match priority {
        "0" | "1" | "2" | "3" => "error".to_owned(),
        "4" => "warning".to_owned(),
        "5" | "6" => "info".to_owned(),
        _ => "debug".to_owned(),
    }
}

fn truncate_and_redact(value: &str) -> String {
    let mut redacted = String::new();
    let mut redact_next = false;
    for word in value.split_inclusive(char::is_whitespace) {
        let trimmed = word.trim();
        let is_token =
            trimmed.starts_with("rmm_") || trimmed.starts_with("hearthdeck_") || redact_next;
        let is_path = trimmed.contains('/');
        redact_next = trimmed.eq_ignore_ascii_case("bearer");
        if is_token || is_path {
            redacted.push_str(if is_path { "[path]" } else { "[redacted]" });
            if word.ends_with(char::is_whitespace) {
                redacted.push(' ');
            }
        } else {
            redacted.push_str(word);
        }
    }
    if redacted.chars().count() > LOG_MESSAGE_LIMIT {
        let mut truncated = redacted.chars().take(LOG_MESSAGE_LIMIT).collect::<String>();
        truncated.push_str("...");
        truncated
    } else {
        redacted
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn romm_compose_service_is_optional_and_session_managed() {
        let service = include_str!("../../../deploy/systemd/romm.service");
        let packaged_service = include_str!("../../../packaging/arch/romm.service");
        let romm_path = include_str!("../../../deploy/systemd/romm.path");
        let log_service = include_str!("../../../deploy/systemd/hearthdeck-log.service");
        let deploy_target = include_str!("../../../deploy/systemd/hearthdeck.target");
        let package_target = include_str!("../../../packaging/arch/hearthdeck.target");
        let session = include_str!("../../../packaging/arch/hearthdeck-session");
        let acceptance = include_str!("../../../scripts/linux-acceptance");
        let package = include_str!("../../../packaging/arch/PKGBUILD");
        let install = include_str!("../../../packaging/arch/hearthdeck.install");
        let justfile = include_str!("../../../justfile");

        assert!(
            service
                .contains("Environment=ROMM_COMPOSE_FILE=/mnt/external/romM/podman-compose.yaml")
        );
        assert!(service.contains("EnvironmentFile=-%h/.config/hearthdeck/romm.env"));
        // Neither copy may carry an ExecCondition: a condition that is not met is
        // a silent skip, which cannot tell "no RomM here" from "the mount is
        // late" - the failure the driver was written to make visible
        // (docs/units-and-logs.md Rule 1). The driver logs every outcome instead.
        assert!(
            !service
                .lines()
                .any(|line| line.trim_start().starts_with("ExecCondition=")),
            "romm.service must not silently skip on a condition (docs/units-and-logs.md Rule 1)"
        );
        assert!(
            !packaged_service
                .lines()
                .any(|line| line.trim_start().starts_with("ExecCondition=")),
            "the packaged romm.service must not silently skip on a condition"
        );
        // The checkout copy runs the driver from ~/.local/bin, the package from
        // /usr/lib, exactly like the daemon and bridge units.
        assert!(service.contains("ExecStart=%h/.local/bin/hearthdeck-romm up"));
        assert!(service.contains("ExecStop=%h/.local/bin/hearthdeck-romm down"));
        assert!(packaged_service.contains("ExecStart=/usr/lib/hearthdeck/hearthdeck-romm up"));
        assert!(packaged_service.contains("ExecStop=/usr/lib/hearthdeck/hearthdeck-romm down"));
        assert!(service.contains("SyslogIdentifier=romm"));
        assert!(service.contains("PartOf=hearthdeck.target"));
        // The watcher is armed from the service too, so it works with an older
        // target that only knows `Wants=romm.service`.
        assert!(service.contains("Wants=romm.path"));
        // A compose stack that takes minutes to come up must not be killed at the
        // 90s/300s defaults; the observed real start exceeded 300s.
        assert!(service.contains("TimeoutStartSec=900"));
        // The compose file is often on an external mount that is not ready when
        // the session starts, and a user unit cannot order itself after a system
        // mount. The watcher is what starts the stack once the mount shows up.
        assert!(romm_path.contains("PartOf=hearthdeck.target"));
        assert!(romm_path.contains("PathExists=/mnt/external/romM/podman-compose.yaml"));
        assert!(romm_path.contains("Unit=romm.service"));
        // No restart policy: `podman-compose up` recreates containers to converge,
        // so retrying a failed up tears down a stack that was already running.
        // Asserted on whole lines so the explanatory comment above is allowed to
        // name the directive.
        assert!(
            !service.lines().any(|line| line.starts_with("Restart=")),
            "romm.service must not auto-restart: retrying a failed `podman-compose up` recreates containers"
        );
        // romm.service's Documentation= must resolve to a file the package ships.
        assert!(service.contains("Documentation=file:///usr/share/doc/hearthdeck/ROMM.md"));
        assert!(package.contains("docs/retroarch-integration.md"));
        assert!(package.contains("usr/share/doc/hearthdeck/ROMM.md"));
        // podman's default networking opens /dev/net/tun, mirroring the uinput
        // module the controller broker already needs.
        assert!(install.contains("modprobe tun"));
        assert!(log_service.contains("StandardOutput=append:%h/hearthdeck.log"));
        assert!(log_service.contains("ExecStartPre=/usr/bin/truncate --size=0 %h/hearthdeck.log"));
        assert!(log_service.contains("ExecStartPre=/usr/bin/chmod 600 %h/hearthdeck.log"));
        // The session's own lines are not a unit's, so they are matched by
        // identifier; everything with a unit is matched by unit, which is the
        // only way systemd's own messages about them reach this file.
        assert!(log_service.contains("--identifier=hearthdeck-session"));
        assert!(log_service.contains("-u hearthdeck-bridge.service"));
        assert!(log_service.contains("-u hearthdeck-daemon.service"));
        assert!(log_service.contains("-u hearthdeck-input.service"));
        assert!(log_service.contains("-u hearthdeck-overlay.service"));
        assert!(log_service.contains("-u romm.service"));
        assert!(log_service.contains("-u romm.path"));
        assert!(session.contains("systemd-cat -t hearthdeck-session"));
        assert!(
            deploy_target.contains(
                "Wants=hearthdeck-log.service hearthdeck-bridge.socket hearthdeck-daemon.service hearthdeck-input.service romm.service romm.path"
            )
        );
        assert!(
            package_target.contains(
                "Wants=hearthdeck-log.service hearthdeck-bridge.socket hearthdeck-daemon.service hearthdeck-input.service romm.service romm.path"
            )
        );
        assert!(package.contains("deploy/systemd/hearthdeck-log.service"));
        assert!(package.contains("packaging/arch/romm.service"));
        assert!(package.contains("deploy/systemd/romm.path"));
        // An install or an upgrade has to apply itself. A running manager keeps
        // the unit definitions it already loaded - including a target from
        // before RomM existed - so the transaction is what reloads it and puts
        // the services, RomM included, back where a session start would have
        // them. Leaving the user a command to run is the failure this asserts
        // against.
        assert!(install.contains("refresh_user_services"));
        assert!(install.contains("user_systemctl"));
        assert!(install.contains("XDG_RUNTIME_DIR"));
        assert!(install.contains("daemon-reload"));
        assert!(install.contains("try-restart"));
        assert!(install.contains("hearthdeck-daemon.service hearthdeck-bridge.service"));
        assert!(install.contains("start romm.path"));
        assert!(install.contains("start romm.service"));
        assert!(
            !install.contains("systemctl --user start hearthdeck.target"),
            "an install must not hand the user a command to activate the target"
        );
        assert!(package.contains("deploy/systemd/romm.env.example"));
        assert!(package.contains("scripts/linux-acceptance"));
        assert!(acceptance.contains("systemctl --user restart hearthdeck.target"));
        assert!(acceptance.contains("http://127.0.0.1:38400/v1/health"));
        assert!(
            acceptance.contains("podman-compose -f \"$romm_compose_file\" $romm_compose_args ps")
        );
        // The acceptance check must reconstruct the same podman-compose argv the
        // unit runs, overrides included, or it verifies a stack nobody started.
        assert!(acceptance.contains("ROMM_COMPOSE_ARGS"));
        assert!(acceptance.contains("stat -c '%a' \"$log_file\""));
        assert!(justfile.contains("./scripts/linux-acceptance --require-romm"));
        assert!(justfile.contains("cp deploy/systemd/hearthdeck-log.service"));
        assert!(justfile.contains("cp deploy/systemd/romm.service"));
        assert!(justfile.contains("cp deploy/systemd/romm.path"));
    }

    /// The gamepad's only path into a text field is cosmic-osk, and what makes
    /// it appear is spread across the three files a package ships: the session
    /// seeds the one show/hide behavior that fits this session (and turns off
    /// the one that collides with it) and starts the unit, the unit asks for a
    /// layer the fullscreen frontend does not hide, carries the compositor's
    /// start race and stops with the session, and the package installs the unit
    /// and declares the optional binary. Drop any one and a TV has no keyboard
    /// anyone can type on, with nothing in the logs to say why.
    #[test]
    fn session_seeds_and_starts_the_optional_on_screen_keyboard() {
        let session = include_str!("../../../packaging/arch/hearthdeck-session");
        let service = include_str!("../../../packaging/arch/hearthdeck-osk.service");
        let package = include_str!("../../../packaging/arch/PKGBUILD");

        // Focus-driven show/hide is the only trigger that works in this
        // session, and it defaults off upstream: seed ime_activation, but only
        // where the user has not already set a value.
        assert!(session.contains(
            "[ -e \"$osk_state/ime_activation\" ] || printf 'true' > \"$osk_state/ime_activation\""
        ));
        // cosmic-osk's gamepad_shortcut is hardcoded to Start+Select. Select is
        // the library's filter button and Start+Select is the RetroArch menu
        // chord this session pins, so the setting is forced off every session
        // rather than seeded - a stale `true` is the conflict itself.
        assert!(session.contains("printf 'false' > \"$osk_state/gamepad_shortcut\""));
        assert!(session.contains("systemctl --user start hearthdeck-osk.service"));

        // `overlay` is not optional decoration: cosmic-comp renders no Top
        // layer at all while a fullscreen window has focus, and the Hearthdeck
        // frontend is exactly that. Without it the OSK is created, logs
        // "get_layer_surface", and is visible nowhere.
        assert!(service.contains("ExecStart=/usr/bin/cosmic-osk overlay"));
        assert!(service.contains("Restart=always"));
        // Nothing else owns this unit, so without the target tie it outlives
        // the compositor it draws on - the overlay's contract, for the same
        // reason.
        assert!(service.contains("PartOf=hearthdeck.target"));
        // The binary is an optdepend: a missing one must skip the unit, never
        // fail it into a restart loop.
        assert!(service.contains("ConditionPathExists=/usr/bin/cosmic-osk"));
        // cosmic-comp imports WAYLAND_DISPLAY only once it is ready, and its
        // GPU EBUSY retry loop can outlast systemd's default start rate limit,
        // which would leave the unit permanently failed and the session
        // without its keyboard.
        assert!(service.contains("StartLimitIntervalSec=0"));

        assert!(package.contains("install -Dm644 packaging/arch/hearthdeck-osk.service"));
        assert!(package.contains("'cosmic-osk: "));
    }

    /// A packaged unit is the deploy unit it is derived from, with one difference:
    /// it runs the installed binary under `/usr/lib` instead of the source
    /// checkout's `~/.local/bin`. Restating a unit in two files is how the packaged
    /// bridge silently lost the `EnvironmentFile` that tells it where RomM keeps
    /// its roms, so the two copies are pinned to each other here: nothing but that
    /// path may drift, which is also what lets a package upgrade be trusted to
    /// install the unit the source tree is testing.
    #[test]
    fn packaged_units_differ_from_their_deploy_source_only_in_the_binary_path() {
        let pairs = [
            (
                "hearthdeck.target",
                include_str!("../../../deploy/systemd/hearthdeck.target"),
                include_str!("../../../packaging/arch/hearthdeck.target"),
            ),
            (
                "hearthdeck-bridge.socket",
                include_str!("../../../deploy/systemd/hearthdeck-bridge.socket"),
                include_str!("../../../packaging/arch/hearthdeck-bridge.socket"),
            ),
            (
                "hearthdeck-bridge.service",
                include_str!("../../../deploy/systemd/hearthdeck-bridge.service"),
                include_str!("../../../packaging/arch/hearthdeck-bridge.service"),
            ),
            (
                "hearthdeck-daemon.service",
                include_str!("../../../deploy/systemd/hearthdeck-daemon.service"),
                include_str!("../../../packaging/arch/hearthdeck-daemon.service"),
            ),
            (
                "hearthdeck-input.service",
                include_str!("../../../deploy/systemd/hearthdeck-input.service"),
                include_str!("../../../packaging/arch/hearthdeck-input.service"),
            ),
            (
                "romm.service",
                include_str!("../../../deploy/systemd/romm.service"),
                include_str!("../../../packaging/arch/romm.service"),
            ),
        ];

        let as_packaged = |unit: &str| unit.replace("%h/.local/bin/", "/usr/lib/hearthdeck/");
        for (name, deploy, packaged) in pairs {
            assert_eq!(
                as_packaged(deploy),
                as_packaged(packaged),
                "{name} drifted from its deploy/systemd copy"
            );
        }
    }

    #[test]
    fn the_session_log_collector_follows_every_packaged_user_unit() {
        let package = include_str!("../../../packaging/arch/PKGBUILD");
        let log_service = include_str!("../../../deploy/systemd/hearthdeck-log.service");

        // Not followed on purpose: the target and the socket emit nothing useful
        // of their own, and following the collector from itself is pointless.
        const NOT_COLLECTED: &[&str] = &[
            "hearthdeck.target",
            "hearthdeck-bridge.socket",
            "hearthdeck-log.service",
        ];

        let mut checked = 0;
        for line in package.lines() {
            let Some(target) = line.split("$pkgdir/usr/lib/systemd/user/").nth(1) else {
                continue;
            };
            let unit = target.trim().trim_end_matches('"');
            if NOT_COLLECTED.contains(&unit) {
                continue;
            }
            checked += 1;
            assert!(
                log_service.contains(&format!("-u {unit}")),
                "hearthdeck-log.service must follow {unit} with `-u`; otherwise its output, \
                 and systemd's own messages about it, never reach ~/hearthdeck.log"
            );
        }
        assert!(checked >= 5, "parsed only {checked} packaged user units");
    }

    #[test]
    fn the_session_does_not_block_on_the_service_target() {
        let session = include_str!("../../../packaging/arch/hearthdeck-session");

        // The compositor must not wait for hearthdeck.target. RomM is a target
        // dependency and `podman-compose up` can take many seconds, so a blocking
        // start is both the black screen and the cause of the intermittent
        // `EBUSY on /dev/dri/cardN`: the compositor starts late and loses the GPU
        // to whatever took it in the meantime. Queue the jobs instead.
        assert!(session.contains("systemctl --user start --no-block hearthdeck.target"));
        assert!(
            !session
                .lines()
                .any(|line| line.trim() == "systemctl --user start hearthdeck.target"),
            "the session must not block on the full target before starting the compositor"
        );
    }

    #[test]
    fn the_session_waits_for_readiness_and_retries_the_compositor() {
        let session = include_str!("../../../packaging/arch/hearthdeck-session");

        // Phase 3: bounded readiness for exactly the session-critical units.
        assert!(session.contains("wait_for_unit"));
        assert!(session.contains(
            "for unit in hearthdeck-log.service hearthdeck-daemon.service hearthdeck-bridge.socket"
        ));
        // RomM is never part of the gate: it is optional and unbounded, and
        // waiting on it is the black screen and the GPU race this avoids. The
        // exact list above is the guarantee; only the loop calls `wait_for_unit`.
        assert_eq!(
            session.matches("wait_for_unit").count(),
            2,
            "only the definition and the single readiness loop may use wait_for_unit"
        );
        // Phase 4: a failed compositor start is retried, not fatal, and the retry
        // names who held the GPU.
        assert!(session.contains("max_attempts="));
        assert!(session.contains("drm holders:"));
    }

    #[test]
    fn the_package_ships_the_hearthdeck_boot_splash() {
        let package = include_str!("../../../packaging/arch/PKGBUILD");
        let theme = include_str!("../../../packaging/arch/plymouth/hearthdeck.plymouth");
        let script = include_str!("../../../packaging/arch/plymouth/hearthdeck.script");

        // The theme is optional (only used where plymouth is installed) but the
        // package must still carry it, and the descriptor must point at the
        // script the package installs beside it.
        assert!(package.contains("usr/share/plymouth/themes/hearthdeck"));
        assert!(package.contains("packaging/arch/plymouth/hearthdeck.plymouth"));
        assert!(package.contains("packaging/arch/plymouth/hearthdeck.script"));
        assert!(package.contains("packaging/arch/plymouth/logo.png"));
        assert!(
            theme.contains("ScriptFile=/usr/share/plymouth/themes/hearthdeck/hearthdeck.script")
        );
        assert!(script.contains("Image(\"logo.png\")"));
    }

    #[test]
    fn the_session_starts_the_user_session_bus() {
        let session = include_str!("../../../packaging/arch/hearthdeck-session");

        // cosmic-session is not run, so nothing else starts the session bus. Without
        // it every cosmic_config watcher fails in a loop, and a client that exits
        // over it takes the compositor (and so the whole login) with it.
        assert!(session.contains("systemctl --user start dbus.socket"));
        assert!(session.contains(
            "import-environment XDG_CURRENT_DESKTOP XDG_SESSION_DESKTOP XDG_SESSION_TYPE DBUS_SESSION_BUS_ADDRESS"
        ));
    }

    #[test]
    fn the_romm_driver_checks_its_prerequisites_and_logs_every_outcome() {
        let driver = include_str!("../../../packaging/arch/hearthdeck-romm");
        let package = include_str!("../../../packaging/arch/PKGBUILD");
        let daemon = include_str!("../../../packaging/arch/hearthdeck-daemon.service");
        let log_service = include_str!("../../../deploy/systemd/hearthdeck-log.service");

        // The checks worth doing before a compose up, each one naming what is
        // missing: podman installed, podman-compose installed, the podman service
        // actually answering, and the compose file present.
        for check in [
            "command -v podman",
            "command -v podman-compose",
            "podman info",
            "no compose file at",
        ] {
            assert!(driver.contains(check), "the driver lost its {check} check");
        }
        // `up` runs the real thing and reports both outcomes instead of staying
        // silent.
        assert!(driver.contains("podman-compose -f \"$compose\" ${ROMM_COMPOSE_ARGS:-} up -d"));
        assert!(driver.contains("stack is up"));
        assert!(driver.contains("podman-compose up failed"));

        // A separate discovery unit used to write the compose path into romm.env
        // before every session. It is gone: the path comes from romm.env (which
        // both romm.service and the daemon read) or the packaged default, and
        // nothing may still reference the removed unit.
        assert!(!package.contains("hearthdeck-romm-discover"));
        assert!(!daemon.contains("hearthdeck-romm-discover"));
        assert!(!log_service.contains("hearthdeck-romm-discover"));
    }

    #[test]
    fn the_romm_watcher_watches_the_default_compose_file() {
        let service = include_str!("../../../deploy/systemd/romm.service");
        let watcher = include_str!("../../../deploy/systemd/romm.path");

        // The watcher cannot read an EnvironmentFile, so it can only ever be right
        // about the default. Pin the two paths together so they cannot drift.
        assert!(
            service
                .contains("Environment=ROMM_COMPOSE_FILE=/mnt/external/romM/podman-compose.yaml")
        );
        assert!(watcher.contains("PathExists=/mnt/external/romM/podman-compose.yaml"));
    }

    /// The RomM unit is a thin wrapper now: everything that decides *what* to run
    /// lives in `packaging/arch/hearthdeck-romm`, because a systemd condition
    /// cannot tell "this host has no RomM" (safe to skip on) apart from "the
    /// compose file is not visible yet" (must not be a skip). Pin the unit, the
    /// driver, and the package that installs the driver together, so a fix to one
    /// cannot miss the others.
    #[test]
    fn romm_service_delegates_to_the_packaged_driver() {
        let service = include_str!("../../../deploy/systemd/romm.service");
        let packaged_service = include_str!("../../../packaging/arch/romm.service");
        let driver = include_str!("../../../packaging/arch/hearthdeck-romm");
        let package = include_str!("../../../packaging/arch/PKGBUILD");

        // The script the package installs at /usr/lib is the one the packaged unit
        // names, and the checkout unit names the ~/.local/bin copy the justfile
        // installs. The destination, not just the source name, is asserted.
        assert!(package.contains("$pkgdir/usr/lib/hearthdeck/hearthdeck-romm\""));
        for verb in ["up", "down"] {
            assert!(
                packaged_service.contains(&format!("/usr/lib/hearthdeck/hearthdeck-romm {verb}")),
                "the packaged romm.service must invoke the driver's `{verb}`"
            );
            assert!(
                service.contains(&format!("%h/.local/bin/hearthdeck-romm {verb}")),
                "the checkout romm.service must invoke the driver's `{verb}`"
            );
            assert!(
                driver.contains(&format!("\n{verb})")),
                "the driver must implement `{verb}`"
            );
        }

        // The argv lives in the driver, overrides included: an unquoted
        // `${ROMM_COMPOSE_ARGS:-}` is what splits it into words and drops it
        // entirely when unset, which a bare `${ROMM_COMPOSE_ARGS}` would not.
        assert!(driver.contains("podman-compose -f \"$compose\" ${ROMM_COMPOSE_ARGS:-} up -d"));
        assert!(driver.contains("podman-compose -f \"$compose\" ${ROMM_COMPOSE_ARGS:-} down"));

        // `up` must not wait. hearthdeck.target Wants= this oneshot, and a target
        // waits for its oneshot dependencies to exit, so a wait here is a wait
        // before the compositor starts. A late compose file is romm.path's job.
        assert!(
            !driver.contains("ROMM_WAIT_SECONDS") && !driver.contains("sleep"),
            "the driver must not block session start; a late compose file is the watcher's job"
        );
        assert!(
            driver.contains("romm.path retries when it appears"),
            "`up` must name who starts a not-yet-present stack"
        );

        // podman-compose arrives through optdepends, so its absence is the one
        // failure worth a plain-English line rather than a bare exit 127.
        assert!(driver.contains("podman-compose is not installed"));
    }

    /// The RomM runbook (`docs/retroarch-integration.md`, shipped as
    /// `/usr/share/doc/hearthdeck/ROMM.md`) is what `systemctl status romm.service`
    /// points a stuck operator at. It went stale once — it still described the old
    /// `ExecCondition` silent-skip design — which is why the same bug was
    /// re-derived and re-fixed. Keep the doc naming the design it documents so
    /// drift fails the build instead of someone's evening.
    #[test]
    fn the_romm_runbook_stays_true_to_the_units_it_documents() {
        let doc = include_str!("../../../docs/retroarch-integration.md");
        let service = include_str!("../../../deploy/systemd/romm.service");
        let package = include_str!("../../../packaging/arch/PKGBUILD");

        // The doc the unit points at is the doc the package ships.
        assert!(service.contains("Documentation=file:///usr/share/doc/hearthdeck/ROMM.md"));
        assert!(package.contains("docs/retroarch-integration.md"));

        // Every moving part of the chain has to be in there...
        for artifact in [
            "hearthdeck-romm up",
            "podman info",
            "romm.path",
            "TimeoutStartSec=900",
        ] {
            assert!(
                doc.contains(artifact),
                "ROMM.md no longer names {artifact}; the runbook has drifted from the units"
            );
        }

        // ...and the mechanism that was replaced is described as replaced, so a
        // reader does not take the old silent-skip design as current.
        assert!(doc.contains("Why there is no `ExecCondition`"));
        assert!(doc.contains("Why `up` never waits"));
    }

    #[test]
    fn renders_structured_journal_messages_for_the_diagnostics_view() {
        let entry = super::parse_journal_entry(
            r#"{"MESSAGE":"{\"level\":\"INFO\",\"message\":\"discovery completed\",\"source_id\":\"heroic\",\"record_count\":2}","PRIORITY":"6","_SYSTEMD_UNIT":"hearthdeck-daemon.service","__REALTIME_TIMESTAMP":"1760000000000000"}"#,
        )
        .unwrap();

        assert_eq!(entry.source, "daemon");
        assert_eq!(entry.level, "info");
        assert_eq!(
            entry.message,
            "discovery completed (source_id=heroic, record_count=2)"
        );
    }

    #[test]
    fn splits_request_access_lines_into_their_own_api_source() {
        let entry = super::parse_journal_entry(
            r#"{"MESSAGE":"{\"level\":\"INFO\",\"message\":\"request completed\",\"status_code\":200}","PRIORITY":"6","_SYSTEMD_UNIT":"hearthdeck-daemon.service","__REALTIME_TIMESTAMP":"1760000000000000"}"#,
        )
        .unwrap();

        assert_eq!(entry.source, "api");
    }

    #[test]
    fn labels_bridge_journal_lines_with_the_bridge_source() {
        let entry = super::parse_journal_entry(
            r#"{"MESSAGE":"{\"level\":\"INFO\",\"message\":\"bridge ready\"}","PRIORITY":"6","_SYSTEMD_UNIT":"hearthdeck-bridge.service","__REALTIME_TIMESTAMP":"1760000000000000"}"#,
        )
        .unwrap();

        assert_eq!(entry.source, "bridge");
    }

    #[test]
    fn labels_romm_service_journal_lines_with_the_romm_source_unredacted() {
        // Plain Podman Compose stdout, not Hearthdeck's own structured JSON
        // log shape, and full of legitimate paths/image refs that a real
        // configuration failure needs to stay readable.
        let entry = super::parse_journal_entry(
            r#"{"MESSAGE":"Error: WorkingDirectory '/home/alex/mnt/external/romM' not found","PRIORITY":"3","_SYSTEMD_UNIT":"romm.service","__REALTIME_TIMESTAMP":"1760000000000000"}"#,
        )
        .unwrap();

        assert_eq!(entry.source, "romm");
        assert_eq!(
            entry.message,
            "Error: WorkingDirectory '/home/alex/mnt/external/romM' not found"
        );
    }

    #[test]
    fn redacts_credentials_before_returning_log_messages() {
        let message =
            super::truncate_and_redact("token rmm_secret_value bearer hearthdeck_client_secret");

        assert_eq!(message, "token [redacted] bearer [redacted]");
    }

    #[test]
    fn redacts_local_paths_before_returning_log_messages() {
        let message =
            super::truncate_and_redact("could not read /home/alex/.config/heroic/installed.json");

        assert_eq!(message, "could not read [path]");
    }
}
