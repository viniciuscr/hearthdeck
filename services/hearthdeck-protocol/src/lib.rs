//! Typed messages and payloads shared across the Hearthdeck processes.
//!
//! Two kinds of thing live here:
//!
//! * The `Bridge*` messages exchanged only between the local daemon and bridge.
//!   These intentionally model allowlisted operations. Neither request type has a
//!   shell-command field, so remote API input cannot become command execution by
//!   forwarding it through this boundary.
//! * The paired-API payloads the daemon serves and the client reads. Only the
//!   ones with **no projection layer** belong here: a payload the client reads in
//!   full, or one that drives client behaviour and must never silently drift.
//!   Response types the client deliberately narrows to the fields it draws stay
//!   client-side; those tolerate daemon additions because serde ignores unknown
//!   fields, and sharing them would couple the client to fields it never reads.

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
        session_id: String,
        #[serde(default)]
        input_profile: InputProfile,
    },
    LaunchHeroicGame {
        runner: HeroicRunner,
        application_id: String,
        session_id: String,
        #[serde(default)]
        input_profile: InputProfile,
    },
    /// Launches a RetroArch core against a locally cached ROM. The daemon
    /// resolves the platform-to-core mapping and fetches/caches the ROM from
    /// RomM (RomM credentials never leave the daemon); this request only
    /// carries the resolved local paths. The bridge re-validates both paths
    /// itself (core is under an allowlisted cores directory, ROM is under
    /// Hearthdeck's own cache directory) before launch, the same way it
    /// re-discovers a desktop entry rather than trusting the daemon's copy.
    LaunchRetroGame {
        core_path: String,
        rom_path: String,
        session_id: String,
        #[serde(default)]
        input_profile: InputProfile,
    },
    ActiveApplicationSession,
    StopApplicationSession {
        session_id: String,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputProfile {
    #[default]
    Native,
    Desktop,
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

/// Capabilities the host advertises on `/v1/health`.
///
/// Shared rather than projected: the client reads every flag to decide what it may
/// offer, so a renamed flag would silently disable a feature instead of failing.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct HostCapabilities {
    pub launch: bool,
    pub application_sessions: bool,
    pub install_requests: bool,
    pub retro_launch: bool,
    /// Whether this deployment provides categorization at all. Distinct from the
    /// user's opt-in: with this false the feature does not exist for the client,
    /// and with it true the client still has to ask.
    #[serde(default)]
    pub categorization: bool,
}

/// One catalog row, as served by `/v1/library` and read back in full.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CatalogItem {
    pub id: String,
    pub source_id: String,
    pub title: String,
    pub kind: String,
    pub launch_id: Option<String>,
    pub icon: Option<String>,
    pub metadata: serde_json::Value,
}

/// One selectable version of a retro game, as offered in the version picker.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RetroRomVersion {
    pub id: i64,
    pub title: String,
    /// The user's chosen main file for this game, per RomM; false when unset.
    #[serde(default)]
    pub is_main_sibling: bool,
}

#[cfg(test)]
mod tests {
    use super::{
        ApplicationSession, ApplicationSessionState, BridgeRequest, BridgeResponse, CatalogItem,
        HeroicRunner, HostCapabilities, InputProfile, RetroRomVersion,
    };

    #[test]
    fn shared_api_payloads_keep_their_wire_names() {
        // These are read by the client by field name, so a rename here is a silent
        // break across the process boundary. Pin the names.
        let capabilities = HostCapabilities {
            launch: true,
            application_sessions: false,
            install_requests: false,
            retro_launch: true,
            categorization: true,
        };
        let value = serde_json::to_value(capabilities).unwrap();
        for field in [
            "launch",
            "application_sessions",
            "install_requests",
            "retro_launch",
            "categorization",
        ] {
            assert!(value.get(field).is_some(), "capabilities lost `{field}`");
        }

        let item = CatalogItem {
            id: "hearthdeck:org.example.App".to_owned(),
            source_id: "desktop-apps".to_owned(),
            title: "Example".to_owned(),
            kind: "application".to_owned(),
            launch_id: None,
            icon: None,
            metadata: serde_json::Value::Null,
        };
        let value = serde_json::to_value(item).unwrap();
        for field in [
            "id",
            "source_id",
            "title",
            "kind",
            "launch_id",
            "icon",
            "metadata",
        ] {
            assert!(value.get(field).is_some(), "catalog item lost `{field}`");
        }

        let version = RetroRomVersion {
            id: 1,
            title: "Disc 1".to_owned(),
            is_main_sibling: true,
        };
        let value = serde_json::to_value(version).unwrap();
        assert_eq!(value["is_main_sibling"], serde_json::Value::Bool(true));
    }

    #[test]
    fn request_serialization_has_no_command_field() {
        let request = BridgeRequest::LaunchApplication {
            source_id: "desktop-apps".to_owned(),
            application_id: "org.example.Launcher.desktop".to_owned(),
            session_id: "session-1".to_owned(),
            input_profile: InputProfile::Desktop,
        };
        let serialized = serde_json::to_value(request).unwrap();

        assert_eq!(serialized["type"], "launch_application");
        assert_eq!(serialized["source_id"], "desktop-apps");
        assert_eq!(serialized["application_id"], "org.example.Launcher.desktop");
        assert_eq!(serialized["session_id"], "session-1");
        assert_eq!(serialized["input_profile"], "desktop");
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
            session_id: "session-1".to_owned(),
            input_profile: InputProfile::Native,
        };
        let serialized = serde_json::to_value(request).unwrap();

        assert_eq!(serialized["type"], "launch_heroic_game");
        assert_eq!(serialized["runner"], "legendary");
        assert!(serialized.get("command").is_none());
        assert!(serialized.get("url").is_none());
    }

    #[test]
    fn retro_launch_serialization_carries_only_resolved_paths() {
        let request = BridgeRequest::LaunchRetroGame {
            core_path: "/usr/lib/libretro/snes9x_libretro.so".to_owned(),
            rom_path: "/home/user/.cache/hearthdeck/romm/42.sfc".to_owned(),
            session_id: "session-1".to_owned(),
            input_profile: InputProfile::Native,
        };
        let serialized = serde_json::to_value(request).unwrap();

        assert_eq!(serialized["type"], "launch_retro_game");
        assert_eq!(
            serialized["core_path"],
            "/usr/lib/libretro/snes9x_libretro.so"
        );
        assert_eq!(
            serialized["rom_path"],
            "/home/user/.cache/hearthdeck/romm/42.sfc"
        );
        assert!(serialized.get("command").is_none());
        assert!(serialized.get("url").is_none());
    }
}
