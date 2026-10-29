//! Typed messages and payloads shared across the Hearthdeck processes.
//!
//! Two kinds of thing live here:
//!
//! * The `Bridge*` messages exchanged only between the local daemon and bridge.
//!   These intentionally model allowlisted operations. Neither request type has a
//!   shell-command field, so remote API input cannot become command execution by
//!   forwarding it through this boundary.
//! * The paired-API payloads the daemon serves and the client reads. Only the
//!   These own the wire shape even when a client draws only part of it. Explicit
//!   projections, such as event kinds or the categorization report summary, let
//!   clients ignore payloads they do not need.
//!
//! The dependency list stays deliberately small, so every process can link this
//! crate without pulling in a transport or UI stack.

use serde::{Deserialize, Serialize};

pub mod categorization;
pub mod paths;
pub mod sources;
pub use categorization::{
    AppCategorization, AppTraits, CategoryMatch, CategoryProposal, DecisionProvenance, ScanFailure,
    ScanReport, UnclassifiedApp,
};

/// Whether a Stremio title is a film or a series.
///
/// The detail route is `stremio:///detail/<type>/<id>`, so an id without an
/// episode needs its type to name the right title: the same `tt` number is a
/// different page for a film and for a series. A `tt:s:e` video id already says
/// series on its own, so this only decides the bare-id form.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StremioMediaType {
    #[default]
    Movie,
    Series,
}

impl StremioMediaType {
    /// The segment the detail route wears.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
        }
    }
}

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
    /// Opens a title's own page in Stremio.
    ///
    /// Carries the video id and nothing else about the title — not a URL — so the bridge
    /// validates the id and builds the `stremio://` URI itself. A client can therefore
    /// never hand the bridge a link to run, which is the same rule `LaunchRetroGame`
    /// follows with a core path and a ROM path.
    ///
    /// `application_id` is the desktop entry that opens titles, discovered the same way
    /// any application id is. The bridge does not look it up again: the caller already
    /// resolved it against this host's own application list, and a second, stricter
    /// lookup in the bridge is how a working launch became "no desktop entry declares
    /// the stremio scheme".
    LaunchStremioTitle {
        application_id: String,
        video_id: String,
        /// What the title is, for the bare-id form above: `tt1234567` is a film
        /// when this says film and a series when it says series. Absent on the
        /// wire means a film, which is what every record sent before this field
        /// existed was.
        #[serde(default)]
        media_type: StremioMediaType,
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

/// A mode change the bridge sends to the `hearthdeck-input` broker as one
/// datagram on [`paths::input_socket_path`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputMode {
    pub profile: InputProfile,
    pub retro_osk: bool,
}

impl InputMode {
    #[must_use]
    pub fn command(self) -> &'static [u8] {
        match (self.profile, self.retro_osk) {
            (InputProfile::Native, false) => b"native",
            (InputProfile::Desktop, false) => b"desktop",
            (InputProfile::Native, true) => b"retro-native",
            (InputProfile::Desktop, true) => b"retro-desktop",
        }
    }

    #[must_use]
    pub fn parse(command: &[u8]) -> Option<Self> {
        let (profile, retro_osk) = match command {
            b"native" => (InputProfile::Native, false),
            b"desktop" => (InputProfile::Desktop, false),
            b"retro-native" => (InputProfile::Native, true),
            b"retro-desktop" => (InputProfile::Desktop, true),
            _ => return None,
        };
        Some(Self { profile, retro_osk })
    }
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
    /// Where this launcher came from. The scanner knows it from the directory it
    /// read the entry out of, so it travels resolved rather than as a path each
    /// consumer would have to interpret for itself.
    pub source: ApplicationSource,
}

/// How an installed application is packaged, and which part of the filesystem
/// its launcher lives under.
///
/// This is the vocabulary a user filters a launcher by ("show me the flatpaks"),
/// so the variants are the distinctions that answer that question, not every
/// directory a desktop entry can appear in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationSource {
    /// The distribution's own packages.
    System,
    /// Installed for one user, under their own data home.
    Local,
    /// A flatpak, system-wide or per-user.
    Flatpak,
    /// A snap.
    Snap,
    /// A nix profile's launchers.
    Nix,
}

/// Top-level library sections shared by categorization results and clients.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    PcGames,
    ConsoleGames,
    Applications,
}

impl Section {
    pub const ALL: [Self; 3] = [Self::PcGames, Self::ConsoleGames, Self::Applications];
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

/// A ROM as returned by the retro library endpoint.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RetroGame {
    pub id: i64,
    pub platform_id: i64,
    pub title: String,
    pub summary: Option<String>,
    pub cover_path: Option<String>,
    #[serde(default)]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub screenshot_paths: Vec<String>,
    #[serde(default)]
    pub has_manual: bool,
    #[serde(default)]
    pub genres: Vec<String>,
    pub player_count: Option<String>,
    pub release_year: Option<i32>,
    #[serde(default)]
    pub regions: Vec<String>,
    #[serde(default)]
    pub sibling_roms: Vec<RetroRomVersion>,
    #[serde(default)]
    pub version_label: Option<String>,
}

