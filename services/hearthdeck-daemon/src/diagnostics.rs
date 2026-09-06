use std::{
    collections::VecDeque,
    process::Stdio,
    sync::{Mutex, OnceLock},
};

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::settings::{RommCredentials, SettingsRepository};

const LOG_LINE_LIMIT: usize = 200;
const LOG_MESSAGE_LIMIT: usize = 600;
const ROMM_LOG_LIMIT: usize = 40;
const MAX_ROM_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024 * 1024;

#[derive(Debug)]
pub enum RommQueryError {
    NotConfigured,
    Failed(anyhow::Error),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RommPlatform {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub rom_count: u64,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub fs_slug: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RommGamePage {
    pub items: Vec<RommGame>,
    pub total: u64,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RommGame {
    pub id: i64,
    pub platform_id: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub fs_name_no_tags: String,
    /// Filename actually stored on RomM's disk, including tags/extension.
    /// Used to build the ROM content download URL for a RetroArch launch;
    /// distinct from `fs_name_no_tags`, which is only for display.
    #[serde(default)]
    pub fs_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub path_cover_small: Option<String>,
    #[serde(default)]
    pub path_cover_large: Option<String>,
    #[serde(default)]
    pub url_cover: Option<String>,
    #[serde(default)]
    pub merged_screenshots: Vec<String>,
    #[serde(default)]
    pub path_manual: Option<String>,
    #[serde(default)]
    pub has_manual: bool,
    #[serde(default)]
    pub metadatum: RommGameMetadata,
    #[serde(default)]
    pub regions: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct RommGameMetadata {
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub player_count: String,
    #[serde(default)]
    pub first_release_date: Option<i64>,
}

pub struct RommAsset {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Serialize)]
pub struct DiagnosticsSnapshot {
    pub generated_at: String,
    pub services: Vec<ServiceStatus>,
    pub romm: RommDiagnostic,
    pub logs: LogTail,
}

#[derive(Serialize)]
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

#[derive(Serialize)]
pub struct LogTail {
    pub available: bool,
    pub error: Option<String>,
    pub entries: Vec<LogEntry>,
}

/// Stable identifier for where a log line came from. The Flutter client
/// groups the diagnostics log tail into tabs keyed on this value: `daemon`
/// and `bridge` come from their respective systemd journals, `api` is the
/// daemon's own per-request access log (normally noisy, so it's split out
/// of the general `daemon` tab instead of being dropped), and `romm` is
/// synthesized locally from the periodic RomM connectivity check since RomM
/// itself is an external server with no journal we can tail.
#[derive(Serialize)]
pub struct LogEntry {
    pub timestamp: Option<String>,
    pub source: String,
    pub level: String,
    pub message: String,
}

pub async fn snapshot(settings: &SettingsRepository) -> DiagnosticsSnapshot {
    let (services, logs, romm) =
        tokio::join!(service_statuses(), recent_logs(), romm_diagnostic(settings));
    DiagnosticsSnapshot {
        generated_at: Utc::now().to_rfc3339(),
        services,
        romm,
        logs,
    }
}

pub async fn romm_platforms(
    settings: &SettingsRepository,
) -> std::result::Result<Vec<RommPlatform>, RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    query_romm(&credentials).await
}

pub async fn romm_games(
    settings: &SettingsRepository,
    platform_id: Option<i64>,
    search_term: Option<&str>,
    limit: u32,
    offset: u32,
) -> std::result::Result<RommGamePage, RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let mut request = reqwest::Client::new()
        .get(format!("{}/api/roms", credentials.base_url))
        .query(&[
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
            ("with_char_index", "false".to_owned()),
            ("with_filter_values", "false".to_owned()),
            ("with_rom_id_index", "false".to_owned()),
            ("order_by", "name".to_owned()),
            ("order_dir", "asc".to_owned()),
        ]);
    if let Some(platform_id) = platform_id {
        request = request.query(&[("platform_ids", platform_id.to_string())]);
    }
    if let Some(search_term) = search_term {
        request = request.query(&[("search_term", search_term)]);
    }
    let response = request
        .header(reqwest::header::ACCEPT, "application/json")
        .bearer_auth(&credentials.token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(RommQueryError::Failed(anyhow::anyhow!(
            "RomM returned {status}"
        )));
    }
    response
        .json::<RommGamePage>()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))
}

