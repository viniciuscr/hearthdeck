//! The Stremio account link: the session it produces, and the flow that produces
//! it.
//!
//! The account and link services are undocumented, and their answers are their
//! own shape: `api.strem.io` answers HTTP **200** for success and failure alike,
//! where success carries `result` and no `error` and failure carries `error` and
//! no `result`. Nothing here trusts a status code.
//!
//! Two things in this module exist to keep the credential contained: the session
//! type is deliberately not `Serialize`, and no error message this module builds
//! ever includes the key.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use tokio::sync::Mutex;

/// The link service, which turns "somebody who is logged in elsewhere" into a
/// session on this machine without this machine ever seeing a password.
const LINK_URL: &str = "https://link.stremio.com/api";

/// The catalog source this integration owns.
///
/// Named here rather than in the provider that will publish under it, so the
/// unlink path and the provider cannot drift apart.
pub const SOURCE_ID: &str = "stremio";

/// How long a link is offered before this daemon stops waiting on it.
///
/// Hearthdeck's own window, not Stremio's. The upstream expiry is undocumented,
/// and a live code and a dead one answer identically — both give
/// `101 Invalid or expired token` — so only the caller's own clock can end the
/// wait. This is what the settings screen counts down.
pub const LINK_WINDOW: Duration = Duration::from_secs(300);

/// The session key Stremio returns is 44 characters. The ceiling is generous so a
/// longer key is never itself the reason a link fails, but bounded so nothing
/// arbitrarily large from a response body can land in the database.
const MAX_AUTH_KEY: usize = 512;

/// A link the user approves on another device.
#[derive(Clone, Debug, Serialize)]
pub struct LinkOffer {
    /// The four characters the user enters in the Stremio app or site.
    pub code: String,
    /// The link to open, which already carries the code.
    pub link: String,
    /// Stremio's own QR image for that link.
    pub qr: String,
    pub expires_in_seconds: u64,
}

/// What one `/api/read` answer says about a pending link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkAnswer {
    /// The account is linked; this is the session key.
    Linked(String),
    /// Not linked yet — or the code is dead. The two are the same answer.
    Waiting,
}

/// What a poll of the pending link found.
pub enum LinkPoll {
    /// Nothing was ever started, or the window closed and the offer was dropped.
    Idle,
    /// The window closed while the user was still deciding.
    Expired,
    /// Still waiting, with the offer and the seconds left in the window.
    Waiting(LinkOffer, u64),
    /// The account linked. The caller stores this; this module never logs it.
    Linked(String),
}

/// The account link as the settings screen may see it.
///
/// There is deliberately no field for the session key, and no account name: the
/// link flow answers with a key and nothing else, so the link time is all there
/// is to show, and nothing here can leak the credential by accident.
#[derive(Clone, Debug, Serialize)]
pub struct StremioConnection {
    pub linked_at: String,
    pub updated_at: String,
}

/// The session itself, for the provider's use.
///
/// Not `Serialize`, for the same reason `RommCredentials` is not: a credential
/// that cannot be serialized cannot be returned by a handler, or written out by
/// a log line that formatted the wrong struct, by accident.
#[derive(Clone, Debug)]
pub struct StremioCredentials {
    pub auth_key: String,
}

#[derive(Clone)]
pub struct StremioRepository {
    pool: SqlitePool,
}

