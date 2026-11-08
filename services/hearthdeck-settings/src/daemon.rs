//! What this window asks the daemon: a read on open, and the writes it owns so
//! far - the categorization opt-in, the RomM connection and the Stremio link.
//!
//! Two things are copied from `hearthdeck_client` rather than shared, because
//! they are the contract a second process has to meet to be trusted by the same
//! daemon:
//!
//! - **Pairing.** A loopback daemon mints a code on its admin port and completes
//!   it on the API port. That is why this window needs no secret handed to it
//!   through the environment: it pairs the way the library app does.
//! - **Endpoint shapes.** `/v1/health`, `/v1/retro/settings`,
//!   `/v1/stremio/settings`, `/v1/retro/readiness`, `/v1/categorization` and its
//!   `enable`/`disable`/`scan` writes.
//!
//! Anything that reaches the admin listener stays out of this file on purpose:
//! TMDB's credential is set there, once, from the host - see
//! `docs/settings-window.md`.

use std::time::Duration;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use url::Url;

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:38400";
const ADMIN_PORT: u16 = 38401;
const CLIENT_NAME: &str = "hearthdeck-settings";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Anything that can go wrong on the way to the daemon. Boxed because the window
/// only ever reports it: a failed read degrades a row, a refused write reverts
/// the control, and neither blocks the UI.
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Everything the window shows that does not come from this machine's disk.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Whether the daemon answered at all.
    pub daemon: bool,
    pub romm: Option<Romm>,
    /// When the Stremio account was linked, if one is.
    pub stremio_linked_at: Option<String>,
    pub ready_systems: usize,
    pub total_systems: usize,
    /// `None` when the daemon did not answer the categorization read: the row
    /// then shows nothing rather than claiming the feature is off.
    pub categorization: Option<Categorization>,
    /// Whether the daemon holds a TMDB key. `None` when the loopback admin
    /// listener could not be reached - which is where the contract serves it.
    pub tmdb_configured: Option<bool>,
    /// The off-site backup configuration. `None` when this build has no backup
    /// units, or the route did not answer: the page then says so rather than
    /// offering a switch that cannot work.
    pub backup: Option<BackupView>,
    /// The last run, read in the same pass as the settings.
    pub backup_status: Option<BackupStatus>,
}

/// The RomM connection the daemon holds.
#[derive(Clone, Debug)]
pub struct Romm {
    pub base_url: String,
    pub configured: bool,
}

/// The local hour and minute the nightly backup runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSchedule {
    pub hour: u8,
    pub minute: u8,
}

/// What the window writes: the destination, the schedule, and whether the timer
/// runs. Sending all four together means one request expresses the whole
/// decision, and a refused write leaves the old configuration alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSettings {
    pub enabled: bool,
    pub remote: String,
    pub root: String,
    pub schedule: BackupSchedule,
}

/// The settings as the daemon answers them, plus the fact that makes them
/// actionable: whether rclone knows the remote.
#[derive(Clone, Debug, Deserialize)]
pub struct BackupView {
    pub enabled: bool,
    pub remote: String,
    pub root: String,
    pub schedule: BackupSchedule,
    pub remote_available: bool,
    pub updated_at: Option<String>,
}

impl BackupView {
    /// The writable half, for prefilling the fields.
    pub fn settings(&self) -> BackupSettings {
        BackupSettings {
            enabled: self.enabled,
            remote: self.remote.clone(),
            root: self.root.clone(),
            schedule: self.schedule,
        }
    }
}

/// The last run and the next one.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct BackupStatus {
    pub timer_active: bool,
    pub service_active: bool,
    pub next_run: Option<String>,
    pub last_result: Option<String>,
    pub last_finished_at: Option<String>,
    pub last_error: Option<String>,
}

/// A device approval: the code to show, and where to approve it. RomM and
/// Stremio answer with the same shape, which is why the window has one flow for
/// both.
///
/// The daemon's Stremio answer also carries `qr`, an image URL; nothing here
/// draws it, so it is not modelled - a field nothing reads rots.
#[derive(Clone, Debug, Deserialize)]
pub struct DeviceOffer {
    /// The characters the user enters, or the link carries.
    pub code: String,
    /// The link to open, which already carries the code.
    pub link: String,
    /// How long the daemon will keep polling for this code.
    pub expires_in_seconds: u64,
}

