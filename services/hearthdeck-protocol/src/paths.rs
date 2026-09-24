//! Host filesystem layout shared by the local Hearthdeck processes.
//!
//! The daemon, bridge and overlay are separate processes, and each has to agree on
//! where the bridge listens and where the RomM deployment lives. They used to
//! derive those independently and drifted — the overlay looked under `/tmp` where
//! the bridge had bound inside the session's runtime directory, so its "close app"
//! request went nowhere. The derivation lives here once, as the contract between
//! the processes rather than a private detail of any one of them.
//!
//! This is path *derivation*, not path *validation*: the bridge still re-checks a
//! rom it is asked to launch against the root it derives, so the two cannot widen
//! what is launchable by agreeing too well.

use std::path::{Path, PathBuf};

/// Overrides the bridge socket path. Honoured by every process, so a deployment
/// can relocate the socket in one place.
pub const BRIDGE_SOCKET_ENV: &str = "HEARTHDECK_BRIDGE_SOCKET";

/// Where the bridge listens for host requests.
///
/// [`BRIDGE_SOCKET_ENV`] wins when set; otherwise the socket lives under the
/// session's runtime directory. Every process must resolve this the same way, so
/// the fallback is here rather than guessed at per caller.
pub fn bridge_socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os(BRIDGE_SOCKET_ENV).filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    runtime_dir().join("hearthdeck").join("bridge.sock")
}

/// The session runtime directory: `XDG_RUNTIME_DIR`, or the data-local `runtime/`
/// when the session has not set one.
fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    directories::ProjectDirs::from("dev", "hearthdeck", "hearthdeck")
        .map(|dirs| dirs.data_local_dir().join("runtime"))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// The default RomM data directory, used when no compose file is configured.
pub const DEFAULT_ROMM_DATA_ROOT: &str = "/mnt/external/romM";

/// The compose file the packaged `romm.service` uses by default.
pub const DEFAULT_ROMM_COMPOSE_FILE: &str = "/mnt/external/romM/podman-compose.yaml";

/// RomM's data directory: the compose file's parent, or [`DEFAULT_ROMM_DATA_ROOT`].
///
/// RomM's compose file sits at the root of its data directory, beside the
/// `library/` and `resources/` mounts, so those mounts follow from it. The daemon
/// reads roms and artwork through this root; the bridge re-derives it to re-check
/// a launch.
pub fn romm_data_root(compose_file: Option<&Path>) -> PathBuf {
    compose_file
        .and_then(Path::parent)
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ROMM_DATA_ROOT))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{DEFAULT_ROMM_DATA_ROOT, romm_data_root};

    #[test]
    fn romm_data_root_follows_the_compose_file() {
        assert_eq!(
            romm_data_root(Some(Path::new("/srv/romm/podman-compose.yaml"))),
            PathBuf::from("/srv/romm")
        );
        // No compose file, or one with no directory part, both fall back to the
        // packaged default root.
        assert_eq!(romm_data_root(None), PathBuf::from(DEFAULT_ROMM_DATA_ROOT));
        assert_eq!(
            romm_data_root(Some(Path::new("podman-compose.yaml"))),
            PathBuf::from(DEFAULT_ROMM_DATA_ROOT)
        );
    }
}
