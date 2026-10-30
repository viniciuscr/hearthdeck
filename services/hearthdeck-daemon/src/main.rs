#[cfg(not(target_os = "linux"))]
compile_error!("hearthdeck-daemon targets Linux only");

mod activity;
mod api;
mod auth;
mod bridge;
mod catalog;
mod categorizer;
mod collections;
mod config;
mod database;
mod diagnostics;
mod discovery;
mod enrichment;
mod launch;
mod provider_worker;
mod readiness;
mod retro;
mod romm;
mod romm_link;
mod sessions;
mod settings;
mod state;
mod stremio;
mod traits;

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
use tracing::{debug, info, info_span, warn};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const REQUEST_ID_HEADER: &str = "x-request-id";

#[tokio::main]
async fn main() -> Result<()> {
    // Check for dev seed flag before initializing logging, so seed output is clean.
    if std::env::args().any(|arg| arg == "--seed-dev") {
        return seed_dev_database().await;
    }

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
    // Logged, with a existence flag each, so a deployment whose compose file is
    // not where the packaged unit puts it is obvious from the journal instead
    // of surfacing later as "RomM has no roms on disk".
    info!(
        library_root = %config.romm.library_root.display(),
        library_root_exists = config.romm.library_root.is_dir(),
        resources_root = %config.romm.resources_root.display(),
        resources_root_exists = config.romm.resources_root.is_dir(),
        "RomM host paths resolved"
    );
    let database = database::Database::connect(&config.database_path).await?;
    database.migrate().await?;
    info!("database ready");
    // The categorizer service is built after the state, so the pool is kept here
    // rather than reached for through `AppState`.
    let pool = database.pool().clone();

    let state = AppState::new(config.clone(), database);
    let discovery = discovery::DiscoveryService::start(
        discovery_providers(&config, state.stremio.clone()),
        state.catalog.clone(),
        state.events.clone(),
    );
    // Kick off the first scan before the service moves into the state, so there is
    // no `Option` left to unwrap afterwards — the value is right here.
    discovery.request_all().await;
    let state = AppState::with_discovery(state, discovery);
    let enrichment = enrichment::EnrichmentService::start(
        enrichment_providers(),
        state.catalog.clone(),
        state.events.clone(),
    );
    enrichment.request_all().await;
    let state = AppState::with_enrichment(state, enrichment);
    // Categorization is wired unconditionally: whether a scan ever runs is the
    // user's preference in their own settings, not something a deployment is
    // configured for. Nothing here loads a checkpoint — a scan runs in the
    // categorizer binary, which the service spawns on demand and lets exit when
    // it is done.
    let categorizer_settings = categorizer::CategorizerSettings::load(&config.database_path);
    let categorization = categorizer::CategorizationService::start(
        categorizer_settings.clone(),
        state.catalog.clone(),
        pool,
        state.events.clone(),
    );
    let state = Arc::new(AppState::with_categorization(state, categorization));
    // A play still open in the journal belongs to a daemon that is gone: the
    // machine slept, the service was restarted, the power went. When it ended is
    // not knowable, so it is closed without a duration rather than with a guess.
    match state.activity.sweep_running_sessions().await {
        Ok(0) => {}
        Ok(count) => info!(count, "closed plays left running by a previous run"),
        Err(error) => warn!(%error, "could not close plays left running by a previous run"),
    }
    // Nothing else asks the bridge whether a play has ended once the frontend's
    // overlay is gone, so a game closed from the couch would never be recorded as
    // finished. Dropping the handle detaches the watcher, which is what a daemon
    // that wants it for its whole life should do.
    let session_state = state.clone();
    let _session_watcher = tokio::spawn(async move {
        session_state.sessions().watch().await;
    });
    // Nothing runs at boot, on purpose: the model is fetched when the user turns
    // smart categorization on, and nothing is classified until a button asks.
    // Until then, the ordinary category tabs stand and no checkpoint is fetched.
    // All this does is say which state the machine is in, because "why did nothing happen" is otherwise a journal question — the
    // binary path included, since a missing worker is the one thing that makes a
    // click do nothing.
    match state.settings.get().await {
        Ok(settings) => info!(
            enabled = settings.categorization_enabled,
            binary = %categorizer_settings.binary.display(),
            "categorization is available"
        ),
        Err(error) => warn!(%error, "could not read the categorization preference"),
    }
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

fn notify_ready() -> Result<()> {
    sd_notify::notify(&[
        sd_notify::NotifyState::Status("Hearthdeck API listeners ready"),
        sd_notify::NotifyState::Ready,
    ])?;
    Ok(())
}

/// The source namespace the bridge resolves desktop entries in, and the source id of
/// the provider that publishes them.
///
/// Mirrors `hearthdeck_bridge`'s own constant of the same name. A launch request
/// means "start a desktop entry", and the bridge refuses every other source id, so
/// the provider a record came *from* is not what that request wants: passing it is
/// refused with "unsupported application source", which reaches the user as a failed
/// launch and no explanation.
pub(crate) use hearthdeck_protocol::sources::DESKTOP_APPS as DESKTOP_APPS_SOURCE;

fn discovery_providers(
    config: &Config,
    stremio: stremio::StremioRepository,
) -> Vec<Arc<dyn discovery::DiscoveryProvider>> {
    vec![
        Arc::new(
            discovery::providers::desktop_apps::DesktopAppsProvider::new(
                config.bridge_socket_path.clone(),
            ),
        ),
        Arc::new(discovery::providers::heroic::HeroicInstalledProvider::from_system()),
        // Publishes the account's Continue Watching list. It reads the session from
        // the same store the settings endpoints write, so linking an account in the
        // UI is all it takes for the next refresh to have something to publish. It
        // also needs the bridge, to find the desktop entry a card launches.
        Arc::new(discovery::providers::stremio::StremioProvider::new(
            stremio,
            config.bridge_socket_path.clone(),
        )),
        // Publishes what has just arrived on Stremio, for the dashboard's hero
        // banner. Anonymous — it reads a public addon catalog, not the account —
        // but it needs the bridge for the same reason as above: a banner slide
        // launches the Stremio desktop entry.
        Arc::new(
            discovery::providers::stremio_arrivals::StremioArrivalsProvider::new(
                config.bridge_socket_path.clone(),
            ),
        ),
    ]
}

fn enrichment_providers() -> Vec<Arc<dyn enrichment::MetadataProvider>> {
    vec![Arc::new(
        enrichment::providers::appstream_local::AppStreamLocalProvider::from_system(),
    )]
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
    // Every client polls several endpoints a second, so a completed request is a
    // heartbeat, not an event. `debug` keeps it one `RUST_LOG` override away
    // without writing the same line to the session log forever.
    debug!(status_code, latency_ms, "request completed");
}

struct HeaderName;

impl HeaderName {
    fn request_id() -> axum::http::HeaderName {
        axum::http::HeaderName::from_static(REQUEST_ID_HEADER)
    }
}

/// Seeds the database with development test data so the dashboard looks populated.
/// Runs when `--seed-dev` is passed, then exits.
async fn seed_dev_database() -> Result<()> {
    use crate::{
        activity::ActivityStore,
        catalog::{CatalogRecord, CatalogStore},
        collections::CollectionStore,
        config::Config,
        database::Database,
    };

    let config = Config::load()?;
    let database = Database::connect(&config.database_path).await?;
    database.migrate().await?;

    let collections = CollectionStore::new(database.pool().clone());
    let activity = ActivityStore::new(database.pool().clone());
    let catalog = CatalogStore::new(database.pool().clone());

    // Seed some catalog items so the dashboard has things to show
    catalog.replace_source("dev-seed", vec![
        CatalogRecord {
            id: "dev:game1".into(),
            title: "Elden Ring".into(),
            kind: "game".into(),
            launch_id: Some("steam:1245620".into()),
            icon: Some("https://cdn.cloudflare.steamstatic.com/steam/apps/1245620/header.jpg".into()),
            metadata: serde_json::json!({"categories": ["Action", "RPG"], "store": "Steam"}),
            updated_at: chrono::Utc::now().to_rfc3339(),
        },
        CatalogRecord {
            id: "dev:game2".into(),
            title: "Cyberpunk 2077".into(),
            kind: "game".into(),
            launch_id: Some("gog:cyberpunk2077".into()),
            icon: Some("https://cdn.cloudflare.steamstatic.com/steam/apps/1091500/header.jpg".into()),
            metadata: serde_json::json!({"categories": ["Action", "RPG"], "store": "GOG"}),
            updated_at: chrono::Utc::now().to_rfc3339(),
        },
        CatalogRecord {
            id: "dev:app1".into(),
            title: "Discord".into(),
            kind: "application".into(),
            launch_id: Some("com.discordapp.Discord.desktop".into()),
            icon: Some("com.discordapp.Discord".into()),
            metadata: serde_json::json!({"categories": ["Network", "InstantMessaging"]}),
            updated_at: chrono::Utc::now().to_rfc3339(),
        },
    ]).await?;

    // Seed some launch activity (recent plays)
    use crate::activity::ActivityEntry;
    use hearthdeck_protocol::ApplicationSession;
    use hearthdeck_protocol::ApplicationSessionState;
    let session = |id: &str| ApplicationSession {
        id: id.into(),
        source_id: "steam".into(),
        application_id: id.into(),
        state: ApplicationSessionState::Running,
    };
    for (id, kind, title) in [
        ("dev:game1", "game", "Elden Ring"),
        ("dev:game2", "game", "Cyberpunk 2077"),
        ("dev:app1", "application", "Discord"),
    ] {
        let entry = ActivityEntry {
            id: format!("hearthdeck:{}", id),
            title: title.into(),
            icon: None,
            categories: vec![],
            kind: Some(kind.into()),
            source: "dev-seed".into(),
            metadata: serde_json::Value::Null,
        };
        let _ = activity.session_started(entry, &session(id)).await;
    }

    // Seed a Stremio continue-watching item
    catalog
        .replace_source(
            "stremio",
            vec![CatalogRecord {
                id: "stremio:tt1234567".into(),
                title: "Stranger Things".into(),
                kind: "series".into(),
                launch_id: None,
                icon: Some("https://image.tmdb.org/t/p/w500/stranger_things.jpg".into()),
                metadata: serde_json::json!({"type": "series", "source": "stremio"}),
                updated_at: chrono::Utc::now().to_rfc3339(),
            }],
        )
        .await?;

    // Seed a RomM console (for consoles rail)
    catalog
        .replace_source(
            "romm",
            vec![CatalogRecord {
                id: "romm:platform:7".into(),
                title: "PlayStation".into(),
                kind: "game".into(),
                launch_id: None,
                icon: Some("/assets/platforms/ps.svg".into()),
                metadata: serde_json::json!({"platform_id": 7}),
                updated_at: chrono::Utc::now().to_rfc3339(),
            }],
        )
        .await?;

    // Seed some favorites
    collections
        .add_item(
            "favorites",
            &crate::collections::CollectionItem {
                item_id: "hearthdeck:dev:game1".into(),
                name: "Elden Ring".into(),
                icon: Some(
                    "https://cdn.cloudflare.steamstatic.com/steam/apps/1245620/header.jpg".into(),
                ),
                background: None,
                logo: None,
                description: None,
                played: None,
            },
        )
        .await?;

    // Seed play-later
    collections
        .add_item(
            "play-later",
            &crate::collections::CollectionItem {
                item_id: "hearthdeck:dev:game2".into(),
                name: "Cyberpunk 2077".into(),
                icon: Some(
                    "https://cdn.cloudflare.steamstatic.com/steam/apps/1091500/header.jpg".into(),
                ),
                background: None,
                logo: None,
                description: None,
                played: None,
            },
        )
        .await?;

    println!(
        "Dev database seeded successfully at {}",
        config.database_path.display()
    );
    Ok(())
}