/// Where a pending approval stands, for either service. No case carries the
/// credential: a linked account is reported as linked, and the session stays in
/// the daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkStatus {
    /// No link is pending: never asked for, or already resolved.
    Idle,
    /// The code ran out before it was approved.
    Expired,
    /// The other side refused the approval.
    Denied,
    /// Waiting on the user, who is approving the code on another device.
    Waiting { seconds_left: u64 },
    /// Linked and stored by the daemon. `account` is what the row shows: when
    /// Stremio was linked, or which RomM server was.
    Linked { account: String },
}

/// The categorization opt-in: the one setting this window writes.
#[derive(Clone, Debug, Default)]
pub struct Categorization {
    /// The user's opt-in. While it is false the daemon downloads nothing and
    /// scans nothing.
    pub enabled: bool,
    /// Whether this daemon can scan at all. `None` when the health read that
    /// carried it was not part of the same pass.
    pub capable: Option<bool>,
}

/// Reads everything the window draws, in one pass. A daemon that cannot be
/// reached yields the default snapshot, which the view draws as "unavailable".
pub fn read_snapshot() -> Snapshot {
    match Client::connect() {
        Ok(client) => client.snapshot(),
        Err(error) => {
            tracing::warn!(%error, "the daemon could not be reached");
            Snapshot::default()
        }
    }
}

/// Turns the opt-in on or off, then reports what the daemon says afterwards - so
/// the switch settles on the daemon's answer, not on the click.
pub fn set_categorization(enabled: bool) -> Result<Categorization, Error> {
    Client::connect()?.set_categorization(enabled)
}

/// Asks for one scan. The daemon answers as soon as it accepts the job.
pub fn scan_categorization() -> Result<(), Error> {
    Client::connect()?.post("/v1/categorization/scan")
}

/// Asks the daemon for a RomM device code. The user approves it in RomM's web UI
/// on another device, so no password ever crosses this window.
pub fn start_romm_link(base_url: String) -> Result<DeviceOffer, Error> {
    Client::connect()?.start_romm_link(&base_url)
}

/// One poll of the pending RomM approval.
pub fn romm_link_status() -> Result<LinkStatus, Error> {
    Client::connect()?.romm_link_status()
}

/// Forgets the stored RomM connection and the token that came with it.
pub fn clear_romm() -> Result<(), Error> {
    Client::connect()?.delete("/v1/retro/settings")
}

/// Asks for a Stremio link code. The user approves it on a device where they are
/// already signed in; no endpoint here accepts a password.
pub fn start_stremio_link() -> Result<DeviceOffer, Error> {
    Client::connect()?.post_empty("/v1/stremio/link")
}

/// One poll of the pending link. The daemon talks to Stremio, so neither the
/// code nor the session crosses this call.
pub fn stremio_link_status() -> Result<LinkStatus, Error> {
    Client::connect()?.stremio_link_status()
}

/// Stores a TMDB key. The contract serves that endpoint on the loopback admin
/// listener rather than the API, so this is the one write that does not carry
/// the pairing token.
/// Writes the backup destination and schedule, and moves the timer. A refusal -
/// the remote is not in rclone, or a field is not valid - comes back as an error
/// the page shows verbatim.
pub fn save_backup(settings: &BackupSettings) -> Result<BackupView, Error> {
    Client::connect()?.save_backup(settings)
}

pub fn run_backup() -> Result<(), Error> {
    Client::connect()?.run_backup()
}

pub fn save_tmdb_key(api_key: String) -> Result<(), Error> {
    Client::connect()?.save_tmdb_key(&api_key)
}

/// Unlinks: the session is gone, and the rows that source owned are removed with
/// it.
pub fn unlink_stremio() -> Result<(), Error> {
    Client::connect()?.delete("/v1/stremio/settings")
}

/// Asks the daemon to install the libretro cores its readiness report is
/// missing. The names come from the daemon's own platform-to-core table; the
/// window never chooses a package.
pub fn install_cores() -> Result<CoreInstall, Error> {
    Client::connect()?.post_empty("/v1/retro/cores/install")
}

/// Asks the daemon to place the missing BIOS files and source-only cores from
/// the source the host configured. Hearthdeck ships no firmware.
pub fn sync_bios() -> Result<BiosSync, Error> {
    Client::connect()?.post_empty("/v1/retro/readiness/sync")
}

/// Removes the dashboard rails the last scan published. Works whether or not
/// categorization is on.
pub fn restore_rails() -> Result<(), Error> {
    Client::connect()?.delete("/v1/categorization/collections")
}

