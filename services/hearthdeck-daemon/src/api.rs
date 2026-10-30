use axum::{
    Json, Router,
    extract::{
        Path, Query, Request, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use chrono::{DateTime, Utc};
use futures_util::{StreamExt, stream};
use hearthdeck_ai::categorization::{ScanReport, Taxonomy, rail_app_ids};
use hearthdeck_protocol::{
    ApplicationSession, BridgeRequest, BridgeResponse, CategorizationSnapshot, Collection,
    CollectionItem, HealthResponse, HeroicRunner, HostCapabilities, InputProfile, ProviderHealth,
    RetroGame, RetroGameDetails, RetroRomVersion, ServerEvent, StremioMediaType,
};
use rand::{RngExt, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{
    activity::{ActivityEntry, RecentActivity},
    categorizer::{ModelStatus, ScanRequest},
    collections,
    diagnostics::{self, RommGame, RommPlatform, RommQueryError},
    retro::RetroLaunchError,
    romm_link::{self, DeviceOffer, DevicePoll},
    sessions::LaunchError,
    settings::{
        self, BackdropMode, RommSettings, SettingsChange, SettingsUpdate, ThemeMode, UserSettings,
    },
    state::SharedState,
    stremio::{self, LinkOffer, LinkPoll, StremioConnection},
};

use crate::discovery::providers::stremio_arrivals;

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/pairing/complete", post(complete_pairing))
        .merge(
            protected_routes().route_layer(middleware::from_fn_with_state(
                state.clone(),
                require_authentication,
            )),
        )
        .with_state(state)
}

fn protected_routes() -> Router<SharedState> {
    Router::new()
        .route("/v1/diagnostics", get(diagnostics))
        .route("/v1/library", get(list_library))
        .route("/v1/activity/recent", get(list_recent_activity))
        .route("/v1/collections", get(list_collections))
        .route(
            "/v1/collections/{slug}/items",
            post(add_collection_item).delete(remove_collection_item),
        )
        .route("/v1/retro/consoles", get(list_retro_consoles))
        .route("/v1/retro/roms", get(list_retro_roms))
        .route("/v1/retro/facets", get(list_retro_facets))
        .route("/v1/retro/roms/{id}", get(retro_rom_details))
        .route("/v1/retro/roms/{id}/favorite", post(set_retro_favorite))
        .route("/v1/retro/roms/{id}/backlog", post(set_retro_backlogged))
        .route("/v1/retro/service/restart", post(restart_romm_service))
        .route("/v1/retro/assets", get(retro_asset))
        .route(
            "/v1/retro/settings",
            get(get_romm_settings)
                .put(update_romm_settings)
                .delete(clear_romm_settings),
        )
        .route("/v1/retro/login", post(romm_login))
        .route("/v1/retro/link", post(start_romm_link).get(poll_romm_link))
        .route(
            "/v1/stremio/link",
            post(start_stremio_link).get(poll_stremio_link),
        )
        .route(
            "/v1/stremio/settings",
            get(get_stremio_settings).delete(unlink_stremio),
        )
        .route("/v1/library/rescan", post(rescan_library))
        .route("/v1/categorization", get(categorization))
        .route("/v1/categorization/enable", post(enable_categorization))
        .route("/v1/categorization/disable", post(disable_categorization))
        .route("/v1/categorization/scan", post(start_categorization))
        .route(
            "/v1/categorization/collections",
            delete(remove_categorization_collections),
        )
        .route("/v1/categorization/form-factors", post(detect_form_factors))
        .route("/v1/settings", get(get_settings).put(update_settings))
        .route("/v1/discovery/{source_id}/refresh", post(refresh_source))
        .route("/v1/metadata/{provider_id}/refresh", post(refresh_metadata))
        .route("/v1/launch/{item_id}", post(launch_item))
        .route("/v1/sessions/active", get(active_application_session))
        .route(
            "/v1/sessions/active/stop",
            post(stop_active_application_session),
        )
        .route("/v1/sessions/{id}/stop", post(stop_application_session))
        .route("/v1/events", get(events))
}

/// Routes served only on the loopback admin listener. A remote device can
/// submit a code, but cannot mint one without host-side user approval.
pub fn local_router(state: SharedState) -> Router {
    Router::new()
        .route("/v1/pairing", post(create_pairing))
        .with_state(state)
}

async fn health(State(state): State<SharedState>) -> Json<HealthResponse> {
    // The platform probe answers for the capabilities a host either has or does
    // not; whether a scan can run is the daemon's own state, so it is filled in
    // here rather than guessed at from the OS. It is a statement about the
    // daemon, not a switch: turning smart categories on is the user's job, in
    // their settings, and needs nothing of the deployment.
    let mut capabilities = host_capabilities();
    capabilities.categorization = state.categorization.is_some();
    Json(HealthResponse {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        lan_enabled: state.config.lan_enabled,
        transport: if state.config.lan_enabled {
            "https".to_owned()
        } else {
            "http".to_owned()
        },
        providers: state
            .provider_health()
            .await
            .into_iter()
            .map(redact_provider_error)
            .collect(),
        capabilities,
    })
}

/// `/v1/health` is unauthenticated, so a provider's own error message never
/// crosses it. Those messages are `error.to_string()` from the discovery and
/// enrichment layers and routinely embed absolute host paths (a Heroic config
/// directory, a resources mount), which a client on the LAN should not be able
/// to read before it is paired. The status still says the provider is degraded;
/// only the string that would leak the host is replaced.
fn redact_provider_error(mut provider: ProviderHealth) -> ProviderHealth {
    if provider.last_error.is_some() {
        provider.last_error = Some("provider error".to_owned());
    }
    provider
}

fn host_capabilities() -> HostCapabilities {
    HostCapabilities {
        launch: true,
        application_sessions: true,
        retro_launch: true,
        // Per-deployment, filled in by `health`.
        categorization: false,
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
) -> Result<Json<Vec<crate::catalog::CatalogItem>>, ApiError> {
    let items = state.catalog.list().await.map_err(ApiError::internal)?;
    info!(item_count = items.len(), "catalog listed");
    Ok(Json(items))
}

async fn list_recent_activity(
    State(state): State<SharedState>,
    Query(query): Query<RecentActivityQuery>,
) -> Result<Json<Vec<RecentActivity>>, ApiError> {
    let items = state
        .activity
        .recent(query.limit.unwrap_or(6))
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(items))
}

async fn list_retro_consoles(
    State(state): State<SharedState>,
) -> Result<Json<Vec<RommPlatform>>, ApiError> {
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
    axum::extract::Query(query): axum::extract::Query<RommGamesQuery>,
) -> Result<Json<RommGamesResponse>, ApiError> {
    let limit = query.limit.unwrap_or(48).clamp(1, 100);
    let offset = query.offset.unwrap_or(0);
    let search_term = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let filters = diagnostics::RommGameFilters {
        genres: query.genre.into_iter().collect(),
        regions: query.region.into_iter().collect(),
    };
    let games = diagnostics::romm_games(
        &state.settings,
        query.platform_id,
        search_term,
        &filters,
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

/// The console grid's filter facets: every genre and region RomM knows for the
/// scope, so the grid offers complete options without loading the library.
///
/// Facets with no values are dropped rather than returned empty, so the client
/// never renders a filter row with nothing to pick.
async fn list_retro_facets(
    State(state): State<SharedState>,
    axum::extract::Query(query): axum::extract::Query<RommFacetsQuery>,
) -> Result<Json<Vec<RetroFacet>>, ApiError> {
    let values = diagnostics::romm_filter_values(&state.settings, query.platform_id)
        .await
        .map_err(ApiError::romm_query)?;
    Ok(Json(retro_facets(values)))
}

/// Maps RomM's library-wide filter values onto the facets the grid shows. Only
/// the fields Hearthdeck offers a chip row for become facets; the rest of
/// RomM's filter dictionary stays unused until a screen asks for it.
fn retro_facets(values: diagnostics::RommFilterValues) -> Vec<RetroFacet> {
    [
        ("genre", "Genre", values.genres),
        ("region", "Region", values.regions),
    ]
    .into_iter()
    .filter(|(_, _, options)| !options.is_empty())
    .map(|(id, label, options)| RetroFacet {
        id: id.to_owned(),
        label: label.to_owned(),
        options: options
            .into_iter()
            .map(|value| RetroFacetOption {
                label: value.clone(),
                value,
            })
            .collect(),
    })
    .collect()
}

/// Full detail for one ROM, backing the frontend's console-game details
/// screen. A separate route rather than more fields on `list_retro_roms`
/// because the list is paged, while this metadata is only worth fetching for
/// the single game the user opened. Reuses `romm_rom`, the same single-ROM
/// read the RetroArch launch path makes.
async fn retro_rom_details(
    State(state): State<SharedState>,
    Path(rom_id): Path<i64>,
) -> Result<Json<RetroGameDetails>, ApiError> {
    let game = diagnostics::romm_rom(&state.settings, rom_id)
        .await
        .map_err(ApiError::romm_query)?;
    let mut details = RetroGameDetails::from(&game);
    // Reading the true favorite state can need one more RomM call, because
    // favorites are a collection rather than a field on the rom. A failure here
    // only costs the button its initial label: the screen still opens, and
    // toggling still writes.
    match diagnostics::romm_favorite(&state.settings, &game).await {
        Ok(favorite) => details.favorite = favorite,
        Err(error) => warn!(?error, rom_id, "could not read RomM favorite state"),
    }
    Ok(Json(details))
}

#[derive(Deserialize)]
struct FavoriteRequest {
    favorite: bool,
}

#[derive(Serialize)]
struct FavoriteResponse {
    favorite: bool,
}

/// Adds or removes one game from the user's RomM favorites, answering with the
/// state that now holds.
///
/// The daemon keeps no favorite list of its own: favorites are RomM's, so every
/// RomM client shows the same thing and the choice survives a reinstall.
async fn set_retro_favorite(
    State(state): State<SharedState>,
    Path(rom_id): Path<i64>,
    Json(request): Json<FavoriteRequest>,
) -> Result<Json<FavoriteResponse>, ApiError> {
    let favorite = diagnostics::romm_set_favorite(&state.settings, rom_id, request.favorite)
        .await
        .map_err(ApiError::romm_query)?;
    info!(rom_id, favorite, "RomM favorite updated");
    Ok(Json(FavoriteResponse { favorite }))
}

#[derive(Deserialize)]
struct BacklogRequest {
    backlogged: bool,
}

#[derive(Serialize)]
struct BacklogResponse {
    backlogged: bool,
}

/// Sets RomM's per-user "play later" flag for one game. RomM's own backlog, not
/// a local list, so the same flag shows up in RomM's UI and its filters.
async fn set_retro_backlogged(
    State(state): State<SharedState>,
    Path(rom_id): Path<i64>,
    Json(request): Json<BacklogRequest>,
) -> Result<Json<BacklogResponse>, ApiError> {
    let backlogged = diagnostics::romm_set_backlogged(&state.settings, rom_id, request.backlogged)
        .await
        .map_err(ApiError::romm_query)?;
    info!(rom_id, backlogged, "RomM backlog flag updated");
    Ok(Json(BacklogResponse { backlogged }))
}

/// The retro half of a launch.
async fn launch_retro(
    state: &SharedState,
    rom_id: i64,
    input_profile: InputProfile,
) -> Result<Json<ApplicationSession>, ApiError> {
    let plan = crate::retro::prepare_launch(&state.settings, &state.config.romm, rom_id)
        .await
        .map_err(ApiError::retro_launch)?;
    let activity = retro_activity_entry(&plan.game);
    let session_id = Uuid::new_v4().to_string();
    let request = BridgeRequest::LaunchRetroGame {
        core_path: plan.core_path.to_string_lossy().into_owned(),
        rom_path: plan.rom_path.to_string_lossy().into_owned(),
        session_id,
        input_profile,
    };
    let session =
        state
            .sessions()
            .launch(request, activity)
            .await
            .map_err(|error| match error {
                LaunchError::Unexpected(_) => {
                    ApiError::bad_gateway("bridge rejected retro game launch")
                }
                other => ApiError::bad_gateway(other),
            })?;
    info!(rom_id, "retro game launch accepted");
    Ok(Json(session))
}

/// Restarts the fixed `romm.service` systemd unit
/// (`deploy/systemd/romm.service`), if the user has configured it.
/// Not a generic "restart any unit" endpoint: the unit name is hardcoded in
/// `diagnostics::restart_romm_service`, never taken from the request.
async fn restart_romm_service() -> Result<StatusCode, ApiError> {
    diagnostics::restart_romm_service()
        .await
        .map_err(ApiError::bad_gateway)?;
    info!("romm.service restart requested");
    Ok(StatusCode::ACCEPTED)
}

async fn retro_asset(
    State(state): State<SharedState>,
    axum::extract::Query(query): axum::extract::Query<RommAssetQuery>,
) -> Result<Response, ApiError> {
    let asset = diagnostics::romm_asset(&state.settings, &state.config.romm, &query.path)
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
) -> Result<Json<diagnostics::DiagnosticsSnapshot>, ApiError> {
    Ok(Json(diagnostics::snapshot(&state.settings).await))
}

async fn get_romm_settings(
    State(state): State<SharedState>,
) -> Result<Json<Option<RommSettings>>, ApiError> {
    state
        .settings
        .romm()
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

async fn update_romm_settings(
    State(state): State<SharedState>,
    Json(request): Json<UpdateRommSettingsRequest>,
) -> Result<Json<RommSettings>, ApiError> {
    state
        .settings
        .save_romm(&request.base_url, &request.token)
        .await
        .map(Json)
        .map_err(ApiError::invalid_romm_settings)
}

async fn clear_romm_settings(State(state): State<SharedState>) -> Result<StatusCode, ApiError> {
    state.romm_link.cancel().await;
    state
        .settings
        .clear_romm()
        .await
        .map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Signs in to RomM once and keeps only the client token it mints.
///
/// The URL is validated before anything is dialled, so a password can only ever
/// be sent to a loopback RomM.
async fn romm_login(
    State(state): State<SharedState>,
    Json(request): Json<RommLoginRequest>,
) -> Result<Json<RommSettings>, ApiError> {
    let base_url =
        settings::normalize_romm_url(&request.base_url).map_err(ApiError::invalid_romm_settings)?;
    let token = romm_link::mint_token(&base_url, &request.username, &request.password)
        .await
        .map_err(ApiError::romm_link)?;
    state.romm_link.cancel().await;
    let saved = state
        .settings
        .save_romm(&base_url, &token)
        .await
        .map_err(ApiError::internal)?;
    info!("romm connected with a password sign-in");
    Ok(Json(saved))
}

/// The pending RomM approval's answer; like Stremio's, it never carries the token.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum RommLinkResponse {
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

async fn start_romm_link(
    State(state): State<SharedState>,
    Json(request): Json<StartRommLinkRequest>,
) -> Result<Json<DeviceOffer>, ApiError> {
    let base_url =
        settings::normalize_romm_url(&request.base_url).map_err(ApiError::invalid_romm_settings)?;
    let offer = state
        .romm_link
        .start(&base_url)
        .await
        .map_err(ApiError::romm_link)?;
    info!(code = %offer.code, "romm device approval started");
    Ok(Json(offer))
}

async fn poll_romm_link(
    State(state): State<SharedState>,
) -> Result<Json<RommLinkResponse>, ApiError> {
    let response = match state.romm_link.poll().await.map_err(ApiError::romm_link)? {
        DevicePoll::Idle => RommLinkResponse::Idle,
        DevicePoll::Expired => RommLinkResponse::Expired,
        DevicePoll::Denied => RommLinkResponse::Denied,
        DevicePoll::Waiting(offer, seconds_left) => RommLinkResponse::Waiting {
            code: offer.code,
            link: offer.link,
            seconds_left,
        },
        DevicePoll::Linked { base_url, token } => {
            let settings = state
                .settings
                .save_romm(&base_url, &token)
                .await
                .map_err(ApiError::internal)?;
            info!("romm connected by device approval");
            RommLinkResponse::Linked { settings }
        }
    };
    Ok(Json(response))
}

/// The pending-link poll's answer.
///
/// Tagged by `status`, so a client cannot read the fields of the wrong case, and
/// carrying the session key in none of them: a linked account says that it
/// linked, and the credential stays inside the daemon.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum StremioLinkResponse {
    /// No link is pending: never asked for, or already resolved.
    Idle,
    /// The window closed while the user was still deciding.
    Expired,
    /// Waiting on the user, who is approving the code on another device.
    Waiting {
        code: String,
        link: String,
        qr: String,
        seconds_left: u64,
    },
    /// Linked and stored. The connection is the credential-free view of it.
    Linked { connection: StremioConnection },
}

/// Asks Stremio for a link code and holds it as the pending link.
///
/// The user approves the code on a device where they are already signed in, which
/// is why no endpoint here accepts a password: there is nothing to encrypt but the
/// key that comes back, and nothing to store but that key.
async fn start_stremio_link(State(state): State<SharedState>) -> Result<Json<LinkOffer>, ApiError> {
    let offer = stremio::request_link()
        .await
        .map_err(ApiError::stremio_link)?;
    info!(code = %offer.code, "stremio link code issued");
    state.stremio_link.start(offer.clone()).await;
    Ok(Json(offer))
}

/// One poll of the pending link.
///
/// The session key is stored here and goes no further: the response reports that
/// the account linked, and says nothing about the credential.
async fn poll_stremio_link(
    State(state): State<SharedState>,
) -> Result<Json<StremioLinkResponse>, ApiError> {
    let response = match state
        .stremio_link
        .poll()
        .await
        .map_err(ApiError::stremio_link)?
    {
        LinkPoll::Idle => StremioLinkResponse::Idle,
        LinkPoll::Expired => StremioLinkResponse::Expired,
        LinkPoll::Waiting(offer, seconds_left) => StremioLinkResponse::Waiting {
            code: offer.code,
            link: offer.link,
            qr: offer.qr,
            seconds_left,
        },
        LinkPoll::Linked(auth_key) => {
            let connection = state
                .stremio
                .save_session(&auth_key)
                .await
                .map_err(ApiError::stremio_link)?;
            info!("stremio account linked");
            StremioLinkResponse::Linked { connection }
        }
    };
    Ok(Json(response))
}

async fn get_stremio_settings(
    State(state): State<SharedState>,
) -> Result<Json<Option<StremioConnection>>, ApiError> {
    state
        .stremio
        .connection()
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

/// Unlinks the account, and takes the catalog rows with it.
///
/// Both halves matter. A session that is gone while its rows survive would leave a
/// Continue Watching rail listing somebody's unfinished films on a box where
/// nobody is linked — so the pending link is dropped first, then the session, then
/// the source's rows (the decision recorded in `docs/stremio-integration.md`).
async fn unlink_stremio(State(state): State<SharedState>) -> Result<StatusCode, ApiError> {
    state.stremio_link.cancel().await;
    state.stremio.clear().await.map_err(ApiError::internal)?;
    state
        .catalog
        .replace_source(stremio::SOURCE_ID, Vec::new())
        .await
        .map_err(ApiError::internal)?;
    info!("stremio account unlinked");
    Ok(StatusCode::NO_CONTENT)
}

async fn rescan_library(State(state): State<SharedState>) -> Result<StatusCode, ApiError> {
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

async fn categorization(
    State(state): State<SharedState>,
) -> Result<Json<CategorizationSnapshot<ScanReport>>, ApiError> {
    let service = state
        .categorization
        .as_ref()
        .ok_or_else(ApiError::service_unavailable)?;
    let enabled = state
        .settings
        .get()
        .await
        .map_err(ApiError::internal)?
        .categorization_enabled;
    let status = service.status().await;
    let report = service.latest_report().await.map_err(ApiError::internal)?;
    Ok(Json(CategorizationSnapshot {
        enabled,
        status,
        report,
    }))
}

/// Turns smart categorization on, which only fetches the checkpoint. Nothing is
/// classified until a button asks for it.
async fn enable_categorization(State(state): State<SharedState>) -> Result<StatusCode, ApiError> {
    let service = state
        .categorization
        .as_ref()
        .ok_or_else(ApiError::service_unavailable)?;
    state
        .settings
        .set_categorization_enabled(true)
        .await
        .map_err(ApiError::internal)?;
    match service.request_download().await {
        ScanRequest::Started => info!("categorization enabled; model is ready or downloading"),
        ScanRequest::AlreadyRunning => info!("categorization enabled; a job is already running"),
    }
    Ok(StatusCode::ACCEPTED)
}

/// Turns it off and undoes what the model changed. A job already running is
/// left to finish, but what it decides is thrown away. The downloaded model is
/// kept, so turning it back on does not fetch it again.
async fn disable_categorization(State(state): State<SharedState>) -> Result<StatusCode, ApiError> {
    state
        .settings
        .set_categorization_enabled(false)
        .await
        .map_err(ApiError::internal)?;
    remove_laya_collections(&state).await;
    if let Some(categorization) = state.categorization.as_ref() {
        match categorization.clear_traits().await {
            Ok(0) => {}
            Ok(count) => info!(count, "categorization verdicts removed"),
            Err(error) => warn!(%error, "could not remove the categorization verdicts"),
        }
    }
    info!("categorization disabled");
    Ok(StatusCode::NO_CONTENT)
}

/// The rails the feature created, and whatever the running job would still
/// write. A person's collections and the built-in ones are not this feature's
/// to touch.
async fn remove_laya_collections(state: &SharedState) {
    if let Some(categorization) = state.categorization.as_ref() {
        categorization.discard_running().await;
    }
    match state
        .collections
        .remove_owned(collections::OWNER_LAYA)
        .await
    {
        Ok(0) => {}
        Ok(count) => info!(count, "categorization rails removed"),
        Err(error) => warn!(%error, "could not remove the categorization rails"),
    }
}

/// Restore dashboard: drops the rails the model published, whether or not the
/// feature is still on.
async fn remove_categorization_collections(
    State(state): State<SharedState>,
) -> Result<StatusCode, ApiError> {
    remove_laya_collections(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Detect couch-friendly apps: asks the model about every application it has
/// no current verdict for. Answers `202`; the result arrives as a library change.
/// Refused with `409` while the model is missing or another job holds it.
async fn detect_form_factors(State(state): State<SharedState>) -> Result<StatusCode, ApiError> {
    let service = state
        .categorization
        .as_ref()
        .ok_or_else(ApiError::service_unavailable)?
        .clone();
    if !state
        .settings
        .get()
        .await
        .map_err(ApiError::internal)?
        .categorization_enabled
    {
        return Err(ApiError::conflict("categorization is not enabled"));
    }
    let status = service.status().await;
    if status.running {
        return Err(ApiError::conflict("another categorization job is running"));
    }
    if status.model == ModelStatus::Absent {
        return Err(ApiError::conflict("the model is not downloaded yet"));
    }
    tokio::spawn(async move {
        match service.classify_pending().await {
            Ok(run) => info!(?run, "form-factor detection finished"),
            Err(error) => warn!(%error, "form-factor detection failed"),
        }
    });
    Ok(StatusCode::ACCEPTED)
}

/// Publish dashboard rails: files the library by rules (no model) and writes the
/// rails that implies. Answers `202`; the result arrives on the event stream.
/// Requesting one while another job runs is not an error: it coalesces.
async fn start_categorization(State(state): State<SharedState>) -> Result<StatusCode, ApiError> {
    let service = state
        .categorization
        .as_ref()
        .ok_or_else(ApiError::service_unavailable)?;
    if !state
        .settings
        .get()
        .await
        .map_err(ApiError::internal)?
        .categorization_enabled
    {
        return Err(ApiError::conflict("categorization is not enabled"));
    }
    match service.request_scan().await {
        ScanRequest::Started => {
            info!("categorization scan started");
            Ok(StatusCode::ACCEPTED)
        }
        ScanRequest::AlreadyRunning => {
            info!("categorization scan coalesced");
            Ok(StatusCode::ACCEPTED)
        }
    }
}

async fn get_settings(State(state): State<SharedState>) -> Result<Json<UserSettings>, ApiError> {
    state
        .settings
        .get()
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

async fn update_settings(
    State(state): State<SharedState>,
    Json(request): Json<UpdateSettingsRequest>,
) -> Result<Json<UserSettings>, ApiError> {
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
    Path(source_id): Path<String>,
) -> Result<StatusCode, ApiError> {
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
    Path(provider_id): Path<String>,
) -> Result<StatusCode, ApiError> {
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

/// Launches whatever an item id names — the single route a provider-agnostic
/// client posts to. Which source sits behind the id (a catalog record's own
/// dispatch, or RomM's launch) stays this daemon's business: the client sends
/// the id its card or tile carries and reads the session back, and never
/// parses a prefix to pick a route.
async fn launch_item(
    State(state): State<SharedState>,
    Path(item_id): Path<String>,
    Query(options): Query<LaunchOptions>,
) -> Result<Json<ApplicationSession>, ApiError> {
    match item_launch(&item_id) {
        ItemLaunch::Retro(rom_id) => launch_retro(&state, rom_id, options.input_profile).await,
        ItemLaunch::Catalog(id) => launch_catalog(&state, id, options.input_profile).await,
    }
}

/// The catalog half of a launch: resolves the record,
/// decides which launch shape its `source_id` asks the bridge for — desktop
/// application, Heroic game, or Stremio title — and runs it.
async fn launch_catalog(
    state: &SharedState,
    id: &str,
    input_profile: InputProfile,
) -> Result<Json<ApplicationSession>, ApiError> {
    let item = state
        .catalog
        .get(id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(ApiError::not_found)?;
    let launch_id = item.launch_id.as_deref().ok_or_else(ApiError::not_found)?;
    let session_id = Uuid::new_v4().to_string();
    // Both catalog sources this integration publishes name Stremio titles: the
    // account's Continue Watching records, and the new arrivals the hero banner
    // shows.
    let stremio_title = (item.source_id == stremio::SOURCE_ID
        || item.source_id == stremio_arrivals::SOURCE_ID)
        .then(|| stremio_title_target(&item))
        .flatten();
    let request = if item.source_id == hearthdeck_protocol::sources::HEROIC {
        let (runner, application_id) =
            heroic_launch_target(launch_id).ok_or_else(ApiError::not_found)?;
        BridgeRequest::LaunchHeroicGame {
            runner,
            application_id,
            session_id,
            input_profile,
        }
    } else if let Some((video_id, media_type)) = stremio_title {
        // A Stremio record names a title, so the bridge is asked for that title rather
        // than for the application the record carries. The id travels, not a URL: the
        // bridge validates it and builds the URI itself.
        BridgeRequest::LaunchStremioTitle {
            // The entry the provider resolved from this host's own application list,
            // which is also what a plain application launch carries. The bridge validates
            // it and the video id, and builds the URI itself.
            application_id: launch_id.to_owned(),
            video_id,
            media_type,
            session_id,
            input_profile,
        }
    } else {
        BridgeRequest::LaunchApplication {
            // Not `item.source_id`: see DESKTOP_APPS_SOURCE. What is being started is
            // a desktop entry whichever provider published the record naming it, and
            // the bridge refuses any other source id.
            source_id: crate::DESKTOP_APPS_SOURCE.to_owned(),
            application_id: launch_id.to_owned(),
            session_id,
            input_profile,
        }
    };
    let session = state
        .sessions()
        .launch(request, catalog_activity_entry(&item))
        .await
        .map_err(|error| match error {
            LaunchError::Unexpected(other) => {
                ApiError::bad_gateway(format!("bridge rejected application launch: {other:?}"))
            }
            other => ApiError::bad_gateway(other),
        })?;
    info!(item_id = %id, "catalog launch accepted");
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

/// The Stremio title a record names, when it names one.
///
/// The Stremio title a record names: its video id, and whether that id is a film
/// or a series.
///
/// The provider publishes both in the record's own metadata, which the catalog keeps
/// under `discovery`. The type matters only for a bare IMDb id — `tt0149460` is a
/// different page for a film and for a series — while `tt0149460:4:7` already says
/// series by carrying an episode. A record without a video id is launched as a plain
/// application instead: an older record, or one whose account has since been
/// unlinked. Either way the card does something rather than failing.
fn stremio_title_target(item: &crate::catalog::CatalogItem) -> Option<(String, StremioMediaType)> {
    let discovery = item.metadata.get("discovery")?;
    let video_id = discovery
        .get("video_id")?
        .as_str()
        .map(str::trim)
        .filter(|video_id| !video_id.is_empty())?;
    let media_type = match discovery.get("type").and_then(serde_json::Value::as_str) {
        Some("series") => StremioMediaType::Series,
        // Anything not spelled series — including records that predate the field —
        // was launched as a film before the two kinds were told apart, and stays one.
        _ => StremioMediaType::Movie,
    };
    Some((video_id.to_owned(), media_type))
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
        kind: Some(item.kind.clone()),
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
        kind: Some("game".to_owned()),
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
) -> Result<Json<Option<ApplicationSession>>, ApiError> {
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

async fn stop_active_application_session(
    State(state): State<SharedState>,
) -> Result<StatusCode, ApiError> {
    let response = crate::bridge::request(
        &state.config.bridge_socket_path,
        BridgeRequest::StopActiveApplicationSession,
    )
    .await
    .map_err(ApiError::bad_gateway)?;
    match response {
        BridgeResponse::StopAccepted { session_id } => {
            state.sessions().finished(&session_id).await;
            Ok(StatusCode::ACCEPTED)
        }
        BridgeResponse::Error {
            code: hearthdeck_protocol::BridgeErrorCode::NotFound,
            message,
        } => Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message,
            settings: None,
        }),
        BridgeResponse::Error { message, .. } => Err(ApiError::bad_gateway(message)),
        _ => Err(ApiError::bad_gateway(
            "bridge rejected application-session stop",
        )),
    }
}

async fn stop_application_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
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
    // A stop the user asked for knows when the play ended, so it does not wait for
    // the watcher's next tick.
    state.sessions().finished(&id).await;
    Ok(StatusCode::ACCEPTED)
}

async fn events(
    State(state): State<SharedState>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
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

async fn require_authentication(
    State(state): State<SharedState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    authenticate(&state, request.headers()).await?;
    Ok(next.run(request).await)
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

/// Deliberately not `Debug`: it carries a password.
#[derive(Deserialize)]
struct RommLoginRequest {
    base_url: String,
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct StartRommLinkRequest {
    base_url: String,
}

#[derive(Deserialize)]
struct RommGamesQuery {
    platform_id: Option<i64>,
    #[serde(default)]
    q: Option<String>,
    /// Narrow to roms carrying this genre (RomM's `genres` filter).
    #[serde(default)]
    genre: Option<String>,
    /// Narrow to roms carrying this region (RomM's `regions` filter).
    #[serde(default)]
    region: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

#[derive(Deserialize)]
struct RommFacetsQuery {
    platform_id: Option<i64>,
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

/// One filter dimension the console grid can narrow by, and the values it
/// offers. Matches the frontend's provider-agnostic `Facet`.
#[derive(Serialize, Debug)]
struct RetroFacet {
    id: String,
    label: String,
    options: Vec<RetroFacetOption>,
}

#[derive(Serialize, Debug)]
struct RetroFacetOption {
    value: String,
    label: String,
}

impl From<&RommGame> for RetroGameDetails {
    fn from(game: &RommGame) -> Self {
        // Reuse the list projection for the fields both responses share, so
        // the title fallback and version labelling rules cannot drift apart.
        let base = RetroGame::from(game);
        Self {
            id: base.id,
            platform_id: base.platform_id,
            title: base.title,
            platform_name: game
                .platform_display_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
            summary: base.summary,
            cover_path: base.cover_path,
            cover_large_path: non_empty_romm_path(game.path_cover_large.as_ref()),
            screenshot_paths: base.screenshot_paths,
            genres: base.genres,
            player_count: base.player_count,
            release_year: base.release_year,
            release_date: game.metadatum.release_date(),
            regions: base.regions,
            languages: game.languages.clone(),
            tags: game.tags.clone(),
            companies: game.metadatum.companies.clone(),
            game_modes: game.metadatum.game_modes.clone(),
            age_ratings: game.metadatum.age_ratings.clone(),
            average_rating: game.metadatum.average_rating,
            file_size_bytes: game.fs_size_bytes,
            sibling_roms: base.sibling_roms,
            version_label: base.version_label,
            // Favorites are a collection in RomM rather than a field on the
            // rom, so this is only right when RomM inlines the flag; the
            // handler corrects it with a lookup when it is not.
            favorite: game.is_favorite.unwrap_or(false),
            backlogged: game.rom_user.backlogged,
        }
    }
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
        let release_year = game.metadatum.release_year();
        // Label every version with its file name (tags kept), which is what
        // distinguishes a disc or region. `name` is the shared game title, so
        // it only serves as a fallback and would otherwise make every row read
        // the same. When two rows still collide, append the RomM id so the
        // picker never shows two identical entries.
        let mut sibling_roms: Vec<RetroRomVersion> = game
            .sibling_roms
            .iter()
            .map(|sibling| RetroRomVersion {
                id: sibling.id,
                title: [
                    sibling.fs_name_no_ext.trim(),
                    sibling.name.as_deref().unwrap_or_default().trim(),
                    sibling.fs_name_no_tags.trim(),
                ]
                .into_iter()
                .find(|candidate| !candidate.is_empty())
                .unwrap_or("Untitled version")
                .to_owned(),
                is_main_sibling: sibling.is_main_sibling,
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        for version in &mut sibling_roms {
            if !seen.insert(version.title.clone()) {
                version.title = format!("{} (#{})", version.title, version.id);
            }
        }
        // The representative's label comes from its own on-disk name (tags
        // kept), so it reads the same way as its siblings in the picker.
        let version_label = (!sibling_roms.is_empty()).then(|| {
            game.fs_name
                .as_deref()
                .map(strip_extension)
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| title.clone())
        });
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
            sibling_roms,
            version_label,
        }
    }
}

/// Strips only the final extension, keeping any `(Disc N)`/`(USA)` tags that
/// follow the title: `Shenmue (Disc 1).chd` -> `Shenmue (Disc 1)`.
fn strip_extension(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => file_name,
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

/// How many items a rule collection may contribute to a rail. Rails are a
/// handful of tiles wide, so this is generous on purpose: the client decides
/// what fits, not the daemon.
const COLLECTION_RULE_ITEM_LIMIT: u32 = 20;

/// The dashboard's collections, in composition order.
///
/// Rule collections are resolved here rather than stored: the daemon is the only
/// place that knows where each rule's data lives.
async fn list_collections(
    State(state): State<SharedState>,
) -> Result<Json<Vec<Collection>>, ApiError> {
    let mut collections = state.collections.list().await.map_err(ApiError::internal)?;
    for collection in &mut collections {
        // A rail that cannot be resolved is an empty rail, not a failed
        // dashboard: one broken source used to take every other rail down with it.
        let resolved = if collection.owner == collections::OWNER_LAYA {
            Some(modeled_rail_items(&state, collection).await)
        } else if collection.owner == collections::OWNER_STREMIO
            && collection.rule.as_deref() == Some(collections::RULE_CONTINUE_WATCHING)
        {
            Some(continue_watching_items(&state).await)
        } else if collection.owner == collections::OWNER_STREMIO
            && collection.rule.as_deref() == Some(collections::RULE_NEW_ARRIVALS)
        {
            Some(new_arrival_items(&state).await)
        } else if collection.owner == collections::OWNER_SYSTEM
            && collection.rule.as_deref() == Some(collections::RULE_LAST_PLAYED)
        {
            Some(last_played_items(&state).await)
        } else {
            None
        };
        match resolved {
            Some(Ok(items)) => collection.items = items,
            Some(Err(error)) => {
                warn!(slug = %collection.slug, error = %error.message, "rail could not be resolved");
            }
            None => {}
        }
        // Last, once every rail holds the items it will serve: an item stored
        // without an icon is given the one its id names, so a client can draw
        // every card in a rail the same way.
        collection.items = with_item_icons(&state, std::mem::take(&mut collection.items)).await;
    }
    Ok(Json(collections))
}

/// How many icons one rail resolves at a time.
///
/// A lookup is either a local database read or a request to the user's own RomM,
/// so the bound is not about a shared remote service: it keeps a long favourites
/// list from opening one socket per item at once.
const ITEM_ICON_CONCURRENCY: usize = 8;

/// The same items, each carrying an icon.
///
/// A curated item is stored exactly as the client added it, and the clients that
/// curate send no icon at all - they draw a card from data they already hold, so
/// they had no reason to store one. A rail drawn from the collection alone
/// therefore had nothing to draw with, which is why every favourite and Play
/// later card showed the same generic mark. The icon is resolved per request
/// rather than written back, so a cover RomM learns about later, or one the
/// catalog picks up on its next scan, shows up without the item being rewritten.
async fn with_item_icons(state: &SharedState, items: Vec<CollectionItem>) -> Vec<CollectionItem> {
    stream::iter(items)
        .map(|mut item| {
            let state = state.clone();
            async move {
                if item.icon.is_none() {
                    item.icon = stored_item_icon(&state, &item.item_id).await;
                }
                item
            }
        })
        .buffered(ITEM_ICON_CONCURRENCY)
        .collect()
        .await
}

/// The icon an item id names, as this daemon knows it.
///
/// Two kinds of id reach a stored item: one of the catalog's, which is what the
/// model's rails and every favourite of an application carry, and one of RomM's,
/// which is what a favourite or a backlogged rom carries. Neither is stored on
/// the item, and either may since have gone - an uninstalled program, a rom
/// removed from the library - which is not an error: that card draws its generic
/// mark, and the rest of the rail is worth having either way.
async fn stored_item_icon(state: &SharedState, item_id: &str) -> Option<String> {
    if let Some(rom_id) = rom_id_from_item(item_id) {
        return match diagnostics::romm_rom(&state.settings, rom_id).await {
            Ok(game) => RetroGame::from(&game).cover_path,
            Err(error) => {
                debug!(?error, rom_id, "could not resolve a rail item's RomM cover");
                None
            }
        };
    }
    let catalog_id = catalog_id_from_item(item_id)?;
    match state.catalog.get(catalog_id).await {
        Ok(item) => item.and_then(|item| item.icon),
        Err(error) => {
            debug!(
                ?error,
                item_id, "could not resolve a rail item's catalog icon"
            );
            None
        }
    }
}

/// The catalog id an item id wears, when it wears one.
fn catalog_id_from_item(item_id: &str) -> Option<&str> {
    item_id
        .strip_prefix(ENTRY_ID_PREFIX)
        .filter(|id| !id.is_empty())
}

/// The RomM rom an item id names, when it names one.
fn rom_id_from_item(item_id: &str) -> Option<i64> {
    item_id.strip_prefix("romm:")?.parse().ok()
}

/// Which launch an item id names: the two shapes an id can carry.
enum ItemLaunch<'a> {
    /// The RomM rom its `romm:` prefix carries.
    Retro(i64),
    /// The catalog record the id wears the entry prefix for — or, unprefixed,
    /// the record it directly names, which is what the per-kind route
    /// addresses.
    Catalog(&'a str),
}

/// Resolves an item id to one of the two launches. This is the whole of what
/// the single launch route decides from the id: a client that posts ids never
/// parses a prefix itself.
fn item_launch(item_id: &str) -> ItemLaunch<'_> {
    if let Some(rom_id) = rom_id_from_item(item_id) {
        return ItemLaunch::Retro(rom_id);
    }
    ItemLaunch::Catalog(catalog_id_from_item(item_id).unwrap_or(item_id))
}

/// The entry-id prefix a client draws its items with. The daemon echoes it for
/// the items it composes, so a rail a feature created and a rail the client
/// curated are addressed the same way.
const ENTRY_ID_PREFIX: &str = "hearthdeck:";

/// The items of a rail the model's scan implied.
///
/// Resolved from the stored report, because that is where the decision lives —
/// the collection only says *which* rail it is. The name and icon come from the
/// catalog when the item is still in it, so a client can draw the card without
/// having that application loaded.
async fn modeled_rail_items(
    state: &SharedState,
    collection: &Collection,
) -> Result<Vec<CollectionItem>, ApiError> {
    let Some(rail_id) = collection
        .rule
        .as_deref()
        .and_then(|rule| collections::owned_rule_id(collections::OWNER_LAYA, rule))
    else {
        return Ok(Vec::new());
    };
    let Some(categorization) = state.categorization.as_ref() else {
        return Ok(Vec::new());
    };
    let Some(report) = categorization
        .latest_report()
        .await
        .map_err(ApiError::internal)?
    else {
        return Ok(Vec::new());
    };

    let mut items = Vec::new();
    for app_id in rail_app_ids(&report, &Taxonomy::baseline(), rail_id) {
        let (name, icon) = match state.catalog.get(&app_id).await {
            Ok(Some(item)) => (item.title, item.icon),
            // Gone from the catalog since the scan: the client resolves the id
            // against what it has, which is why the id is sent either way.
            _ => (String::new(), None),
        };
        items.push(CollectionItem {
            item_id: format!("{ENTRY_ID_PREFIX}{app_id}"),
            name,
            icon,
            background: None,
            logo: None,
            description: None,
            played: None,
        });
    }
    Ok(items)
}

/// One play, as a collection item.
///
/// The record rides along whole, exactly as the activity endpoint sends it, so a
/// rail composed from the collection draws the card the same way whether the
/// client asked for the play or for the collection it landed in. The duplicate
/// name and icon are for curated items' sake: every item carries those two, and a
/// client should not have to know which kind of collection it is looking at.
fn played_collection_item(play: RecentActivity) -> CollectionItem {
    CollectionItem {
        item_id: play.entry.id.clone(),
        name: play.entry.title.clone(),
        icon: play.entry.icon.clone(),
        background: None,
        logo: None,
        description: None,
        played: Some(play),
    }
}

/// The dashboard's "Recently Played" rail: the most recently played games.
///
/// Games are what the games sections play — a Heroic PC game or a RomM console
/// game — so a launched application and a Stremio film or series are plays the
/// activity endpoint serves but not cards this rail holds.
async fn last_played_items(state: &SharedState) -> Result<Vec<CollectionItem>, ApiError> {
    Ok(state
        .activity
        .recent_games(COLLECTION_RULE_ITEM_LIMIT)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(played_collection_item)
        .collect())
}

/// The Stremio account's unfinished titles, as rail items.
///
/// Read from the records the Stremio provider published rather than from the
/// account: the provider has already applied Stremio's own rule for what belongs in
/// this row, so all that is left is the order and the two fields a card draws with.
///
/// The ids carry the same `hearthdeck:` prefix the model's rails use. That prefix is
/// what makes a card launchable: the client strips it and hands the remainder to
/// `/v1/launch/{id}`, which resolves it against the catalog to find the Stremio
/// desktop entry the record carries. Sending the bare catalog id instead leaves the
/// client's own launch gate refusing it, and the card does nothing when pressed.
///
/// Nothing an account is not linked to is here: an unlinked account has published no
/// records, and a link that was removed took its records with it.
///
/// Only movies and series belong on the "Continue Watching" rail. The Stremio
/// provider may publish other types (e.g. channels), which are filtered out here.
async fn continue_watching_items(state: &SharedState) -> Result<Vec<CollectionItem>, ApiError> {
    let records = state
        .catalog
        .list_source(stremio::SOURCE_ID, COLLECTION_RULE_ITEM_LIMIT as usize)
        .await
        .map_err(ApiError::internal)?;
    debug!(
        source_id = stremio::SOURCE_ID,
        items = records.len(),
        "continue watching rail composed"
    );
    Ok(records
        .into_iter()
        .filter(|record| {
            // Only movies and series belong on the Continue Watching rail.
            // The catalog's `kind` field carries the Stremio type ("movie", "series", "other").
            matches!(record.kind.as_str(), "movie" | "series")
        })
        .map(|record| CollectionItem {
            item_id: format!("{ENTRY_ID_PREFIX}{}", record.id),
            name: record.title,
            icon: record.icon,
            background: None,
            logo: None,
            description: None,
            played: None,
        })
        .collect())
}

/// What has just arrived on Stremio, as the hero banner's slides.
///
/// The records the arrivals provider published, newest release first — the same
/// order the provider wrote them in, since it dates each record by its release.
/// The banner's art rides on each item: these are the records that hold a wide
/// background and the title's own wordmark, which a rail card has no use for.
async fn new_arrival_items(state: &SharedState) -> Result<Vec<CollectionItem>, ApiError> {
    let records = state
        .catalog
        .list_source(
            stremio_arrivals::SOURCE_ID,
            COLLECTION_RULE_ITEM_LIMIT as usize,
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(records
        .into_iter()
        .map(|record| {
            let art = |key: &str| {
                record
                    .metadata
                    .get("discovery")
                    .and_then(|metadata| metadata.get(key))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            CollectionItem {
                item_id: format!("{ENTRY_ID_PREFIX}{}", record.id),
                name: record.title,
                icon: record.icon,
                background: art("background"),
                logo: art("logo"),
                description: art("description"),
                played: None,
            }
        })
        .collect())
}

#[derive(Deserialize)]
struct CollectionItemRequest {
    item_id: String,
    name: String,
    #[serde(default)]
    icon: Option<String>,
}

#[derive(Deserialize)]
struct CollectionItemRef {
    item_id: String,
}

/// Adds an item to a curated collection, answering with the collection as it now
/// stands.
async fn add_collection_item(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
    Json(request): Json<CollectionItemRequest>,
) -> Result<Json<Collection>, ApiError> {
    ensure_curated(&state, &slug).await?;
    let item = CollectionItem {
        item_id: request.item_id,
        name: request.name,
        icon: request.icon,
        // A curated item is what the client added, not a play: the rule items are
        // the only ones that carry a record.
        background: None,
        logo: None,
        description: None,
        played: None,
    };
    state
        .collections
        .add_item(&slug, &item)
        .await
        .map_err(ApiError::internal)?;
    info!(slug, item_id = %item.item_id, "collection item added");
    curated_collection(&state, &slug).await
}

/// Removes an item from a curated collection, answering with the collection as it
/// now stands. Removing one that is not there is not an error: the caller asked
/// for it to be gone.
async fn remove_collection_item(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
    Json(request): Json<CollectionItemRef>,
) -> Result<Json<Collection>, ApiError> {
    ensure_curated(&state, &slug).await?;
    state
        .collections
        .remove_item(&slug, &request.item_id)
        .await
        .map_err(ApiError::internal)?;
    curated_collection(&state, &slug).await
}

/// Rejects a slug that is unknown or whose contents are derived.
async fn ensure_curated(state: &SharedState, slug: &str) -> Result<(), ApiError> {
    match state
        .collections
        .accepts_items(slug)
        .await
        .map_err(ApiError::internal)?
    {
        None => Err(ApiError::not_found()),
        Some(false) => Err(ApiError::derived_collection()),
        Some(true) => Ok(()),
    }
}

async fn curated_collection(state: &SharedState, slug: &str) -> Result<Json<Collection>, ApiError> {
    state
        .collections
        .collection(slug)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(ApiError::not_found)
        .map(Json)
}

impl ApiError {
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "authentication required".to_owned(),
            settings: None,
        }
    }

    /// A collection whose contents are derived cannot be edited: an item added to
    /// it would be recomputed away on the next read, so the write is refused
    /// rather than silently lost.
    fn derived_collection() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "collection contents are derived and cannot be edited".to_owned(),
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
            // Deliberately says "that service" rather than naming one: this is the
            // 503 for every service a build may not have — discovery, metadata,
            // and the opt-in categorization — and naming discovery made the
            // message a lie for the other two.
            message: "this deployment does not provide that service".to_owned(),
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
            RetroLaunchError::RomNotOnDisk { .. } => Self::not_found(),
        }
    }

    /// The link flow could not be completed.
    ///
    /// Every failure here is upstream's, or is upstream's answer being unusable:
    /// this daemon contributes nothing to the flow but the token that
    /// authenticates the client asking for it. The message is deliberately the
    /// upstream's own wording, and it never contains the session key — see
    /// `stremio::save_session`, whose refusals describe the key without quoting
    /// it.
    fn stremio_link(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
            settings: None,
        }
    }

    /// RomM refused or could not finish a sign-in. `romm_link` builds messages
    /// that never quote the username, password or token.
    fn romm_link(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
            settings: None,
        }
    }

    fn invalid_romm_settings(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: error.to_string(),
            settings: None,
        }
    }

    /// Well formed, but the daemon's state says no, and the message says which.
    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
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
        if self.status.is_server_error() {
            warn!(
                status_code = self.status.as_u16(),
                error = %self.message,
                "API request failed"
            );
        }
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

    use super::{local_router, redact_provider_error, router};
    use crate::{
        catalog::CatalogRecord,
        collections::CollectionItem,
        config::Config,
        database::Database,
        discovery::{DiscoveryProvider, DiscoveryService},
        state::{AppState, ProviderHealth, ProviderKind, ProviderStatus, SharedState},
    };

    #[test]
    fn health_redacts_provider_error_strings() {
        // The status still says the provider is degraded, but the string that would
        // have carried a host path does not cross the unauthenticated endpoint.
        let degraded = ProviderHealth {
            id: "heroic".to_owned(),
            kind: ProviderKind::Discovery,
            status: ProviderStatus::Degraded,
            record_count: None,
            last_attempt_at: None,
            last_success_at: None,
            last_error: Some("read /home/user/.config/heroic/legendaryConfig failed".to_owned()),
        };
        assert_eq!(
            redact_provider_error(degraded).last_error.as_deref(),
            Some("provider error")
        );

        // A provider with no error is left alone.
        let healthy =
            crate::state::starting_provider_health("desktop-apps", ProviderKind::Discovery);
        assert!(redact_provider_error(healthy).last_error.is_none());
    }

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
        // Bind before returning so callers cannot race the launch request
        // against the test bridge becoming available.
        let listener = UnixListener::bind(&socket_path).expect("bind fake bridge socket");
        tokio::spawn(async move {
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

    #[tokio::test]
    async fn protected_routes_authenticate_before_extracting_invalid_requests() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let state = Arc::new(AppState::new(
            Config {
                bind_address: "127.0.0.1:38400".parse().unwrap(),
                local_admin_address: "127.0.0.1:38401".parse().unwrap(),
                database_path: temporary.path().join("hearthdeck.db"),
                bridge_socket_path: temporary.path().join("bridge.sock"),
                lan_enabled: false,
                tls: None,
                romm: crate::config::RommPaths::resolve(None, None, None),
            },
            database,
        ));
        let app = router(state.clone());
        for (method, path) in [
            ("GET", "/v1/diagnostics"),
            ("GET", "/v1/library"),
            ("GET", "/v1/activity/recent?limit=invalid"),
            ("GET", "/v1/collections"),
            ("POST", "/v1/collections/favorites/items"),
            ("DELETE", "/v1/collections/favorites/items"),
            ("GET", "/v1/retro/consoles"),
            ("GET", "/v1/retro/roms?limit=invalid"),
            ("GET", "/v1/retro/facets"),
            ("GET", "/v1/retro/roms/invalid"),
            ("POST", "/v1/retro/roms/invalid/favorite"),
            ("POST", "/v1/retro/roms/invalid/backlog"),
            ("POST", "/v1/retro/service/restart"),
            ("GET", "/v1/retro/assets"),
            ("GET", "/v1/retro/settings"),
            ("PUT", "/v1/retro/settings"),
            ("DELETE", "/v1/retro/settings"),
            ("POST", "/v1/retro/login"),
            ("POST", "/v1/retro/link"),
            ("GET", "/v1/retro/link"),
            ("POST", "/v1/stremio/link"),
            ("GET", "/v1/stremio/link"),
            ("GET", "/v1/stremio/settings"),
            ("DELETE", "/v1/stremio/settings"),
            ("POST", "/v1/library/rescan"),
            ("GET", "/v1/categorization"),
            ("POST", "/v1/categorization/enable"),
            ("POST", "/v1/categorization/disable"),
            ("POST", "/v1/categorization/scan"),
            ("DELETE", "/v1/categorization/collections"),
            ("POST", "/v1/categorization/form-factors"),
            ("GET", "/v1/settings"),
            ("PUT", "/v1/settings"),
            ("POST", "/v1/discovery/test/refresh"),
            ("POST", "/v1/metadata/test/refresh"),
            ("POST", "/v1/launch/test?input_profile=invalid"),
            ("GET", "/v1/sessions/active"),
            ("POST", "/v1/sessions/active/stop"),
            ("POST", "/v1/sessions/test/stop"),
            ("GET", "/v1/events"),
        ] {
            for token in [None, Some("Bearer invalid"), Some("Basic invalid")] {
                let mut request = Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json");
                if let Some(token) = token {
                    request = request.header("authorization", token);
                }
                let response = app
                    .clone()
                    .oneshot(request.body(Body::from("{")).unwrap())
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    StatusCode::UNAUTHORIZED,
                    "{method} {path}, {token:?}"
                );
            }
        }
        assert_eq!(
            app.clone()
                .oneshot(Request::get("/v1/health").body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::OK,
        );
        assert_eq!(
            app.clone()
                .oneshot(Request::post("/v1/pairing").body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND,
        );
        assert_eq!(
            app.clone()
                .oneshot(Request::get("/not-a-route").body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND,
        );
        let (_, pairing) = response_json(
            local_router(state.clone()),
            Request::post("/v1/pairing").body(Body::empty()).unwrap(),
        )
        .await;
        let (status, paired) = response_json(
            app.clone(),
            Request::post("/v1/pairing/complete")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "code": pairing["code"], "client_name": "route-auth-test"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let token = paired["token"].as_str().unwrap();
        let response = app
            .oneshot(
                Request::put("/v1/settings")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from("{"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "paired requests reach extractors"
        );
    }

    #[tokio::test]
    async fn stop_active_requires_auth_and_finishes_only_the_bridge_selected_play() {
        use crate::activity::ActivityEntry;
        use hearthdeck_protocol::BridgeErrorCode;

        for (reply, expected_status, expected_message) in [
            (
                BridgeResponse::StopAccepted {
                    session_id: "selected".to_owned(),
                },
                StatusCode::ACCEPTED,
                None,
            ),
            (
                BridgeResponse::Error {
                    code: BridgeErrorCode::NotFound,
                    message: "no active session".to_owned(),
                },
                StatusCode::NOT_FOUND,
                Some("no active session"),
            ),
            (
                BridgeResponse::Error {
                    code: BridgeErrorCode::LaunchFailed,
                    message: "systemd stop failed".to_owned(),
                },
                StatusCode::BAD_GATEWAY,
                Some("systemd stop failed"),
            ),
            (
                BridgeResponse::Health {
                    version: "unexpected".to_owned(),
                },
                StatusCode::BAD_GATEWAY,
                Some("bridge rejected application-session stop"),
            ),
        ] {
            let temporary = tempfile::tempdir().unwrap();
            let database = Database::connect(&temporary.path().join("hearthdeck.db"))
                .await
                .unwrap();
            database.migrate().await.unwrap();
            let state = Arc::new(AppState::new(
                Config {
                    bind_address: "127.0.0.1:38400".parse().unwrap(),
                    local_admin_address: "127.0.0.1:38401".parse().unwrap(),
                    database_path: temporary.path().join("hearthdeck.db"),
                    bridge_socket_path: temporary.path().join("bridge.sock"),
                    lan_enabled: false,
                    tls: None,
                    romm: crate::config::RommPaths::resolve(None, None, None),
                },
                database,
            ));
            for id in ["selected", "other"] {
                state
                    .activity
                    .session_started(
                        ActivityEntry {
                            id: id.to_owned(),
                            title: id.to_owned(),
                            icon: None,
                            categories: Vec::new(),
                            kind: Some("game".to_owned()),
                            source: "test".to_owned(),
                            metadata: serde_json::Value::Null,
                        },
                        &ApplicationSession {
                            id: id.to_owned(),
                            source_id: "test".to_owned(),
                            application_id: id.to_owned(),
                            state: ApplicationSessionState::Running,
                        },
                    )
                    .await
                    .unwrap();
            }
            let (status, _) = response_json(
                router(state.clone()),
                Request::post("/v1/sessions/active/stop")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(state.activity.running_sessions().await.unwrap().len(), 2);

            let (_, pairing) = response_json(
                local_router(state.clone()),
                Request::post("/v1/pairing").body(Body::empty()).unwrap(),
            )
            .await;
            let (_, paired) = response_json(
                router(state.clone()),
                Request::post("/v1/pairing/complete")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "code": pairing["code"], "client_name": "stop-active-test"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await;
            let token = paired["token"].as_str().unwrap();
            let bridge =
                spawn_fake_bridge(state.config.bridge_socket_path.clone(), move |request| {
                    assert!(matches!(
                        request,
                        BridgeRequest::StopActiveApplicationSession
                    ));
                    reply
                });
            let response = router(state.clone())
                .oneshot(
                    Request::post("/v1/sessions/active/stop")
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected_status);
            timeout(Duration::from_secs(5), bridge)
                .await
                .unwrap()
                .unwrap();
            let running = state.activity.running_sessions().await.unwrap();
            assert_eq!(
                running.contains(&"selected".to_owned()),
                expected_status != StatusCode::ACCEPTED
            );
            assert!(running.contains(&"other".to_owned()));
            if let Some(message) = expected_message {
                let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
                let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(json["error"], message);
            }
        }
    }

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

    /// The Stremio routes sit behind the daemon's own authentication like every
    /// other account-touching route. One of them starts a session and another says
    /// whether a session exists, so an unpaired client on the LAN must reach
    /// neither. Authentication is checked before anything is dialled, so this
    /// holds without a network and without a Stremio account.
    #[tokio::test]
    async fn the_stremio_routes_require_authentication() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let state: SharedState = Arc::new(AppState::new(
            Config {
                bind_address: "127.0.0.1:38400".parse::<SocketAddr>().unwrap(),
                local_admin_address: "127.0.0.1:38401".parse::<SocketAddr>().unwrap(),
                database_path: temporary.path().join("hearthdeck.db"),
                bridge_socket_path: temporary.path().join("bridge.sock"),
                lan_enabled: false,
                tls: None,
                romm: crate::config::RommPaths::resolve(None, None, None),
            },
            database,
        ));

        for request in [
            Request::post("/v1/stremio/link")
                .body(Body::empty())
                .unwrap(),
            Request::get("/v1/stremio/link")
                .body(Body::empty())
                .unwrap(),
            Request::get("/v1/stremio/settings")
                .body(Body::empty())
                .unwrap(),
            Request::delete("/v1/stremio/settings")
                .body(Body::empty())
                .unwrap(),
            Request::post("/v1/retro/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"base_url":"http://127.0.0.1:8080","username":"u","password":"p"}"#,
                ))
                .unwrap(),
            Request::post("/v1/retro/link")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"base_url":"http://127.0.0.1:8080"}"#))
                .unwrap(),
            Request::get("/v1/retro/link").body(Body::empty()).unwrap(),
        ] {
            let method = request.method().clone();
            let path = request.uri().path().to_owned();
            let response = router(state.clone()).oneshot(request).await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {path} must require authentication"
            );
        }
    }

    /// A Stremio record asks the bridge to open the *title*, not merely the application:
    /// the video id travels and the bridge builds the URI from it.
    #[tokio::test]
    async fn launching_a_stremio_record_opens_the_title() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let state: SharedState = Arc::new(AppState::new(
            Config {
                bind_address: "127.0.0.1:38400".parse::<SocketAddr>().unwrap(),
                local_admin_address: "127.0.0.1:38401".parse::<SocketAddr>().unwrap(),
                database_path: temporary.path().join("hearthdeck.db"),
                bridge_socket_path: temporary.path().join("bridge.socket"),
                lan_enabled: false,
                tls: None,
                romm: crate::config::RommPaths::resolve(None, None, None),
            },
            database,
        ));

        state
            .catalog
            .replace_source(
                "stremio",
                vec![
                    CatalogRecord {
                        id: "stremio:tt0149460".to_owned(),
                        title: "Futurama".to_owned(),
                        kind: "series".to_owned(),
                        launch_id: Some("com.stremio.Stremio.desktop".to_owned()),
                        icon: None,
                        // The provider's own payload: the catalog wraps it under
                        // `discovery` when it serves the record, which is the shape
                        // `stremio_title_target` reads back.
                        metadata: serde_json::json!({
                            "source": "stremio",
                            "video_id": "tt0149460:4:7"
                        }),
                        updated_at: "2026-01-01T00:00:00Z".to_owned(),
                    },
                    // Published before the provider carried video ids, so it has none.
                    CatalogRecord {
                        id: "stremio:tt0373074".to_owned(),
                        title: "Kung Fu Hustle".to_owned(),
                        kind: "movie".to_owned(),
                        launch_id: Some("com.stremio.Stremio.desktop".to_owned()),
                        icon: None,
                        metadata: serde_json::json!({"source": "stremio"}),
                        updated_at: "2026-01-01T00:00:00Z".to_owned(),
                    },
                ],
            )
            .await
            .unwrap();

        // The launch route is authenticated, so pair first.
        let (_, pairing) = response_json(
            local_router(state.clone()),
            Request::post("/v1/pairing").body(Body::empty()).unwrap(),
        )
        .await;
        let code = pairing["code"].as_str().unwrap();
        let (_, paired) = response_json(
            router(state.clone()),
            Request::post("/v1/pairing/complete")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"code": code, "client_name": "launch-test"}).to_string(),
                ))
                .unwrap(),
        )
        .await;
        let token = paired["token"].as_str().unwrap().to_owned();

        let bridge = spawn_fake_bridge(state.config.bridge_socket_path.clone(), |request| {
            let BridgeRequest::LaunchStremioTitle {
                application_id,
                video_id,
                session_id,
                input_profile,
                ..
            } = request
            else {
                panic!("expected a LaunchStremioTitle request, got {request:?}");
            };
            // The entry the provider resolved travels with the request, so the bridge
            // never has to recognise Stremio itself.
            assert_eq!(application_id, "com.stremio.Stremio.desktop");
            assert_eq!(video_id, "tt0149460:4:7");
            assert_eq!(input_profile, InputProfile::Desktop);
            BridgeResponse::LaunchAccepted {
                session: ApplicationSession {
                    id: session_id,
                    source_id: "desktop-apps".to_owned(),
                    application_id: "com.stremio.Stremio.desktop".to_owned(),
                    state: ApplicationSessionState::Running,
                },
            }
        });

        let (status, launched) = response_json(
            router(state.clone()),
            Request::post("/v1/launch/stremio:tt0149460?input_profile=desktop")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // The session is the desktop application underneath, so it reports that.
        assert_eq!(launched["source_id"], "desktop-apps");
        assert_eq!(launched["application_id"], "com.stremio.Stremio.desktop");
        bridge.await.unwrap();

        // A record with no video id falls back to opening the application: the rail
        // keeps working for anything published before the ids existed. The first fake
        // bridge leaves its socket file behind, and binding a path that exists fails.
        let _ = std::fs::remove_file(&state.config.bridge_socket_path);
        let fallback = spawn_fake_bridge(state.config.bridge_socket_path.clone(), |request| {
            let BridgeRequest::LaunchApplication {
                source_id,
                application_id,
                session_id,
                ..
            } = request
            else {
                panic!("expected a LaunchApplication request, got {request:?}");
            };
            assert_eq!(source_id, "desktop-apps");
            assert_eq!(application_id, "com.stremio.Stremio.desktop");
            BridgeResponse::LaunchAccepted {
                session: ApplicationSession {
                    id: session_id,
                    source_id,
                    application_id,
                    state: ApplicationSessionState::Running,
                },
            }
        });
        let (status, _) = response_json(
            router(state.clone()),
            Request::post("/v1/launch/stremio:tt0373074")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        fallback.await.unwrap();
    }

    /// One route serves every launch kind: the id travels untouched and the
    /// daemon decides what it names — the entry prefix stripped against the
    /// catalog, where the record's own `source_id` picks the launch shape. A
    /// client never parses a prefix, and an id naming nothing is refused
    /// rather than guessed at.
    #[tokio::test]
    async fn the_single_launch_route_decides_what_an_item_id_names() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let state: SharedState = Arc::new(AppState::new(
            Config {
                bind_address: "127.0.0.1:38400".parse::<SocketAddr>().unwrap(),
                local_admin_address: "127.0.0.1:38401".parse::<SocketAddr>().unwrap(),
                database_path: temporary.path().join("hearthdeck.db"),
                bridge_socket_path: temporary.path().join("bridge.socket"),
                lan_enabled: false,
                tls: None,
                romm: crate::config::RommPaths::resolve(None, None, None),
            },
            database,
        ));

        state
            .catalog
            .replace_source(
                "stremio",
                vec![CatalogRecord {
                    id: "stremio:tt0149460".to_owned(),
                    title: "Futurama".to_owned(),
                    kind: "series".to_owned(),
                    launch_id: Some("com.stremio.Stremio.desktop".to_owned()),
                    icon: None,
                    metadata: serde_json::json!({
                        "source": "stremio",
                        "video_id": "tt0149460:4:7"
                    }),
                    updated_at: "2026-01-01T00:00:00Z".to_owned(),
                }],
            )
            .await
            .unwrap();

        // The launch route is authenticated, so pair first.
        let (_, pairing) = response_json(
            local_router(state.clone()),
            Request::post("/v1/pairing").body(Body::empty()).unwrap(),
        )
        .await;
        let code = pairing["code"].as_str().unwrap();
        let (_, paired) = response_json(
            router(state.clone()),
            Request::post("/v1/pairing/complete")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"code": code, "client_name": "launch-test"}).to_string(),
                ))
                .unwrap(),
        )
        .await;
        let token = paired["token"].as_str().unwrap().to_owned();

        let unauthorized = router(state.clone())
            .oneshot(
                Request::post("/v1/launch/hearthdeck:stremio:tt0149460")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let bridge = spawn_fake_bridge(state.config.bridge_socket_path.clone(), |request| {
            let BridgeRequest::LaunchStremioTitle {
                application_id,
                video_id,
                session_id,
                input_profile,
                ..
            } = request
            else {
                panic!("expected a LaunchStremioTitle request, got {request:?}");
            };
            // The prefixed item id resolved through the catalog to a Stremio
            // record, and that record's video id reached the bridge: which
            // provider sits behind the id is the daemon's decision, not the
            // client's.
            assert_eq!(application_id, "com.stremio.Stremio.desktop");
            assert_eq!(video_id, "tt0149460:4:7");
            assert_eq!(input_profile, InputProfile::Desktop);
            BridgeResponse::LaunchAccepted {
                session: ApplicationSession {
                    id: session_id,
                    source_id: "desktop-apps".to_owned(),
                    application_id: "com.stremio.Stremio.desktop".to_owned(),
                    state: ApplicationSessionState::Running,
                },
            }
        });

        let (status, launched) = response_json(
            router(state.clone()),
            Request::post("/v1/launch/hearthdeck:stremio:tt0149460?input_profile=desktop")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(launched["source_id"], "desktop-apps");
        assert_eq!(launched["application_id"], "com.stremio.Stremio.desktop");
        bridge.await.unwrap();

        // An id that names nothing is refused, not guessed at.
        let (status, _) = response_json(
            router(state),
            Request::post("/v1/launch/does-not-exist")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// The Stremio rail is composed from what the provider published: only that
    /// source's records, newest change first, and carrying the catalog ids a client
    /// hands straight back to the launch route.
    #[tokio::test]
    async fn the_stremio_rail_reads_the_records_the_provider_published() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let state: SharedState = Arc::new(AppState::new(
            Config {
                bind_address: "127.0.0.1:38400".parse::<SocketAddr>().unwrap(),
                local_admin_address: "127.0.0.1:38401".parse::<SocketAddr>().unwrap(),
                database_path: temporary.path().join("hearthdeck.db"),
                bridge_socket_path: temporary.path().join("bridge.socket"),
                lan_enabled: false,
                tls: None,
                romm: crate::config::RommPaths::resolve(None, None, None),
            },
            database,
        ));

        let record = |id: &str, title: &str, updated_at: &str| CatalogRecord {
            id: id.to_owned(),
            title: title.to_owned(),
            kind: "movie".to_owned(),
            launch_id: Some("com.stremio.Stremio.desktop".to_owned()),
            icon: Some("https://images.metahub.space/poster/medium/tt1/img".to_owned()),
            metadata: serde_json::json!({"source": "stremio", "progress": 0.34}),
            updated_at: updated_at.to_owned(),
        };
        state
            .catalog
            .replace_source(
                "stremio",
                vec![
                    record("stremio:tt1", "Older", "2026-01-01T00:00:00Z"),
                    record("stremio:tt2", "Newer", "2026-02-01T00:00:00Z"),
                ],
            )
            .await
            .unwrap();
        // Another source's records must not leak into this rail.
        state
            .catalog
            .replace_source(
                "desktop-apps",
                vec![CatalogRecord {
                    id: "desktop:other.desktop".to_owned(),
                    title: "Not Stremio".to_owned(),
                    kind: "application".to_owned(),
                    launch_id: None,
                    icon: None,
                    metadata: serde_json::Value::Null,
                    updated_at: "2026-03-01T00:00:00Z".to_owned(),
                }],
            )
            .await
            .unwrap();

        // `ApiError` carries no `Debug`, so a failure is reported by its message
        // rather than unwrapped.
        let items = match super::continue_watching_items(&state).await {
            Ok(items) => items,
            Err(error) => panic!("the rail should have composed: {}", error.message),
        };
        let ids: Vec<&str> = items.iter().map(|item| item.item_id.as_str()).collect();
        // Prefixed, because that prefix is what the client's launch gate strips
        // before handing the catalog id back to the daemon.
        assert_eq!(
            ids,
            vec!["hearthdeck:stremio:tt2", "hearthdeck:stremio:tt1"]
        );
        assert_eq!(items[0].name, "Newer");
        assert!(items[0].icon.is_some());
        // The play record rides along only for the rules that have one.
        assert!(items[0].played.is_none());

        // Nothing published yet reads as an empty rail rather than a failure.
        state
            .catalog
            .replace_source("stremio", Vec::new())
            .await
            .unwrap();
        let empty = match super::continue_watching_items(&state).await {
            Ok(items) => items,
            Err(error) => panic!(
                "an unlinked account composes an empty rail: {}",
                error.message
            ),
        };
        assert!(empty.is_empty());
    }

    /// The hero banner's collection is composed from what the arrivals provider
    /// published: newest release first, and carrying the wide art — a background,
    /// the title's own wordmark, a synopsis — that a rail item never has.
    #[tokio::test]
    async fn the_banner_collection_carries_the_art_the_hero_draws_with() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let state: SharedState = Arc::new(AppState::new(
            Config {
                bind_address: "127.0.0.1:38400".parse::<SocketAddr>().unwrap(),
                local_admin_address: "127.0.0.1:38401".parse::<SocketAddr>().unwrap(),
                database_path: temporary.path().join("hearthdeck.db"),
                bridge_socket_path: temporary.path().join("bridge.socket"),
                lan_enabled: false,
                tls: None,
                romm: crate::config::RommPaths::resolve(None, None, None),
            },
            database,
        ));

        let arrival = |tt: &str, title: &str, released: &str| CatalogRecord {
            id: format!("stremio-arrivals:{tt}"),
            title: title.to_owned(),
            kind: "series".to_owned(),
            launch_id: Some("com.stremio.Stremio.desktop".to_owned()),
            icon: Some(format!(
                "https://images.metahub.space/poster/medium/{tt}/img"
            )),
            metadata: serde_json::json!({
                "source": "stremio-arrivals",
                "type": "series",
                "video_id": tt,
                "background": format!("https://images.metahub.space/background/medium/{tt}/img"),
                "logo": format!("https://images.metahub.space/logo/medium/{tt}/img"),
                "description": format!("About {title}"),
                "released": released,
            }),
            // The provider dates a record by its release, so newest release first
            // is the order this listing answers with.
            updated_at: released.to_owned(),
        };
        state
            .catalog
            .replace_source(
                "stremio-arrivals",
                vec![
                    arrival("tt1", "Older", "2026-10-01T00:00:00.000Z"),
                    arrival("tt2", "Newer", "2026-10-20T00:00:00.000Z"),
                ],
            )
            .await
            .unwrap();
        // Another source's records must not leak into the banner.
        state
            .catalog
            .replace_source(
                "stremio",
                vec![CatalogRecord {
                    id: "stremio:tt3".to_owned(),
                    title: "Not an arrival".to_owned(),
                    kind: "movie".to_owned(),
                    launch_id: None,
                    icon: None,
                    metadata: serde_json::json!({"source": "stremio"}),
                    updated_at: "2026-09-01T00:00:00.000Z".to_owned(),
                }],
            )
            .await
            .unwrap();

        let items = match super::new_arrival_items(&state).await {
            Ok(items) => items,
            Err(error) => panic!("the banner should have composed: {}", error.message),
        };
        let ids: Vec<&str> = items.iter().map(|item| item.item_id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "hearthdeck:stremio-arrivals:tt2",
                "hearthdeck:stremio-arrivals:tt1"
            ]
        );
        assert_eq!(items[0].name, "Newer");
        // The whole reason this collection is not just names and posters like
        // every rail: the art the hero banner draws with.
        assert_eq!(
            items[0].background.as_deref(),
            Some("https://images.metahub.space/background/medium/tt2/img")
        );
        assert_eq!(
            items[0].logo.as_deref(),
            Some("https://images.metahub.space/logo/medium/tt2/img")
        );
        assert_eq!(items[0].description.as_deref(), Some("About Newer"));
    }

    /// A new arrival names its own kind, and the launch route carries it to the
    /// bridge: a bare IMDb id is a film or a series only as the record says, so a
    /// series banner slide opens the series' page rather than the film one.
    #[tokio::test]
    async fn launching_an_arrival_opens_the_kind_of_title_the_record_says() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let state: SharedState = Arc::new(AppState::new(
            Config {
                bind_address: "127.0.0.1:38400".parse::<SocketAddr>().unwrap(),
                local_admin_address: "127.0.0.1:38401".parse::<SocketAddr>().unwrap(),
                database_path: temporary.path().join("hearthdeck.db"),
                bridge_socket_path: temporary.path().join("bridge.socket"),
                lan_enabled: false,
                tls: None,
                romm: crate::config::RommPaths::resolve(None, None, None),
            },
            database,
        ));

        state
            .catalog
            .replace_source(
                "stremio-arrivals",
                vec![CatalogRecord {
                    id: "stremio-arrivals:tt1234567".to_owned(),
                    title: "A series".to_owned(),
                    kind: "series".to_owned(),
                    launch_id: Some("com.stremio.Stremio.desktop".to_owned()),
                    icon: None,
                    metadata: serde_json::json!({
                        "source": "stremio-arrivals",
                        "type": "series",
                        "video_id": "tt1234567"
                    }),
                    updated_at: "2026-10-20T00:00:00.000Z".to_owned(),
                }],
            )
            .await
            .unwrap();

        // The launch route is authenticated, so pair first.
        let (_, pairing) = response_json(
            local_router(state.clone()),
            Request::post("/v1/pairing").body(Body::empty()).unwrap(),
        )
        .await;
        let code = pairing["code"].as_str().unwrap();
        let (_, paired) = response_json(
            router(state.clone()),
            Request::post("/v1/pairing/complete")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"code": code, "client_name": "arrival-launch"}).to_string(),
                ))
                .unwrap(),
        )
        .await;
        let token = paired["token"].as_str().unwrap().to_owned();

        let bridge = spawn_fake_bridge(state.config.bridge_socket_path.clone(), |request| {
            let BridgeRequest::LaunchStremioTitle {
                application_id,
                video_id,
                media_type,
                session_id,
                ..
            } = request
            else {
                panic!("expected a LaunchStremioTitle request, got {request:?}");
            };
            assert_eq!(application_id, "com.stremio.Stremio.desktop");
            // The bare id reaches the bridge, and its kind with it: the two
            // together are what names the series' own page.
            assert_eq!(video_id, "tt1234567");
            assert_eq!(media_type, hearthdeck_protocol::StremioMediaType::Series);
            BridgeResponse::LaunchAccepted {
                session: ApplicationSession {
                    id: session_id,
                    source_id: "desktop-apps".to_owned(),
                    application_id: "com.stremio.Stremio.desktop".to_owned(),
                    state: ApplicationSessionState::Running,
                },
            }
        });

        let (status, launched) = response_json(
            router(state.clone()),
            Request::post("/v1/launch/hearthdeck:stremio-arrivals:tt1234567?input_profile=desktop")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(launched["application_id"], "com.stremio.Stremio.desktop");
        bridge.await.unwrap();
    }

    #[tokio::test]
    async fn restoring_the_dashboard_drops_only_the_models_rails() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let state: SharedState = Arc::new(AppState::new(
            Config {
                bind_address: "127.0.0.1:38400".parse::<SocketAddr>().unwrap(),
                local_admin_address: "127.0.0.1:38401".parse::<SocketAddr>().unwrap(),
                database_path: temporary.path().join("hearthdeck.db"),
                bridge_socket_path: temporary.path().join("bridge.socket"),
                lan_enabled: false,
                tls: None,
                romm: crate::config::RommPaths::resolve(None, None, None),
            },
            database,
        ));
        state
            .collections
            .replace_owned(
                crate::collections::OWNER_LAYA,
                &[crate::collections::OwnedCollection {
                    slug: "laya:watch".to_owned(),
                    name: Some("Watch".to_owned()),
                    rule: crate::collections::owned_rule(crate::collections::OWNER_LAYA, "watch"),
                }],
            )
            .await
            .unwrap();
        let before = state.collections.list().await.unwrap();
        assert!(before.iter().any(|row| row.slug == "laya:watch"));

        super::remove_laya_collections(&state).await;

        let after = state.collections.list().await.unwrap();
        assert!(
            after
                .iter()
                .all(|row| row.owner != crate::collections::OWNER_LAYA)
        );
        assert_eq!(after.len(), before.len() - 1, "the built-in rails stay");
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
                romm: crate::config::RommPaths::resolve(None, None, None),
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

        // Nothing exercised the actual launch endpoints before this: launch
        // dispatch (daemon -> bridge socket -> LaunchAccepted -> response)
        // had zero route-level coverage. `test:app`'s launch_id ("test.app")
        // comes from ApiTestProvider/discovery above.
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
            // The bridge resolves desktop entries in its own namespace and refuses
            // every other source, so the record's own source ("test-apps" here) must
            // not be what arrives. It was, and that is why launching a record from any
            // other provider failed with "unsupported application source".
            assert_eq!(source_id, "desktop-apps");
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
            Request::post("/v1/launch/test:app?input_profile=desktop")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // The session names the namespace it was launched in, which is the desktop
        // one whatever provider published the record.
        assert_eq!(launched["source_id"], "desktop-apps");
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
                Request::post("/v1/launch/does-not-exist")
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

    /// The retro facets route is another account-touching frontier: it dials the
    /// user's RomM server, so an unpaired client must not reach it. The check
    /// runs before any HTTP, so this holds without a network.
    #[tokio::test]
    async fn the_retro_facets_route_requires_authentication() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let state: SharedState = Arc::new(AppState::new(
            Config {
                bind_address: "127.0.0.1:38400".parse::<SocketAddr>().unwrap(),
                local_admin_address: "127.0.0.1:38401".parse::<SocketAddr>().unwrap(),
                database_path: temporary.path().join("hearthdeck.db"),
                bridge_socket_path: temporary.path().join("bridge.sock"),
                lan_enabled: false,
                tls: None,
                romm: crate::config::RommPaths::resolve(None, None, None),
            },
            database,
        ));

        let response = router(state)
            .oneshot(
                Request::get("/v1/retro/facets")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// Facets come from RomM's library-wide filter values: one chip row per
    /// populated dimension, and no row for a dimension RomM reported empty.
    #[test]
    fn retro_facets_offer_populated_dimensions_only() {
        let facets = super::retro_facets(crate::diagnostics::RommFilterValues {
            genres: vec!["Action".to_owned(), "RPG".to_owned()],
            regions: Vec::new(),
        });

        assert_eq!(facets.len(), 1, "facets: {facets:?}");
        assert_eq!(facets[0].id, "genre");
        assert_eq!(facets[0].label, "Genre");
        assert_eq!(
            facets[0]
                .options
                .iter()
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["Action", "RPG"]
        );
    }

    fn romm_game_with_siblings(
        siblings: Vec<crate::diagnostics::RommSiblingRom>,
    ) -> super::RetroGame {
        use crate::diagnostics::{RommGame, RommGameMetadata};

        super::RetroGame::from(&RommGame {
            id: 1,
            platform_id: 7,
            name: Some("Shenmue".to_owned()),
            fs_name_no_tags: "Shenmue".to_owned(),
            fs_name: Some("Shenmue (Disc 1).chd".to_owned()),
            summary: None,
            path_cover_small: None,
            path_cover_large: None,
            url_cover: None,
            merged_screenshots: Vec::new(),
            path_manual: None,
            has_manual: false,
            metadatum: RommGameMetadata::default(),
            regions: Vec::new(),
            sibling_roms: siblings,
            platform_display_name: None,
            languages: Vec::new(),
            tags: Vec::new(),
            fs_size_bytes: None,
            fs_path: None,
            is_favorite: None,
            rom_user: crate::diagnostics::RommRomUser::default(),
        })
    }

    #[test]
    fn details_projection_carries_the_fields_the_grid_omits() {
        use crate::diagnostics::{RommGame, RommGameMetadata};

        let game = RommGame {
            id: 4014,
            platform_id: 9,
            name: Some("Zoop".to_owned()),
            fs_name_no_tags: "Zoop".to_owned(),
            fs_name: Some("Zoop (NA).sfc".to_owned()),
            summary: Some("A puzzle game".to_owned()),
            path_cover_small: Some("/assets/romm/resources/roms/9/4014/cover/small.png".to_owned()),
            path_cover_large: Some("/assets/romm/resources/roms/9/4014/cover/big.png".to_owned()),
            url_cover: None,
            merged_screenshots: vec![
                "/assets/romm/resources/roms/9/4014/screenshots/0.jpg".to_owned(),
            ],
            path_manual: None,
            has_manual: true,
            metadatum: RommGameMetadata {
                genres: vec!["Puzzle".to_owned()],
                player_count: "1".to_owned(),
                first_release_date: Some(791_596_800_000),
                companies: vec!["PanelComp".to_owned()],
                game_modes: vec!["Single player".to_owned()],
                age_ratings: vec!["E".to_owned()],
                average_rating: Some(62.47),
            },
            regions: Vec::new(),
            sibling_roms: Vec::new(),
            platform_display_name: Some("Super Nintendo Entertainment System".to_owned()),
            languages: Vec::new(),
            tags: vec!["NA".to_owned()],
            fs_size_bytes: Some(524_288),
            fs_path: None,
            is_favorite: Some(true),
            rom_user: crate::diagnostics::RommRomUser { backlogged: true },
        };

        let details = super::RetroGameDetails::from(&game);

        // The hero uses the large cover; the grid's small cover stays as the
        // fallback so a missing large cover still draws something.
        assert_eq!(
            details.cover_large_path.as_deref(),
            Some("/assets/romm/resources/roms/9/4014/cover/big.png")
        );
        assert_eq!(
            details.cover_path.as_deref(),
            Some("/assets/romm/resources/roms/9/4014/cover/small.png")
        );
        assert_eq!(
            details.platform_name.as_deref(),
            Some("Super Nintendo Entertainment System")
        );
        // The release date must be readable, not year 27057: RomM sent the
        // merged timestamp in milliseconds.
        assert_eq!(details.release_year, Some(1995));
        assert_eq!(details.release_date.as_deref(), Some("1995-02-01"));
        assert_eq!(details.companies, vec!["PanelComp"]);
        assert_eq!(details.game_modes, vec!["Single player"]);
        assert_eq!(details.age_ratings, vec!["E"]);
        assert_eq!(details.average_rating, Some(62.47));
        assert_eq!(details.file_size_bytes, Some(524_288));
        assert_eq!(details.tags, vec!["NA"]);
        // User state travels with the game, so the details screen can open with
        // its favorite and play-later buttons already in the right state.
        assert!(details.favorite);
        assert!(details.backlogged);
    }

    #[test]
    fn sibling_versions_are_labelled_by_file_name_not_the_shared_title() {
        use crate::diagnostics::RommSiblingRom;

        // RomM's `name` is the same game title on every rival, so labelling
        // from it makes the discs indistinguishable; `fs_name_no_ext` keeps
        // the `(Disc N)` tag.
        let game = romm_game_with_siblings(vec![
            RommSiblingRom {
                id: 2,
                name: Some("Shenmue".to_owned()),
                fs_name_no_ext: "Shenmue (Disc 2)".to_owned(),
                fs_name_no_tags: "Shenmue".to_owned(),
                is_main_sibling: false,
            },
            RommSiblingRom {
                id: 3,
                name: Some("Shenmue".to_owned()),
                fs_name_no_ext: "Shenmue (Disc 3)".to_owned(),
                fs_name_no_tags: "Shenmue".to_owned(),
                is_main_sibling: false,
            },
        ]);

        let titles: Vec<&str> = game
            .sibling_roms
            .iter()
            .map(|version| version.title.as_str())
            .collect();
        assert_eq!(titles, vec!["Shenmue (Disc 2)", "Shenmue (Disc 3)"]);
        // The representative (what "Run" launches) is labelled from its own
        // file name too, so disc 1 is not left implicit.
        assert_eq!(game.version_label.as_deref(), Some("Shenmue (Disc 1)"));
    }

    #[test]
    fn duplicate_sibling_labels_get_the_rom_id_appended() {
        use crate::diagnostics::RommSiblingRom;

        let game = romm_game_with_siblings(vec![
            RommSiblingRom {
                id: 2,
                name: Some("Shenmue".to_owned()),
                fs_name_no_ext: "Shenmue".to_owned(),
                fs_name_no_tags: "Shenmue".to_owned(),
                is_main_sibling: false,
            },
            RommSiblingRom {
                id: 3,
                name: Some("Shenmue".to_owned()),
                fs_name_no_ext: "Shenmue".to_owned(),
                fs_name_no_tags: "Shenmue".to_owned(),
                is_main_sibling: false,
            },
        ]);

        let titles: Vec<&str> = game
            .sibling_roms
            .iter()
            .map(|version| version.title.as_str())
            .collect();
        assert_eq!(titles, vec!["Shenmue", "Shenmue (#3)"]);
    }

    #[test]
    fn single_file_games_carry_no_version_label() {
        let game = romm_game_with_siblings(Vec::new());
        assert!(game.version_label.is_none());
        assert!(game.sibling_roms.is_empty());
    }

    #[test]
    fn a_resolved_play_rides_along_with_its_collection_item() {
        use crate::activity::{ActivityEntry, RecentActivity};

        let item = super::played_collection_item(RecentActivity {
            entry: ActivityEntry {
                id: "romm:7".into(),
                title: "Chrono Trigger".into(),
                icon: Some("/covers/chrono.png".into()),
                categories: vec!["Game".into()],
                kind: Some("game".into()),
                source: "romm".into(),
                metadata: serde_json::json!({"release_year": 1995}),
            },
            last_launched_at: "2026-09-23T10:00:00Z".into(),
            launch_count: 3,
        });

        // The two fields every item carries, so a client can draw it without
        // knowing which kind of collection it came from...
        assert_eq!(item.item_id, "romm:7");
        assert_eq!(item.name, "Chrono Trigger");
        // ...and the play, so it can classify and launch it the way it already
        // does for a play it fetched from the activity endpoint.
        let played = item.played.expect("the play travels with the item");
        assert_eq!(played.entry.kind.as_deref(), Some("game"));
        assert_eq!(played.entry.source, "romm");
        assert_eq!(played.launch_count, 3);
    }

    /// The dashboard's "Recently Played" rail is composed from the games a person
    /// played, not from every launch: the application is still a play, and it is
    /// not a card on this rail.
    #[tokio::test]
    async fn the_recently_played_rail_holds_games_and_not_other_plays() {
        use crate::activity::ActivityEntry;

        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let state: SharedState = Arc::new(AppState::new(
            Config {
                bind_address: "127.0.0.1:38400".parse::<SocketAddr>().unwrap(),
                local_admin_address: "127.0.0.1:38401".parse::<SocketAddr>().unwrap(),
                database_path: temporary.path().join("hearthdeck.db"),
                bridge_socket_path: temporary.path().join("bridge.socket"),
                lan_enabled: false,
                tls: None,
                romm: crate::config::RommPaths::resolve(None, None, None),
            },
            database,
        ));

        let entry = |id: &str, kind: &str| ActivityEntry {
            id: id.to_owned(),
            title: id.to_owned(),
            icon: None,
            categories: Vec::new(),
            kind: Some(kind.to_owned()),
            source: "test".to_owned(),
            metadata: serde_json::Value::Null,
        };
        let session = |id: &str| ApplicationSession {
            id: id.to_owned(),
            source_id: "test".to_owned(),
            application_id: id.to_owned(),
            state: ApplicationSessionState::Running,
        };

        for (id, kind) in [
            ("romm:7", "game"),
            ("hearthdeck:desktop:browser", "application"),
        ] {
            state
                .activity
                .session_started(entry(id, kind), &session(id))
                .await
                .unwrap();
        }

        let items = super::last_played_items(&state)
            .await
            .map_err(|error| error.message)
            .expect("the recently played rail composes");
        let ids: Vec<&str> = items.iter().map(|item| item.item_id.as_str()).collect();
        assert_eq!(ids, vec!["romm:7"], "the rail holds the game and no app");
    }

    /// The two id shapes a stored rail item can carry are read back as the thing
    /// they name, and an id naming neither resolves to nothing rather than to a
    /// guess.
    #[test]
    fn an_item_id_is_read_back_as_the_thing_it_names() {
        assert_eq!(
            super::catalog_id_from_item("hearthdeck:desktop:org.example.App"),
            Some("desktop:org.example.App")
        );
        assert_eq!(super::catalog_id_from_item("hearthdeck:"), None);
        assert_eq!(super::catalog_id_from_item("romm:4014"), None);

        assert_eq!(super::rom_id_from_item("romm:4014"), Some(4014));
        assert_eq!(super::rom_id_from_item("romm:"), None);
        assert_eq!(super::rom_id_from_item("romm:not-a-number"), None);
        assert_eq!(super::rom_id_from_item("hearthdeck:romm:4014"), None);
    }

    /// The single route's whole decision from the id alone: the two shapes a
    /// client can send, and an id naming neither falling through to the catalog
    /// lookup that will not find it — a refusal, never a wrong launch.
    #[test]
    fn an_item_id_names_one_of_the_two_launches() {
        assert!(matches!(
            super::item_launch("hearthdeck:stremio:tt0111161"),
            super::ItemLaunch::Catalog("stremio:tt0111161")
        ));
        // The bare catalog id the per-kind route addresses stays launchable.
        assert!(matches!(
            super::item_launch("test:app"),
            super::ItemLaunch::Catalog("test:app")
        ));
        assert!(matches!(
            super::item_launch("romm:4014"),
            super::ItemLaunch::Retro(4014)
        ));
        // An unparseable rom id is not a rom id, so it falls through to a
        // catalog lookup that will not find it.
        assert!(matches!(
            super::item_launch("romm:not-a-number"),
            super::ItemLaunch::Catalog("romm:not-a-number")
        ));
    }

    /// A rail item stored without an icon is served with the one its id names.
    ///
    /// Every client that curates a collection sends no icon - it draws a card
    /// from data it already holds - so a rail drawn from the collection alone had
    /// nothing to draw with, and every favourite showed the same generic mark
    /// whatever it was a favourite of.
    #[tokio::test]
    async fn a_stored_rail_item_is_served_with_the_icon_its_id_names() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::connect(&temporary.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let state: SharedState = Arc::new(AppState::new(
            Config {
                bind_address: "127.0.0.1:38400".parse::<SocketAddr>().unwrap(),
                local_admin_address: "127.0.0.1:38401".parse::<SocketAddr>().unwrap(),
                database_path: temporary.path().join("hearthdeck.db"),
                bridge_socket_path: temporary.path().join("bridge.socket"),
                lan_enabled: false,
                tls: None,
                romm: crate::config::RommPaths::resolve(None, None, None),
            },
            database,
        ));

        state
            .catalog
            .replace_source(
                "desktop-apps",
                vec![CatalogRecord {
                    id: "desktop:org.example.App".to_owned(),
                    title: "Example".to_owned(),
                    kind: "application".to_owned(),
                    launch_id: Some("org.example.App.desktop".to_owned()),
                    icon: Some("/usr/share/icons/example.png".to_owned()),
                    metadata: serde_json::Value::Null,
                    updated_at: "2026-01-01T00:00:00Z".to_owned(),
                }],
            )
            .await
            .unwrap();
        // Stored the way a curating client writes one: an id and a name, no icon.
        state
            .collections
            .add_item(
                "favorites",
                &CollectionItem {
                    item_id: "hearthdeck:desktop:org.example.App".to_owned(),
                    name: "Example".to_owned(),
                    icon: None,
                    background: None,
                    logo: None,
                    description: None,
                    played: None,
                },
            )
            .await
            .unwrap();

        let (_, pairing) = response_json(
            local_router(state.clone()),
            Request::post("/v1/pairing").body(Body::empty()).unwrap(),
        )
        .await;
        let code = pairing["code"].as_str().unwrap();
        let (_, paired) = response_json(
            router(state.clone()),
            Request::post("/v1/pairing/complete")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"code": code, "client_name": "rail-test"}).to_string(),
                ))
                .unwrap(),
        )
        .await;
        let token = paired["token"].as_str().unwrap().to_owned();

        let (status, collections) = response_json(
            router(state.clone()),
            Request::get("/v1/collections")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let favorites = collections
            .as_array()
            .unwrap()
            .iter()
            .find(|collection| collection["slug"] == "favorites")
            .expect("the favorites rail is composed");
        assert_eq!(
            favorites["items"][0]["icon"], "/usr/share/icons/example.png",
            "the catalog's icon answers for the item's own empty one"
        );
    }
}
