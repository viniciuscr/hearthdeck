use std::{path::Path, time::Duration};

use hearthdeck_protocol::{
    ApplicationSession, BridgeErrorCode, BridgeRequest, BridgeResponse, ServerEvent,
};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::{
    activity::{ActivityEntry, ActivityStore, SessionOutcome},
    discovery::DiscoveryService,
};

const SESSION_WATCH_INTERVAL: Duration = Duration::from_secs(10);

pub(crate) struct SessionService<'a> {
    pub bridge_socket: &'a Path,
    pub activity: &'a ActivityStore,
    pub discovery: Option<&'a DiscoveryService>,
    pub events: &'a broadcast::Sender<ServerEvent>,
}

#[derive(Debug)]
pub(crate) enum LaunchError {
    Transport(anyhow::Error),
    Rejected {
        code: BridgeErrorCode,
        message: String,
    },
    Unexpected(BridgeResponse),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ControlError {
    #[error("{0}")]
    Transport(#[from] anyhow::Error),
    #[error("{message}")]
    Rejected {
        code: BridgeErrorCode,
        message: String,
    },
    #[error("bridge rejected application-session lookup")]
    UnexpectedLookup,
    #[error("bridge rejected application-session stop")]
    UnexpectedStop,
}

pub(crate) enum StopTarget<'a> {
    Active,
    Session(&'a str),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(error) => write!(formatter, "{error}"),
            Self::Rejected { message, .. } => write!(formatter, "{message}"),
            Self::Unexpected(response) => {
                write!(formatter, "unexpected bridge launch response: {response:?}")
            }
        }
    }
}

impl std::error::Error for LaunchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error.as_ref()),
            Self::Rejected { .. } | Self::Unexpected(_) => None,
        }
    }
}

impl SessionService<'_> {
    pub async fn active(&self) -> Result<Option<ApplicationSession>, ControlError> {
        match crate::bridge::request(self.bridge_socket, BridgeRequest::ActiveApplicationSession)
            .await?
        {
            BridgeResponse::ApplicationSession { session } => Ok(session),
            _ => Err(ControlError::UnexpectedLookup),
        }
    }

    pub async fn stop(&self, target: StopTarget<'_>) -> Result<(), ControlError> {
        let request = match target {
            StopTarget::Active => BridgeRequest::StopActiveApplicationSession,
            StopTarget::Session(id) => BridgeRequest::StopApplicationSession {
                session_id: id.to_owned(),
            },
        };
        match crate::bridge::request(self.bridge_socket, request).await? {
            BridgeResponse::StopAccepted { session_id } => {
                let id = match target {
                    StopTarget::Active => session_id.as_str(),
                    StopTarget::Session(id) => id,
                };
                // Explicit stops record the end now, rather than at the next watcher tick.
                self.finished(id).await;
                Ok(())
            }
            BridgeResponse::Error { code, message } => {
                Err(ControlError::Rejected { code, message })
            }
            _ => Err(ControlError::UnexpectedStop),
        }
    }

    pub async fn launch(
        &self,
        request: BridgeRequest,
        entry: ActivityEntry,
    ) -> Result<ApplicationSession, LaunchError> {
        let response = crate::bridge::request(self.bridge_socket, request)
            .await
            .map_err(LaunchError::Transport)?;
        let session = match response {
            BridgeResponse::LaunchAccepted { session } => session,
            BridgeResponse::Error { code, message } => {
                return Err(LaunchError::Rejected { code, message });
            }
            other => return Err(LaunchError::Unexpected(other)),
        };
        if let Err(error) = self.activity.session_started(entry, &session).await {
            warn!(%error, session_id = %session.id, "failed to record launch activity");
        }
        let _ = self.events.send(ServerEvent::ApplicationSessionChanged {
            session: Some(session.clone()),
        });
        Ok(session)
    }

    async fn finished(&self, session_id: &str) {
        let source = match self
            .activity
            .session_ended(session_id, SessionOutcome::Closed)
            .await
        {
            Ok(Some(source)) => Some(source),
            Ok(None) => return,
            Err(error) => {
                warn!(%error, %session_id, "could not close a finished play");
                None
            }
        };
        info!(%session_id, source = source.as_deref(), "play finished");
        if let (Some(source), Some(discovery)) = (source, self.discovery) {
            discovery.request(&source).await;
        }
        let _ = self
            .events
            .send(ServerEvent::ApplicationSessionChanged { session: None });
    }

    pub async fn watch(&self) {
        loop {
            tokio::time::sleep(SESSION_WATCH_INTERVAL).await;
            self.reconcile().await;
        }
    }

    async fn reconcile(&self) {
        let running = match self.activity.running_sessions().await {
            Ok(running) => running,
            Err(error) => {
                warn!(%error, "could not list running plays");
                return;
            }
        };
        if running.is_empty() {
            return;
        }
        let active = match self.active().await {
            Ok(session) => session.map(|it| it.id),
            Err(ControlError::Transport(error)) => {
                debug!(%error, "bridge unreachable while watching a play");
                return;
            }
            Err(error) => {
                warn!(%error, "could not query the active session while watching a play");
                return;
            }
        };
        for session_id in finished_sessions(&running, active.as_deref()) {
            self.finished(&session_id).await;
        }
    }
}