/// What the daemon's core install did, package by package.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CoreInstall {
    pub installed: Vec<String>,
    pub failed: Vec<CoreFailure>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CoreFailure {
    pub package: String,
    pub reason: String,
}

/// What the daemon's BIOS sync placed, and what it could not.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct BiosSync {
    pub fetched: Vec<String>,
    pub failed: Vec<BiosFailure>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BiosFailure {
    pub filename: String,
    pub reason: String,
}

/// How many systems the daemon can start right now, out of how many it knows.
pub fn readiness() -> Result<(usize, usize), Error> {
    Client::connect()?.readiness()
}

struct Client {
    http: reqwest::blocking::Client,
    base_url: String,
    token: Option<String>,
    /// Where the loopback admin listener is. A field rather than the constant so
    /// a test can point it at its own stub.
    admin_port: u16,
}

impl Client {
    fn connect() -> Result<Self, Error> {
        let base_url = std::env::var("HEARTHDECK_BACKEND_URL")
            .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let token = std::env::var("HEARTHDECK_PAIRING_TOKEN")
            .ok()
            .filter(|token| !token.is_empty());
        Self::connect_to(base_url, token)
    }

    /// The base URL and token named explicitly. Tests use it to point at a local
    /// stub, and to skip the pairing handshake they do not need.
    fn connect_to(base_url: String, token: Option<String>) -> Result<Self, Error> {
        let http = reqwest::blocking::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        let mut client = Self {
            http,
            base_url,
            token,
            admin_port: ADMIN_PORT,
        };
        client.pair_if_needed()?;
        Ok(client)
    }

    /// The admin listener, as a test's stub serves it.
    #[cfg(test)]
    fn with_admin_port(mut self, admin_port: u16) -> Self {
        self.admin_port = admin_port;
        self
    }

