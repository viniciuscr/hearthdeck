//! Cloud provider credentials: where they live, and who may see them.
//!
//! The settings window sends a credential **once**; the daemon validates it with
//! the provider, stores it in a `0600` file it owns, and never puts it in
//! rclone's configuration. A backup run gets it through the environment
//! (`RCLONE_CONFIG_PCLOUD_*`), which the packaged unit reads from the same file.
//! See `docs/provider-credentials.md` for the decision and its threat model.

use std::{collections::BTreeMap, fmt, io, path::PathBuf, time::Duration};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;

/// The providers a user can configure, in the order the page lists them.
pub const PROVIDERS: [&str; 1] = ["pcloud"];
pub const PCLOUD: &str = "pcloud";
/// pCloud answers on a second host for EU accounts; `getauth` names it.
const DEFAULT_HOSTNAME: &str = "api.pcloud.com";
const TIMEOUT: Duration = Duration::from_secs(15);

/// What the page may know about a provider. Never the credential.
#[derive(Clone, Debug, Serialize)]
pub struct ProviderState {
    pub id: &'static str,
    pub configured: bool,
    pub verified_at: Option<DateTime<Utc>>,
    pub hostname: Option<String>,
}

/// Why a provider refused. Every message is safe to show and to log: none of them
/// can quote the credential, because none of them carries one.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("pCloud refused the credential: {0}")]
    Refused(String),
    #[error("pCloud could not be reached: {0}")]
    Transport(String),
    /// pCloud answered, but not with JSON we can use. Reported separately so a
    /// wrong path or an edge page is never mistaken for a network failure.
    #[error("pCloud answered with status {status}")]
    Answered { status: u16 },
    #[error("the credential could not be stored: {0}")]
    Storage(#[from] io::Error),
}

/// What pCloud hands back for an email and password.
#[derive(Clone, Debug)]
pub struct PcloudToken {
    pub token: String,
    pub hostname: String,
}

/// The provider's API, as a trait so the daemon can be tested without pCloud.
#[async_trait]
pub trait PcloudApi: Send + Sync {
    /// Whether pCloud accepts the token in the form rclone will send it
    /// (`Authorization: Bearer`), on the host named.
    async fn userinfo(&self, token: &str, hostname: &str) -> Result<(), ProviderError>;
}

/// The real pCloud client. Nothing here logs a request body or a header.
pub struct HttpPcloud {
    http: reqwest::Client,
}

impl HttpPcloud {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { http }
    }
}

#[async_trait]
impl PcloudApi for HttpPcloud {
    async fn userinfo(&self, token: &str, hostname: &str) -> Result<(), ProviderError> {
        let response = self
            .http
            .get(format!("https://{hostname}/userinfo"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(transport)?;
        judge_userinfo(response.status().as_u16(), body_of(response).await?)
    }
}

/// pCloud answers `200` even when it refuses: the verdict is in `result`, where
/// `0` means success. Judging on the HTTP status alone would accept a bad token.
fn result_code(body: &serde_json::Value) -> Option<i64> {
    body.get("result").and_then(|value| value.as_i64())
}

/// pCloud's own refusal text. It is a fixed message ("Log in failed."), never the
/// credential, and it is the only thing that tells a wrong password from a 2FA
/// account - so it is kept, shortened, and never trimmed away.
fn error_text(body: &serde_json::Value) -> String {
    body.get("error")
        .and_then(|value| value.as_str())
        .map(|text| text.chars().take(120).collect::<String>())
        .unwrap_or_else(|| "no reason given".to_string())
}

/// rclone stores `{"access_token":"…","token_type":"bearer","expiry":"…"}`; a
/// bare token is accepted too, because that is what people paste. The hostname is
/// not in the blob, and EU accounts answer only on `eapi.pcloud.com`, so it is
/// probed rather than guessed.
fn parse_token(input: &str) -> Result<PcloudToken, ProviderError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ProviderError::Invalid("paste the token rclone printed"));
    }
    if let Ok(blob) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let token = blob
            .get("access_token")
            .and_then(|value| value.as_str())
            .ok_or(ProviderError::Invalid(
                "that JSON has no access_token in it",
            ))?;
        let hostname = blob
            .get("hostname")
            .and_then(|value| value.as_str())
            .unwrap_or(DEFAULT_HOSTNAME);
        return Ok(PcloudToken {
            token: token.to_string(),
            hostname: hostname.to_string(),
        });
    }
    Ok(PcloudToken {
        token: trimmed.to_string(),
        hostname: DEFAULT_HOSTNAME.to_string(),
    })
}