/// The full per-ROM response; list-only payloads use [`RetroGame`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RetroGameDetails {
    pub id: i64,
    pub platform_id: i64,
    pub title: String,
    pub platform_name: Option<String>,
    pub summary: Option<String>,
    pub cover_path: Option<String>,
    pub cover_large_path: Option<String>,
    #[serde(default)]
    pub screenshot_paths: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    pub player_count: Option<String>,
    pub release_year: Option<i32>,
    pub release_date: Option<String>,
    #[serde(default)]
    pub regions: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub companies: Vec<String>,
    #[serde(default)]
    pub game_modes: Vec<String>,
    #[serde(default)]
    pub age_ratings: Vec<String>,
    pub average_rating: Option<f64>,
    pub file_size_bytes: Option<u64>,
    #[serde(default)]
    pub sibling_roms: Vec<RetroRomVersion>,
    pub version_label: Option<String>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub backlogged: bool,
}

/// The unauthenticated daemon health response.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HealthResponse {
    pub version: String,
    pub lan_enabled: bool,
    pub transport: String,
    pub providers: Vec<ProviderHealth>,
    pub capabilities: HostCapabilities,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Discovery,
    Metadata,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Starting,
    Refreshing,
    Ready,
    Degraded,
}

/// Health of one daemon provider, safe to expose from `/v1/health`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProviderHealth {
    pub id: String,
    pub kind: ProviderKind,
    pub status: ProviderStatus,
    pub record_count: Option<usize>,
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
}

/// A play snapshot shared by the activity endpoint and dashboard collections.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ActivityEntry {
    pub id: String,
    pub title: String,
    pub icon: Option<String>,
    pub categories: Vec<String>,
    /// Absent only in play snapshots saved before the catalog kind was recorded.
    #[serde(default)]
    pub kind: Option<String>,
    pub source: String,
    pub metadata: serde_json::Value,
}

/// A play and its aggregate launch counts.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RecentActivity {
    #[serde(flatten)]
    pub entry: ActivityEntry,
    pub last_launched_at: String,
    pub launch_count: i64,
}

/// A composed dashboard collection.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Collection {
    pub slug: String,
    #[serde(default)]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub owner: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub items: Vec<CollectionItem>,
}

/// A collection card, optionally carrying artwork and the play it represents.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CollectionItem {
    pub item_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub played: Option<RecentActivity>,
}

/// An event sent to paired clients when daemon state changes.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
// The variant names preserve the event names already used on the wire.
#[allow(clippy::enum_variant_names)]
pub enum ServerEvent {
    LibraryChanged {
        source_id: String,
        record_count: usize,
    },
    MetadataChanged {
        provider_id: String,
        record_count: usize,
    },
    ApplicationSessionChanged {
        session: Option<ApplicationSession>,
    },
    CategorizationChanged {
        categorizer: String,
        app_count: usize,
        categories: usize,
    },
}

/// Event kinds for clients that refresh state instead of applying event payloads.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
// The variant names preserve the event names already used on the wire.
#[allow(clippy::enum_variant_names)]
pub enum ServerEventKind {
    LibraryChanged,
    MetadataChanged,
    ApplicationSessionChanged,
    CategorizationChanged,
    #[serde(other)]
    Unknown,
}

/// The `/v1/categorization` reply: the user's opt-in, the job, and the last report.
///
/// Generic over the report because the client reads only a summary of it, while
/// the daemon serves the whole thing.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CategorizationSnapshot<R> {
    /// The user's opt-in. Everything heavy — the download, the scan, replacing
    /// the tabs — waits on it.
    pub enabled: bool,
    pub status: CategorizationStatus,
    /// `None` until the library has been categorized once.
    pub report: Option<R>,
}

/// Is a scan running, how far along, is the model installed, and how did the last
/// one end.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CategorizationStatus {
    pub running: bool,
    pub phase: CategorizationPhase,
    /// Applications decided so far, while `phase` is `Scanning`.
    pub completed: usize,
    pub total: usize,
    pub model: ModelStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completed_at: Option<String>,
}

/// Where a running scan has got to.
///
/// `Downloading` is the daemon's own refinement, not something the categorizer
/// child reports: fetching the checkpoint happens inside the engine load, and the
/// daemon knows whether a checkpoint was already on disk when it started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CategorizationPhase {
    Idle,
    Downloading,
    Loading,
    Scanning,
}