fn finished_sessions(running: &[String], active: Option<&str>) -> Vec<String> {
    running
        .iter()
        .filter(|session_id| Some(session_id.as_str()) != active)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use hearthdeck_protocol::{ApplicationSessionState, InputProfile};
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::UnixListener,
        sync::broadcast::error::TryRecvError,
        task::JoinHandle,
    };

    use super::*;
    use crate::database::Database;

    fn entry() -> ActivityEntry {
        ActivityEntry {
            id: "hearthdeck:test".to_owned(),
            title: "Test".to_owned(),
            icon: None,
            categories: Vec::new(),
            kind: Some("game".to_owned()),
            source: "test".to_owned(),
            metadata: serde_json::Value::Null,
        }
    }

    fn session(id: &str) -> ApplicationSession {
        ApplicationSession {
            id: id.to_owned(),
            source_id: "desktop-apps".to_owned(),
            application_id: "test.desktop".to_owned(),
            state: ApplicationSessionState::Running,
        }
    }

    async fn store() -> (tempfile::TempDir, ActivityStore) {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("activity.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        (directory, ActivityStore::new(database.pool().clone()))
    }

    fn bridge(path: &Path, response: &BridgeResponse) -> JoinHandle<BridgeRequest> {
        let listener = UnixListener::bind(path).unwrap();
        let mut response = serde_json::to_vec(response).unwrap();
        response.push(b'\n');
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut request)
                .await
                .unwrap();
            let request = serde_json::from_str(&request).unwrap();
            stream.write_all(&response).await.unwrap();
            request
        })
    }

    #[tokio::test]
    async fn lookup_reports_the_bridge_session_without_finishing_activity() {
        for reply in [
            Some(BridgeResponse::ApplicationSession {
                session: Some(session("selected")),
            }),
            Some(BridgeResponse::ApplicationSession { session: None }),
            Some(BridgeResponse::Error {
                code: BridgeErrorCode::NotFound,
                message: "no active session".to_owned(),
            }),
            Some(BridgeResponse::Health {
                version: "unexpected".to_owned(),
            }),
            None,
        ] {
            let (directory, activity) = store().await;
            activity
                .session_started(entry(), &session("selected"))
                .await
                .unwrap();
            let socket = directory.path().join("bridge.sock");
            let (events, mut receiver) = broadcast::channel(8);
            let service = SessionService {
                bridge_socket: &socket,
                activity: &activity,
                discovery: None,
                events: &events,
            };
            let server = reply.as_ref().map(|reply| bridge(&socket, reply));
            let result = service.active().await;
            match reply {
                Some(BridgeResponse::ApplicationSession { session }) => {
                    assert_eq!(
                        serde_json::to_value(result.unwrap()).unwrap(),
                        serde_json::to_value(session).unwrap()
                    );
                }
                Some(_) => assert!(matches!(result, Err(ControlError::UnexpectedLookup))),
                None => assert!(matches!(result, Err(ControlError::Transport(_)))),
            }
            if let Some(server) = server {
                assert!(matches!(
                    tokio::time::timeout(Duration::from_secs(5), server)
                        .await
                        .unwrap()
                        .unwrap(),
                    BridgeRequest::ActiveApplicationSession
                ));
            }
            assert_eq!(activity.running_sessions().await.unwrap(), ["selected"]);
            assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        }
    }

    #[tokio::test]
    async fn stops_finish_only_the_selected_play_and_only_after_bridge_acceptance() {
        for reply in [
            Some(BridgeResponse::StopAccepted {
                session_id: "selected".to_owned(),
            }),
            Some(BridgeResponse::Error {
                code: BridgeErrorCode::NotFound,
                message: "no active session".to_owned(),
            }),
            Some(BridgeResponse::Health {
                version: "unexpected".to_owned(),
            }),
            None,
        ] {
            for target in [StopTarget::Active, StopTarget::Session("selected")] {
                let active = matches!(target, StopTarget::Active);
                let (directory, activity) = store().await;
                for id in ["selected", "other"] {
                    activity
                        .session_started(entry(), &session(id))
                        .await
                        .unwrap();
                }
                let socket = directory.path().join("bridge.sock");
                let (events, mut receiver) = broadcast::channel(8);
                let service = SessionService {
                    bridge_socket: &socket,
                    activity: &activity,
                    discovery: None,
                    events: &events,
                };
                let server = reply.as_ref().map(|reply| bridge(&socket, reply));
                let result = service.stop(target).await;
                if let Some(server) = server {
                    let request = tokio::time::timeout(Duration::from_secs(5), server)
                        .await
                        .unwrap()
                        .unwrap();
                    match request {
                        BridgeRequest::StopActiveApplicationSession => assert!(active),
                        BridgeRequest::StopApplicationSession { session_id } => {
                            assert!(!active);
                            assert_eq!(session_id, "selected");
                        }
                        other => panic!("unexpected stop request: {other:?}"),
                    }
                }
                match &reply {
                    Some(BridgeResponse::StopAccepted { .. }) => {
                        result.unwrap();
                        assert_eq!(activity.running_sessions().await.unwrap(), ["other"]);
                        assert!(matches!(
                            receiver.try_recv().unwrap(),
                            ServerEvent::ApplicationSessionChanged { session: None }
                        ));
                        service.finished("selected").await;
                    }
                    other => {
                        match other {
                            Some(BridgeResponse::Error { .. }) => assert!(matches!(result,
                                Err(ControlError::Rejected { code: BridgeErrorCode::NotFound, message })
                                if message == "no active session")),
                            Some(_) => {
                                assert!(matches!(result, Err(ControlError::UnexpectedStop)))
                            }
                            None => assert!(matches!(result, Err(ControlError::Transport(_)))),
                        }
                        assert_eq!(activity.running_sessions().await.unwrap().len(), 2);
                    }
                }
                assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
            }
        }
    }

    #[tokio::test]
    async fn only_accepted_launches_record_activity_and_publish_the_bridge_session() {
        for reply in [
            BridgeResponse::LaunchAccepted {
                session: session("bridge-selected"),
            },
            BridgeResponse::Error {
                code: BridgeErrorCode::LaunchFailed,
                message: "launch failed".to_owned(),
            },
            BridgeResponse::Health {
                version: "unexpected".to_owned(),
            },
        ] {
            let (directory, activity) = store().await;
            let socket = directory.path().join("bridge.sock");
            let (events, mut receiver) = broadcast::channel(8);
            let service = SessionService {
                bridge_socket: &socket,
                activity: &activity,
                discovery: None,
                events: &events,
            };
            let accepted = matches!(reply, BridgeResponse::LaunchAccepted { .. });
            let rejected = matches!(reply, BridgeResponse::Error { .. });
            let server = bridge(&socket, &reply);
            let result = service
                .launch(
                    BridgeRequest::LaunchApplication {
                        source_id: "desktop-apps".to_owned(),
                        application_id: "test.desktop".to_owned(),
                        session_id: "requested".to_owned(),
                        input_profile: InputProfile::default(),
                    },
                    entry(),
                )
                .await;
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(5), server).await.unwrap().unwrap(),
                BridgeRequest::LaunchApplication { session_id, .. } if session_id == "requested"
            ));
            if accepted {
                assert_eq!(result.unwrap().id, "bridge-selected");
                assert_eq!(
                    activity.running_sessions().await.unwrap(),
                    ["bridge-selected"]
                );
                let recent = activity.recent(1).await.unwrap();
                assert_eq!(recent[0].entry.id, "hearthdeck:test");
                assert_eq!(recent[0].launch_count, 1);
                assert!(matches!(receiver.try_recv().unwrap(),
                    ServerEvent::ApplicationSessionChanged { session: Some(session) }
                    if session.id == "bridge-selected"));
            } else {
                if rejected {
                    assert!(
                        matches!(result, Err(LaunchError::Rejected { code: BridgeErrorCode::LaunchFailed, message }) if message == "launch failed")
                    );
                } else {
                    assert!(matches!(
                        result,
                        Err(LaunchError::Unexpected(BridgeResponse::Health { .. }))
                    ));
                }
                assert!(activity.running_sessions().await.unwrap().is_empty());
                assert!(activity.recent(1).await.unwrap().is_empty());
                assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
            }
        }
    }

    #[tokio::test]
    async fn reconciliation_keeps_running_plays_until_the_bridge_confirms_they_ended() {
        let (directory, activity) = store().await;
        let (events, mut receiver) = broadcast::channel(8);
        for id in ["selected", "other"] {
            activity
                .session_started(entry(), &session(id))
                .await
                .unwrap();
        }
        // A missing bridge is not evidence that a play stopped.
        let missing = directory.path().join("missing.sock");
        let service = SessionService {
            bridge_socket: &missing,
            activity: &activity,
            discovery: None,
            events: &events,
        };
        assert!(matches!(
            service
                .launch(
                    BridgeRequest::LaunchApplication {
                        source_id: "desktop-apps".to_owned(),
                        application_id: "test.desktop".to_owned(),
                        session_id: "new".to_owned(),
                        input_profile: InputProfile::default(),
                    },
                    entry()
                )
                .await,
            Err(LaunchError::Transport(_))
        ));
        service.reconcile().await;
        assert_eq!(activity.running_sessions().await.unwrap().len(), 2);
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));

        for (index, reply) in [
            BridgeResponse::Health {
                version: "unexpected".to_owned(),
            },
            BridgeResponse::ApplicationSession {
                session: Some(session("selected")),
            },
            BridgeResponse::ApplicationSession { session: None },
        ]
        .into_iter()
        .enumerate()
        {
            let socket = directory.path().join(format!("bridge-{index}.sock"));
            let service = SessionService {
                bridge_socket: &socket,
                activity: &activity,
                discovery: None,
                events: &events,
            };
            let server = bridge(&socket, &reply);
            service.reconcile().await;
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(5), server)
                    .await
                    .unwrap()
                    .unwrap(),
                BridgeRequest::ActiveApplicationSession
            ));
            let running = activity.running_sessions().await.unwrap();
            match index {
                0 => {
                    assert_eq!(running.len(), 2);
                    assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
                }
                1 => assert_eq!(running, ["selected"]),
                2 => assert!(running.is_empty()),
                _ => unreachable!(),
            }
            if index > 0 {
                assert!(matches!(
                    receiver.try_recv().unwrap(),
                    ServerEvent::ApplicationSessionChanged { session: None }
                ));
            }
        }
        service.finished("selected").await;
        service.finished("other").await;
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn finished_sessions_are_the_ones_the_bridge_stopped_reporting() {
        let running = vec!["a".to_owned(), "b".to_owned()];
        assert!(finished_sessions(&running, Some("a")).eq(&["b"]));
        assert!(finished_sessions(&running, None).eq(&["a", "b"]));
        assert!(finished_sessions(&running, Some("b")).eq(&["a"]));
        assert!(finished_sessions(&[], None).is_empty());
    }
}