impl StremioRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The linked account, without its credential.
    pub async fn connection(&self) -> Result<Option<StremioConnection>> {
        let row = sqlx::query("SELECT linked_at, updated_at FROM stremio_settings WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| StremioConnection {
            linked_at: row.get("linked_at"),
            updated_at: row.get("updated_at"),
        }))
    }

    /// The session, for the calls that need it.
    ///
    /// Never served to a client: the discovery provider is the only caller, and
    /// nothing else in the daemon may hold this key.
    pub async fn credentials(&self) -> Result<Option<StremioCredentials>> {
        let row = sqlx::query("SELECT auth_key FROM stremio_settings WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| StremioCredentials {
            auth_key: row.get("auth_key"),
        }))
    }

    /// Stores a freshly linked session.
    ///
    /// A re-link keeps the original `linked_at`: that is when this account was
    /// linked, and re-linking is a repair rather than a new account.
    pub async fn save_session(&self, auth_key: &str) -> Result<StremioConnection> {
        let auth_key = auth_key.trim();
        // The messages below describe the key without quoting it: this error can
        // reach a client and a log, and the key must reach neither.
        if auth_key.is_empty() {
            bail!("the Stremio link produced an empty session key");
        }
        if auth_key.len() > MAX_AUTH_KEY {
            bail!("the Stremio link produced a session key longer than {MAX_AUTH_KEY} characters");
        }
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO stremio_settings (id, auth_key, linked_at, updated_at) VALUES (1, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET auth_key = excluded.auth_key, updated_at = excluded.updated_at",
        )
        .bind(auth_key)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        self.connection()
            .await?
            .ok_or_else(|| anyhow!("the Stremio session did not persist"))
    }

    pub async fn clear(&self) -> Result<()> {
        sqlx::query("DELETE FROM stremio_settings WHERE id = 1")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// The one link a user may be waiting on at a time.
///
/// Held in memory rather than in the database: a link is approved within minutes
/// or not at all, and a restart is a fine reason to ask for a fresh code. Holding
/// it here is also what lets the settings screen poll without having to carry the
/// code itself, and without the code ever reaching a query string on our side.
#[derive(Clone, Default)]
pub struct LinkState {
    pending: Arc<Mutex<Option<PendingLink>>>,
}

#[derive(Clone)]
struct PendingLink {
    code: String,
    offer: LinkOffer,
    started: Instant,
}

impl LinkState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops any pending link.
    ///
    /// Used when the account is unlinked: a code that is still live would let a
    /// link arrive after the session it belongs to has already been removed.
    pub async fn cancel(&self) {
        self.pending.lock().await.take();
    }

    /// Offers a fresh link, replacing any earlier one.
    pub async fn start(&self, offer: LinkOffer) {
        let code = offer.code.clone();
        *self.pending.lock().await = Some(PendingLink {
            code,
            offer,
            started: Instant::now(),
        });
    }

    /// Polls the pending link once.
    ///
    /// The lock is released across the network call, so a slow poll cannot hold up
    /// the next one and `start` can replace the offer at any moment.
    pub async fn poll(&self) -> Result<LinkPoll> {
        let Some((code, offer, started)) = self
            .pending
            .lock()
            .await
            .as_ref()
            .map(|pending| (pending.code.clone(), pending.offer.clone(), pending.started))
        else {
            return Ok(LinkPoll::Idle);
        };

        if started.elapsed() >= LINK_WINDOW {
            self.pending.lock().await.take();
            return Ok(LinkPoll::Expired);
        }

        match poll_link(&code).await? {
            LinkAnswer::Linked(auth_key) => {
                self.pending.lock().await.take();
                Ok(LinkPoll::Linked(auth_key))
            }
            LinkAnswer::Waiting => {
                let left = LINK_WINDOW.saturating_sub(started.elapsed());
                Ok(LinkPoll::Waiting(offer, left.as_secs()))
            }
        }
    }
}

/// Asks the link service for a fresh code.
///
/// Codes are single-use — the first attempt consumes the one it was given — so
/// this is called per attempt and never cached.
pub async fn request_link() -> Result<LinkOffer> {
    let body = get_json(&format!("{LINK_URL}/create"), &[]).await?;
    if let Some(error) = envelope_error(&body) {
        bail!("the Stremio link service refused the request: {error}");
    }
    // `/api/create` carries the same fields at the top level and inside `result`.
    let payload = body.get("result").filter(|value| value.is_object());
    let payload = payload.unwrap_or(&body);
    let code = string_field(payload, "code")
        .ok_or_else(|| anyhow!("the Stremio link service returned no code"))?;
    let link =
        string_field(payload, "link").unwrap_or_else(|| format!("https://link.stremio.com/{code}"));
    let qr = string_field(payload, "qrcode")
        .or_else(|| string_field(payload, "qr"))
        .unwrap_or_else(|| format!("https://link.stremio.com/qr?data={link}"));
    Ok(LinkOffer {
        code,
        link,
        qr,
        expires_in_seconds: LINK_WINDOW.as_secs(),
    })
}

/// One poll of a pending code.
pub async fn poll_link(code: &str) -> Result<LinkAnswer> {
    let body = get_json(&format!("{LINK_URL}/read"), &[("code", code)]).await?;
    Ok(interpret_read(&body))
}