/// Whether a checkpoint is available, and what it costs on disk.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelStatus {
    /// Nothing to run yet. The first scan will fetch one.
    Absent,
    /// A checkpoint in the daemon's own cache.
    Downloaded {
        bytes: u64,
        path: std::path::PathBuf,
    },
    /// A checkpoint the deployment pointed at directly, which is how an offline
    /// install sideloads one. Never deleted by Hearthdeck.
    External { path: std::path::PathBuf },
}

#[cfg(test)]
mod tests {
    use super::{
        ApplicationSession, ApplicationSessionState, ApplicationSource, BridgeRequest,
        BridgeResponse, CatalogItem, CategorizationPhase, CategorizationSnapshot,
        CategorizationStatus, Collection, CollectionItem, DiscoveredApplication, HealthResponse,
        HeroicRunner, HostCapabilities, InputMode, InputProfile, ModelStatus, ProviderHealth,
        ProviderKind, ProviderStatus, RetroGame, RetroGameDetails, RetroRomVersion, Section,
        ServerEvent, ServerEventKind,
    };

    #[test]
    fn section_values_keep_their_wire_names_and_order() {
        let cases = [
            (Section::PcGames, "\"pc_games\""),
            (Section::ConsoleGames, "\"console_games\""),
            (Section::Applications, "\"applications\""),
        ];
        assert_eq!(
            Section::ALL,
            [
                Section::PcGames,
                Section::ConsoleGames,
                Section::Applications
            ]
        );
        for (section, json) in cases {
            assert_eq!(serde_json::to_string(&section).unwrap(), json);
        }
    }

