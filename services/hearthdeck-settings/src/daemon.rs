//! What this window asks the daemon: read-only, and nothing else.
//!
//! Settings that can be *changed* are still changed in the library app, so
//! everything here is a GET. Two things are copied from `hearthdeck_client`
//! rather than shared, because they are the contract a second process has to
//! meet to be trusted by the same daemon:
//!
//! - **Pairing.** A loopback daemon mints a code on its admin port and completes
//!   it on the API port. That is why this window needs no secret handed to it
//!   through the environment: it pairs the way the library app does.
//! - **Endpoint shapes.** `/v1/health`, `/v1/retro/settings`,
//!   `/v1/stremio/settings`, `/v1/retro/readiness`.

use std::time::Duration;

use serde::Deserialize;
use url::Url;

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:38400";
const ADMIN_PORT: u16 = 38401;
const CLIENT_NAME: &str = "hearthdeck-settings";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Anything that can go wrong on the way to the daemon. Boxed because the window
/// only ever logs it: a failed read degrades a row, it never blocks the UI.
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
}

/// The RomM connection the daemon holds.
#[derive(Clone, Debug)]
pub struct Romm {
    pub base_url: String,
    pub configured: bool,
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

    fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, Error> {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        let mut request = self.http.get(url);
        if let Some(token) = self.token.as_deref() {
            request = request.bearer_auth(token);
        }
        Ok(request.send()?.error_for_status()?.json()?)
    }

    fn snapshot(&self) -> Snapshot {
        if let Err(error) = self.get::<serde_json::Value>("/v1/health") {
            tracing::warn!(%error, "the daemon did not answer /v1/health");
            return Snapshot::default();
        }

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
        let (ready_systems, total_systems) =
            match self.get::<Vec<RetroSystem>>("/v1/retro/readiness") {
                Ok(systems) => (
                    systems
                        .iter()
                        .filter(|system| system.core.installed)
                        .count(),
                    systems.len(),
                ),
                Err(error) => {
                    tracing::warn!(%error, "retro readiness could not be read");
                    (0, 0)
                }
            };

        Snapshot {
            daemon: true,
            romm,
            stremio_linked_at,
            ready_systems,
            total_systems,
        }
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
struct RommSettings {
    base_url: String,
    configured: bool,
}

#[derive(Deserialize)]
struct StremioConnection {
    linked_at: String,
}

#[derive(Deserialize)]
struct RetroSystem {
    core: RetroCore,
}

#[derive(Deserialize)]
struct RetroCore {
    installed: bool,
}