/// Reads the meaning out of a `/api/read` body.
///
/// Kept separate from the request so it can be tested without a network, because
/// the one distinction it draws is the one that matters: an unlinked code and an
/// expired one answer identically, so only the caller's window can tell "not yet"
/// from "never".
pub fn interpret_read(body: &serde_json::Value) -> LinkAnswer {
    let auth_key = body
        .get("result")
        .and_then(|result| result.get("authKey"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty());
    match auth_key {
        Some(key) => LinkAnswer::Linked(key.to_owned()),
        None => LinkAnswer::Waiting,
    }
}

/// The `error` message of either envelope shape, if the answer was a failure.
fn envelope_error(body: &serde_json::Value) -> Option<String> {
    let error = body.get("error").filter(|value| !value.is_null())?;
    Some(
        error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown error")
            .to_owned(),
    )
}

fn string_field(value: &serde_json::Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .map(str::to_owned)
}

/// The one client these calls share, mirroring the RomM calls' pattern: a single
/// pool, DNS cache and TLS session rather than a fresh one per poll. The
/// per-request timeout stays on the request builder.
fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// A JSON GET against one of the two Stremio services.
///
/// The URL in the error context is the path without its query, so a failed poll
/// cannot put a link code into a log line or an error a client can read.
async fn get_json(url: &str, query: &[(&str, &str)]) -> Result<serde_json::Value> {
    let response = http()
        .get(url)
        .query(query)
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .with_context(|| format!("could not reach {url}"))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("could not read the answer from {url}"))?;

    // Both services answer 200 for failure, so a non-JSON body is a proxy or a
    // captive portal rather than an answer. The body itself is not quoted into the
    // error: it is not ours, and it is not needed to say what happened.
    serde_json::from_str(&text)
        .map_err(|_| anyhow!("{url} answered with HTTP {status} and a body that is not JSON"))
}

#[cfg(test)]
mod tests {
    use super::{LinkAnswer, StremioRepository, interpret_read};
    use crate::database::Database;
    use serde_json::json;

    /// The living and the dead code answer the same body, so the parser must not
    /// invent a distinction the service does not make.
    #[test]
    fn a_linked_read_carries_the_key_and_everything_else_waits() {
        assert_eq!(
            interpret_read(&json!({"result": {"authKey": "session_key", "success": true}})),
            LinkAnswer::Linked("session_key".to_owned())
        );
        // The observed unlinked answer, which is also the expired one.
        assert_eq!(
            interpret_read(&json!({
                "result": null,
                "error": {"code": 101, "message": "Invalid or expired token"}
            })),
            LinkAnswer::Waiting
        );
        assert_eq!(
            interpret_read(&json!({"result": {"authKey": "   "}})),
            LinkAnswer::Waiting
        );
        assert_eq!(interpret_read(&json!({})), LinkAnswer::Waiting);
    }

    /// The public view is what a handler returns, so it must not be able to carry
    /// the credential even if somebody serializes the wrong thing.
    #[tokio::test]
    async fn the_settings_view_never_carries_the_session_key() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let stremio = StremioRepository::new(database.pool().clone());

        assert!(stremio.connection().await.unwrap().is_none());
        assert!(stremio.credentials().await.unwrap().is_none());

        let linked = stremio.save_session("  session_key  ").await.unwrap();
        assert_eq!(linked.linked_at, linked.updated_at);

        let credentials = stremio.credentials().await.unwrap().unwrap();
        assert_eq!(
            credentials.auth_key, "session_key",
            "the key is trimmed on the way in"
        );

        let public = serde_json::to_string(&stremio.connection().await.unwrap().unwrap()).unwrap();
        assert!(
            !public.contains("session_key"),
            "the settings view must not carry the key: {public}"
        );

        // Re-linking is a repair: the account was first linked when it was linked.
        let relinked = stremio.save_session("second_key").await.unwrap();
        assert_eq!(relinked.linked_at, linked.linked_at);
        assert!(!relinked.updated_at.is_empty());
        assert_eq!(
            stremio.credentials().await.unwrap().unwrap().auth_key,
            "second_key"
        );

        stremio.clear().await.unwrap();
        assert!(stremio.connection().await.unwrap().is_none());
    }

    /// A key this daemon will not store is described, never quoted: the error can
    /// reach a client and a log.
    #[tokio::test]
    async fn refuses_a_session_key_that_is_empty_or_absurd_without_quoting_it() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let stremio = StremioRepository::new(database.pool().clone());

        let empty = stremio
            .save_session("   \n ")
            .await
            .unwrap_err()
            .to_string();
        assert!(empty.contains("empty"), "{empty}");

        let absurd = "k".repeat(super::MAX_AUTH_KEY + 1);
        let refused = stremio.save_session(&absurd).await.unwrap_err().to_string();
        assert!(refused.contains("longer than"), "{refused}");
        assert!(
            !refused.contains(&absurd),
            "the refusal must not quote the key: {refused}"
        );

        assert!(stremio.connection().await.unwrap().is_none());
    }
}