    /// The loopback handshake, the same two calls `hearthdeck_client` makes.
    fn pair_if_needed(&mut self) -> Result<(), Error> {
        if self.token.is_some() {
            return Ok(());
        }
        let mut api = Url::parse(&self.base_url)?;
        if !matches!(
            api.host_str(),
            Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
        ) {
            return Err("automatic pairing needs a loopback daemon".into());
        }
        let mut admin = api.clone();
        admin
            .set_port(Some(ADMIN_PORT))
            .map_err(|()| "the admin port cannot be set")?;
        admin.set_path("/v1/pairing");
        api.set_path("/v1/pairing/complete");

        let pairing: PairingCode = self
            .http
            .post(admin)
            .send()?
            .error_for_status()?
            .json()
            .map_err(|error| format!("the pairing code could not be read: {error}"))?;
        let complete: PairingComplete = self
            .http
            .post(api)
            .json(&serde_json::json!({
                "code": pairing.code,
                "client_name": CLIENT_NAME,
            }))
            .send()?
            .error_for_status()?
            .json()
            .map_err(|error| format!("the pairing token could not be read: {error}"))?;
        self.token = Some(complete.token);
        Ok(())
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url.trim_end_matches('/'))
    }

    /// Sends one already-built request with the pairing token, and turns a
    /// refusal into an error the window can print.
    fn send(
        &self,
        mut request: reqwest::blocking::RequestBuilder,
    ) -> Result<reqwest::blocking::Response, Error> {
        if let Some(token) = self.token.as_deref() {
            request = request.bearer_auth(token);
        }
        let response = request.send()?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        Err(refusal(status).into())
    }

    fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, Error> {
        Ok(self.send(self.http.get(self.url(path)))?.json()?)
    }

    fn post(&self, path: &str) -> Result<(), Error> {
        self.send(self.http.post(self.url(path))).map(|_| ())
    }

    fn post_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T, Error> {
        Ok(self
            .send(self.http.post(self.url(path)).json(body))?
            .json()?)
    }

    fn delete(&self, path: &str) -> Result<(), Error> {
        self.send(self.http.delete(self.url(path))).map(|_| ())
    }

    /// A POST with no body whose answer is the result.
    fn post_empty<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, Error> {
        Ok(self.send(self.http.post(self.url(path)))?.json()?)
    }

    fn save_backup(&self, settings: &BackupSettings) -> Result<BackupView, Error> {
        Ok(self
            .send(
                self.http
                    .put(self.url("/v1/backup/settings"))
                    .json(settings),
            )?
            .json()?)
    }

    fn run_backup(&self) -> Result<(), Error> {
        self.post("/v1/backup/run")
    }

    fn start_romm_link(&self, base_url: &str) -> Result<DeviceOffer, Error> {
        self.post_json(
            "/v1/retro/link",
            &serde_json::json!({ "base_url": base_url }),
        )
    }

    fn romm_link_status(&self) -> Result<LinkStatus, Error> {
        let status: RommLinkResponse = self.get("/v1/retro/link")?;
        Ok(match status {
            RommLinkResponse::Idle => LinkStatus::Idle,
            RommLinkResponse::Expired => LinkStatus::Expired,
            RommLinkResponse::Denied => LinkStatus::Denied,
            RommLinkResponse::Waiting { seconds_left } => LinkStatus::Waiting { seconds_left },
            RommLinkResponse::Linked { settings } => LinkStatus::Linked {
                account: settings.base_url,
            },
        })
    }

    fn stremio_link_status(&self) -> Result<LinkStatus, Error> {
        let status: StremioLinkResponse = self.get("/v1/stremio/link")?;
        Ok(match status {
            StremioLinkResponse::Idle => LinkStatus::Idle,
            StremioLinkResponse::Expired => LinkStatus::Expired,
            StremioLinkResponse::Waiting { seconds_left } => LinkStatus::Waiting { seconds_left },
            StremioLinkResponse::Linked { connection } => LinkStatus::Linked {
                account: connection.linked_at,
            },
        })
    }

    /// The loopback admin listener, where `/v1/pairing` and `/v1/tmdb/settings`
    /// live. It is a different port from the API and takes no bearer token,
    /// which is why the TMDB write cannot reuse `send`.
    fn admin_url(&self, path: &str) -> Result<String, Error> {
        let mut url = Url::parse(&self.base_url)?;
        if !matches!(
            url.host_str(),
            Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
        ) {
            return Err("the admin listener is loopback only".into());
        }
        url.set_port(Some(self.admin_port))
            .map_err(|()| "the admin port cannot be set")?;
        url.set_path(path);
        Ok(url.to_string())
    }

    fn save_tmdb_key(&self, api_key: &str) -> Result<(), Error> {
        let response = self
            .http
            .put(self.admin_url("/v1/tmdb/settings")?)
            .json(&serde_json::json!({ "api_key": api_key }))
            .send()?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        Err(refusal(status).into())
    }

    fn tmdb_configured(&self) -> Result<bool, Error> {
        let settings: Option<TmdbSettings> = self
            .http
            .get(self.admin_url("/v1/tmdb/settings")?)
            .send()?
            .error_for_status()?
            .json()?;
        Ok(settings.is_some_and(|settings| settings.configured))
    }

    fn categorization(&self) -> Result<Categorization, Error> {
        let response: Option<CategorizationResponse> = self.get("/v1/categorization")?;
        let response = response.ok_or("the daemon reported no categorization state")?;
        Ok(Categorization {
            enabled: response.enabled,
            capable: None,
        })
    }

    fn set_categorization(&self, enabled: bool) -> Result<Categorization, Error> {
        self.post(if enabled {
            "/v1/categorization/enable"
        } else {
            "/v1/categorization/disable"
        })?;
        self.categorization()
    }

    fn readiness(&self) -> Result<(usize, usize), Error> {
        let systems: Vec<RetroSystem> = self.get("/v1/retro/readiness")?;
        Ok((
            systems
                .iter()
                .filter(|system| system.core.installed)
                .count(),
            systems.len(),
        ))
    }

    fn snapshot(&self) -> Snapshot {
        let health = match self.get::<Health>("/v1/health") {
            Ok(health) => health,
            Err(error) => {
                tracing::warn!(%error, "the daemon did not answer /v1/health");
                return Snapshot::default();
            }
        };

        let romm = match self.get::<Option<RommSettings>>("/v1/retro/settings") {
            Ok(settings) => settings.map(|settings| Romm {
                base_url: settings.base_url,
                configured: settings.configured,
            }),
            Err(error) => {
                tracing::warn!(%error, "RomM settings could not be read");
                None
            }
        };
        let stremio_linked_at = match self.get::<Option<StremioConnection>>("/v1/stremio/settings")
        {
            Ok(connection) => connection.map(|connection| connection.linked_at),
            Err(error) => {
                tracing::warn!(%error, "the Stremio connection could not be read");
                None
            }
        };
        let (ready_systems, total_systems) = match self.readiness() {
            Ok(counts) => counts,
            Err(error) => {
                tracing::warn!(%error, "retro readiness could not be read");
                (0, 0)
            }
        };
        let categorization = match self.categorization() {
            Ok(mut categorization) => {
                categorization.capable = Some(health.capabilities.categorization);
                Some(categorization)
            }
            Err(error) => {
                tracing::warn!(%error, "categorization could not be read");
                None
            }
        };

        let tmdb_configured = match self.tmdb_configured() {
            Ok(configured) => Some(configured),
            Err(error) => {
                tracing::warn!(%error, "the TMDB credential could not be read");
                None
            }
        };

        let backup = match self.get::<BackupView>("/v1/backup/settings") {
            Ok(backup) => Some(backup),
            Err(error) => {
                tracing::warn!(%error, "the backup settings could not be read");
                None
            }
        };
        let backup_status = match self.get::<BackupStatus>("/v1/backup/status") {
            Ok(status) => Some(status),
            Err(error) => {
                tracing::warn!(%error, "the backup status could not be read");
                None
            }
        };

        Snapshot {
            daemon: true,
            romm,
            stremio_linked_at,
            ready_systems,
            total_systems,
            categorization,
            tmdb_configured,
            backup,
            backup_status,
        }
    }
}

