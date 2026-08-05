mod platform;

use std::{
    cmp::Ordering,
    collections::HashMap,
    env,
    os::unix::{fs::PermissionsExt, io::FromRawFd, net::UnixListener as StdUnixListener},
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
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

/// Stable session identity for Heroic launches - see
/// `platform::linux::HEROIC_UNIT_NAME`'s own docs for why Heroic is tracked
/// as one reused resource rather than a fresh session per game launch.
const HEROIC_SESSION_ID: &str = "heroic";

#[tokio::main]
async fn main() -> Result<()> {
    hearthdeck_observability::init("hearthdeck-bridge", "hearthdeck_bridge=info");
    let socket_path = bridge_socket_path()?;
    let (listener, socket_activated) = bridge_listener(&socket_path).await?;
    let session_directory = bridge_session_directory(&socket_path);
    let sessions = Arc::new(Mutex::new(load_managed_sessions(&session_directory).await?));
    info!(socket_activated, "bridge listening");
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

/// Hearthdeck is a single-focus console launcher, not a multitasking
/// desktop: exactly one desktop app, Heroic game, or RetroArch game should be
/// running at a time. Enforcing that here - stopping whatever is currently
/// tracked before starting something new - is what makes "the active
/// session" unambiguous everywhere else that asks (the overlay's close
/// button, the daemon's `GET .../sessions/active`): there is structurally at
/// most one tracked session, never a choice between several launched at
/// different times. This replaces trying to detect "the one in focus" via
/// compositor/window APIs, which - see the overlay's own history - proved
/// fragile and unreliable in practice even when implemented correctly
/// against the real protocol. The one deliberate exception: launching a
/// second Heroic game while Heroic is already the active session does NOT
/// stop it first - Heroic is intentionally kept running between games (see
/// `HEROIC_UNIT_NAME`'s own docs) rather than treated as something to close
/// and relaunch on every game switch.
async fn stop_other_active_sessions(
    new_source_id: &str,
    sessions: &Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_directory: &Path,
) {
    let Some((session_id, managed)) = active_managed_session(sessions, session_directory).await
    else {
        return;
    };
    if !should_replace_active_session(new_source_id, &managed.session.source_id) {
        return;
    }
    if let Err(error) = platform::stop_application(managed.unit_name.as_deref()).await {
        warn!(session_id, %error, "could not stop previous application session before launching a new one");
        return;
    }
    sessions.lock().await.remove(&session_id);
    if let Err(error) = remove_managed_session(session_directory, &session_id).await {
        warn!(session_id, %error, "could not remove stopped application session record");
    }
}

/// Whether launching `new_source_id` should first stop the currently active
/// session (from `active_source_id`). The only case it should not: a second
/// Heroic game while Heroic is already the active session - see
/// `stop_other_active_sessions`'s own docs for why.
fn should_replace_active_session(new_source_id: &str, active_source_id: &str) -> bool {
    !(new_source_id == "heroic" && active_source_id == "heroic")
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
        } => {
            stop_other_active_sessions(&source_id, sessions, session_directory).await;
            match platform::launch_application(&source_id, &application_id, &session_id).await {
                Ok(launched) => {
                    register_launch(
                        sessions,
                        session_directory,
                        source_id,
                        application_id,
                        session_id,
                        launched,
                    )
                    .await
                }
                Err(error) => {
                    warn!(source_id, application_id, %error, "registered application launch rejected");
                    BridgeResponse::Error {
                        code: BridgeErrorCode::LaunchFailed,
                        message: error.to_string(),
                    }
                }
            }
        }
        BridgeRequest::LaunchHeroicGame {
            runner,
            application_id,
            session_id: _,
        } => {
            stop_other_active_sessions("heroic", sessions, session_directory).await;
            match platform::launch_heroic_game(runner, &application_id).await {
                Ok(launched) => {
                    // Heroic is a shared, reused resource (see HEROIC_UNIT_NAME's
                    // own docs), not a fresh session per launch, so this
                    // deliberately ignores the daemon-generated session_id above
                    // and always registers/overwrites the same stable session
                    // record - the second game launched through an already-running
                    // Heroic replaces the first's record rather than leaking a
                    // second, orphaned one that nothing will ever stop on its own.
                    register_launch(
                        sessions,
                        session_directory,
                        "heroic".to_owned(),
                        application_id,
                        HEROIC_SESSION_ID.to_owned(),
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
            }
        }
        BridgeRequest::LaunchRetroGame {
            core_path,
            rom_path,
            session_id,
        } => {
            stop_other_active_sessions("retroarch", sessions, session_directory).await;
            match platform::launch_retro_game(&core_path, &rom_path, &session_id).await {
                Ok(launched) => {
                    register_launch(
                        sessions,
                        session_directory,
                        "retroarch".to_owned(),
                        rom_path,
                        session_id,
                        launched,
                    )
                    .await
                }
                Err(error) => {
                    warn!(%error, "RetroArch game launch rejected");
                    BridgeResponse::Error {
                        code: BridgeErrorCode::LaunchFailed,
                        message: error.to_string(),
                    }
                }
            }
        }
        BridgeRequest::ActiveApplicationSession => {
            let session = active_managed_session(sessions, session_directory)
                .await
                .map(|(_, managed)| managed.session);
            BridgeResponse::ApplicationSession { session }
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

async fn active_managed_session(
    sessions: &Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_directory: &Path,
) -> Option<(String, ManagedSession)> {
    let candidates = {
        sessions
            .lock()
            .await
            .iter()
            .map(|(session_id, managed)| (session_id.clone(), managed.clone()))
            .collect::<Vec<_>>()
    };
    let mut running = Vec::new();

    for (session_id, managed) in candidates {
        if !platform::application_is_running(managed.unit_name.as_deref())
            .await
            .unwrap_or(false)
        {
            sessions.lock().await.remove(&session_id);
            if let Err(error) = remove_managed_session(session_directory, &session_id).await {
                warn!(session_id, %error, "could not remove inactive application session record");
            }
            continue;
        }
        let modified_at = managed_session_modified_at(session_directory, &session_id).await;
        running.push((session_id, managed, modified_at));
    }

    select_active_session(running)
}

fn select_active_session(
    candidates: Vec<(String, ManagedSession, SystemTime)>,
) -> Option<(String, ManagedSession)> {
    candidates
        .into_iter()
        .max_by(
            |left, right| match left.2.partial_cmp(&right.2).unwrap_or(Ordering::Equal) {
                Ordering::Equal => left.0.cmp(&right.0),
                other => other,
            },
        )
        .map(|(session_id, managed, _)| (session_id, managed))
}

async fn managed_session_modified_at(session_directory: &Path, session_id: &str) -> SystemTime {
    let Ok(path) = managed_session_path(session_directory, session_id) else {
        warn!(
            session_id,
            "could not resolve managed application session record path"
        );
        return SystemTime::UNIX_EPOCH;
    };
    match tokio::fs::metadata(&path)
        .await
        .and_then(|metadata| metadata.modified())
    {
        Ok(modified_at) => modified_at,
        Err(error) => {
            warn!(session_id, %error, "could not read managed application session record timestamp");
            SystemTime::UNIX_EPOCH
        }
    }
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
        select_active_session, should_replace_active_session,
    };
    use hearthdeck_protocol::{ApplicationSession, ApplicationSessionState};
    use std::time::{Duration, SystemTime};

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

    #[test]
    fn selects_the_newest_running_session_as_active() {
        let older = managed_session();
        let newer = ManagedSession {
            session: ApplicationSession {
                id: "session-2".to_owned(),
                source_id: "retroarch".to_owned(),
                application_id: "rom-1".to_owned(),
                state: ApplicationSessionState::Running,
            },
            unit_name: Some("hearthdeck-app-session-2.service".to_owned()),
        };

        let selected = select_active_session(vec![
            (
                older.session.id.clone(),
                older,
                SystemTime::UNIX_EPOCH + Duration::from_secs(1),
            ),
            (
                newer.session.id.clone(),
                newer.clone(),
                SystemTime::UNIX_EPOCH + Duration::from_secs(2),
            ),
        ]);

        assert!(
            matches!(selected, Some((session_id, session)) if session_id == "session-2" && session.session.id == "session-2")
        );
    }

    #[test]
    fn replaces_the_active_session_for_a_different_source() {
        assert!(should_replace_active_session("desktop-apps", "retroarch"));
        assert!(should_replace_active_session("retroarch", "heroic"));
        assert!(should_replace_active_session("heroic", "desktop-apps"));
    }

    #[test]
    fn keeps_heroic_running_for_a_second_heroic_game() {
        assert!(!should_replace_active_session("heroic", "heroic"));
    }
}
