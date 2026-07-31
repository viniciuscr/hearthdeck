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
    let session_directory = bridge_session_directory(&socket_path);
    let sessions = Arc::new(Mutex::new(load_managed_sessions(&session_directory).await?));
    info!(socket = %socket_path.display(), socket_activated, "bridge listening");
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let sessions = sessions.clone();
                let session_directory = session_directory.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, &sessions, &session_directory)
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
    session_directory: &Path,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let response = match serde_json::from_str::<BridgeRequest>(&line) {
        Ok(request) => handle_request(request, sessions, session_directory).await,
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

#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct ManagedSession {
    session: hearthdeck_protocol::ApplicationSession,
    unit_name: Option<String>,
}

async fn handle_request(
    request: BridgeRequest,
    sessions: &Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_directory: &Path,
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
                let managed = ManagedSession {
                    session: session.clone(),
                    unit_name: launched.unit_name,
                };
                if let Err(error) = save_managed_session(session_directory, &managed).await {
                    let _ = platform::stop_application(managed.unit_name.as_deref()).await;
                    warn!(source_id, application_id, %error, "could not persist managed application session");
                    return BridgeResponse::Error {
                        code: BridgeErrorCode::LaunchFailed,
                        message: "could not persist managed application session".to_owned(),
                    };
                }
                sessions.lock().await.insert(session_id, managed);
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
        BridgeRequest::LaunchHeroicGame {
            runner,
            application_id,
            session_id,
        } => match platform::launch_heroic_game(runner, &application_id, &session_id).await {
            Ok(launched) => {
                register_launch(
                    sessions,
                    session_directory,
                    "heroic".to_owned(),
                    application_id,
                    session_id,
                    launched,
                )
                .await
            }
            Err(error) => {
                warn!(%error, "Heroic game launch rejected");
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
                    if let Err(error) = remove_managed_session(session_directory, &session_id).await
                    {
                        warn!(session_id, %error, "could not remove inactive application session record");
                    }
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
            let managed = sessions.lock().await.get(&session_id).cloned();
            let Some(managed) = managed else {
                return BridgeResponse::Error {
                    code: BridgeErrorCode::NotFound,
                    message: "application session is not running".to_owned(),
                };
            };
            match platform::stop_application(managed.unit_name.as_deref()).await {
                Ok(()) => {
                    sessions.lock().await.remove(&session_id);
                    if let Err(error) = remove_managed_session(session_directory, &session_id).await
                    {
                        warn!(session_id, %error, "could not remove stopped application session record");
                    }
                    BridgeResponse::StopAccepted { session_id }
                }
                Err(error) => BridgeResponse::Error {
                    code: BridgeErrorCode::LaunchFailed,
                    message: error.to_string(),
                },
            }
        }
    }
}

async fn register_launch(
    sessions: &Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_directory: &Path,
    source_id: String,
    application_id: String,
    session_id: String,
    launched: platform::LaunchedApplication,
) -> BridgeResponse {
    let session = hearthdeck_protocol::ApplicationSession {
        id: session_id.clone(),
        source_id: source_id.clone(),
        application_id: application_id.clone(),
        state: hearthdeck_protocol::ApplicationSessionState::Running,
    };
    let managed = ManagedSession {
        session: session.clone(),
        unit_name: launched.unit_name,
    };
    if let Err(error) = save_managed_session(session_directory, &managed).await {
        let _ = platform::stop_application(managed.unit_name.as_deref()).await;
        warn!(source_id, application_id, %error, "could not persist managed application session");
        return BridgeResponse::Error {
            code: BridgeErrorCode::LaunchFailed,
            message: "could not persist managed application session".to_owned(),
        };
    }
    sessions.lock().await.insert(session_id, managed);
    info!(
        source_id,
        application_id, "registered application launch accepted"
    );
    BridgeResponse::LaunchAccepted { session }
}

fn bridge_session_directory(socket_path: &Path) -> PathBuf {
    socket_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("sessions")
}

async fn load_managed_sessions(
    session_directory: &Path,
) -> Result<HashMap<String, ManagedSession>> {
    tokio::fs::create_dir_all(session_directory).await?;
    tokio::fs::set_permissions(session_directory, std::fs::Permissions::from_mode(0o700)).await?;
    let mut sessions = HashMap::new();
    let mut entries = tokio::fs::read_dir(session_directory).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        match tokio::fs::read(&path)
            .await
            .ok()
            .and_then(|contents| serde_json::from_slice::<ManagedSession>(&contents).ok())
        {
            Some(session) if valid_session_id(&session.session.id) => {
                sessions.insert(session.session.id.clone(), session);
            }
            _ => {
                warn!(session_record = %path.display(), "ignoring invalid managed application session record")
            }
        }
    }
    Ok(sessions)
}

async fn save_managed_session(session_directory: &Path, session: &ManagedSession) -> Result<()> {
    let path = managed_session_path(session_directory, &session.session.id)?;
    let contents = serde_json::to_vec(session)?;
    tokio::fs::write(&path, contents).await?;
    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await?;
    Ok(())
}

async fn remove_managed_session(session_directory: &Path, session_id: &str) -> Result<()> {
    let path = managed_session_path(session_directory, session_id)?;
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn managed_session_path(session_directory: &Path, session_id: &str) -> Result<PathBuf> {
    if !valid_session_id(session_id) {
        anyhow::bail!("application session has an invalid identifier")
    }
    Ok(session_directory.join(format!("{session_id}.json")))
}

fn valid_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id.len() <= 128
        && session_id
            .bytes()
            .all(|character| character.is_ascii_alphanumeric() || character == b'-')
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

#[cfg(test)]
mod tests {
    use super::{
        ManagedSession, load_managed_sessions, managed_session_path, save_managed_session,
    };
    use hearthdeck_protocol::{ApplicationSession, ApplicationSessionState};

    fn managed_session() -> ManagedSession {
        ManagedSession {
            session: ApplicationSession {
                id: "session-1".to_owned(),
                source_id: "desktop-apps".to_owned(),
                application_id: "org.example.App.desktop".to_owned(),
                state: ApplicationSessionState::Running,
            },
            unit_name: Some("hearthdeck-app-session-1.service".to_owned()),
        }
    }

    #[tokio::test]
    async fn restores_persisted_managed_sessions() {
        let temporary = tempfile::tempdir().unwrap();
        let session = managed_session();

        save_managed_session(temporary.path(), &session)
            .await
            .unwrap();
        let loaded = load_managed_sessions(temporary.path()).await.unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["session-1"].unit_name, session.unit_name);
    }

    #[test]
    fn rejects_unsafe_session_identifiers() {
        assert!(managed_session_path(std::path::Path::new("/tmp"), "../../scope").is_err());
    }
}