/// What a refused write says. The status is the daemon's own vocabulary, so the
/// three that a user can act on are spelled out and the rest is left as it came.
fn refusal(status: StatusCode) -> String {
    match status {
        StatusCode::CONFLICT => "the daemon says categorization is off".to_string(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            "this window is not paired with the daemon".to_string()
        }
        StatusCode::BAD_REQUEST => "the daemon rejected the address it was given".to_string(),
        StatusCode::BAD_GATEWAY => "the daemon could not reach the service".to_string(),
        StatusCode::SERVICE_UNAVAILABLE => {
            "this deployment cannot do that, or is not set up for it".to_string()
        }
        other => format!("the daemon answered {other}"),
    }
}

#[derive(Deserialize)]
struct PairingCode {
    code: String,
}

#[derive(Deserialize)]
struct PairingComplete {
    token: String,
}

#[derive(Deserialize)]
struct Health {
    capabilities: Capabilities,
}

#[derive(Deserialize)]
struct Capabilities {
    categorization: bool,
}

#[derive(Deserialize)]
struct CategorizationResponse {
    enabled: bool,
}

/// The two link answers differ only in what a successful link reports, so each
/// is mapped onto the one [`LinkStatus`] the window draws.
#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum StremioLinkResponse {
    Idle,
    Expired,
    Waiting { seconds_left: u64 },
    Linked { connection: StremioConnection },
}

#[derive(Deserialize)]
struct StremioConnection {
    linked_at: String,
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum RommLinkResponse {
    Idle,
    Expired,
    Denied,
    Waiting { seconds_left: u64 },
    Linked { settings: RommSettings },
}

#[derive(Deserialize)]
struct TmdbSettings {
    configured: bool,
}

#[derive(Deserialize)]
struct RommSettings {
    base_url: String,
    configured: bool,
}

#[derive(Deserialize)]
struct RetroSystem {
    core: RetroCore,
}

#[derive(Deserialize)]
struct RetroCore {
    installed: bool,
}

#[cfg(test)]
mod tests {
    use super::{Client, StatusCode};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// A stub daemon: answers each connection with the next canned response and
    /// returns every request it saw, so a test can assert what was sent.
    fn serve(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
        let base = format!("http://{}", listener.local_addr().expect("an address"));
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (status, body) in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                seen.push(read_request(&mut stream));
                let response = format!(
                    "HTTP/1.1 {status} Stub\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
            seen
        });
        (base, handle)
    }

