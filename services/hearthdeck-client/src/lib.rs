use std::sync::Arc;

use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    configured_token: String,
    client_name: &'static str,
    token: Arc<OnceCell<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("connection failed: {0}")]
    Connection(reqwest::Error),
    #[error("unexpected status: {0}")]
    UnexpectedStatus(reqwest::StatusCode),
    #[error("{status}: {message}")]
    Api {
        status: reqwest::StatusCode,
        message: String,
    },
    #[error("deserialization failed: {0}")]
    Deserialization(reqwest::Error),
    #[error("invalid daemon base URL: {0}")]
    InvalidBaseUrl(url::ParseError),
    #[error("invalid pairing token: {0}")]
    InvalidToken(reqwest::header::InvalidHeaderValue),
    #[error("automatic pairing requires a loopback daemon")]
    PairingRequiresLoopback,
}

#[derive(Deserialize)]
struct PairingCodeResponse {
    code: String,
}

#[derive(Serialize)]
struct CompletePairingRequest<'a> {
    code: &'a str,
    client_name: &'a str,
}

#[derive(Deserialize)]
struct PairingCompleteResponse {
    token: String,
}

impl Client {
    #[must_use]
    pub fn new(
        http: reqwest::Client,
        base_url: String,
        token: String,
        client_name: &'static str,
    ) -> Self {
        Self {
            http,
            base_url,
            configured_token: token,
            client_name,
            token: Arc::new(OnceCell::new()),
        }
    }

    #[must_use]
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    #[must_use]
    pub fn api_url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// # Errors
    /// Returns pairing, transport, or invalid-token errors without retrying.
    pub async fn auth_headers(&self) -> Result<HeaderMap, Error> {
        let token = self
            .token
            .get_or_try_init(|| async {
                if self.configured_token.is_empty() {
                    self.pair_local().await
                } else {
                    Ok(self.configured_token.clone())
                }
            })
            .await?;
        let mut headers = HeaderMap::new();
        let mut value =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(Error::InvalidToken)?;
        value.set_sensitive(true);
        headers.insert(AUTHORIZATION, value);
        Ok(headers)
    }

    async fn pair_local(&self) -> Result<String, Error> {
        let mut api_url = url::Url::parse(&self.base_url).map_err(Error::InvalidBaseUrl)?;
        if !matches!(
            api_url.host_str(),
            Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
        ) {
            return Err(Error::PairingRequiresLoopback);
        }
        let mut admin_url = api_url.clone();
        admin_url
            .set_port(Some(38401))
            .map_err(|_| Error::PairingRequiresLoopback)?;
        admin_url.set_path("/v1/pairing");
        admin_url.set_query(None);
        api_url.set_path("/v1/pairing/complete");
        api_url.set_query(None);
        self.pair_at(admin_url, api_url).await
    }

    async fn pair_at(&self, admin_url: url::Url, api_url: url::Url) -> Result<String, Error> {
        let response = self
            .http
            .post(admin_url)
            .send()
            .await
            .map_err(Error::Connection)?;
        if !response.status().is_success() {
            return Err(response_error(response).await);
        }
        let pairing = response
            .json::<PairingCodeResponse>()
            .await
            .map_err(Error::Deserialization)?;
        let response = self
            .http
            .post(api_url)
            .json(&CompletePairingRequest {
                code: &pairing.code,
                client_name: self.client_name,
            })
            .send()
            .await
            .map_err(Error::Connection)?;
        if !response.status().is_success() {
            return Err(response_error(response).await);
        }
        response
            .json::<PairingCompleteResponse>()
            .await
            .map(|pairing| pairing.token)
            .map_err(Error::Deserialization)
    }

    /// # Errors
    /// Returns authorization, transport, or daemon rejection errors. No retries:
    /// a timeout does not prove that the bridge failed to stop the session.
    pub async fn stop_active_session(&self) -> Result<(), Error> {
        let response = self
            .http
            .post(self.api_url("/v1/sessions/active/stop"))
            .headers(self.auth_headers().await?)
            .send()
            .await
            .map_err(Error::Connection)?;
        if response.status() == reqwest::StatusCode::ACCEPTED {
            Ok(())
        } else {
            Err(response_error(response).await)
        }
    }
}

