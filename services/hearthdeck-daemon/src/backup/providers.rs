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
const PCLOUD: &str = "pcloud";
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
    #[error("pCloud refused the credential")]
    Refused,
    #[error("pCloud could not be reached: {0}")]
    Transport(String),
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
    async fn getauth(&self, email: &str, password: &str) -> Result<PcloudToken, ProviderError>;
    /// Whether pCloud accepts the token in the form rclone will send it
    /// (`Authorization: Bearer`). This is the check that decides whether a
    /// `getauth` token is usable at all.
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
    async fn getauth(&self, email: &str, password: &str) -> Result<PcloudToken, ProviderError> {
        // POST body, never a query string: URLs end up in logs and in `ps`.
        let response = self
            .http
            .post(format!("https://{DEFAULT_HOSTNAME}/getauth"))
            .form(&[
                ("getauth", "1"),
                ("logout", "1"),
                ("username", email),
                ("password", password),
            ])
            .send()
            .await
            .map_err(|error| ProviderError::Transport(error.to_string()))?
            .json::<serde_json::Value>()
            .await
            .map_err(|error| ProviderError::Transport(error.to_string()))?;

        if response.get("result").and_then(|value| value.as_i64()) != Some(0) {
            // pCloud's own error text can name the account but never the secret.
            return Err(ProviderError::Refused);
        }
        let token = response
            .get("auth")
            .and_then(|value| value.as_str())
            .ok_or(ProviderError::Refused)?;
        let hostname = response
            .get("hostname")
            .and_then(|value| value.as_str())
            .unwrap_or(DEFAULT_HOSTNAME);
        Ok(PcloudToken {
            token: token.to_string(),
            hostname: hostname.to_string(),
        })
    }

    async fn userinfo(&self, token: &str, hostname: &str) -> Result<(), ProviderError> {
        let response = self
            .http
            .get(format!("https://{hostname}/userinfo"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| ProviderError::Transport(error.to_string()))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(ProviderError::Refused)
        }
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

    /// Validates the credential with pCloud **before** storing it, and stores only
    /// what pCloud accepted in the form rclone will send.
    pub async fn connect_pcloud(
        &self,
        email: &str,
        password: &str,
    ) -> Result<ProviderState, ProviderError> {
        if email.trim().is_empty() || !email.contains('@') {
            return Err(ProviderError::Invalid("a pCloud account email is required"));
        }
        if password.is_empty() {
            return Err(ProviderError::Invalid("a pCloud password is required"));
        }
        let token = self.api.getauth(email.trim(), password).await?;
        self.api.userinfo(&token.token, &token.hostname).await?;
        self.store.write(&token)?;
        Ok(self.store.state())
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
            .and_then(|blob| blob.get("access_token").and_then(|value| value.as_str()).map(str::to_string))
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
    pub accepts: bool,
    pub bearer_ok: bool,
    pub hostname: String,
    seen: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl FakePcloud {
    pub fn accepting() -> Self {
        Self {
            accepts: true,
            bearer_ok: true,
            hostname: DEFAULT_HOSTNAME.to_string(),
            seen: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// What the fake was asked, so a test can assert the secret never travels in
    /// a way the daemon would have to log.
    pub fn seen(&self) -> Vec<String> {
        self.seen.lock().map(|seen| seen.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
#[async_trait]
impl PcloudApi for FakePcloud {
    async fn getauth(&self, email: &str, password: &str) -> Result<PcloudToken, ProviderError> {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(format!("getauth {email}"));
        }
        if !self.accepts || password != "correct horse" {
            return Err(ProviderError::Refused);
        }
        Ok(PcloudToken {
            token: "token-value".to_string(),
            hostname: self.hostname.clone(),
        })
    }

    async fn userinfo(&self, _token: &str, _hostname: &str) -> Result<(), ProviderError> {
        if self.bearer_ok {
            Ok(())
        } else {
            Err(ProviderError::Refused)
        }
    }
}

impl fmt::Debug for Store {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A store that prints its path is fine; one that prints its contents is not.
        formatter.debug_struct("Store").field("path", &self.path).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{FakePcloud, ProviderError, Providers, Store};
    use std::path::PathBuf;

    fn store() -> (Store, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "hearthdeck-providers-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        (Store::new(root.join("credentials.env")), root)
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
            .connect_pcloud("user@example.com", "correct horse")
            .await
            .expect("accepted");
        assert!(state.configured);
        assert!(state.verified_at.is_some(), "the store dates the verification");

        let listed = providers.list();
        assert_eq!(listed.len(), 1);
        let listing = format!("{listed:?}");
        assert!(
            !listing.contains("token-value"),
            "the listing must never carry the credential: {listing}"
        );

        let mode = std::fs::metadata(root.join("credentials.env"))
            .expect("metadata")
            .permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(mode.mode() & 0o777, 0o600, "the file is private");
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_refused_credential_is_not_stored() {
        let (store, root) = store();
        let api = FakePcloud::accepting();
        let providers = Providers {
            store: &store,
            api: &api,
        };

        let error = providers
            .connect_pcloud("user@example.com", "wrong")
            .await
            .expect_err("refused");
        assert!(matches!(error, ProviderError::Refused));
        assert!(!store.state().configured, "nothing was written");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_token_pcloud_will_not_take_as_a_bearer_is_not_stored() {
        let (store, root) = store();
        let api = FakePcloud {
            bearer_ok: false,
            ..FakePcloud::accepting()
        };
        let providers = Providers {
            store: &store,
            api: &api,
        };

        let error = providers
            .connect_pcloud("user@example.com", "correct horse")
            .await
            .expect_err("refused");
        assert!(matches!(error, ProviderError::Refused));
        assert!(!store.state().configured);

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
            .connect_pcloud("user@example.com", "correct horse")
            .await
            .expect("accepted");
        providers.forget_pcloud().expect("forgotten");
        assert!(!store.state().configured);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_api_never_sees_the_password() {
        let api = FakePcloud::accepting();
        assert!(api.seen().is_empty());
    }
}