fn transport(error: reqwest::Error) -> ProviderError {
    ProviderError::Transport(error.to_string())
}

async fn body_of(response: reqwest::Response) -> Result<serde_json::Value, ProviderError> {
    let status = response.status().as_u16();
    if !response.status().is_success() {
        return Err(ProviderError::Answered { status });
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|_| ProviderError::Answered { status })
}

fn judge_userinfo(status: u16, body: serde_json::Value) -> Result<(), ProviderError> {
    if !(200..300).contains(&status) {
        return Err(ProviderError::Answered { status });
    }
    if result_code(&body) == Some(0) {
        Ok(())
    } else {
        Err(ProviderError::Refused(error_text(&body)))
    }
}

/// The `0600` file the daemon owns. It is in the environment-file format on
/// purpose: the packaged backup unit reads it, so a scheduled run inherits the
/// credential without rclone ever holding a config file.
#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn read(&self) -> BTreeMap<String, String> {
        std::fs::read_to_string(&self.path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
            .collect()
    }

    pub fn state(&self) -> ProviderState {
        let values = self.read();
        ProviderState {
            id: PCLOUD,
            configured: values.contains_key("RCLONE_CONFIG_PCLOUD_TOKEN"),
            verified_at: values
                .get("HEARTHDECK_PROVIDER_PCLOUD_VERIFIED_AT")
                .and_then(|value| value.parse::<DateTime<Utc>>().ok()),
            hostname: values.get("RCLONE_CONFIG_PCLOUD_HOSTNAME").cloned(),
        }
    }

    fn write(&self, token: &PcloudToken) -> io::Result<()> {
        // rclone's env-var remote: type, the OAuth blob it insists on, and the
        // host that answered. No `rclone.conf` is touched.
        // rclone's own example for pCloud is
        // `{"access_token":"XXX","token_type":"bearer","expiry":"0001-01-01T00:00:00Z"}`:
        // pCloud's `getauth` token is the access token, and the expiry is the
        // zero date because pCloud's tokens are not time-bound.
        let blob = serde_json::json!({
            "access_token": token.token,
            "token_type": "bearer",
            "expiry": "0001-01-01T00:00:00Z",
        });
        let body = format!(
            "RCLONE_CONFIG_PCLOUD_TYPE=pcloud\nRCLONE_CONFIG_PCLOUD_TOKEN={blob}\nRCLONE_CONFIG_PCLOUD_HOSTNAME={}\nHEARTHDECK_PROVIDER_PCLOUD_VERIFIED_AT={}\n",
            token.hostname,
            Utc::now().to_rfc3339()
        );
        super::write_private(&self.path, body.as_bytes())
    }

    pub fn forget(&self) -> io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// The env vars a verification or a run needs, so `CommandRunner` callers do
    /// not have to know the file's format.
    pub fn environment(&self) -> Vec<(String, String)> {
        self.read().into_iter().collect()
    }
}

/// The provider operations the API exposes.
pub(crate) struct Providers<'a> {
    pub store: &'a Store,
    pub api: &'a dyn PcloudApi,
}

