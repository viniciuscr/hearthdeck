use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Datelike, Utc};
use hearthdeck_protocol::{
    ApplicationSession, BridgeRequest, BridgeResponse, HeroicRunner, InputProfile,
};
use rand::{RngExt, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    activity::{ActivityEntry, RecentActivity},
    diagnostics::{self, RommGame, RommPlatform, RommQueryError},
    retro::RetroLaunchError,
    settings::{
        BackdropMode, RommSettings, SettingsChange, SettingsUpdate, ThemeMode, UserSettings,
    },
    state::{ProviderHealth, ServerEvent, SharedState},
};

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/diagnostics", get(diagnostics))
        .route("/v1/pairing/complete", post(complete_pairing))
        .route("/v1/library", get(list_library))
        .route("/v1/activity/recent", get(list_recent_activity))
        .route("/v1/retro/consoles", get(list_retro_consoles))
        .route("/v1/retro/roms", get(list_retro_roms))
        .route("/v1/retro/roms/{id}/launch", post(launch_retro_rom))
        .route("/v1/retro/service/restart", post(restart_romm_service))
        .route("/v1/retro/assets", get(retro_asset))
        .route(
            "/v1/retro/settings",
            get(get_romm_settings)
                .put(update_romm_settings)
                .delete(clear_romm_settings),
        )
        .route("/v1/library/rescan", post(rescan_library))
        .route("/v1/settings", get(get_settings).put(update_settings))
        .route("/v1/discovery/{source_id}/refresh", post(refresh_source))
        .route("/v1/metadata/{provider_id}/refresh", post(refresh_metadata))
        .route("/v1/apps/{id}/launch", post(launch_app))
        .route("/v1/sessions/active", get(active_application_session))
        .route("/v1/sessions/{id}/stop", post(stop_application_session))
        .route("/v1/install-requests", post(request_install))
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
        providers: state.provider_health().await,
        capabilities: host_capabilities(),
    })
}

#[cfg(target_os = "linux")]
fn host_capabilities() -> HostCapabilities {
    HostCapabilities {
        launch: true,
        application_sessions: true,
        install_requests: false,
        retro_launch: true,
    }
}

