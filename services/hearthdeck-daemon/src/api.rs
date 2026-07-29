use axum::{
    Json, Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use hearthdeck_protocol::{BridgeRequest, BridgeResponse};
use rand::{RngExt, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;

use crate::state::{ServerEvent, SharedState};

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/pairing/complete", post(complete_pairing))
        .route("/v1/library", get(list_library))
        .route("/v1/library/rescan", post(rescan_library))
        .route("/v1/discovery/{source_id}/refresh", post(refresh_source))
        .route("/v1/metadata/{provider_id}/refresh", post(refresh_metadata))
        .route("/v1/apps/{id}/launch", post(launch_app))
        .route("/v1/events", get(events))
        .with_state(state)
}

/// Routes served only on the loopback admin listener. A remote device can
/// submit a code, but cannot mint one without host-side user approval.
pub fn local_router(state: SharedState) -> Router {
    Router::new()
        .route("/v1/pairing", post(create_pairing))
        .with_state(state)
}

async fn health(State(state): State<SharedState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        version: env!("CARGO_PKG_VERSION"),
        lan_enabled: state.config.lan_enabled,
        transport: if state.config.lan_enabled {
            "https"
        } else {
            "http"
        },
    })
}

async fn create_pairing(
    State(state): State<SharedState>,
) -> Result<Json<PairingResponse>, ApiError> {
    let code: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(12)
        .map(char::from)
        .collect::<String>()
        .to_uppercase();
    let expires_at = state
        .auth
        .create_pairing_code(hash_secret(&code))
        .await
        .map_err(ApiError::internal)?;
    info!(expires_at = %expires_at, "local pairing code created");
    Ok(Json(PairingResponse { code, expires_at }))
}

async fn complete_pairing(
    State(state): State<SharedState>,
    Json(request): Json<CompletePairingRequest>,
) -> Result<Json<PairingCompleteResponse>, ApiError> {
    if request.client_name.trim().is_empty() || request.client_name.chars().count() > 128 {
        return Err(ApiError::invalid_pairing_request());
    }
    let token = format!("hearthdeck_{}", Uuid::new_v4().simple());
    let paired_client = state
        .auth
        .consume_pairing_code(
            &hash_secret(&request.code),
            request.client_name,
            hash_secret(&token),
        )
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(ApiError::unauthorized)?;
    info!(client_id = %paired_client.client_id, "client paired");
    Ok(Json(PairingCompleteResponse {
        client_id: paired_client.client_id,
        token,
    }))
}