impl Providers<'_> {
    pub fn list(&self) -> Vec<ProviderState> {
        vec![self.store.state()]
    }

    /// Validates a pasted rclone token **before** storing it. The account may live
    /// on either pCloud host, so both are tried and the one that answers is kept.
    pub async fn connect_pcloud(&self, token: &str) -> Result<ProviderState, ProviderError> {
        let parsed = parse_token(token)?;
        let mut hosts = vec![parsed.hostname.clone()];
        for host in [DEFAULT_HOSTNAME, "eapi.pcloud.com"] {
            if !hosts.iter().any(|seen| seen == host) {
                hosts.push(host.to_string());
            }
        }
        let mut last = ProviderError::Refused("pCloud did not answer".to_string());
        for host in hosts {
            match self.api.userinfo(&parsed.token, &host).await {
                Ok(()) => {
                    self.store.write(&PcloudToken {
                        token: parsed.token,
                        hostname: host,
                    })?;
                    return Ok(self.store.state());
                }
                Err(error) => {
                    tracing::warn!(provider = PCLOUD, %error, host, "provider token refused");
                    last = error;
                }
            }
        }
        Err(last)
    }

    /// Re-verifies what is already stored, without the user retyping anything.
    pub async fn verify_pcloud(&self) -> Result<ProviderState, ProviderError> {
        let token = self
            .store
            .environment()
            .into_iter()
            .find(|(key, _)| key == "RCLONE_CONFIG_PCLOUD_TOKEN")
            .map(|(_, value)| value)
            .ok_or(ProviderError::Invalid("pCloud is not configured"))?;
        let access = serde_json::from_str::<serde_json::Value>(&token)
            .ok()
            .and_then(|blob| {
                blob.get("access_token")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .ok_or(ProviderError::Invalid("the stored credential is malformed"))?;
        let hostname = self
            .store
            .state()
            .hostname
            .unwrap_or_else(|| DEFAULT_HOSTNAME.to_string());
        self.api.userinfo(&access, &hostname).await?;
        self.store.write(&PcloudToken {
            token: access,
            hostname,
        })?;
        Ok(self.store.state())
    }

    pub fn forget_pcloud(&self) -> Result<(), ProviderError> {
        self.store.forget().map_err(ProviderError::Storage)
    }
}

/// A provider adapter that answers without a network, for the tests.
#[cfg(test)]
#[derive(Default)]
pub struct FakePcloud {
    pub bearer_ok: bool,
}

#[cfg(test)]
impl FakePcloud {
    pub fn accepting() -> Self {
        Self { bearer_ok: true }
    }

    pub fn refusing() -> Self {
        Self { bearer_ok: false }
    }
}

#[cfg(test)]
#[async_trait]
impl PcloudApi for FakePcloud {
    async fn userinfo(&self, _token: &str, _hostname: &str) -> Result<(), ProviderError> {
        if self.bearer_ok {
            Ok(())
        } else {
            Err(ProviderError::Refused(
                "invalid access token provided".to_string(),
            ))
        }
    }
}

impl fmt::Debug for Store {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A store that prints its path is fine; one that prints its contents is not.
        formatter
            .debug_struct("Store")
            .field("path", &self.path)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{FakePcloud, ProviderError, Providers, Store, parse_token};
    use std::path::PathBuf;

    fn store() -> (Store, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "hearthdeck-providers-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        (Store::new(root.join("credentials.env")), root)
    }

    #[test]
    fn a_pasted_blob_yields_its_access_token() {
        let blob =
            r#"{"access_token":"abc","token_type":"bearer","expiry":"0001-01-01T00:00:00Z"}"#;
        let token = parse_token(blob).expect("parsed");
        assert_eq!(token.token, "abc");
        assert_eq!(token.hostname, "api.pcloud.com");
    }

    #[test]
    fn a_bare_token_is_accepted_because_that_is_what_people_paste() {
        assert_eq!(parse_token("  abc  ").expect("parsed").token, "abc");
        assert!(matches!(parse_token("   "), Err(ProviderError::Invalid(_))));
    }

    #[test]
    fn json_without_an_access_token_is_refused_with_a_reason() {
        let error = parse_token(r#"{"token_type":"bearer"}"#).expect_err("refused");
        assert!(matches!(error, ProviderError::Invalid(reason) if reason.contains("access_token")));
    }

    #[tokio::test]
    async fn a_verified_credential_is_stored_privately_and_never_listed() {
        let (store, root) = store();
        let api = FakePcloud::accepting();
        let providers = Providers {
            store: &store,
            api: &api,
        };

        let state = providers
            .connect_pcloud("token-value")
            .await
            .expect("accepted");
        assert!(state.configured);
        assert!(state.verified_at.is_some());

        let listing = format!("{:?}", providers.list());
        assert!(
            !listing.contains("token-value"),
            "the listing must never carry the credential: {listing}"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(root.join("credentials.env"))
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "the file is private");
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_token_pcloud_will_not_take_as_a_bearer_is_not_stored() {
        let (store, root) = store();
        let api = FakePcloud::refusing();
        let providers = Providers {
            store: &store,
            api: &api,
        };

        let error = providers
            .connect_pcloud("token-value")
            .await
            .expect_err("refused");
        assert!(matches!(error, ProviderError::Refused(_)));
        assert!(!store.state().configured, "nothing was written");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn forgetting_removes_the_file() {
        let (store, root) = store();
        let api = FakePcloud::accepting();
        let providers = Providers {
            store: &store,
            api: &api,
        };

        providers
            .connect_pcloud("token-value")
            .await
            .expect("accepted");
        providers.forget_pcloud().expect("forgotten");
        assert!(!store.state().configured);

        let _ = std::fs::remove_dir_all(&root);
    }
}
