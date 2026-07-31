//! Typed messages exchanged only between the local daemon and bridge.
//!
//! These messages intentionally model allowlisted operations. Neither message
//! type has a shell-command field, so remote API input cannot become command
//! execution by forwarding it through this boundary.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeRequest {
    Health,
    DiscoverApplications {
        source_id: String,
    },
    LaunchApplication {
        source_id: String,
        application_id: String,
        application_name: String,
        session_id: String,
    },
    LaunchHeroicGame {
        runner: HeroicRunner,
        application_id: String,
        application_name: String,
        session_id: String,
    },
    ActiveApplicationSession,
    StopApplicationSession {
        session_id: String,
    },
}

/// The Heroic runners Hearthdeck can delegate to. The bridge constructs the
/// URI from this enum and a validated application ID; callers cannot supply a
/// free-form URI or command line.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HeroicRunner {
    Legendary,
    Gog,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeResponse {
    Health {
        version: String,
    },
    Applications {
        source_id: String,
        applications: Vec<DiscoveredApplication>,
    },
    LaunchAccepted {
        session: ApplicationSession,
    },
    ApplicationSession {
        session: Option<ApplicationSession>,
    },
    StopAccepted {
        session_id: String,
    },
    Error {
        code: BridgeErrorCode,
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ApplicationSession {
    pub id: String,
    pub source_id: String,
    pub application_id: String,
    /// The human-readable title to display for this session (a desktop
    /// entry's `Name=`, or a Heroic game's title). Falls back to
    /// `application_id` when a display name could not be resolved, so
    /// callers always have something reasonable to show.
    pub application_name: String,
    pub state: ApplicationSessionState,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationSessionState {
    Running,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DiscoveredApplication {
    pub application_id: String,
    pub name: String,
    pub comment: Option<String>,
    pub icon: Option<String>,
    pub categories: Vec<String>,
    pub launch_scheme: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeErrorCode {
    InvalidRequest,
    NotFound,
    LaunchFailed,
    Internal,
}

#[cfg(test)]
mod tests {
    use super::{
        ApplicationSession, ApplicationSessionState, BridgeRequest, BridgeResponse, HeroicRunner,
    };

    #[test]
    fn request_serialization_has_no_command_field() {
        let request = BridgeRequest::LaunchApplication {
            source_id: "desktop-apps".to_owned(),
            application_id: "org.example.Launcher.desktop".to_owned(),
            application_name: "Example Launcher".to_owned(),
            session_id: "session-1".to_owned(),
        };
        let serialized = serde_json::to_value(request).unwrap();

        assert_eq!(serialized["type"], "launch_application");
        assert_eq!(serialized["source_id"], "desktop-apps");
        assert_eq!(serialized["application_id"], "org.example.Launcher.desktop");
        assert_eq!(serialized["session_id"], "session-1");
        assert!(serialized.get("command").is_none());
        assert!(serialized.get("args").is_none());
    }

    #[test]
    fn response_round_trips() {
        let response = BridgeResponse::LaunchAccepted {
            session: ApplicationSession {
                id: "session-1".to_owned(),
                source_id: "desktop-apps".to_owned(),
                application_id: "org.example.App.desktop".to_owned(),
                application_name: "Example App".to_owned(),
                state: ApplicationSessionState::Running,
            },
        };
        let serialized = serde_json::to_string(&response).unwrap();
        let parsed: BridgeResponse = serde_json::from_str(&serialized).unwrap();

        assert!(
            matches!(parsed, BridgeResponse::LaunchAccepted { session } if session.id == "session-1")
        );
    }

    #[test]
    fn heroic_launch_serialization_is_typed() {
        let request = BridgeRequest::LaunchHeroicGame {
            runner: HeroicRunner::Legendary,
            application_id: "Fortnite".to_owned(),
            application_name: "Fortnite".to_owned(),
            session_id: "session-1".to_owned(),
        };
        let serialized = serde_json::to_value(request).unwrap();

        assert_eq!(serialized["type"], "launch_heroic_game");
        assert_eq!(serialized["runner"], "legendary");
        assert!(serialized.get("command").is_none());
        assert!(serialized.get("url").is_none());
    }
}