#[cfg(target_os = "macos")]
fn host_capabilities() -> HostCapabilities {
    HostCapabilities {
        launch: true,
        application_sessions: false,
        install_requests: false,
        retro_launch: false,
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn host_capabilities() -> HostCapabilities {
    HostCapabilities {
        launch: false,
        application_sessions: false,
        install_requests: false,
        retro_launch: false,
    }
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

async fn list_recent_activity(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(query): Query<RecentActivityQuery>,
) -> Result<Json<Vec<RecentActivity>>, ApiError> {
    authenticate(&state, &headers).await?;
    let items = state
        .activity
        .recent(query.limit.unwrap_or(6))
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(items))
}

async fn list_retro_consoles(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Vec<RommPlatform>>, ApiError> {
    authenticate(&state, &headers).await?;
    let mut platforms = diagnostics::romm_platforms(&state.settings)
        .await
        .map_err(ApiError::romm_query)?;
    platforms.sort_by(|left, right| {
        left.display_name
            .as_deref()
            .unwrap_or(&left.name)
            .cmp(right.display_name.as_deref().unwrap_or(&right.name))
    });
    Ok(Json(platforms))
}

async fn list_retro_roms(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<RommGamesQuery>,
) -> Result<Json<RommGamesResponse>, ApiError> {
    authenticate(&state, &headers).await?;
    let limit = query.limit.unwrap_or(48).clamp(1, 100);
    let offset = query.offset.unwrap_or(0);
    let search_term = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let games = diagnostics::romm_games(
        &state.settings,
        query.platform_id,
        search_term,
        limit,
        offset,
    )
    .await
    .map_err(ApiError::romm_query)?;
    Ok(Json(RommGamesResponse {
        items: games.items.iter().map(RetroGame::from).collect(),
        total: games.total,
        limit: games.limit,
        offset: games.offset,
    }))
}

/// Launches a RomM ROM through RetroArch. Deliberately not the generic
/// `/v1/apps/{id}/launch` path: RomM is not a `DiscoveryProvider`/
/// `CatalogRecord` source (see docs/retroarch-integration.md decision 6),
/// so this takes RomM's own rom ID directly and reuses only the
/// source-agnostic session/bridge machinery the generic route also uses.
async fn launch_retro_rom(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(rom_id): Path<i64>,
    Query(options): Query<LaunchOptions>,
) -> Result<Json<ApplicationSession>, ApiError> {
    authenticate(&state, &headers).await?;
    if !host_capabilities().retro_launch {
        return Err(ApiError::capability_unavailable("retro game launch"));
    }
    let plan = crate::retro::prepare_launch(&state.settings, rom_id)
        .await
        .map_err(ApiError::retro_launch)?;
    let activity = retro_activity_entry(&plan.game);
    let session_id = Uuid::new_v4().to_string();
    let request = BridgeRequest::LaunchRetroGame {
        core_path: plan.core_path.to_string_lossy().into_owned(),
        rom_path: plan.rom_path.to_string_lossy().into_owned(),
        session_id,
        input_profile: options.input_profile,
    };
    let response = crate::bridge::request(&state.config.bridge_socket_path, request)
        .await
        .map_err(ApiError::bad_gateway)?;
    let BridgeResponse::LaunchAccepted { session } = response else {
        return Err(ApiError::bad_gateway("bridge rejected retro game launch"));
    };
    if let Err(error) = state.activity.record_launch(activity).await {
        warn!(%error, rom_id, "failed to record retro launch activity");
    }
    info!(rom_id, "retro game launch accepted");
    let _ = state.events.send(ServerEvent::ApplicationSessionChanged {
        session: Some(session.clone()),
    });
    Ok(Json(session))
}

/// Restarts the fixed `romm.service` systemd unit
/// (`deploy/systemd/romm.service`), if the user has configured it.
/// Not a generic "restart any unit" endpoint: the unit name is hardcoded in
/// `diagnostics::restart_romm_service`, never taken from the request.
async fn restart_romm_service(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    authenticate(&state, &headers).await?;
    diagnostics::restart_romm_service()
        .await
        .map_err(ApiError::bad_gateway)?;
    info!("romm.service restart requested");
    Ok(StatusCode::ACCEPTED)
}

async fn retro_asset(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<RommAssetQuery>,
) -> Result<Response, ApiError> {
    authenticate(&state, &headers).await?;
    let asset = diagnostics::romm_asset(&state.settings, &query.path)
        .await
        .map_err(ApiError::romm_query)?;
    let content_type = HeaderValue::from_str(&asset.content_type)
        .map_err(|_| ApiError::bad_gateway("RomM returned an invalid image type"))?;
    let mut response = asset.bytes.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    Ok(response)
}

async fn diagnostics(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<diagnostics::DiagnosticsSnapshot>, ApiError> {
    authenticate(&state, &headers).await?;
    Ok(Json(diagnostics::snapshot(&state.settings).await))
}

async fn get_romm_settings(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Option<RommSettings>>, ApiError> {
    authenticate(&state, &headers).await?;
    state
        .settings
        .romm()
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

async fn update_romm_settings(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<UpdateRommSettingsRequest>,
) -> Result<Json<RommSettings>, ApiError> {
    authenticate(&state, &headers).await?;
    state
        .settings
        .save_romm(&request.base_url, &request.token)
        .await
        .map(Json)
        .map_err(ApiError::invalid_romm_settings)
}

async fn clear_romm_settings(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    authenticate(&state, &headers).await?;
    state
        .settings
        .clear_romm()
        .await
        .map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
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

async fn get_settings(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<UserSettings>, ApiError> {
    authenticate(&state, &headers).await?;
    state
        .settings
        .get()
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

async fn update_settings(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<UpdateSettingsRequest>,
) -> Result<Json<UserSettings>, ApiError> {
    authenticate(&state, &headers).await?;
    let theme_mode = request
        .theme_mode
        .as_deref()
        .map(|mode| ThemeMode::parse(mode).ok_or_else(ApiError::invalid_theme_mode))
        .transpose()?;
    let backdrop_mode = request
        .backdrop_mode
        .as_deref()
        .map(|mode| BackdropMode::parse(mode).ok_or_else(ApiError::invalid_backdrop_mode))
        .transpose()?;
    let change = SettingsChange {
        theme_mode,
        backdrop_mode,
    };
    if change.is_empty() {
        return Err(ApiError::empty_settings_update());
    }
    match state
        .settings
        .update(change, request.revision)
        .await
        .map_err(ApiError::internal)?
    {
        SettingsUpdate::Saved(settings) => Ok(Json(settings)),
        SettingsUpdate::Conflict(settings) => Err(ApiError::settings_conflict(settings)),
    }
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
    Query(options): Query<LaunchOptions>,
) -> Result<Json<ApplicationSession>, ApiError> {
    authenticate(&state, &headers).await?;
    if !host_capabilities().launch {
        return Err(ApiError::capability_unavailable("application launch"));
    }
    let item = state
        .catalog
        .get(&id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(ApiError::not_found)?;
    let launch_id = item.launch_id.as_deref().ok_or_else(ApiError::not_found)?;
    let session_id = Uuid::new_v4().to_string();
    let request = if item.source_id == "heroic" {
        let (runner, application_id) =
            heroic_launch_target(launch_id).ok_or_else(ApiError::not_found)?;
        BridgeRequest::LaunchHeroicGame {
            runner,
            application_id,
            session_id,
            input_profile: options.input_profile,
        }
    } else {
        BridgeRequest::LaunchApplication {
            source_id: item.source_id.clone(),
            application_id: launch_id.to_owned(),
            session_id,
            input_profile: options.input_profile,
        }
    };
    let response = crate::bridge::request(&state.config.bridge_socket_path, request)
        .await
        .map_err(ApiError::bad_gateway)?;
    let BridgeResponse::LaunchAccepted { session } = response else {
        return Err(ApiError::bad_gateway("bridge rejected application launch"));
    };
    if let Err(error) = state
        .activity
        .record_launch(catalog_activity_entry(&item))
        .await
    {
        warn!(%error, item_id = %id, "failed to record catalog launch activity");
    }
    info!(item_id = %id, "catalog launch accepted");
    let _ = state.events.send(ServerEvent::ApplicationSessionChanged {
        session: Some(session.clone()),
    });
    Ok(Json(session))
}

#[derive(Default, Deserialize)]
struct LaunchOptions {
    #[serde(default)]
    input_profile: InputProfile,
}

#[derive(Deserialize)]
struct RecentActivityQuery {
    limit: Option<u32>,
}

fn catalog_activity_entry(item: &crate::catalog::CatalogItem) -> ActivityEntry {
    let categories = item
        .metadata
        .get("categories")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned)
        .collect();
    ActivityEntry {
        id: format!("hearthdeck:{}", item.id),
        title: item.title.clone(),
        icon: item.icon.clone(),
        categories,
        source: item.source_id.clone(),
        metadata: item.metadata.clone(),
    }
}

fn retro_activity_entry(game: &RommGame) -> ActivityEntry {
    let game = RetroGame::from(game);
    ActivityEntry {
        id: format!("romm:{}", game.id),
        title: game.title,
        icon: game.cover_path,
        categories: vec![
            "Game".to_owned(),
            format!("hearthdeck-console:{}", game.platform_id),
        ],
        source: "romm".to_owned(),
        metadata: serde_json::json!({
            "summary": game.summary,
            "genres": game.genres,
            "release_year": game.release_year,
            "screenshot_paths": game.screenshot_paths,
        }),
    }
}

fn heroic_launch_target(launch_id: &str) -> Option<(HeroicRunner, String)> {
    let (runner, application_id) = launch_id.split_once(':')?;
    if application_id.is_empty() {
        return None;
    }
    let runner = match runner {
        "legendary" => HeroicRunner::Legendary,
        "gog" => HeroicRunner::Gog,
        _ => return None,
    };
    Some((runner, application_id.to_owned()))
}

async fn active_application_session(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Option<ApplicationSession>>, ApiError> {
    authenticate(&state, &headers).await?;
    if !host_capabilities().application_sessions {
        return Err(ApiError::capability_unavailable("application sessions"));
    }
    let response = crate::bridge::request(
        &state.config.bridge_socket_path,
        BridgeRequest::ActiveApplicationSession,
    )
    .await
    .map_err(ApiError::bad_gateway)?;
    let BridgeResponse::ApplicationSession { session } = response else {
        return Err(ApiError::bad_gateway(
            "bridge rejected application-session lookup",
        ));
    };
    Ok(Json(session))
}

async fn stop_application_session(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    authenticate(&state, &headers).await?;
    if !host_capabilities().application_sessions {
        return Err(ApiError::capability_unavailable("application sessions"));
    }
    let response = crate::bridge::request(
        &state.config.bridge_socket_path,
        BridgeRequest::StopApplicationSession {
            session_id: id.clone(),
        },
    )
    .await
    .map_err(ApiError::bad_gateway)?;
    if !matches!(response, BridgeResponse::StopAccepted { .. }) {
        return Err(ApiError::bad_gateway(
            "bridge rejected application-session stop",
        ));
    }
    let _ = state
        .events
        .send(ServerEvent::ApplicationSessionChanged { session: None });
    Ok(StatusCode::ACCEPTED)
}

async fn request_install(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    authenticate(&state, &headers).await?;
    Err(ApiError::capability_unavailable("install requests"))
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
    loop {
        match events.recv().await {
            Ok(event) => {
                let Ok(payload) = serde_json::to_string(&event) else {
                    continue;
                };
                if socket.send(Message::Text(payload.into())).await.is_err() {
                    return;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                warn!(
                    skipped,
                    "event client lagged; resuming with the newest event"
                );
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
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
    providers: Vec<ProviderHealth>,
    capabilities: HostCapabilities,
}

#[derive(Clone, Copy, Serialize)]
struct HostCapabilities {
    launch: bool,
    application_sessions: bool,
    install_requests: bool,
    retro_launch: bool,
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

#[derive(Deserialize)]
struct UpdateSettingsRequest {
    theme_mode: Option<String>,
    backdrop_mode: Option<String>,
    revision: Option<i64>,
}

#[derive(Deserialize)]
struct UpdateRommSettingsRequest {
    base_url: String,
    token: String,
}

#[derive(Deserialize)]
struct RommGamesQuery {
    platform_id: Option<i64>,
    #[serde(default)]
    q: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

#[derive(Deserialize)]
struct RommAssetQuery {
    path: String,
}

#[derive(Serialize)]
struct RommGamesResponse {
    items: Vec<RetroGame>,
    total: u64,
    limit: u32,
    offset: u32,
}

#[derive(Serialize)]
struct RetroGame {
    id: i64,
    platform_id: i64,
    title: String,
    summary: Option<String>,
    cover_path: Option<String>,
    cover_url: Option<String>,
    screenshot_paths: Vec<String>,
    has_manual: bool,
    genres: Vec<String>,
    player_count: Option<String>,
    release_year: Option<i32>,
    regions: Vec<String>,
}

impl From<&RommGame> for RetroGame {
    fn from(game: &RommGame) -> Self {
        let title = game
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&game.fs_name_no_tags)
            .to_owned();
        let player_count = (!game.metadatum.player_count.trim().is_empty())
            .then(|| game.metadatum.player_count.clone());
        let release_year = game
            .metadatum
            .first_release_date
            .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
            .map(|date| date.year());
        Self {
            id: game.id,
            platform_id: game.platform_id,
            title,
            summary: game.summary.clone(),
            cover_path: non_empty_romm_path(game.path_cover_small.as_ref())
                .or_else(|| non_empty_romm_path(game.path_cover_large.as_ref())),
            cover_url: http_url(game.url_cover.as_deref()),
            screenshot_paths: game
                .merged_screenshots
                .iter()
                .filter_map(|path| non_empty_romm_path(Some(path)))
                .collect(),
            has_manual: game.has_manual
                || game
                    .path_manual
                    .as_deref()
                    .is_some_and(|path| !path.trim().is_empty()),
            genres: game.metadatum.genres.clone(),
            player_count,
            release_year,
            regions: game.regions.clone(),
        }
    }
}

fn non_empty_romm_path(value: Option<&String>) -> Option<String> {
    value
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
}

fn http_url(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    let url = reqwest::Url::parse(value).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

struct ApiError {
    status: StatusCode,
    message: String,
    settings: Option<UserSettings>,
}

impl ApiError {
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "authentication required".to_owned(),
            settings: None,
        }
    }

    fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "resource not found".to_owned(),
            settings: None,
        }
    }

    fn invalid_pairing_request() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "client_name must contain 1 to 128 characters".to_owned(),
            settings: None,
        }
    }

    fn invalid_theme_mode() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "theme_mode must be system, aurora, ember, indigo, or noir".to_owned(),
            settings: None,
        }
    }

    fn invalid_backdrop_mode() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "backdrop_mode must be solid, edge_wash, or quiet_grid".to_owned(),
            settings: None,
        }
    }

    fn empty_settings_update() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "at least one settings field is required".to_owned(),
            settings: None,
        }
    }

    fn settings_conflict(settings: UserSettings) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: format!(
                "settings version conflict at revision {}",
                settings.revision
            ),
            settings: Some(settings),
        }
    }

    fn service_unavailable() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "discovery service is unavailable".to_owned(),
            settings: None,
        }
    }

    fn romm_unavailable() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "RomM is not configured".to_owned(),
            settings: None,
        }
    }

    fn romm_query(error: RommQueryError) -> Self {
        match error {
            RommQueryError::NotConfigured => Self::romm_unavailable(),
            RommQueryError::Failed(error) => Self::bad_gateway(error),
        }
    }

    fn retro_launch(error: RetroLaunchError) -> Self {
        let message = error.to_string();
        match error {
            RetroLaunchError::Romm(error) => Self::romm_query(error),
            RetroLaunchError::PlatformNotFound => Self::not_found(),
            RetroLaunchError::UnsupportedPlatform { .. } => Self {
                status: StatusCode::NOT_IMPLEMENTED,
                message,
                settings: None,
            },
            RetroLaunchError::CoreNotInstalled { .. } => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message,
                settings: None,
            },
            RetroLaunchError::RomHasNoContentFile | RetroLaunchError::InvalidContentFileName => {
                Self {
                    status: StatusCode::BAD_GATEWAY,
                    message,
                    settings: None,
                }
            }
            RetroLaunchError::Cache(_) => Self::internal(message),
        }
    }

    fn invalid_romm_settings(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: error.to_string(),
            settings: None,
        }
    }

    fn capability_unavailable(capability: &str) -> Self {
        Self {
            status: StatusCode::NOT_IMPLEMENTED,
            message: format!("{capability} are unavailable on this host"),
            settings: None,
        }
    }

    fn bad_gateway(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
            settings: None,
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
            settings: None,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = serde_json::json!({ "error": self.message });
        if let Some(settings) = self.settings {
            body["settings"] = serde_json::to_value(settings).unwrap_or_default();
        }
        (self.status, Json(body)).into_response()
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
    use hearthdeck_protocol::{
        ApplicationSession, ApplicationSessionState, BridgeRequest, BridgeResponse, HeroicRunner,
        InputProfile,
    };
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::UnixListener,
        time::timeout,
    };
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

    /// Accepts exactly one connection on `socket_path` (standing in for
    /// `hearthdeck-bridge`), hands the decoded request to `respond` (which
    /// should assert whatever it expects and build the reply - e.g. using
    /// the request's own freshly generated `session_id`, which the caller
    /// can't know in advance), and writes back whatever it returns. Panics
    /// (via the awaited `JoinHandle`) on a failed assertion inside
    /// `respond`, rather than a silent hang.
    fn spawn_fake_bridge(
        socket_path: std::path::PathBuf,
        respond: impl FnOnce(BridgeRequest) -> BridgeResponse + Send + 'static,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let listener = UnixListener::bind(&socket_path).unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let received: BridgeRequest = serde_json::from_str(&line).unwrap();
            let response = respond(received);
            let payload = serde_json::to_string(&response).unwrap();
            writer.write_all(payload.as_bytes()).await.unwrap();
            writer.write_all(b"\n").await.unwrap();
            writer.flush().await.unwrap();
        })
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

        let unauthenticated_settings = router(state.clone())
            .oneshot(Request::get("/v1/settings").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauthenticated_settings.status(), StatusCode::UNAUTHORIZED);

        let retro_without_romm = router(state.clone())
            .oneshot(
                Request::get("/v1/retro/consoles")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retro_without_romm.status(), StatusCode::SERVICE_UNAVAILABLE);

        // platform_id is optional so search can span every console; q is a
        // free-text search term forwarded to RomM. Reaching the "RomM isn't
        // configured" 503 (rather than a 400/422 query-parsing error) proves
        // both are accepted without platform_id.
        let roms_search_without_platform = router(state.clone())
            .oneshot(
                Request::get("/v1/retro/roms?q=mario")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            roms_search_without_platform.status(),
            StatusCode::SERVICE_UNAVAILABLE,
        );

        state
            .settings
            .save_romm("http://127.0.0.1:8080", "rmm_private_token")
            .await
            .unwrap();
        let (status, romm_settings) = response_json(
            router(state.clone()),
            Request::get("/v1/retro/settings")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(romm_settings["base_url"], "http://127.0.0.1:8080");
        assert_eq!(romm_settings["configured"], true);
        assert!(romm_settings.get("token").is_none());

        let (status, diagnostics) = response_json(
            router(state.clone()),
            Request::get("/v1/diagnostics")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(diagnostics["romm"]["configured"], true);
        assert!(diagnostics["romm"].get("token").is_none());

        let (status, settings) = response_json(
            router(state.clone()),
            Request::get("/v1/settings")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(settings["theme_mode"], "noir");
        assert_eq!(settings["backdrop_mode"], "solid");
        assert_eq!(settings["revision"], 0);

        let (status, settings) = response_json(
            router(state.clone()),
            Request::put("/v1/settings")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"theme_mode":"ember","backdrop_mode":"solid","revision":0}"#,
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(settings["theme_mode"], "ember");
        assert_eq!(settings["backdrop_mode"], "solid");
        assert_eq!(settings["revision"], 1);

        let (status, persisted_settings) = response_json(
            router(state.clone()),
            Request::get("/v1/settings")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(persisted_settings["theme_mode"], "ember");
        assert_eq!(persisted_settings["backdrop_mode"], "solid");
        assert_eq!(persisted_settings["revision"], 1);

        let (status, settings) = response_json(
            router(state.clone()),
            Request::put("/v1/settings")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"backdrop_mode":"quiet_grid","revision":1}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(settings["theme_mode"], "ember");
        assert_eq!(settings["backdrop_mode"], "quiet_grid");
        assert_eq!(settings["revision"], 2);

        let (status, conflict) = response_json(
            router(state.clone()),
            Request::put("/v1/settings")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"theme_mode":"indigo","backdrop_mode":"solid","revision":0}"#,
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(conflict["settings"]["theme_mode"], "ember");
        assert_eq!(conflict["settings"]["backdrop_mode"], "quiet_grid");
        assert_eq!(conflict["settings"]["revision"], 2);

        let (status, health) = response_json(
            router(state.clone()),
            Request::get("/v1/health").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(health["providers"][0]["id"], "test-apps");
        assert_eq!(health["providers"][0]["status"], "starting");
        assert!(
            !health["capabilities"]["install_requests"]
                .as_bool()
                .unwrap()
        );

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

        let (status, health) = response_json(
            router(state.clone()),
            Request::get("/v1/health").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(health["providers"][0]["status"], "ready");
        assert_eq!(health["providers"][0]["record_count"], 1);
        assert!(health["providers"][0]["last_attempt_at"].is_string());

        let (status, library) = response_json(
            router(state.clone()),
            Request::get("/v1/library")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(library[0]["id"], "test:app");

        let unsupported_install = router(state.clone())
            .oneshot(
                Request::post("/v1/install-requests")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"item_id":"test:app"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unsupported_install.status(), StatusCode::NOT_IMPLEMENTED);

        // Nothing exercised the actual launch endpoints before this: launch
        // dispatch (daemon -> bridge socket -> LaunchAccepted -> response)
        // had zero route-level coverage. `test:app`'s launch_id ("test.app")
        // and source_id ("test-apps") come from ApiTestProvider/discovery
        // above.
        let bridge = spawn_fake_bridge(state.config.bridge_socket_path.clone(), |request| {
            let BridgeRequest::LaunchApplication {
                source_id,
                application_id,
                session_id,
                input_profile,
            } = request
            else {
                panic!("expected a LaunchApplication request, got {request:?}");
            };
            assert_eq!(source_id, "test-apps");
            assert_eq!(application_id, "test.app");
            assert_eq!(input_profile, InputProfile::Desktop);
            BridgeResponse::LaunchAccepted {
                session: ApplicationSession {
                    id: session_id,
                    source_id,
                    application_id,
                    state: ApplicationSessionState::Running,
                },
            }
        });
        let (status, launched) = response_json(
            router(state.clone()),
            Request::post("/v1/apps/test:app/launch?input_profile=desktop")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(launched["source_id"], "test-apps");
        assert_eq!(launched["application_id"], "test.app");
        assert_eq!(launched["state"], "running");
        assert!(launched["id"].as_str().is_some_and(|id| !id.is_empty()));
        bridge.await.unwrap();

        let (status, recent) = response_json(
            router(state.clone()),
            Request::get("/v1/activity/recent?limit=6")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(recent[0]["id"], "hearthdeck:test:app");
        assert_eq!(recent[0]["title"], "Hearthdeck Test App");
        assert_eq!(recent[0]["launch_count"], 1);
        assert!(recent[0]["last_launched_at"].is_string());

        let launch_unknown_item = router(state)
            .oneshot(
                Request::post("/v1/apps/does-not-exist/launch")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(launch_unknown_item.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn parses_only_supported_heroic_launch_targets() {
        assert!(matches!(
            super::heroic_launch_target("legendary:Fortnite"),
            Some((HeroicRunner::Legendary, application_id)) if application_id == "Fortnite"
        ));
        assert!(matches!(
            super::heroic_launch_target("gog:1091500"),
            Some((HeroicRunner::Gog, application_id)) if application_id == "1091500"
        ));
        assert!(super::heroic_launch_target("steam:570").is_none());
        assert!(super::heroic_launch_target("legendary:").is_none());
    }

    #[test]
    fn ignores_empty_romm_artwork_paths() {
        assert_eq!(super::non_empty_romm_path(Some(&"  ".to_owned())), None);
        assert_eq!(
            super::non_empty_romm_path(Some(&"/assets/romm/resources/cover.webp".to_owned())),
            Some("/assets/romm/resources/cover.webp".to_owned())
        );
    }

    #[test]
    fn accepts_only_http_cover_urls() {
        assert_eq!(
            super::http_url(Some("https://images.example.com/cover.jpg")),
            Some("https://images.example.com/cover.jpg".to_owned())
        );
        assert_eq!(
            super::http_url(Some("file:///mnt/external/cover.jpg")),
            None
        );
        assert_eq!(super::http_url(Some("not a url")), None);
    }
}