/// Fetches a single ROM by ID, for the RetroArch launch path: `romm_games`
/// only returns the fields needed for browsing, and does not carry the
/// on-disk filename a launch needs to build a download URL.
pub async fn romm_rom(
    settings: &SettingsRepository,
    rom_id: i64,
) -> std::result::Result<RommGame, RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let response = reqwest::Client::new()
        .get(format!("{}/api/roms/{rom_id}", credentials.base_url))
        .header(reqwest::header::ACCEPT, "application/json")
        .bearer_auth(&credentials.token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(RommQueryError::Failed(anyhow::anyhow!("rom not found")));
    }
    if !status.is_success() {
        return Err(RommQueryError::Failed(anyhow::anyhow!(
            "RomM returned {status}"
        )));
    }
    response
        .json::<RommGame>()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))
}

/// Downloads a ROM into a temporary cache file ahead of a RetroArch launch.
/// Multi-file ROMs (discs, `.m3u` sets) are out of scope for now.
pub async fn download_rom_content(
    settings: &SettingsRepository,
    rom_id: i64,
    fs_name: &str,
    destination: &std::path::Path,
) -> std::result::Result<(), RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let mut url = reqwest::Url::parse(&format!(
        "{}/api/roms/{rom_id}/content/",
        credentials.base_url
    ))
    .map_err(|error| RommQueryError::Failed(error.into()))?;
    url.path_segments_mut()
        .map_err(|_| RommQueryError::Failed(anyhow::anyhow!("invalid RomM content URL")))?
        .push(fs_name);
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(&credentials.token)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(RommQueryError::Failed(anyhow::anyhow!(
            "RomM returned {status} downloading rom content"
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ROM_DOWNLOAD_BYTES)
    {
        return Err(RommQueryError::Failed(anyhow::anyhow!(
            "RomM content exceeds the 16 GiB download limit"
        )));
    }

    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    let mut downloaded = 0_u64;
    let chunks = response.bytes_stream();
    tokio::pin!(chunks);
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|error| RommQueryError::Failed(error.into()))?;
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .filter(|length| *length <= MAX_ROM_DOWNLOAD_BYTES)
            .ok_or_else(|| {
                RommQueryError::Failed(anyhow::anyhow!(
                    "RomM content exceeds the 16 GiB download limit"
                ))
            })?;
        file.write_all(&chunk)
            .await
            .map_err(|error| RommQueryError::Failed(error.into()))?;
    }
    file.sync_all()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))
}

