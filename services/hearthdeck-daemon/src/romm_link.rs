//! Connecting to RomM without anyone pasting a token.
//!
//! Two ways in, both ending in a long-lived RomM client token (`rmm_…`) that is
//! the only thing stored:
//!
//! - **Device approval**: RomM's `/api/auth/device` flow. The user approves a
//!   short code on a phone where they are signed in to RomM; no password crosses
//!   this box at all.
//! - **Password**: a one-shot `/api/token` password grant whose short-lived
//!   session token is used once, to create the client token, and then dropped.
//!   The username and password are never stored and never logged.
//!
//! Both need RomM 4.8 or newer, where client tokens were introduced.

use std::net::{IpAddr, UdpSocket};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::sync::Mutex;

/// What the daemon reads and writes on RomM: the catalog, cover art, favourites
/// and backlog (collection membership), and per-ROM user properties.
const SCOPES: [&str; 7] = [
    "platforms.read",
    "roms.read",
    "roms.user.read",
    "roms.user.write",
    "collections.read",
    "collections.write",
    "assets.read",
];

/// The name the token and device carry in RomM's own settings screens, so the
/// user can see and revoke it there.
const CLIENT_NAME: &str = "Hearthdeck";

const TIMEOUT: Duration = Duration::from_secs(10);

/// A code the user approves on another device.
#[derive(Clone, Debug, Serialize)]
pub struct DeviceOffer {
    /// The short code RomM shows on its approval page.
    pub code: String,
    /// The approval page with the code filled in, addressed so a phone on the
    /// same network can open it.
    pub link: String,
    pub expires_in_seconds: u64,
}

/// What a poll of the pending approval found.
pub enum DevicePoll {
    Idle,
    Expired,
    Denied,
    Waiting(DeviceOffer, u64),
    Linked { base_url: String, token: String },
}

/// The one approval a user may be waiting on, held in memory like the Stremio
/// link: it is approved within minutes or not at all.
#[derive(Clone, Default)]
pub struct DeviceLinkState {
    pending: Arc<Mutex<Option<PendingDevice>>>,
}

#[derive(Clone)]
struct PendingDevice {
    base_url: String,
    device_code: String,
    offer: DeviceOffer,
    started: Instant,
    window: Duration,
    interval: Duration,
    last_poll: Option<Instant>,
}

impl DeviceLinkState {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn cancel(&self) {
        self.pending.lock().await.take();
    }

    /// Asks RomM for a fresh code and makes it the pending approval.
    pub async fn start(&self, base_url: &str) -> Result<DeviceOffer> {
        let init: DeviceInit = read_json(
            http()
                .post(format!("{base_url}/api/auth/device/init"))
                .json(&serde_json::json!({
                    "client_device_identifier": "hearthdeck",
                    "name": CLIENT_NAME,
                    "client": "hearthdeck",
                    "platform": "linux",
                    "client_version": env!("CARGO_PKG_VERSION"),
                    "requested_scopes": SCOPES,
                }))
                .timeout(TIMEOUT)
                .send()
                .await
                .with_context(|| format!("could not reach RomM at {base_url}"))?,
        )
        .await?;
        let offer = DeviceOffer {
            link: phone_link(base_url, &init.verification_path_complete, lan_ip()),
            code: init.user_code,
            expires_in_seconds: init.expires_in,
        };
        *self.pending.lock().await = Some(PendingDevice {
            base_url: base_url.to_owned(),
            device_code: init.device_code,
            offer: offer.clone(),
            started: Instant::now(),
            window: Duration::from_secs(init.expires_in),
            interval: Duration::from_secs(init.interval.max(1)),
            last_poll: None,
        });
        Ok(offer)
    }