    #[test]
    fn shared_daemon_payloads_keep_their_wire_shape() {
        let health = HealthResponse {
            version: "1.0".to_owned(),
            lan_enabled: false,
            transport: "http".to_owned(),
            providers: vec![ProviderHealth {
                id: "desktop-apps".to_owned(),
                kind: ProviderKind::Discovery,
                status: ProviderStatus::Ready,
                record_count: Some(2),
                last_attempt_at: None,
                last_success_at: None,
                last_error: None,
            }],
            capabilities: HostCapabilities {
                launch: true,
                application_sessions: true,
                retro_launch: true,
                categorization: false,
            },
        };
        let health_json = serde_json::to_value(health).unwrap();
        assert_eq!(health_json["providers"][0]["kind"], "discovery");
        assert_eq!(health_json["providers"][0]["status"], "ready");
        assert!(health_json.get("capabilities").is_some());

        let collection = Collection {
            slug: "favorites".to_owned(),
            kind: "membership".to_owned(),
            rule: None,
            role: "favorite".to_owned(),
            position: 0,
            owner: "system".to_owned(),
            name: None,
            items: vec![CollectionItem {
                item_id: "hearthdeck:game-1".to_owned(),
                name: "Game".to_owned(),
                icon: None,
                background: None,
                logo: None,
                description: None,
                played: None,
            }],
        };
        let collection_json = serde_json::to_value(collection).unwrap();
        assert_eq!(collection_json["kind"], "membership");
        assert_eq!(collection_json["items"][0]["item_id"], "hearthdeck:game-1");
        assert!(collection_json["items"][0].get("played").is_none());

        let retro_game = RetroGame {
            id: 1,
            platform_id: 2,
            title: "Game".to_owned(),
            summary: None,
            cover_path: None,
            cover_url: None,
            screenshot_paths: Vec::new(),
            has_manual: false,
            genres: Vec::new(),
            player_count: None,
            release_year: None,
            regions: Vec::new(),
            sibling_roms: Vec::new(),
            version_label: None,
        };
        let retro_json = serde_json::to_value(retro_game).unwrap();
        assert_eq!(retro_json["platform_id"], 2);
        assert_eq!(retro_json["has_manual"], false);
        let decoded: RetroGame = serde_json::from_value(retro_json.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), retro_json);
    }

    #[test]
    fn event_projections_match_every_daemon_event_and_tolerate_new_kinds() {
        let cases = [
            (
                ServerEvent::LibraryChanged {
                    source_id: "romm".to_owned(),
                    record_count: 2,
                },
                "library_changed",
                ServerEventKind::LibraryChanged,
            ),
            (
                ServerEvent::MetadataChanged {
                    provider_id: "romm".to_owned(),
                    record_count: 2,
                },
                "metadata_changed",
                ServerEventKind::MetadataChanged,
            ),
            (
                ServerEvent::ApplicationSessionChanged { session: None },
                "application_session_changed",
                ServerEventKind::ApplicationSessionChanged,
            ),
            (
                ServerEvent::CategorizationChanged {
                    categorizer: "heuristic".to_owned(),
                    app_count: 2,
                    categories: 1,
                },
                "categorization_changed",
                ServerEventKind::CategorizationChanged,
            ),
        ];
        for (event, name, kind) in cases {
            let json = serde_json::to_value(event).unwrap();
            assert_eq!(json["type"], name);
            assert_eq!(
                serde_json::from_value::<ServerEventKind>(json.clone()).unwrap(),
                kind,
                "event: {name}"
            );
            let decoded: ServerEvent = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(
                serde_json::to_value(decoded).unwrap(),
                json,
                "event: {name}"
            );
        }
        assert_eq!(
            serde_json::from_str::<ServerEventKind>(r#"{"type":"future_event"}"#).unwrap(),
            ServerEventKind::Unknown
        );
    }

    #[test]
    fn collection_plays_keep_the_activity_endpoints_flattened_shape() {
        let json = serde_json::json!({
            "item_id": "desktop:game",
            "name": "Game",
            "played": {
                "id": "desktop:game",
                "title": "Game",
                "icon": null,
                "categories": ["Game"],
                "source": "desktop",
                "metadata": {},
                "last_launched_at": "2026-10-29T12:00:00Z",
                "launch_count": 2
            }
        });
        let item: CollectionItem = serde_json::from_value(json).unwrap();
        let played = item.played.as_ref().unwrap();
        assert_eq!(played.entry.id, "desktop:game");
        assert_eq!(played.launch_count, 2);
        assert!(played.entry.kind.is_none());
        let encoded = serde_json::to_value(item).unwrap();
        assert_eq!(encoded["played"]["id"], "desktop:game");
        assert!(encoded["played"].get("entry").is_none());
        assert!(encoded.get("icon").is_none());
    }

    #[test]
    fn retro_details_preserve_identity_and_default_optional_lists_and_flags() {
        let detail: RetroGameDetails = serde_json::from_str(
            r#"{"id":42,"platform_id":7,"title":"Game","average_rating":83.5}"#,
        )
        .unwrap();
        assert_eq!(detail.id, 42);
        assert_eq!(detail.platform_id, 7);
        assert_eq!(detail.title, "Game");
        assert_eq!(detail.average_rating, Some(83.5));
        assert!(detail.sibling_roms.is_empty());
        assert!(detail.screenshot_paths.is_empty());
        assert!(!detail.favorite);
        assert!(!detail.backlogged);
    }

    #[test]
    fn shared_api_payloads_keep_their_wire_names() {
        // These are read by the client by field name, so a rename here is a silent
        // break across the process boundary. Pin the names.
        let capabilities = HostCapabilities {
            launch: true,
            application_sessions: false,
            retro_launch: true,
            categorization: true,
        };
        let value = serde_json::to_value(capabilities).unwrap();
        for field in [
            "launch",
            "application_sessions",
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

        // The origin a scanner resolved is read back by name, and its variants
        // are what a filter compares against, so both spellings are pinned.
        let application = DiscoveredApplication {
            application_id: "org.example.App.desktop".to_owned(),
            name: "Example".to_owned(),
            comment: None,
            icon: None,
            categories: Vec::new(),
            launch_scheme: None,
            source: ApplicationSource::Flatpak,
        };
        let value = serde_json::to_value(application).unwrap();
        assert_eq!(value["source"], "flatpak");
        for source in [
            (ApplicationSource::System, "system"),
            (ApplicationSource::Local, "local"),
            (ApplicationSource::Snap, "snap"),
            (ApplicationSource::Nix, "nix"),
        ] {
            assert_eq!(
                serde_json::to_value(source.0).unwrap(),
                serde_json::Value::String(source.1.to_owned()),
                "{:?} lost its wire name",
                source.0
            );
        }
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

    #[test]
    fn categorization_snapshot_keeps_its_wire_shape() {
        let snapshot = CategorizationSnapshot::<()> {
            enabled: true,
            status: CategorizationStatus {
                running: true,
                phase: CategorizationPhase::Downloading,
                completed: 0,
                total: 3,
                model: ModelStatus::Downloaded {
                    bytes: 7,
                    path: "/cache/model".into(),
                },
                last_error: None,
                last_completed_at: None,
            },
            report: None,
        };

        let json = serde_json::to_value(&snapshot).unwrap();

        assert_eq!(json["status"]["phase"], "downloading");
        assert_eq!(json["status"]["model"]["state"], "downloaded");
        assert!(json["status"].get("last_error").is_none());
        let back: CategorizationSnapshot<()> = serde_json::from_value(json).unwrap();
        assert_eq!(back.status.model, snapshot.status.model);
    }

    #[test]
    fn input_mode_commands_round_trip() {
        for profile in [InputProfile::Native, InputProfile::Desktop] {
            for retro_osk in [false, true] {
                let mode = InputMode { profile, retro_osk };
                assert_eq!(InputMode::parse(mode.command()), Some(mode));
            }
        }
        assert_eq!(
            InputMode::parse(b"retro-desktop").map(|m| m.profile),
            Some(InputProfile::Desktop)
        );
        assert_eq!(InputMode::parse(b"bogus"), None);
    }
}
