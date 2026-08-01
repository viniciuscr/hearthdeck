use std::process::Stdio;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;

use crate::settings::{RommCredentials, SettingsRepository};

const LOG_LINE_LIMIT: usize = 30;
const LOG_MESSAGE_LIMIT: usize = 600;

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
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub path_cover_small: Option<String>,
    #[serde(default)]
    pub merged_screenshots: Vec<String>,
    #[serde(default)]
    pub path_manual: Option<String>,
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

#[derive(Serialize)]
pub struct LogEntry {
    pub timestamp: Option<String>,
    pub service: String,
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
    platform_id: i64,
    limit: u32,
    offset: u32,
) -> std::result::Result<RommGamePage, RommQueryError> {
    let credentials = settings
        .romm_credentials()
        .await
        .map_err(RommQueryError::Failed)?
        .ok_or(RommQueryError::NotConfigured)?;
    let response = reqwest::Client::new()
        .get(format!("{}/api/roms", credentials.base_url))
        .query(&[
            ("platform_ids", platform_id.to_string()),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
            ("with_char_index", "false".to_owned()),
            ("with_filter_values", "false".to_owned()),
            ("with_rom_id_index", "false".to_owned()),
            ("order_by", "name".to_owned()),
            ("order_dir", "asc".to_owned()),
        ])
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
            return RommDiagnostic {
                configured: false,
                status: "degraded",
                base_url: None,
                console_count: None,
                checked_at,
                error: Some(format!("Could not read RomM settings: {error}")),
            };
        }
    };
    let Some(configured) = configured else {
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
        Ok(platforms) => RommDiagnostic {
            configured: true,
            status: "ready",
            base_url: Some(configured.base_url),
            console_count: Some(platforms.len()),
            checked_at,
            error: None,
        },
        Err(error) => RommDiagnostic {
            configured: true,
            status: "degraded",
            base_url: Some(configured.base_url),
            console_count: None,
            checked_at,
            error: Some(match error {
                RommQueryError::NotConfigured => "RomM is not configured".to_owned(),
                RommQueryError::Failed(error) => error.to_string(),
            }),
        },
    }
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
    let (session, daemon, bridge_socket, bridge) = tokio::join!(
        service_status("session", "hearthdeck.target", false),
        service_status("daemon", "hearthdeck-daemon.service", false),
        service_status("bridge_socket", "hearthdeck-bridge.socket", false),
        service_status("bridge", "hearthdeck-bridge.service", true),
    );
    vec![session, daemon, bridge_socket, bridge]
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
    let output = Command::new("journalctl")
        .args([
            "--user",
            "--no-pager",
            "--output=json",
            "--reverse",
            "--lines=120",
            "--unit=hearthdeck-daemon.service",
            "--unit=hearthdeck-bridge.service",
        ])
        .stdin(Stdio::null())
        .output()
        .await;
    let Ok(output) = output else {
        return LogTail {
            available: false,
            error: Some("Could not start journalctl for Hearthdeck services.".to_owned()),
            entries: Vec::new(),
        };
    };
    if !output.status.success() {
        return LogTail {
            available: false,
            error: Some("The user journal is unavailable for Hearthdeck services.".to_owned()),
            entries: Vec::new(),
        };
    }
    let entries = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_journal_entry)
        .filter(|entry| entry.message != "request completed")
        .take(LOG_LINE_LIMIT)
        .collect();
    LogTail {
        available: true,
        error: None,
        entries,
    }
}

fn parse_journal_entry(line: &str) -> Option<LogEntry> {
    let record = serde_json::from_str::<Value>(line).ok()?;
    let raw_message = record.get("MESSAGE")?.as_str()?;
    let message = display_message(raw_message);
    let timestamp = record
        .get("__REALTIME_TIMESTAMP")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(DateTime::from_timestamp_micros)
        .map(|timestamp| timestamp.to_rfc3339());
    let service = record
        .get("_SYSTEMD_UNIT")
        .and_then(Value::as_str)
        .map(service_label)
        .unwrap_or_else(|| "Service".to_owned());
    let level = record
        .get("PRIORITY")
        .and_then(Value::as_str)
        .map(priority_label)
        .unwrap_or_else(|| "info".to_owned());
    Some(LogEntry {
        timestamp,
        service,
        level,
        message,
    })
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

fn service_label(unit: &str) -> String {
    match unit {
        "hearthdeck-daemon.service" => "Daemon".to_owned(),
        "hearthdeck-bridge.service" => "Bridge".to_owned(),
        _ => "Service".to_owned(),
    }
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
    fn renders_structured_journal_messages_for_the_diagnostics_view() {
        let entry = super::parse_journal_entry(
            r#"{"MESSAGE":"{\"level\":\"INFO\",\"message\":\"discovery completed\",\"source_id\":\"heroic\",\"record_count\":2}","PRIORITY":"6","_SYSTEMD_UNIT":"hearthdeck-daemon.service","__REALTIME_TIMESTAMP":"1760000000000000"}"#,
        )
        .unwrap();

        assert_eq!(entry.service, "Daemon");
        assert_eq!(entry.level, "info");
        assert_eq!(
            entry.message,
            "discovery completed (source_id=heroic, record_count=2)"
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
}