    /// One whole request: headers and, when `content-length` says so, the body -
    /// which is how a test checks the JSON a write carries.
    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        while let Ok(read) = stream.read(&mut chunk) {
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
            let Some(end) = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|position| position + 4)
            else {
                continue;
            };
            let head = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
            let declared = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if request.len() >= end + declared {
                break;
            }
        }
        String::from_utf8_lossy(&request).to_string()
    }

    fn client(base: String) -> Client {
        Client::connect_to(base, Some("test-token".to_string())).expect("a stub client")
    }

    /// The stub's port, so a test can point the admin listener at the same
    /// server instead of the real 38401.
    fn admin_port(base: &str) -> u16 {
        base.rsplit(':')
            .next()
            .and_then(|port| port.parse().ok())
            .expect("a stub port")
    }

    #[test]
    fn saving_a_destination_sends_every_field_in_one_request() {
        let (base, server) = serve(vec![(
            200,
            r#"{"enabled":true,"remote":"pcloud","root":"hearthdeck-backups","schedule":{"hour":4,"minute":30},"remote_available":true,"updated_at":"2026-11-08T15:00:00Z"}"#,
        )]);
        let settings = super::BackupSettings {
            enabled: true,
            remote: "pcloud".to_string(),
            root: "hearthdeck-backups".to_string(),
            schedule: super::BackupSchedule {
                hour: 4,
                minute: 30,
            },
        };

        let view = client(base).save_backup(&settings).expect("the saved view");
        assert!(view.remote_available, "the answer is read back");

        let requests = server.join().expect("the stub thread");
        assert!(
            requests[0].starts_with("PUT /v1/backup/settings"),
            "{}",
            requests[0]
        );
        assert!(requests[0].contains("\"enabled\":true"));
        assert!(requests[0].contains("\"remote\":\"pcloud\""));
        assert!(requests[0].contains("\"hour\":4"));
    }

    #[test]
    fn run_now_posts_and_does_not_wait_for_the_run() {
        let (base, server) = serve(vec![(202, "")]);

        client(base).run_backup().expect("accepted");

        let requests = server.join().expect("the stub thread");
        assert!(requests[0].starts_with("POST /v1/backup/run"));
    }

    #[test]
    fn enabling_posts_the_endpoint_and_reports_the_state_after_it() {
        let (base, server) = serve(vec![
            (202, ""),
            (
                200,
                r#"{"enabled":true,"status":{"running":false,"phase":"idle","completed":0,"total":0,"model":{"state":"absent"}},"report":null}"#,
            ),
        ]);

        let state = client(base)
            .set_categorization(true)
            .expect("the write is accepted");

        assert!(state.enabled, "the window reports the daemon's answer");

        let seen = server.join().expect("the stub finished");
        assert!(
            seen[0].starts_with("POST /v1/categorization/enable "),
            "unexpected request: {}",
            seen[0]
        );
        assert!(
            seen[0]
                .to_ascii_lowercase()
                .contains("authorization: bearer test-token"),
            "the write carries the pairing token: {}",
            seen[0]
        );
        assert!(
            seen[1].starts_with("GET /v1/categorization "),
            "the write is followed by a read: {}",
            seen[1]
        );
    }

    #[test]
    fn turning_it_off_posts_the_disable_endpoint() {
        let (base, server) = serve(vec![
            (204, ""),
            (
                200,
                r#"{"enabled":false,"status":{"running":false,"phase":"idle","completed":0,"total":0,"model":{"state":"absent"}},"report":null}"#,
            ),
        ]);

        let state = client(base)
            .set_categorization(false)
            .expect("the write is accepted");

        assert!(!state.enabled);
        let seen = server.join().expect("the stub finished");
        assert!(
            seen[0].starts_with("POST /v1/categorization/disable "),
            "unexpected request: {}",
            seen[0]
        );
    }

    #[test]
    fn a_refused_write_is_an_error_the_window_can_show() {
        let (base, server) = serve(vec![(409, "")]);

        let error = client(base)
            .post("/v1/categorization/scan")
            .expect_err("a conflict is not success");

        assert!(
            error.to_string().contains("categorization is off"),
            "unexpected error: {error}"
        );
        server.join().expect("the stub finished");
    }

    #[test]
    fn a_refused_write_keeps_the_daemons_own_status_when_it_has_no_better_meaning() {
        let (base, server) = serve(vec![(500, "")]);

        let error = client(base)
            .post("/v1/categorization/enable")
            .expect_err("a server error is not success");

        assert!(
            error.to_string().contains("500"),
            "unexpected error: {error}"
        );
        server.join().expect("the stub finished");
    }

    #[test]
    fn the_read_reports_capability_and_opt_in_separately() {
        let (base, server) = serve(vec![
            (
                200,
                r#"{"version":"0.1.0","lan_enabled":false,"transport":"http","providers":[],"capabilities":{"launch":true,"application_sessions":true,"retro_launch":true,"categorization":true}}"#,
            ),
            (200, "null"),
            (200, "null"),
            (200, "[]"),
            (
                200,
                r#"{"enabled":true,"status":{"running":true,"phase":"scanning","completed":3,"total":9,"model":{"state":"absent"}},"report":null}"#,
            ),
        ]);

        let snapshot = client(base).snapshot();

        assert!(snapshot.daemon);
        let categorization = snapshot.categorization.expect("categorization was read");
        assert!(categorization.enabled, "the opt-in is the daemon's");
        assert_eq!(
            categorization.capable,
            Some(true),
            "capability is reported from health, not from the opt-in"
        );
        server.join().expect("the stub finished");
    }

    #[test]
    fn a_daemon_that_does_not_answer_degrades_to_the_default_snapshot() {
        // Nothing is listening: the read must not claim a connection.
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
        let base = format!("http://{}", listener.local_addr().expect("an address"));
        drop(listener);

        let snapshot = Client::connect_to(base, Some("test-token".to_string()))
            .expect("a client")
            .snapshot();

        assert!(!snapshot.daemon);
        assert!(snapshot.categorization.is_none());
    }

    #[test]
    fn the_statuses_a_user_can_act_on_are_spelled_out() {
        assert!(super::refusal(StatusCode::CONFLICT).contains("categorization is off"));
        assert!(super::refusal(StatusCode::UNAUTHORIZED).contains("not paired"));
        assert!(super::refusal(StatusCode::BAD_REQUEST).contains("rejected the address"));
        assert!(super::refusal(StatusCode::BAD_GATEWAY).contains("could not reach"));
        assert!(super::refusal(StatusCode::FORBIDDEN).contains("not paired"));
    }

    #[test]
    fn forgetting_the_connection_deletes_the_settings() {
        let (base, server) = serve(vec![(204, "")]);

        client(base)
            .delete("/v1/retro/settings")
            .expect("the daemon removed the connection");

        let seen = server.join().expect("the stub finished");
        assert!(
            seen[0].starts_with("DELETE /v1/retro/settings "),
            "unexpected request: {}",
            seen[0]
        );
    }

    #[test]
    fn starting_a_stremio_link_asks_for_a_code_and_reads_it() {
        let (base, server) = serve(vec![(
            200,
            r#"{"code":"ABCD","link":"https://link.stremio.com/ABCD","qr":"https://image.example/qr.png","expires_in_seconds":300}"#,
        )]);

        let offer = client(base)
            .post_empty::<super::DeviceOffer>("/v1/stremio/link")
            .expect("an offer");

        assert_eq!(offer.code, "ABCD");
        assert_eq!(offer.link, "https://link.stremio.com/ABCD");
        assert_eq!(offer.expires_in_seconds, 300);
        let seen = server.join().expect("the stub finished");
        assert!(
            seen[0].starts_with("POST /v1/stremio/link "),
            "unexpected request: {}",
            seen[0]
        );
    }

    #[test]
    fn a_romm_link_posts_the_address_and_reads_the_code() {
        let (base, server) = serve(vec![(
            200,
            r#"{"code":"WXYZ","link":"http://127.0.0.1:8080/link","expires_in_seconds":120}"#,
        )]);

        let offer = client(base.clone())
            .start_romm_link("http://127.0.0.1:8080")
            .expect("an offer");

        assert_eq!(offer.code, "WXYZ");
        assert_eq!(offer.expires_in_seconds, 120);
        let seen = server.join().expect("the stub finished");
        assert!(
            seen[0].starts_with("POST /v1/retro/link "),
            "unexpected request: {}",
            seen[0]
        );
        let body = seen[0].split("\r\n\r\n").nth(1).unwrap_or_default();
        assert!(
            body.contains("\"base_url\":\"http://127.0.0.1:8080\""),
            "the address is what the link is asked for: {body}"
        );
    }

    #[test]
    fn a_romm_poll_reports_denied_and_then_linked() {
        let (base, server) = serve(vec![
            (200, r#"{"status":"denied"}"#),
            (
                200,
                r#"{"status":"linked","settings":{"base_url":"http://127.0.0.1:8080","configured":true,"updated_at":"2026-11-07T00:00:00Z"}}"#,
            ),
        ]);
        let client = client(base);

        assert_eq!(
            client.romm_link_status().expect("a status"),
            super::LinkStatus::Denied
        );
        assert_eq!(
            client.romm_link_status().expect("a status"),
            super::LinkStatus::Linked {
                account: "http://127.0.0.1:8080".to_string()
            }
        );
        server.join().expect("the stub finished");
    }

    #[test]
    fn a_stremio_poll_reports_waiting_then_the_linked_account() {
        let (base, server) = serve(vec![
            (
                200,
                r#"{"status":"waiting","code":"ABCD","link":"https://link.stremio.com/ABCD","qr":"https://image.example/qr.png","seconds_left":42}"#,
            ),
            (
                200,
                r#"{"status":"linked","connection":{"linked_at":"2026-11-07T00:00:00Z","updated_at":"2026-11-07T00:00:00Z"}}"#,
            ),
        ]);
        let client = client(base);

        assert_eq!(
            client.stremio_link_status().expect("a status"),
            super::LinkStatus::Waiting { seconds_left: 42 }
        );
        assert_eq!(
            client.stremio_link_status().expect("a status"),
            super::LinkStatus::Linked {
                account: "2026-11-07T00:00:00Z".to_string()
            }
        );
        server.join().expect("the stub finished");
    }

    #[test]
    fn saving_a_tmdb_key_writes_through_the_admin_listener() {
        let (base, server) = serve(vec![(
            200,
            r#"{"configured":true,"updated_at":"2026-11-07T00:00:00Z"}"#,
        )]);
        let port = admin_port(&base);

        client(base)
            .with_admin_port(port)
            .save_tmdb_key("tmdb-key")
            .expect("the key was stored");

        let seen = server.join().expect("the stub finished");
        assert!(
            seen[0].starts_with("PUT /v1/tmdb/settings "),
            "unexpected request: {}",
            seen[0]
        );
        assert!(
            !seen[0]
                .to_ascii_lowercase()
                .contains("authorization: bearer"),
            "the admin listener takes no pairing token: {}",
            seen[0]
        );
        let body = seen[0].split("\r\n\r\n").nth(1).unwrap_or_default();
        assert!(body.contains("\"api_key\":\"tmdb-key\""), "{body}");
    }

    #[test]
    fn the_tmdb_state_is_read_from_the_admin_listener() {
        let (base, server) = serve(vec![(
            200,
            r#"{"configured":true,"updated_at":"2026-11-07T00:00:00Z"}"#,
        )]);
        let port = admin_port(&base);

        let configured = client(base)
            .with_admin_port(port)
            .tmdb_configured()
            .expect("a state");

        assert!(configured, "the daemon reports a stored key");
        let seen = server.join().expect("the stub finished");
        assert!(
            seen[0].starts_with("GET /v1/tmdb/settings "),
            "unexpected request: {}",
            seen[0]
        );
    }

    #[test]
    fn unlinking_deletes_the_stremio_settings() {
        let (base, server) = serve(vec![(204, "")]);

        client(base)
            .delete("/v1/stremio/settings")
            .expect("the daemon unlinked the account");

        let seen = server.join().expect("the stub finished");
        assert!(
            seen[0].starts_with("DELETE /v1/stremio/settings "),
            "unexpected request: {}",
            seen[0]
        );
    }

    #[test]
    fn installing_cores_posts_the_endpoint_and_reads_the_report() {
        let (base, server) = serve(vec![(
            200,
            r#"{"installed":["libretro-snes9x"],"failed":[{"package":"libretro-dolphin","reason":"no such package"}]}"#,
        )]);

        let report = client(base)
            .post_empty::<super::CoreInstall>("/v1/retro/cores/install")
            .expect("a report");

        assert_eq!(report.installed, ["libretro-snes9x"]);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].reason, "no such package");
        let seen = server.join().expect("the stub finished");
        assert!(
            seen[0].starts_with("POST /v1/retro/cores/install "),
            "unexpected request: {}",
            seen[0]
        );
    }

    #[test]
    fn syncing_bios_posts_the_endpoint_and_reads_the_report() {
        let (base, server) = serve(vec![(
            200,
            r#"{"fetched":["bios/SCPH1001.BIN"],"failed":[{"filename":"bios/scph5500.bin","reason":"not found"}],"readiness":[]}"#,
        )]);

        let report = client(base)
            .post_empty::<super::BiosSync>("/v1/retro/readiness/sync")
            .expect("a report");

        assert_eq!(report.fetched, ["bios/SCPH1001.BIN"]);
        assert_eq!(report.failed[0].filename, "bios/scph5500.bin");
        let seen = server.join().expect("the stub finished");
        assert!(
            seen[0].starts_with("POST /v1/retro/readiness/sync "),
            "unexpected request: {}",
            seen[0]
        );
    }

    #[test]
    fn restoring_rails_deletes_the_published_collections() {
        let (base, server) = serve(vec![(204, "")]);

        client(base)
            .delete("/v1/categorization/collections")
            .expect("the daemon removed the rails");

        let seen = server.join().expect("the stub finished");
        assert!(
            seen[0].starts_with("DELETE /v1/categorization/collections "),
            "unexpected request: {}",
            seen[0]
        );
    }
}