/// Preserves daemon rejection messages, or the status for non-daemon responses.
pub async fn response_error(response: reqwest::Response) -> Error {
    let status = response.status();
    let body = match response.bytes().await {
        Ok(body) => body,
        Err(error) => return Error::Connection(error),
    };
    error_from_body(status, &body)
}

fn error_from_body(status: reqwest::StatusCode, body: &[u8]) -> Error {
    #[derive(Deserialize)]
    struct ErrorBody {
        error: String,
    }
    match serde_json::from_slice::<ErrorBody>(body) {
        Ok(body) if !body.error.trim().is_empty() => Error::Api {
            status,
            message: body.error.trim().to_owned(),
        },
        _ => Error::UnexpectedStatus(status),
    }
}

#[cfg(test)]
mod tests {
    use super::{Client, Error};
    use axum::{
        Json, Router,
        http::{HeaderMap, StatusCode},
        routing::post,
    };
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    fn client(base_url: String, token: &str) -> Client {
        Client::new(
            reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            base_url,
            token.to_owned(),
            "test-client",
        )
    }

    async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (format!("http://{address}"), task)
    }

    #[test]
    fn daemon_errors_keep_the_reason_and_other_bodies_keep_the_status() {
        let status = StatusCode::BAD_GATEWAY;
        assert_eq!(
            super::error_from_body(
                status,
                br#"{"error":"input compatibility service failed to start"}"#
            )
            .to_string(),
            "502 Bad Gateway: input compatibility service failed to start",
        );
        for body in [
            b"<html>proxy error</html>".as_slice(),
            b"",
            br#"{"error":" "}"#,
        ] {
            assert!(matches!(
                super::error_from_body(status, body),
                Error::UnexpectedStatus(StatusCode::BAD_GATEWAY)
            ));
        }
    }

    #[tokio::test]
    async fn configured_and_cached_tokens_are_shared_by_clones_and_marked_sensitive() {
        let client = client("http://127.0.0.1:0".to_owned(), "configured");
        for instance in [client.clone(), client.clone()] {
            let headers = instance.auth_headers().await.unwrap();
            assert_eq!(headers["authorization"], "Bearer configured");
            assert!(headers["authorization"].is_sensitive());
        }
        let cached = super::Client::new(
            client.http.clone(),
            client.base_url.clone(),
            String::new(),
            "test-client",
        );
        cached.token.set("cached".to_owned()).unwrap();
        assert_eq!(
            cached.clone().auth_headers().await.unwrap()["authorization"],
            "Bearer cached"
        );
    }

    #[tokio::test]
    async fn invalid_tokens_and_remote_automatic_pairing_fail_before_network_io() {
        assert!(matches!(
            client("http://127.0.0.1:0".to_owned(), "bad\nheader")
                .auth_headers()
                .await,
            Err(Error::InvalidToken(_))
        ));
        assert!(matches!(
            client("http://192.0.2.1:38400".to_owned(), "")
                .auth_headers()
                .await,
            Err(Error::PairingRequiresLoopback)
        ));
        assert!(matches!(
            client("not a url".to_owned(), "").auth_headers().await,
            Err(Error::InvalidBaseUrl(_))
        ));
    }

    #[tokio::test]
    async fn local_pairing_exchanges_the_code_with_the_callers_identity() {
        let app = Router::new()
            .route(
                "/v1/pairing",
                post(|| async { Json(serde_json::json!({"code": "123456"})) }),
            )
            .route(
                "/v1/pairing/complete",
                post(|Json(body): Json<serde_json::Value>| async move {
                    assert_eq!(
                        body,
                        serde_json::json!({"code": "123456", "client_name": "test-client"})
                    );
                    Json(serde_json::json!({"token": "paired-token"}))
                }),
            );
        let (base, server) = serve(app).await;
        let client = client(base.clone(), "");
        assert_eq!(
            client
                .pair_at(
                    format!("{base}/v1/pairing").parse().unwrap(),
                    format!("{base}/v1/pairing/complete").parse().unwrap(),
                )
                .await
                .unwrap(),
            "paired-token",
        );
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn pairing_preserves_rejections_and_malformed_replies_at_both_steps() {
        for (admin_status, admin_body, complete_status, complete_body, expected_status) in [
            (
                StatusCode::FORBIDDEN,
                r#"{"error":"pairing rejected"}"#,
                StatusCode::OK,
                "unused",
                Some(StatusCode::FORBIDDEN),
            ),
            (StatusCode::OK, "invalid", StatusCode::OK, "unused", None),
            (
                StatusCode::OK,
                r#"{"code":"123456"}"#,
                StatusCode::FORBIDDEN,
                r#"{"error":"pairing rejected"}"#,
                Some(StatusCode::FORBIDDEN),
            ),
            (
                StatusCode::OK,
                r#"{"code":"123456"}"#,
                StatusCode::OK,
                "invalid",
                None,
            ),
        ] {
            let app = Router::new()
                .route(
                    "/v1/pairing",
                    post(move || async move { (admin_status, admin_body) }),
                )
                .route(
                    "/v1/pairing/complete",
                    post(move || async move { (complete_status, complete_body) }),
                );
            let (base, server) = serve(app).await;
            let error = client(base.clone(), "")
                .pair_at(
                    format!("{base}/v1/pairing").parse().unwrap(),
                    format!("{base}/v1/pairing/complete").parse().unwrap(),
                )
                .await
                .unwrap_err();
            match expected_status {
                Some(expected) => assert!(
                    matches!(error, Error::Api { status, message } if status == expected && message == "pairing rejected")
                ),
                None => assert!(matches!(error, Error::Deserialization(_))),
            }
            server.abort();
            assert!(server.await.unwrap_err().is_cancelled());
        }
    }

    #[tokio::test]
    async fn stop_active_uses_one_authenticated_post_and_preserves_rejections() {
        for (status, body) in [
            (StatusCode::ACCEPTED, ""),
            (StatusCode::NOT_FOUND, r#"{"error":"no active session"}"#),
            (
                StatusCode::BAD_GATEWAY,
                r#"{"error":"systemd stop failed"}"#,
            ),
            (StatusCode::UNAUTHORIZED, r#"{"error":"invalid token"}"#),
            (StatusCode::OK, "unexpected response"),
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let observed = calls.clone();
            let app = Router::new().route(
                "/v1/sessions/active/stop",
                post(move |headers: HeaderMap| {
                    observed.fetch_add(1, Ordering::SeqCst);
                    async move {
                        assert_eq!(headers["authorization"], "Bearer configured");
                        (status, body)
                    }
                }),
            );
            let (base, server) = serve(app).await;
            let result = client(base, "configured").stop_active_session().await;
            match status {
                StatusCode::ACCEPTED => result.unwrap(),
                StatusCode::OK => assert!(matches!(
                    result,
                    Err(Error::UnexpectedStatus(StatusCode::OK))
                )),
                _ => match result.unwrap_err() {
                    Error::Api {
                        status: actual,
                        message,
                    } => {
                        assert_eq!(actual, status);
                        assert_eq!(
                            message,
                            serde_json::from_str::<serde_json::Value>(body).unwrap()["error"]
                                .as_str()
                                .unwrap()
                        );
                    }
                    error => panic!("expected daemon rejection, got {error}"),
                },
            }
            assert_eq!(calls.load(Ordering::SeqCst), 1, "status: {status}");
            server.abort();
            assert!(server.await.unwrap_err().is_cancelled());
        }
    }

    #[tokio::test]
    async fn a_stalled_stop_times_out_without_retrying() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let app = Router::new().route(
            "/v1/sessions/active/stop",
            post(move || {
                observed.fetch_add(1, Ordering::SeqCst);
                std::future::pending::<StatusCode>()
            }),
        );
        let (base, server) = serve(app).await;
        let client = Client::new(
            reqwest::Client::builder()
                .timeout(Duration::from_millis(100))
                .build()
                .unwrap(),
            base,
            "configured".to_owned(),
            "test-client",
        );
        let result = tokio::time::timeout(Duration::from_secs(5), client.stop_active_session())
            .await
            .unwrap();
        assert!(matches!(result, Err(Error::Connection(error)) if error.is_timeout()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }
}
