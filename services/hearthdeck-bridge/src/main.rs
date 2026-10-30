mod core_install;
mod desktop_apps;
mod heroic;
mod input;
mod retroarch;
mod stremio;
mod systemd;

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
use hearthdeck_protocol::{BridgeErrorCode, BridgeRequest, BridgeResponse, InputProfile, sources};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::Mutex,
};
use tracing::{Instrument, error, info, info_span, warn};

/// Stable session identity for Heroic launches - see
/// `heroic::HEROIC_UNIT_NAME`'s own docs for why Heroic is tracked
/// as one reused resource rather than a fresh session per game launch.
const HEROIC_SESSION_ID: &str = "heroic";

#[tokio::main]
async fn main() -> Result<()> {
    hearthdeck_observability::init("hearthdeck-bridge", "hearthdeck_bridge=info");
    let socket_path = hearthdeck_protocol::paths::bridge_socket_path();
    let (listener, socket_activated) = bridge_listener(&socket_path).await?;
    let session_directory = bridge_session_directory(&socket_path);
    let sessions = Arc::new(Mutex::new(load_managed_sessions(&session_directory).await?));
    let session_transition = Arc::new(Mutex::new(()));
    info!(socket_activated, "bridge listening");
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let sessions = sessions.clone();
                let session_transition = session_transition.clone();
                let session_directory = session_directory.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(
                        stream,
                        &sessions,
                        &session_transition,
                        &session_directory,
                    )
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
    session_transition: &Mutex<()>,
    session_directory: &Path,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let response = match serde_json::from_str::<BridgeRequest>(&line) {
        Ok(request) => {
            handle_request(request, sessions, session_transition, session_directory).await
        }
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
    #[serde(default)]
    input_profile: InputProfile,
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
) -> Result<()> {
    let Some((session_id, managed)) = active_managed_session(sessions, session_directory).await
    else {
        return Ok(());
    };
    if !should_replace_active_session(new_source_id, &managed.session.source_id) {
        return Ok(());
    }
    systemd::stop_application(managed.unit_name.as_deref())
        .await
        .context("could not stop previous application session before launching a new one")?;
    input::set_input_profile(InputProfile::Native, false).await?;
    sessions.lock().await.remove(&session_id);
    if let Err(error) = remove_managed_session(session_directory, &session_id).await {
        warn!(session_id, %error, "could not remove stopped application session record");
    }
    Ok(())
}

/// Whether launching `new_source_id` should first stop the currently active
/// session (from `active_source_id`). The only case it should not: a second
/// Heroic game while Heroic is already the active session - see
/// `stop_other_active_sessions`'s own docs for why.
fn should_replace_active_session(new_source_id: &str, active_source_id: &str) -> bool {
    !(new_source_id == sources::HEROIC && active_source_id == sources::HEROIC)
}

async fn handle_request(
    request: BridgeRequest,
    sessions: &Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_transition: &Mutex<()>,
    session_directory: &Path,
) -> BridgeResponse {
    match request {
        BridgeRequest::Health => BridgeResponse::Health {
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        BridgeRequest::DiscoverApplications { source_id } => {
            match desktop_apps::discover_applications(&source_id).await {
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
            input_profile,
        } => {
            let session = running_session(source_id, application_id, session_id);
            let launch = desktop_apps::launch_application(
                &session.source_id,
                &session.application_id,
                &session.id,
            );
            launch_session(
                sessions,
                session_transition,
                session_directory,
                &session,
                input_profile,
                launch,
            )
            .await
        }
        // A Stremio title is a desktop application launch underneath, so it occupies
        // the same single-session slot and is registered under the same source.
        BridgeRequest::LaunchStremioTitle {
            application_id,
            video_id,
            media_type,
            session_id,
            input_profile,
        } => {
            let session = running_session(
                desktop_apps::DESKTOP_APPS_SOURCE.to_owned(),
                application_id,
                session_id,
            );
            let launch = stremio::launch_stremio_title(
                &session.application_id,
                &video_id,
                media_type,
                &session.id,
            );
            launch_session(
                sessions,
                session_transition,
                session_directory,
                &session,
                input_profile,
                launch,
            )
            .await
        }
        // Heroic is a shared, reused resource (see HEROIC_UNIT_NAME's own docs),
        // not a fresh session per launch, so this deliberately ignores the
        // daemon-generated session_id and always registers/overwrites the same
        // stable session record - the second game launched through an
        // already-running Heroic replaces the first's record rather than leaking a
        // second, orphaned one that nothing will ever stop on its own.
        BridgeRequest::LaunchHeroicGame {
            runner,
            application_id,
            session_id: _,
            input_profile,
        } => {
            let session = running_session(
                sources::HEROIC.to_owned(),
                application_id,
                HEROIC_SESSION_ID.to_owned(),
            );
            let launch = heroic::launch_heroic_game(runner, &session.application_id);
            launch_session(
                sessions,
                session_transition,
                session_directory,
                &session,
                input_profile,
                launch,
            )
            .await
        }
        BridgeRequest::LaunchRetroGame {
            core_path,
            rom_path,
            session_id,
            input_profile,
        } => {
            let session = running_session(sources::RETROARCH.to_owned(), rom_path, session_id);
            let launch =
                retroarch::launch_retro_game(&core_path, &session.application_id, &session.id);
            launch_session(
                sessions,
                session_transition,
                session_directory,
                &session,
                input_profile,
                launch,
            )
            .await
        }
        BridgeRequest::InstallRetroCores { packages } => {
            let packages = match core_install::validate_install_packages(&packages) {
                Ok(packages) => packages,
                Err(error) => {
                    warn!(%error, "core install rejected");
                    return BridgeResponse::Error {
                        code: BridgeErrorCode::InvalidRequest,
                        message: error.to_string(),
                    };
                }
            };
            match core_install::install_cores(&packages).await {
                Ok(report) => BridgeResponse::RetroCoresInstalled { report },
                Err(error) => {
                    warn!(%error, "core install failed");
                    BridgeResponse::Error {
                        code: BridgeErrorCode::Internal,
                        message: error.to_string(),
                    }
                }
            }
        }
        BridgeRequest::RestartRommService => match systemd::restart_romm_service().await {
            Ok(()) => BridgeResponse::RommServiceRestarted,
            Err(error) => {
                warn!(%error, "romm.service restart failed");
                BridgeResponse::Error {
                    code: BridgeErrorCode::Internal,
                    message: error.to_string(),
                }
            }
        },
        BridgeRequest::ActiveApplicationSession => {
            let _transition = session_transition.lock().await;
            let active = active_managed_session(sessions, session_directory).await;
            let profile = active.as_ref().map(|(_, managed)| managed);
            if let Err(error) = sync_managed_input(profile).await {
                warn!(%error, "could not synchronize active input profile");
            }
            let session = active.map(|(_, managed)| managed.session);
            BridgeResponse::ApplicationSession { session }
        }
        BridgeRequest::StopApplicationSession { session_id } => {
            let transition = session_transition.lock().await;
            let managed = sessions.lock().await.get(&session_id).cloned();
            let Some(managed) = managed else {
                warn!(
                    session_id,
                    "stop requested for a session the bridge is not tracking"
                );
                return BridgeResponse::Error {
                    code: BridgeErrorCode::NotFound,
                    message: "application session is not running".to_owned(),
                };
            };
            stop_managed_session(sessions, session_directory, managed, &transition).await
        }
        BridgeRequest::StopActiveApplicationSession => {
            let transition = session_transition.lock().await;
            let Some((_, managed)) = active_managed_session(sessions, session_directory).await
            else {
                return BridgeResponse::Error {
                    code: BridgeErrorCode::NotFound,
                    message: "no application session is active".to_owned(),
                };
            };
            stop_managed_session(sessions, session_directory, managed, &transition).await
        }
    }
}

async fn stop_managed_session(
    sessions: &Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_directory: &Path,
    managed: ManagedSession,
    _transition: &tokio::sync::MutexGuard<'_, ()>,
) -> BridgeResponse {
    let session_id = managed.session.id;
    info!(
        session_id,
        unit_name = managed.unit_name.as_deref().unwrap_or("<none>"),
        "stopping application session"
    );
    match systemd::stop_application(managed.unit_name.as_deref()).await {
        Ok(()) => {
            sessions.lock().await.remove(&session_id);
            if let Err(error) = remove_managed_session(session_directory, &session_id).await {
                warn!(session_id, %error, "could not remove stopped application session record");
            }
            if let Err(error) = input::set_input_profile(InputProfile::Native, false).await {
                warn!(%error, "could not reset input profile after application stop");
            }
            info!(session_id, "application session stopped");
            BridgeResponse::StopAccepted { session_id }
        }
        Err(error) => BridgeResponse::Error {
            code: BridgeErrorCode::LaunchFailed,
            message: error.to_string(),
        },
    }
}

fn running_session(
    source_id: String,
    application_id: String,
    id: String,
) -> hearthdeck_protocol::ApplicationSession {
    hearthdeck_protocol::ApplicationSession {
        id,
        source_id,
        application_id,
        state: hearthdeck_protocol::ApplicationSessionState::Running,
    }
}

/// Every launch, whatever it starts: the session id is checked before it can
/// name a unit or a file, the one running session is replaced, the input
/// profile is set before the app starts, and the record is persisted or the
/// launch undone. `launch` is lazy, so a rejected request starts nothing.
async fn launch_session(
    sessions: &Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_transition: &Mutex<()>,
    session_directory: &Path,
    session: &hearthdeck_protocol::ApplicationSession,
    input_profile: InputProfile,
    launch: impl Future<Output = Result<systemd::LaunchedApplication>>,
) -> BridgeResponse {
    let source_id = session.source_id.as_str();
    let application_id = session.application_id.as_str();
    if !valid_session_id(&session.id) {
        warn!(
            source_id,
            application_id, "launch rejected: invalid session id"
        );
        return BridgeResponse::Error {
            code: BridgeErrorCode::InvalidRequest,
            message: "application session has an invalid identifier".to_owned(),
        };
    }
    let _transition = session_transition.lock().await;
    let prepared = async {
        stop_other_active_sessions(source_id, sessions, session_directory).await?;
        input::set_input_profile(input_profile, source_id == sources::RETROARCH).await
    };
    if let Err(error) = prepared.await {
        warn!(source_id, application_id, %error, "application launch rejected");
        return BridgeResponse::Error {
            code: BridgeErrorCode::LaunchFailed,
            message: error.to_string(),
        };
    }
    let launched = match launch.await {
        Ok(launched) => launched,
        Err(error) => {
            sync_input_profile(sessions, session_directory).await;
            warn!(source_id, application_id, %error, "application launch rejected");
            return BridgeResponse::Error {
                code: BridgeErrorCode::LaunchFailed,
                message: error.to_string(),
            };
        }
    };
    let managed = ManagedSession {
        session: session.clone(),
        unit_name: launched.unit_name,
        input_profile,
    };
    if let Err(error) = save_managed_session(session_directory, &managed).await {
        let _ = systemd::stop_application(managed.unit_name.as_deref()).await;
        let _ = input::set_input_profile(InputProfile::Native, false).await;
        warn!(source_id, application_id, %error, "could not persist managed application session");
        return BridgeResponse::Error {
            code: BridgeErrorCode::LaunchFailed,
            message: "could not persist managed application session".to_owned(),
        };
    }
    sessions.lock().await.insert(session.id.clone(), managed);
    info!(source_id, application_id, "application launch accepted");
    BridgeResponse::LaunchAccepted {
        session: session.clone(),
    }
}

async fn sync_input_profile(
    sessions: &Arc<Mutex<HashMap<String, ManagedSession>>>,
    session_directory: &Path,
) {
    let active = active_managed_session(sessions, session_directory).await;
    if let Err(error) = sync_managed_input(active.as_ref().map(|(_, managed)| managed)).await {
        warn!(%error, "could not restore active input profile");
    }
}

async fn sync_managed_input(managed: Option<&ManagedSession>) -> Result<()> {
    let profile = managed
        .map(|managed| managed.input_profile)
        .unwrap_or_default();
    let retro_osk = managed.is_some_and(|managed| managed.session.source_id == sources::RETROARCH);
    input::set_input_profile(profile, retro_osk).await
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
        match systemd::application_is_running(managed.unit_name.as_deref()).await {
            // Definitively gone: drop it from the map and from disk.
            Ok(false) => {
                sessions.lock().await.remove(&session_id);
                if let Err(error) = remove_managed_session(session_directory, &session_id).await {
                    warn!(session_id, %error, "could not remove inactive application session record");
                }
                continue;
            }
            // Still running.
            Ok(true) => {}
            // Could not tell. A transient `systemctl` error must not be read as
            // "stopped": pruning a session whose application is still up makes the
            // application unclosable, because stopping it goes through the very
            // record we would have deleted.
            Err(error) => {
                warn!(
                    session_id,
                    %error,
                    "could not determine whether the application is running"
                );
            }
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

#[cfg(test)]
mod tests {
    use super::{
        ManagedSession, handle_request, load_managed_sessions, managed_session_path,
        save_managed_session, select_active_session, should_replace_active_session,
    };
    use hearthdeck_protocol::{
        ApplicationSession, ApplicationSessionState, BridgeErrorCode, BridgeRequest,
        BridgeResponse, InputProfile,
    };
    use std::collections::HashMap;
    use std::future::Future;
    use std::pin::pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Waker};
    use std::time::{Duration, SystemTime};
    use tokio::sync::Mutex;

    fn managed_session() -> ManagedSession {
        ManagedSession {
            session: ApplicationSession {
                id: "session-1".to_owned(),
                source_id: "desktop-apps".to_owned(),
                application_id: "org.example.App.desktop".to_owned(),
                state: ApplicationSessionState::Running,
            },
            unit_name: Some("hearthdeck-app-session-1.service".to_owned()),
            input_profile: InputProfile::Native,
        }
    }

    #[tokio::test]
    async fn stop_active_waits_for_launch_and_holds_transition_during_selection() {
        let temporary = tempfile::tempdir().unwrap();
        let sessions = Arc::new(Mutex::new(HashMap::new()));
        let transition = Mutex::new(());
        let launch = transition.lock().await;
        let selection = sessions.lock().await;
        let mut stop = pin!(handle_request(
            BridgeRequest::StopActiveApplicationSession,
            &sessions,
            &transition,
            temporary.path(),
        ));
        let mut context = Context::from_waker(Waker::noop());

        assert!(matches!(stop.as_mut().poll(&mut context), Poll::Pending));
        drop(launch);
        assert!(matches!(stop.as_mut().poll(&mut context), Poll::Pending));
        assert!(
            transition.try_lock().is_err(),
            "selection must exclude launches"
        );
        drop(selection);

        assert!(matches!(
            stop.await,
            BridgeResponse::Error {
                code: BridgeErrorCode::NotFound,
                ..
            }
        ));
        assert!(
            transition.try_lock().is_ok(),
            "failed stop must release the transition"
        );
    }

    #[tokio::test]
    async fn a_launch_with_an_unsafe_session_id_is_refused_before_anything_starts() {
        let temporary = tempfile::tempdir().unwrap();
        let sessions = Arc::new(Mutex::new(HashMap::new()));

        let response = handle_request(
            BridgeRequest::LaunchApplication {
                source_id: "desktop-apps".to_owned(),
                application_id: "org.example.App.desktop".to_owned(),
                session_id: "x.service ../../evil".to_owned(),
                input_profile: InputProfile::Native,
            },
            &sessions,
            &Mutex::new(()),
            temporary.path(),
        )
        .await;

        assert!(matches!(
            response,
            BridgeResponse::Error {
                code: BridgeErrorCode::InvalidRequest,
                ..
            }
        ));
        assert!(sessions.lock().await.is_empty());
        assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn rejected_stop_retains_the_tracked_session() {
        let temporary = tempfile::tempdir().unwrap();
        let mut managed = managed_session();
        managed.unit_name = None;
        save_managed_session(temporary.path(), &managed)
            .await
            .unwrap();
        let sessions = Arc::new(Mutex::new(HashMap::from([(
            managed.session.id.clone(),
            managed,
        )])));

        let response = handle_request(
            BridgeRequest::StopApplicationSession {
                session_id: "session-1".to_owned(),
            },
            &sessions,
            &Mutex::new(()),
            temporary.path(),
        )
        .await;

        assert!(matches!(
            response,
            BridgeResponse::Error {
                code: BridgeErrorCode::LaunchFailed,
                ..
            }
        ));
        assert!(sessions.lock().await.contains_key("session-1"));
        assert!(
            load_managed_sessions(temporary.path())
                .await
                .unwrap()
                .contains_key("session-1")
        );
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
            input_profile: InputProfile::Desktop,
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
