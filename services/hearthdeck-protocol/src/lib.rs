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
    },
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
        source_id: String,
        application_id: String,
    },
    Error {
        code: BridgeErrorCode,
        message: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DiscoveredApplication {
    pub application_id: String,
    pub name: String,
    pub comment: Option<String>,
    pub icon: Option<String>,
    pub categories: Vec<String>,
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
    use super::{BridgeRequest, BridgeResponse};

    #[test]
    fn request_serialization_has_no_command_field() {
        let request = BridgeRequest::LaunchApplication {
            source_id: "desktop-apps".to_owned(),
            application_id: "org.example.Launcher.desktop".to_owned(),
        };
        let serialized = serde_json::to_value(request).unwrap();

        assert_eq!(serialized["type"], "launch_application");
        assert_eq!(serialized["source_id"], "desktop-apps");
        assert_eq!(serialized["application_id"], "org.example.Launcher.desktop");
        assert!(serialized.get("command").is_none());
        assert!(serialized.get("args").is_none());
    }

    #[test]
    fn response_round_trips() {
        let response = BridgeResponse::Health {
            version: "0.1.0".to_owned(),
        };
        let serialized = serde_json::to_string(&response).unwrap();
        let parsed: BridgeResponse = serde_json::from_str(&serialized).unwrap();

        assert!(matches!(parsed, BridgeResponse::Health { version } if version == "0.1.0"));
    }
}