pub async fn romm_asset(
    settings: &SettingsRepository,
    path: &str,
) -> std::result::Result<RommAsset, RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let path = normalized_romm_asset_path(path).map_err(RommQueryError::Failed)?;
    let response = reqwest::Client::new()
        .get(format!("{}{}", credentials.base_url, path))
        .bearer_auth(&credentials.token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(RommQueryError::Failed(anyhow::anyhow!(
            "RomM artwork request returned {status}"
        )));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if !content_type.starts_with("image/") {
        return Err(RommQueryError::Failed(anyhow::anyhow!(
            "RomM artwork response was not an image"
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?
        .to_vec();
    Ok(RommAsset {
        content_type,
        bytes,
    })
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
    match romm_platforms(settings).await {
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
                RommQueryError::NotConfigured => "RomM is not configured".to_owned(),
                RommQueryError::Failed(error) => error.to_string(),
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

async fn query_romm(
    credentials: &RommCredentials,
) -> std::result::Result<Vec<RommPlatform>, RommQueryError> {
    let response = reqwest::Client::new()
        .get(format!("{}/api/platforms", credentials.base_url))
        .header(reqwest::header::ACCEPT, "application/json")
        .bearer_auth(&credentials.token)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    let status = response.status();
    if !status.is_success() {
        let error = anyhow::anyhow!("RomM returned {status}");
        // Diagnostics polls this check periodically; the UI carries a degraded
        // state without turning one unavailable RomM server into a log flood.
        tracing::debug!(base_url = %credentials.base_url, %error, "RomM console check failed");
        return Err(RommQueryError::Failed(error));
    }
    let platforms = response
        .json::<Vec<RommPlatform>>()
        .await
        .map_err(|error| RommQueryError::Failed(error.into()))?;
    tracing::debug!(
        base_url = %credentials.base_url,
        console_count = platforms.len(),
        "RomM console check completed"
    );
    Ok(platforms)
}

fn normalized_romm_asset_path(value: &str) -> anyhow::Result<String> {
    let path = value.trim();
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    if !path.starts_with("/assets/romm/resources/")
        || path.contains("..")
        || path.contains('#')
        || path.contains("//")
    {
        anyhow::bail!("invalid RomM artwork path");
    }
    Ok(path)
}

async fn service_statuses() -> Vec<ServiceStatus> {
    let (session, daemon, bridge_socket, bridge, romm_container) = tokio::join!(
        service_status("session", "hearthdeck.target", false),
        service_status("daemon", "hearthdeck-daemon.service", false),
        service_status("bridge_socket", "hearthdeck-bridge.socket", false),
        service_status("bridge", "hearthdeck-bridge.service", true),
        // Optional: the packaged unit is skipped until the user supplies its
        // environment file. This is safe to query unconditionally.
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

/// Restarts the optional RomM systemd unit
/// (`deploy/systemd/romm.service`). The unit name is a fixed
/// constant, never caller-supplied: this is a narrowly scoped action on one
/// specific service, not a generic "restart any unit" capability. A missing
/// unit fails the same way `systemctl` itself reports it, surfaced to the
/// caller rather than silently ignored.
pub async fn restart_romm_service() -> anyhow::Result<()> {
    let output = Command::new("systemctl")
        .args(["--user", "restart", "romm.service"])
        .stdin(Stdio::null())
        .output()
        .await?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if message.is_empty() {
            anyhow::anyhow!("systemd rejected the romm.service restart")
        } else {
            anyhow::anyhow!("systemd rejected the romm.service restart: {message}")
        });
    }
    Ok(())
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
        let deploy_target = include_str!("../../../deploy/systemd/hearthdeck.target");
        let package_target = include_str!("../../../packaging/arch/hearthdeck.target");
        let package = include_str!("../../../packaging/arch/PKGBUILD");
        let justfile = include_str!("../../../justfile");

        assert!(service.contains("ConditionPathExists=%h/.config/hearthdeck/romm.env"));
        assert!(service.contains("EnvironmentFile=%h/.config/hearthdeck/romm.env"));
        assert!(service.contains("podman-compose -f ${ROMM_COMPOSE_FILE} up -d"));
        assert!(service.contains("PartOf=hearthdeck.target"));
        assert!(
            deploy_target
                .contains("Wants=hearthdeck-bridge.socket hearthdeck-daemon.service romm.service")
        );
        assert!(
            package_target
                .contains("Wants=hearthdeck-bridge.socket hearthdeck-daemon.service romm.service")
        );
        assert!(package.contains("deploy/systemd/romm.service"));
        assert!(package.contains("deploy/systemd/romm.env.example"));
        assert!(justfile.contains("cp deploy/systemd/romm.service"));
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

    #[test]
    fn accepts_only_romm_managed_resource_paths() {
        assert_eq!(
            super::normalized_romm_asset_path("/assets/romm/resources/roms/1/cover.webp?ts=1")
                .unwrap(),
            "/assets/romm/resources/roms/1/cover.webp?ts=1"
        );
        assert!(super::normalized_romm_asset_path("/api/roms").is_err());
        assert!(super::normalized_romm_asset_path("/resources/roms/1/cover.webp").is_err());
        assert!(super::normalized_romm_asset_path("https://example.com/cover.webp").is_err());
        assert!(super::normalized_romm_asset_path("/assets/romm/resources/../secret").is_err());
    }

    #[tokio::test]
    async fn restart_romm_service_fails_safely_when_the_unit_is_unavailable() {
        // No systemd user session (or no romm.service unit) is available in
        // the test/CI environment; this only asserts the call surfaces a
        // failure instead of panicking or hanging.
        assert!(super::restart_romm_service().await.is_err());
    }
}
