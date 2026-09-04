mod api;
mod auth;
mod bridge;
mod catalog;
mod config;
mod database;
mod diagnostics;
mod discovery;
mod enrichment;
mod retro;
mod settings;
mod state;

use std::{net::TcpListener as StdTcpListener, sync::Arc};

use anyhow::{Context, Result};
use axum::extract::DefaultBodyLimit;
use axum_server::tls_rustls::RustlsConfig;
use config::Config;
use state::AppState;
use tokio::net::TcpListener;
use tower_http::{
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::{info, info_span};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const REQUEST_ID_HEADER: &str = "x-request-id";

#[tokio::main]
async fn main() -> Result<()> {
    hearthdeck_observability::init(
        "hearthdeck-daemon",
        "hearthdeck_daemon=info,tower_http=info",
    );

    let config = Config::load()?;
    info!(
        bind_address = %config.bind_address,
        local_admin_address = %config.local_admin_address,
        lan_enabled = config.lan_enabled,
        "daemon configuration loaded"
    );
    let database = database::Database::connect(&config.database_path).await?;
    database.migrate().await?;
    info!("database ready");

    let state = Arc::new(AppState::new(config.clone(), database));
    let discovery = discovery::DiscoveryService::start(
        discovery_providers(&config),
        state.catalog.clone(),
        state.events.clone(),
    );
    let state = Arc::new(AppState::with_discovery((*state).clone(), discovery));
    state
        .discovery
        .as_ref()
        .expect("discovery service must be registered")
        .request_all()
        .await;
    let enrichment = enrichment::EnrichmentService::start(
        enrichment_providers(),
        state.catalog.clone(),
        state.events.clone(),
    );
    let state = Arc::new(AppState::with_enrichment((*state).clone(), enrichment));
    state
        .enrichment
        .as_ref()
        .expect("enrichment service must be registered")
        .request_all()
        .await;
    let router = api::router(state.clone())
        .layer(DefaultBodyLimit::max(32 * 1024))
        .layer(RequestBodyLimitLayer::new(32 * 1024))
        .layer(PropagateRequestIdLayer::new(HeaderName::request_id()))
        .layer(SetRequestIdLayer::new(
            HeaderName::request_id(),
            MakeRequestUuid,
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(request_span)
                .on_response(request_response),
        );

    let local_listener = TcpListener::bind(config.local_admin_address)
        .await
        .with_context(|| {
            format!(
                "failed to bind local admin listener {}",
                config.local_admin_address
            )
        })?;
    let local_router = api::local_router(state)
        .layer(DefaultBodyLimit::max(4 * 1024))
        .layer(RequestBodyLimitLayer::new(4 * 1024))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(request_span)
                .on_response(request_response),
        );
    let local_server = axum::serve(local_listener, local_router);

    if let Some(tls) = config.tls {
        let tls = RustlsConfig::from_pem_file(tls.certificate_path, tls.private_key_path).await?;
        let listener = StdTcpListener::bind(config.bind_address)
            .with_context(|| format!("failed to bind API listener {}", config.bind_address))?;
        listener.set_nonblocking(true)?;
        info!(address = %config.bind_address, version = VERSION, transport = "https", "daemon listening");
        notify_ready()?;
        tokio::try_join!(
            local_server,
            axum_server::from_tcp_rustls(listener, tls)?.serve(router.into_make_service()),
        )?;
    } else {
        let listener = TcpListener::bind(config.bind_address)
            .await
            .with_context(|| format!("failed to bind API listener {}", config.bind_address))?;
        info!(address = %config.bind_address, version = VERSION, transport = "http", "daemon listening");
        notify_ready()?;
        tokio::try_join!(local_server, axum::serve(listener, router))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn notify_ready() -> Result<()> {
    sd_notify::notify(&[
        sd_notify::NotifyState::Status("Hearthdeck API listeners ready"),
        sd_notify::NotifyState::Ready,
    ])?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn notify_ready() -> Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn discovery_providers(config: &Config) -> Vec<Arc<dyn discovery::DiscoveryProvider>> {
    vec![
        Arc::new(
            discovery::providers::desktop_apps::DesktopAppsProvider::new(
                config.bridge_socket_path.clone(),
            ),
        ),
        Arc::new(discovery::providers::heroic::HeroicInstalledProvider::from_system()),
    ]
}

#[cfg(target_os = "macos")]
fn discovery_providers(config: &Config) -> Vec<Arc<dyn discovery::DiscoveryProvider>> {
    vec![Arc::new(
        discovery::providers::macos_apps::MacosAppsProvider::new(config.bridge_socket_path.clone()),
    )]
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn discovery_providers(_config: &Config) -> Vec<Arc<dyn discovery::DiscoveryProvider>> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn enrichment_providers() -> Vec<Arc<dyn enrichment::MetadataProvider>> {
    vec![Arc::new(
        enrichment::providers::appstream_local::AppStreamLocalProvider::from_system(),
    )]
}

#[cfg(not(target_os = "linux"))]
fn enrichment_providers() -> Vec<Arc<dyn enrichment::MetadataProvider>> {
    Vec::new()
}

fn request_span(request: &axum::http::Request<axum::body::Body>) -> tracing::Span {
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("missing");
    info_span!(
        "http.request",
        method = %request.method(),
        path = %request.uri().path(),
        request_id,
        status_code = tracing::field::Empty,
        latency_ms = tracing::field::Empty,
    )
}

fn request_response(
    response: &axum::http::Response<axum::body::Body>,
    latency: std::time::Duration,
    span: &tracing::Span,
) {
    let status_code = response.status().as_u16();
    let latency_ms = latency.as_millis() as u64;
    span.record("status_code", status_code);
    span.record("latency_ms", latency_ms);
    info!(status_code, latency_ms, "request completed");
}

struct HeaderName;

impl HeaderName {
    fn request_id() -> axum::http::HeaderName {
        axum::http::HeaderName::from_static(REQUEST_ID_HEADER)
    }
}