    /// Polls the pending approval once.
    ///
    /// RomM answers `slow_down` and rate-limits a client that polls faster than
    /// the interval it gave, so a poll inside that interval answers locally.
    pub async fn poll(&self) -> Result<DevicePoll> {
        let Some(pending) = self.pending.lock().await.clone() else {
            return Ok(DevicePoll::Idle);
        };
        let elapsed = pending.started.elapsed();
        if elapsed >= pending.window {
            self.cancel().await;
            return Ok(DevicePoll::Expired);
        }
        let waiting = DevicePoll::Waiting(
            pending.offer.clone(),
            pending.window.saturating_sub(elapsed).as_secs(),
        );
        if pending
            .last_poll
            .is_some_and(|at| at.elapsed() < pending.interval)
        {
            return Ok(waiting);
        }
        if let Some(current) = self.pending.lock().await.as_mut() {
            current.last_poll = Some(Instant::now());
        }

        let response = http()
            .post(format!("{}/api/auth/device/token", pending.base_url))
            .json(&serde_json::json!({ "device_code": pending.device_code }))
            .timeout(TIMEOUT)
            .send()
            .await
            .with_context(|| format!("could not reach RomM at {}", pending.base_url))?;
        let status = response.status();
        let body = response.json().await.unwrap_or(serde_json::Value::Null);
        let answer = interpret_device_token(status, &body)?;
        if !matches!(answer, DeviceAnswer::Pending) {
            self.cancel().await;
        }
        Ok(match answer {
            DeviceAnswer::Pending => waiting,
            DeviceAnswer::Denied => DevicePoll::Denied,
            DeviceAnswer::Expired => DevicePoll::Expired,
            DeviceAnswer::Approved(token) => DevicePoll::Linked {
                base_url: pending.base_url,
                token,
            },
        })
    }
}

/// Signs in once with a password and returns a new client token.
///
/// The session token from the password grant lives only inside this call. Its
/// one scope, `me.write`, is what creating a client token requires.
pub async fn mint_token(base_url: &str, username: &str, password: &str) -> Result<String> {
    let session: SessionToken = read_json(
        http()
            .post(format!("{base_url}/api/token"))
            .form(&[
                ("grant_type", "password"),
                ("username", username),
                ("password", password),
                ("scope", "me.write"),
            ])
            .timeout(TIMEOUT)
            .send()
            .await
            .with_context(|| format!("could not reach RomM at {base_url}"))?,
    )
    .await?;
    let created: CreatedToken = read_json(
        http()
            .post(format!("{base_url}/api/client-tokens"))
            .bearer_auth(&session.access_token)
            .json(&serde_json::json!({
                "name": CLIENT_NAME,
                "scopes": SCOPES,
                "expires_in": "never",
            }))
            .timeout(TIMEOUT)
            .send()
            .await
            .with_context(|| format!("could not reach RomM at {base_url}"))?,
    )
    .await?;
    Ok(created.raw_token)
}

#[derive(Deserialize)]
struct SessionToken {
    access_token: String,
}

#[derive(Deserialize)]
struct CreatedToken {
    raw_token: String,
}

#[derive(Deserialize)]
struct DeviceInit {
    device_code: String,
    user_code: String,
    verification_path_complete: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, PartialEq, Eq)]
enum DeviceAnswer {
    Pending,
    Denied,
    Expired,
    Approved(String),
}

/// Reads one `/api/auth/device/token` answer. RomM reports every not-yet state
/// as a 400 whose `detail` names it, the way RFC 8628 does.
fn interpret_device_token(status: StatusCode, body: &serde_json::Value) -> Result<DeviceAnswer> {
    if status.is_success() {
        return body
            .get("access_token")
            .and_then(serde_json::Value::as_str)
            .filter(|token| !token.trim().is_empty())
            .map(|token| DeviceAnswer::Approved(token.trim().to_owned()))
            .ok_or_else(|| anyhow!("RomM approved the device but sent no token"));
    }
    if status == StatusCode::BAD_REQUEST {
        match body.get("detail").and_then(serde_json::Value::as_str) {
            Some("authorization_pending" | "slow_down") => return Ok(DeviceAnswer::Pending),
            Some("access_denied") => return Ok(DeviceAnswer::Denied),
            Some("expired_token") => return Ok(DeviceAnswer::Expired),
            _ => {}
        }
    }
    // Rate limiting is RomM asking us to wait, not a failure.
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Ok(DeviceAnswer::Pending);
    }
    Err(refusal(status))
}