async fn list_library(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::catalog::CatalogItem>>, ApiError> {
    authenticate(&state, &headers).await?;
    let items = state.catalog.list().await.map_err(ApiError::internal)?;
    info!(item_count = items.len(), "catalog listed");
    Ok(Json(items))
}

async fn rescan_library(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    authenticate(&state, &headers).await?;
    state
        .discovery
        .as_ref()
        .ok_or_else(ApiError::service_unavailable)?
        .request_all()
        .await;
    if let Some(enrichment) = &state.enrichment {
        enrichment.request_all().await;
    }
    info!("all discovery and metadata providers refresh requested");
    Ok(StatusCode::ACCEPTED)
}

async fn refresh_source(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    authenticate(&state, &headers).await?;
    match state
        .discovery
        .as_ref()
        .ok_or_else(ApiError::service_unavailable)?
        .request(&source_id)
        .await
    {
        crate::discovery::RefreshRequest::UnknownProvider => {
            warn!(source_id, "unknown discovery provider requested");
            Err(ApiError::not_found())
        }
        crate::discovery::RefreshRequest::Queued => {
            info!(source_id, "discovery provider refresh queued");
            Ok(StatusCode::ACCEPTED)
        }
        crate::discovery::RefreshRequest::AlreadyScheduled => {
            info!(source_id, "discovery provider refresh coalesced");
            Ok(StatusCode::ACCEPTED)
        }
    }
}

async fn refresh_metadata(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    authenticate(&state, &headers).await?;
    match state
        .enrichment
        .as_ref()
        .ok_or_else(ApiError::service_unavailable)?
        .request(&provider_id)
        .await
    {
        crate::enrichment::EnrichmentRequest::UnknownProvider => {
            warn!(provider_id, "unknown metadata provider requested");
            Err(ApiError::not_found())
        }
        crate::enrichment::EnrichmentRequest::Queued => {
            info!(provider_id, "metadata provider refresh queued");
            Ok(StatusCode::ACCEPTED)
        }
        crate::enrichment::EnrichmentRequest::AlreadyScheduled => {
            info!(provider_id, "metadata provider refresh coalesced");
            Ok(StatusCode::ACCEPTED)
        }
    }
}

async fn launch_app(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    authenticate(&state, &headers).await?;
    let launch_id = state
        .catalog
        .launch_id_for(&id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(ApiError::not_found)?;
    let response = crate::bridge::request(
        &state.config.bridge_socket_path,
        BridgeRequest::LaunchApplication {
            source_id: state
                .catalog
                .source_id_for(&id)
                .await
                .map_err(ApiError::internal)?
                .ok_or_else(ApiError::not_found)?,
            application_id: launch_id,
        },
    )
    .await
    .map_err(ApiError::bad_gateway)?;
    if !matches!(response, BridgeResponse::LaunchAccepted { .. }) {
        return Err(ApiError::bad_gateway("bridge rejected application launch"));
    }
    info!(item_id = %id, "catalog launch accepted");
    let _ = state
        .events
        .send(ServerEvent::ActionCompleted { item_id: id });
    Ok(StatusCode::ACCEPTED)
}

async fn events(
    State(state): State<SharedState>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    authenticate(&state, &headers).await?;
    Ok(websocket.on_upgrade(move |socket| event_socket(socket, state.events.subscribe())))
}

async fn event_socket(
    mut socket: WebSocket,
    mut events: tokio::sync::broadcast::Receiver<ServerEvent>,
) {
    while let Ok(event) = events.recv().await {
        let Ok(payload) = serde_json::to_string(&event) else {
            continue;
        };
        if socket.send(Message::Text(payload.into())).await.is_err() {
            return;
        }
    }
}

async fn authenticate(state: &SharedState, headers: &HeaderMap) -> Result<(), ApiError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(ApiError::unauthorized)?;
    if !state
        .auth
        .authenticate(hash_secret(token))
        .await
        .map_err(ApiError::internal)?
    {
        return Err(ApiError::unauthorized());
    }
    Ok(())
}

fn hash_secret(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}

#[derive(Serialize)]
struct HealthResponse {
    version: &'static str,
    lan_enabled: bool,
    transport: &'static str,
}

#[derive(Serialize)]
struct PairingResponse {
    code: String,
    expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct CompletePairingRequest {
    code: String,
    client_name: String,
}

#[derive(Serialize)]
struct PairingCompleteResponse {
    client_id: String,
    token: String,
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "authentication required".to_owned(),
        }
    }

    fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "resource not found".to_owned(),
        }
    }

    fn invalid_pairing_request() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "client_name must contain 1 to 128 characters".to_owned(),
        }
    }

    fn service_unavailable() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "discovery service is unavailable".to_owned(),
        }
    }

    fn bad_gateway(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, sync::Arc, time::Duration};

    use async_trait::async_trait;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tokio::time::timeout;
    use tower::ServiceExt;

    use super::{local_router, router};
    use crate::{
        catalog::CatalogRecord,
        config::Config,
        database::Database,
        discovery::{DiscoveryProvider, DiscoveryService},
        state::{AppState, SharedState},
    };

    async fn response_json(
        app: axum::Router,
        request: Request<Body>,
    ) -> (StatusCode, serde_json::Value) {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&body).unwrap();
        (status, json)
    }

    struct ApiTestProvider;

    #[async_trait]
    impl DiscoveryProvider for ApiTestProvider {
        fn source_id(&self) -> &'static str {
            "test-apps"
        }

        fn refresh_interval(&self) -> Option<Duration> {
            None
        }

        async fn discover(&self) -> anyhow::Result<Vec<CatalogRecord>> {
            Ok(vec![CatalogRecord {
                id: "test:app".to_owned(),
                title: "Hearthdeck Test App".to_owned(),
                kind: "application".to_owned(),
                launch_id: Some("test.app".to_owned()),
                icon: None,
                metadata: serde_json::Value::Null,
                updated_at: "2026-01-01T00:00:00Z".to_owned(),
            }])
        }
    }

    #[tokio::test]
    async fn pairing_and_rescan_require_the_intended_boundaries() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let base_state = AppState::new(
            Config {
                bind_address: "127.0.0.1:38400".parse::<SocketAddr>().unwrap(),
                local_admin_address: "127.0.0.1:38401".parse::<SocketAddr>().unwrap(),
                database_path: temporary.path().join("hearthdeck.db"),
                bridge_socket_path: temporary.path().join("bridge.sock"),
                lan_enabled: false,
                tls: None,
            },
            database,
        );
        let discovery = DiscoveryService::start(
            vec![Arc::new(ApiTestProvider)],
            base_state.catalog.clone(),
            base_state.events.clone(),
        );
        let state: SharedState = Arc::new(base_state.with_discovery(discovery));

        let public_pairing = router(state.clone())
            .oneshot(Request::post("/v1/pairing").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(public_pairing.status(), StatusCode::NOT_FOUND);

        let (status, pairing) = response_json(
            local_router(state.clone()),
            Request::post("/v1/pairing").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let code = pairing["code"].as_str().unwrap();
        assert_eq!(code.len(), 12);

        let (status, paired) = response_json(
            router(state.clone()),
            Request::post("/v1/pairing/complete")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"code": code, "client_name": "test-client"}).to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let token = paired["token"].as_str().unwrap();
        let mut events = state.events.subscribe();

        let rescan = router(state.clone())
            .oneshot(
                Request::post("/v1/library/rescan")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rescan.status(), StatusCode::ACCEPTED);
        timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();

        let (status, library) = response_json(
            router(state),
            Request::get("/v1/library")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(library[0]["id"], "test:app");
    }
}
