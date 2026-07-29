mod platform;

use std::{
    collections::HashMap,
    env,
    os::unix::{fs::PermissionsExt, io::FromRawFd, net::UnixListener as StdUnixListener},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use hearthdeck_protocol::{BridgeErrorCode, BridgeRequest, BridgeResponse};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::Mutex,
};
use tracing::{Instrument, error, info, info_span, warn};

#[tokio::main]
async fn main() -> Result<()> {
    hearthdeck_observability::init("hearthdeck-bridge", "hearthdeck_bridge=info");
    let socket_path = bridge_socket_path()?;
    let (listener, socket_activated) = bridge_listener(&socket_path).await?;
    let sessions = Arc::new(Mutex::new(HashMap::<String, ManagedSession>::new()));
    info!(socket = %socket_path.display(), socket_activated, "bridge listening");
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let sessions = sessions.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, &sessions)
                        .instrument(info_span!("bridge.request"))
                        .await
                    {
                        error!(%error, "bridge request failed");
                    }
                });
            }
            Err(error) => error!(%error, "bridge accept failed"),
        }
    }
}

async fn bridge_listener(socket_path: &Path) -> Result<(UnixListener, bool)> {
    if let Some(listener) = inherited_listener()? {
        return Ok((listener, true));
    }

    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if socket_path.exists() {
        tokio::fs::remove_file(socket_path).await?;
    }
    let listener = UnixListener::bind(socket_path)?;
    tokio::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600)).await?;
    Ok((listener, false))
}

fn inherited_listener() -> Result<Option<UnixListener>> {
    let mut listen_fds = sd_notify::listen_fds()?;
    let Some(file_descriptor) = listen_fds.next() else {
        return Ok(None);
    };
    if listen_fds.next().is_some() {
        anyhow::bail!("hearthdeck-bridge requires exactly one activated socket")
    }

    let listener = unsafe { StdUnixListener::from_raw_fd(file_descriptor) };
    listener.set_nonblocking(true)?;
    Ok(Some(UnixListener::from_std(listener)?))
}

async fn handle_connection(
    stream: UnixStream,
    sessions: &Arc<Mutex<HashMap<String, ManagedSession>>>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let response = match serde_json::from_str::<BridgeRequest>(&line) {
        Ok(request) => handle_request(request, sessions).await,
        Err(error) => BridgeResponse::Error {
            code: BridgeErrorCode::InvalidRequest,
            message: error.to_string(),
        },
    };
    writer
        .write_all(serde_json::to_string(&response)?.as_bytes())
        .await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

struct ManagedSession {
    session: hearthdeck_protocol::ApplicationSession,
    unit_name: Option<String>,
}

async fn handle_request(
    request: BridgeRequest,
    sessions: &Arc<Mutex<HashMap<String, ManagedSession>>>,
) -> BridgeResponse {
    match request {
        BridgeRequest::Health => BridgeResponse::Health {
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        BridgeRequest::DiscoverApplications { source_id } => {
            match platform::discover_applications(&source_id).await {
                Ok(applications) => {
                    info!(
                        source_id,
                        application_count = applications.len(),
                        "application discovery completed"
                    );
                    BridgeResponse::Applications {
                        source_id,
                        applications,
                    }
                }
                Err(error) => BridgeResponse::Error {
                    code: BridgeErrorCode::Internal,
                    message: error.to_string(),
                },
            }
        }
        BridgeRequest::LaunchApplication {
            source_id,
            application_id,
            session_id,
        } => match platform::launch_application(&source_id, &application_id, &session_id).await {
            Ok(launched) => {
                let session = hearthdeck_protocol::ApplicationSession {
                    id: session_id.clone(),
                    source_id: source_id.clone(),
                    application_id: application_id.clone(),
                    state: hearthdeck_protocol::ApplicationSessionState::Running,
                };
                sessions.lock().await.insert(
                    session_id,
                    ManagedSession {
                        session: session.clone(),
                        unit_name: launched.unit_name,
                    },
                );
                info!(
                    source_id,
                    application_id, "registered application launch accepted"
                );
                BridgeResponse::LaunchAccepted { session }
            }
            Err(error) => {
                warn!(source_id, application_id, %error, "registered application launch rejected");
                BridgeResponse::Error {
                    code: BridgeErrorCode::LaunchFailed,
                    message: error.to_string(),
                }
            }
        },
        BridgeRequest::ActiveApplicationSession => {
            let candidates = {
                sessions
                    .lock()
                    .await
                    .values()
                    .map(|managed| (managed.session.id.clone(), managed.unit_name.clone()))
                    .collect::<Vec<_>>()
            };
            for (session_id, unit_name) in candidates {
                if !platform::application_is_running(unit_name.as_deref())
                    .await
                    .unwrap_or(false)
                {
                    sessions.lock().await.remove(&session_id);
                }
            }
            BridgeResponse::ApplicationSession {
                session: sessions
                    .lock()
                    .await
                    .values()
                    .next()
                    .map(|managed| managed.session.clone()),
            }
        }
        BridgeRequest::StopApplicationSession { session_id } => {
            let managed = sessions.lock().await.remove(&session_id);
            let Some(managed) = managed else {
                return BridgeResponse::Error {
                    code: BridgeErrorCode::NotFound,
                    message: "application session is not running".to_owned(),
                };
            };
            match platform::stop_application(managed.unit_name.as_deref()).await {
                Ok(()) => BridgeResponse::StopAccepted { session_id },
                Err(error) => BridgeResponse::Error {
                    code: BridgeErrorCode::LaunchFailed,
                    message: error.to_string(),
                },
            }
        }
    }
}

fn bridge_socket_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("HEARTHDECK_BRIDGE_SOCKET") {
        return Ok(PathBuf::from(path));
    }
    let project_dirs = ProjectDirs::from("dev", "hearthdeck", "hearthdeck")
        .context("could not determine Hearthdeck data directories")?;
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| project_dirs.data_local_dir().join("runtime"));
    Ok(runtime_dir.join("hearthdeck/bridge.sock"))
}