/// Says what a refusal means for the user, without quoting anything they sent.
fn refusal(status: StatusCode) -> anyhow::Error {
    match status {
        StatusCode::UNAUTHORIZED => anyhow!("RomM rejected that username or password"),
        StatusCode::FORBIDDEN => anyhow!(
            "this RomM account cannot create API tokens; sign in with an editor or admin account"
        ),
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => {
            anyhow!("this RomM server is too old; Hearthdeck needs RomM 4.8 or newer")
        }
        StatusCode::TOO_MANY_REQUESTS => anyhow!("RomM is rate limiting; wait a minute and retry"),
        status => anyhow!("RomM answered {status}"),
    }
}

async fn read_json<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    if !status.is_success() {
        return Err(refusal(status));
    }
    response
        .json()
        .await
        .context("RomM sent an answer Hearthdeck could not read")
}

/// The approval page as a phone on the LAN would reach it.
///
/// The stored URL is loopback by design, which a phone cannot open. Swapping in
/// this box's LAN address keeps RomM's port, so it works when RomM publishes that
/// port on the network; the code shown beside it covers the case where it does
/// not.
fn phone_link(base_url: &str, path_complete: &str, lan_ip: Option<IpAddr>) -> String {
    let mut url = match reqwest::Url::parse(base_url) {
        Ok(url) => url,
        Err(_) => return format!("{base_url}{path_complete}"),
    };
    if let Some(ip) = lan_ip {
        // Only fails for URLs that cannot carry a host, which a parsed http(s) URL can.
        let _ = url.set_ip_host(ip);
    }
    format!("{}{path_complete}", url.as_str().trim_end_matches('/'))
}

/// This box's address on its default route. Connecting a UDP socket only picks
/// the route; nothing is sent.
fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_phone_link_swaps_loopback_for_the_lan_address_and_keeps_the_port() {
        let ip = "192.168.1.20".parse().ok();
        assert_eq!(
            phone_link(
                "http://127.0.0.1:8080",
                "/pair/device?user_code=ABCD-1234",
                ip
            ),
            "http://192.168.1.20:8080/pair/device?user_code=ABCD-1234"
        );
        assert_eq!(
            phone_link("http://localhost:8080/romm", "/pair/device", ip),
            "http://192.168.1.20:8080/romm/pair/device"
        );
    }

    #[test]
    fn without_a_lan_address_the_phone_link_stays_on_the_saved_url() {
        assert_eq!(
            phone_link("http://127.0.0.1:8080", "/pair/device", None),
            "http://127.0.0.1:8080/pair/device"
        );
    }

    #[test]
    fn device_token_answers_map_to_their_meaning() {
        let bad = StatusCode::BAD_REQUEST;
        for (detail, expected) in [
            ("authorization_pending", DeviceAnswer::Pending),
            ("slow_down", DeviceAnswer::Pending),
            ("access_denied", DeviceAnswer::Denied),
            ("expired_token", DeviceAnswer::Expired),
        ] {
            assert_eq!(
                interpret_device_token(bad, &json!({ "detail": detail })).unwrap(),
                expected
            );
        }
        assert_eq!(
            interpret_device_token(
                StatusCode::OK,
                &json!({ "access_token": "rmm_abc", "device_id": "d", "scopes": [] })
            )
            .unwrap(),
            DeviceAnswer::Approved("rmm_abc".to_owned())
        );
        assert_eq!(
            interpret_device_token(StatusCode::TOO_MANY_REQUESTS, &json!(null)).unwrap(),
            DeviceAnswer::Pending
        );
    }

    #[test]
    fn an_approval_without_a_token_or_an_unknown_refusal_is_an_error() {
        assert!(interpret_device_token(StatusCode::OK, &json!({})).is_err());
        assert!(
            interpret_device_token(StatusCode::BAD_REQUEST, &json!({ "detail": "?" })).is_err()
        );
        let old = interpret_device_token(StatusCode::NOT_FOUND, &json!(null)).unwrap_err();
        assert!(old.to_string().contains("4.8"));
    }

    #[tokio::test]
    async fn polling_with_nothing_pending_is_idle() {
        assert!(matches!(
            DeviceLinkState::new().poll().await.unwrap(),
            DevicePoll::Idle
        ));
    }
}
