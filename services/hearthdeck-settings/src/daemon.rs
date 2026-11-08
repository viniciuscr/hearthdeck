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
use serde::Deserialize;
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
}

/// The RomM connection the daemon holds.
#[derive(Clone, Debug)]
pub struct Romm {
    pub base_url: String,
    pub configured: bool,
}

/// A Stremio link the user approves on another device.
///
/// The daemon's answer also carries `qr`, an image URL; nothing here draws it, so
/// it is not modelled - a field nothing reads rots.
#[derive(Clone, Debug, Deserialize)]
pub struct StremioOffer {
    /// The four characters the user enters in the Stremio app or site.
    pub code: String,
    /// The link to open, which already carries the code.
    pub link: String,
    /// How long the daemon will keep polling for this code.
    pub expires_in_seconds: u64,
}

/// Where a pending link stands. No case carries the session key: a linked
/// account is reported as linked, and the credential stays in the daemon.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum StremioLink {
    /// No link is pending: never asked for, or already resolved.
    Idle,
    /// The code ran out before it was approved.
    Expired,
    /// Waiting on the user, who is approving the code on another device.
    Waiting { seconds_left: u64 },
    /// Linked and stored by the daemon.
    Linked { connection: StremioConnection },
}

/// The linked Stremio account, as the daemon reports it. There is no field for
/// the session: the daemon holds that and never sends it.
#[derive(Clone, Debug, Deserialize)]
pub struct StremioConnection {
    pub linked_at: String,
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

/// Signs in to RomM once. The username and password are used for that exchange
/// only - the daemon stores the client token it mints, never the credentials -
/// and nothing here logs or keeps them.
pub fn sign_in_romm(base_url: String, username: String, password: String) -> Result<Romm, Error> {
    Client::connect()?.sign_in_romm(&base_url, &username, &password)
}

/// Forgets the stored RomM connection and the token that came with it.
pub fn clear_romm() -> Result<(), Error> {
    Client::connect()?.delete("/v1/retro/settings")
}

/// Asks for a Stremio link code. The user approves it on a device where they are
/// already signed in; no endpoint here accepts a password.
pub fn start_stremio_link() -> Result<StremioOffer, Error> {
    Client::connect()?.post_empty("/v1/stremio/link")
}

/// One poll of the pending link. The daemon talks to Stremio, so neither the
/// code nor the session crosses this call.
pub fn stremio_link_status() -> Result<StremioLink, Error> {
    Client::connect()?.get("/v1/stremio/link")
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
        };
        client.pair_if_needed()?;
        Ok(client)
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

    fn sign_in_romm(&self, base_url: &str, username: &str, password: &str) -> Result<Romm, Error> {
        let settings: RommSettings = self.post_json(
            "/v1/retro/login",
            &serde_json::json!({
                "base_url": base_url,
                "username": username,
                "password": password,
            }),
        )?;
        Ok(Romm {
            base_url: settings.base_url,
            configured: settings.configured,
        })
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

        Snapshot {
            daemon: true,
            romm,
            stremio_linked_at,
            ready_systems,
            total_systems,
            categorization,
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
    fn signing_in_posts_the_form_once_and_keeps_only_the_connection() {
        let (base, server) = serve(vec![(
            200,
            r#"{"base_url":"http://127.0.0.1:8080","configured":true,"updated_at":"2026-11-07T00:00:00Z"}"#,
        )]);

        let connection = client(base)
            .sign_in_romm("http://127.0.0.1:8080", "someone", "secret")
            .expect("the daemon accepted the sign-in");

        assert_eq!(connection.base_url, "http://127.0.0.1:8080");
        assert!(connection.configured, "the daemon reports it is signed in");

        let seen = server.join().expect("the stub finished");
        assert!(
            seen[0].starts_with("POST /v1/retro/login "),
            "unexpected request: {}",
            seen[0]
        );
        assert!(
            seen[0]
                .to_ascii_lowercase()
                .contains("authorization: bearer test-token"),
            "the sign-in carries the pairing token"
        );
        let body = seen[0].split("\r\n\r\n").nth(1).unwrap_or_default();
        assert!(
            body.contains("\"base_url\":\"http://127.0.0.1:8080\""),
            "the form's address is what is posted: {body}"
        );
        assert!(
            body.contains("\"username\":\"someone\""),
            "the form's username is what is posted: {body}"
        );
        assert!(
            body.contains("\"password\":\"secret\""),
            "the form's password is what is posted: {body}"
        );
    }

    #[test]
    fn a_refused_sign_in_says_the_service_could_not_be_reached() {
        let (base, server) = serve(vec![(502, "")]);

        let error = client(base)
            .sign_in_romm("http://127.0.0.1:8080", "someone", "secret")
            .expect_err("a bad gateway is not a sign-in");

        assert!(
            error.to_string().contains("could not reach"),
            "unexpected error: {error}"
        );
        server.join().expect("the stub finished");
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
            .post_empty::<super::StremioOffer>("/v1/stremio/link")
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
    fn a_poll_reports_the_pending_code_then_the_linked_account() {
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

        match client
            .get::<super::StremioLink>("/v1/stremio/link")
            .expect("the pending link")
        {
            super::StremioLink::Waiting { seconds_left } => assert_eq!(seconds_left, 42),
            other => panic!("unexpected status: {other:?}"),
        }
        match client
            .get::<super::StremioLink>("/v1/stremio/link")
            .expect("the linked account")
        {
            super::StremioLink::Linked { connection } => {
                assert_eq!(connection.linked_at, "2026-11-07T00:00:00Z");
            }
            other => panic!("unexpected status: {other:?}"),
        }
        server.join().expect("the stub finished");
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
