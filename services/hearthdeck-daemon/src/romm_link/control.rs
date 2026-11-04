use serde::Serialize;
use tracing::info;

use super::{DeviceLinkState, DeviceOffer, DevicePoll, mint_token};
use crate::settings::{RommSettings, SettingsRepository, normalize_romm_url};

pub(crate) struct Control<'a> {
    pub settings: &'a SettingsRepository,
    pub link: &'a DeviceLinkState,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ControlError {
    #[error("{0}")]
    InvalidSettings(#[source] anyhow::Error),
    #[error("{0}")]
    Link(#[source] anyhow::Error),
    #[error("could not store RomM connection")]
    Storage(#[source] anyhow::Error),
}

/// The pending approval's credential-free view.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum LinkStatus {
    Idle,
    Expired,
    Denied,
    Waiting {
        code: String,
        link: String,
        seconds_left: u64,
    },
    Linked {
        settings: RommSettings,
    },
}

impl Control<'_> {
    pub(crate) async fn settings(&self) -> Result<Option<RommSettings>, ControlError> {
        self.settings.romm().await.map_err(ControlError::Storage)
    }

    pub(crate) async fn save(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<RommSettings, ControlError> {
        // Direct settings writes retain their existing bad-request error contract.
        self.settings
            .save_romm(base_url, token)
            .await
            .map_err(ControlError::InvalidSettings)
    }

    pub(crate) async fn clear(&self) -> Result<(), ControlError> {
        self.link.cancel().await;
        self.settings
            .clear_romm()
            .await
            .map_err(ControlError::Storage)
    }

    pub(crate) async fn login(
        &self,
        base_url: &str,
        username: &str,
        password: &str,
    ) -> Result<RommSettings, ControlError> {
        let base_url = normalize_romm_url(base_url).map_err(ControlError::InvalidSettings)?;
        let token = mint_token(&base_url, username, password)
            .await
            .map_err(ControlError::Link)?;
        self.link.cancel().await;
        let saved = self
            .settings
            .save_romm(&base_url, &token)
            .await
            .map_err(ControlError::Storage)?;
        info!("romm connected with a password sign-in");
        Ok(saved)
    }

    pub(crate) async fn start_link(&self, base_url: &str) -> Result<DeviceOffer, ControlError> {
        let base_url = normalize_romm_url(base_url).map_err(ControlError::InvalidSettings)?;
        let offer = self
            .link
            .start(&base_url)
            .await
            .map_err(ControlError::Link)?;
        info!("romm device approval started");
        Ok(offer)
    }

    pub(crate) async fn poll_link(&self) -> Result<LinkStatus, ControlError> {
        let status = match self.link.poll().await.map_err(ControlError::Link)? {
            DevicePoll::Idle => LinkStatus::Idle,
            DevicePoll::Expired => LinkStatus::Expired,
            DevicePoll::Denied => LinkStatus::Denied,
            DevicePoll::Waiting(offer, seconds_left) => LinkStatus::Waiting {
                code: offer.code,
                link: offer.link,
                seconds_left,
            },
            DevicePoll::Linked { base_url, token } => {
                let settings = self
                    .settings
                    .save_romm(&base_url, &token)
                    .await
                    .map_err(ControlError::Storage)?;
                info!("romm connected by device approval");
                LinkStatus::Linked { settings }
            }
        };
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Instant};

    use axum::{
        Form, Json, Router,
        http::{HeaderMap, StatusCode},
        routing::post,
    };
    use serde_json::json;
    use tempfile::{TempDir, tempdir};
    use tokio::{net::TcpListener, task::JoinHandle};

    use super::*;
    use crate::{
        database::Database,
        romm_link::{PendingDevice, SCOPES},
    };

    async fn storage() -> (TempDir, Database, SettingsRepository) {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let settings = SettingsRepository::new(database.pool().clone());
        (directory, database, settings)
    }

    async fn pending(link: &DeviceLinkState, base_url: &str) {
        *link.pending.lock().await = Some(PendingDevice {
            base_url: base_url.to_owned(),
            device_code: "device-secret".to_owned(),
            offer: DeviceOffer {
                code: "ABCD-1234".to_owned(),
                link: format!("{base_url}/approve"),
                expires_in_seconds: 300,
            },
            started: Instant::now(),
            window: std::time::Duration::from_secs(300),
            interval: std::time::Duration::MAX,
            last_poll: None,
        });
    }

    async fn serve(app: Router) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (base_url, task)
    }

    #[tokio::test]
    async fn direct_settings_keep_pending_approval_but_disconnect_cancels_it() {
        let (_directory, _database, settings) = storage().await;
        let link = DeviceLinkState::new();
        let control = Control {
            settings: &settings,
            link: &link,
        };
        assert!(control.settings().await.unwrap().is_none());
        pending(&link, "http://localhost:8080").await;

        let saved = control
            .save(" http://localhost:8080/ ", " rmm_direct ")
            .await
            .unwrap();
        assert_eq!(saved.base_url, "http://localhost:8080");
        assert!(saved.configured);
        assert!(link.pending.lock().await.is_some());
        assert_eq!(
            settings.romm_credentials().await.unwrap().unwrap().token,
            "rmm_direct"
        );
        let public = serde_json::to_value(control.settings().await.unwrap().unwrap()).unwrap();
        assert_eq!(public["base_url"], "http://localhost:8080");
        assert!(public.get("token").is_none());

        control.clear().await.unwrap();
        assert!(matches!(
            control.poll_link().await.unwrap(),
            LinkStatus::Idle
        ));
        assert!(control.settings().await.unwrap().is_none());
        assert!(settings.romm_credentials().await.unwrap().is_none());
        control.clear().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_urls_leave_connection_and_approval_untouched() {
        let (_directory, database, settings) = storage().await;
        let link = DeviceLinkState::new();
        let control = Control {
            settings: &settings,
            link: &link,
        };
        control
            .save("http://localhost:8080", "rmm_original")
            .await
            .unwrap();
        pending(&link, "http://localhost:8080").await;

        assert!(matches!(
            control
                .login("https://romm.example.com", "alice", "password")
                .await,
            Err(ControlError::InvalidSettings(_))
        ));
        assert!(matches!(
            control.start_link("not a URL").await,
            Err(ControlError::InvalidSettings(_))
        ));
        assert!(matches!(
            control.save("http://localhost:8080", "").await,
            Err(ControlError::InvalidSettings(_))
        ));
        assert!(link.pending.lock().await.is_some());
        assert_eq!(
            settings.romm_credentials().await.unwrap().unwrap().token,
            "rmm_original"
        );

        database.pool().close().await;
        assert!(matches!(
            control.settings().await,
            Err(ControlError::Storage(_))
        ));
        assert!(matches!(
            control.save("http://localhost:8080", "rmm_new").await,
            Err(ControlError::InvalidSettings(_))
        ));
        assert!(matches!(
            control.clear().await,
            Err(ControlError::Storage(_))
        ));
        assert!(link.pending.lock().await.is_none());
    }

    #[tokio::test]
    async fn password_sign_in_stores_only_the_client_token_and_then_cancels_approval() {
        let app = Router::new()
            .route(
                "/api/token",
                post(|Form(form): Form<HashMap<String, String>>| async move {
                    if form["username"] != "alice" {
                        return (StatusCode::UNAUTHORIZED, Json(json!({})));
                    }
                    assert_eq!(form["password"], "password-secret");
                    assert_eq!(form["scope"], "me.write");
                    assert_eq!(form["grant_type"], "password");
                    (
                        StatusCode::OK,
                        Json(json!({"access_token": "session-secret"})),
                    )
                }),
            )
            .route(
                "/api/client-tokens",
                post(
                    |headers: HeaderMap, Json(body): Json<serde_json::Value>| async move {
                        assert_eq!(headers["authorization"], "Bearer session-secret");
                        assert_eq!(body["scopes"], json!(SCOPES));
                        Json(json!({"raw_token": "rmm_client-secret"}))
                    },
                ),
            );
        let (base_url, server) = serve(app).await;
        let (_directory, database, settings) = storage().await;
        let link = DeviceLinkState::new();
        let control = Control {
            settings: &settings,
            link: &link,
        };
        pending(&link, &base_url).await;

        let error = control
            .login(&base_url, "wrong", "password-secret")
            .await
            .unwrap_err();
        assert!(matches!(error, ControlError::Link(_)));
        assert_eq!(error.to_string(), "RomM rejected that username or password");
        assert!(link.pending.lock().await.is_some());
        assert!(control.settings().await.unwrap().is_none());

        let saved = control
            .login(&format!("{base_url}/"), "alice", "password-secret")
            .await
            .unwrap();
        assert_eq!(saved.base_url, base_url);
        assert!(link.pending.lock().await.is_none());
        let credentials = settings.romm_credentials().await.unwrap().unwrap();
        assert_eq!(credentials.token, "rmm_client-secret");
        assert!(!serde_json::to_string(&saved).unwrap().contains("secret"));

        pending(&link, &base_url).await;
        database.pool().close().await;
        assert!(matches!(
            control.login(&base_url, "alice", "password-secret").await,
            Err(ControlError::Storage(_))
        ));
        assert!(link.pending.lock().await.is_none());
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn device_approval_persists_the_token_without_returning_it() {
        let app = Router::new()
            .route(
                "/api/auth/device/init",
                post(|Json(body): Json<serde_json::Value>| async move {
                    assert_eq!(body["requested_scopes"], json!(SCOPES));
                    Json(json!({
                        "device_code": "device-secret",
                        "user_code": "ABCD-1234",
                        "verification_path_complete": "/approve",
                        "expires_in": 300,
                        "interval": 5
                    }))
                }),
            )
            .route(
                "/api/auth/device/token",
                post(|Json(body): Json<serde_json::Value>| async move {
                    assert_eq!(body["device_code"], "device-secret");
                    Json(json!({"access_token": "rmm_approved-secret"}))
                }),
            );
        let (base_url, server) = serve(app).await;
        let (_directory, database, settings) = storage().await;
        let link = DeviceLinkState::new();
        let control = Control {
            settings: &settings,
            link: &link,
        };

        let offer = control.start_link(&base_url).await.unwrap();
        assert_eq!(offer.code, "ABCD-1234");
        assert_eq!(offer.expires_in_seconds, 300);
        assert!(offer.link.ends_with("/approve"));
        let status = control.poll_link().await.unwrap();
        let public = serde_json::to_value(status).unwrap();
        assert_eq!(public["status"], "linked");
        assert_eq!(public["settings"]["base_url"], base_url);
        assert_eq!(public["settings"]["configured"], true);
        assert_eq!(public["settings"].as_object().unwrap().len(), 3);
        assert!(!public.to_string().contains("secret"));
        assert_eq!(
            settings.romm_credentials().await.unwrap().unwrap().token,
            "rmm_approved-secret"
        );
        assert!(matches!(
            control.poll_link().await.unwrap(),
            LinkStatus::Idle
        ));

        control.start_link(&base_url).await.unwrap();
        database.pool().close().await;
        assert!(matches!(
            control.poll_link().await,
            Err(ControlError::Storage(_))
        ));
        assert!(link.pending.lock().await.is_none());
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn polling_keeps_waiting_and_terminal_states_and_reports_upstream_errors() {
        let (base_url, server) = serve(Router::new().route(
            "/api/auth/device/token",
            post(|| async {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"detail": "access_denied"})),
                )
            }),
        ))
        .await;
        let (_directory, _database, settings) = storage().await;
        let link = DeviceLinkState::new();
        let control = Control {
            settings: &settings,
            link: &link,
        };

        pending(&link, &base_url).await;
        link.pending.lock().await.as_mut().unwrap().last_poll = Some(Instant::now());
        let waiting = serde_json::to_value(control.poll_link().await.unwrap()).unwrap();
        assert_eq!(waiting["status"], "waiting");
        assert_eq!(waiting["code"], "ABCD-1234");
        assert_eq!(waiting["link"], format!("{base_url}/approve"));
        assert!(waiting["seconds_left"].as_u64().unwrap() <= 300);
        assert_eq!(waiting.as_object().unwrap().len(), 4);
        link.pending.lock().await.as_mut().unwrap().last_poll = None;
        assert_eq!(
            serde_json::to_value(control.poll_link().await.unwrap()).unwrap(),
            json!({"status": "denied"})
        );
        assert_eq!(
            serde_json::to_value(control.poll_link().await.unwrap()).unwrap(),
            json!({"status": "idle"})
        );

        pending(&link, &base_url).await;
        link.pending.lock().await.as_mut().unwrap().window = std::time::Duration::ZERO;
        assert_eq!(
            serde_json::to_value(control.poll_link().await.unwrap()).unwrap(),
            json!({"status": "expired"})
        );
        assert!(link.pending.lock().await.is_none());

        pending(&link, &base_url).await;
        link.pending
            .lock()
            .await
            .as_mut()
            .unwrap()
            .base_url
            .push_str("/missing");
        assert!(matches!(
            control.poll_link().await,
            Err(ControlError::Link(_))
        ));
        assert!(link.pending.lock().await.is_some());
        assert!(matches!(
            control.start_link(&base_url).await,
            Err(ControlError::Link(_))
        ));
        assert!(link.pending.lock().await.is_some());
        assert!(control.settings().await.unwrap().is_none());
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }
}
