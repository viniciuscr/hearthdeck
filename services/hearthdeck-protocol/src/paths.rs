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

/// The `hearthdeck-input` broker's control socket. The broker only runs inside a
/// session, so there is no fallback: `None` when `XDG_RUNTIME_DIR` is unset.
pub fn input_socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(|dir| PathBuf::from(dir).join("hearthdeck-input.sock"))
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

/// Replaces the whole RetroArch tree root, which is how a box points saves at a
/// synced folder instead of relying on a symlink inside the provider's directory.
pub const RETRO_SAVE_ROOT_ENV: &str = "HEARTHDECK_RETRO_SAVE_ROOT";

/// Root of the RetroArch tree Hearthdeck owns: per-game `saves/` and `states/`,
/// the `system/` BIOS directory and the user's own `cores/`.
///
/// Under the XDG data home by default, not the config directory: these are user
/// data, and backing them up should not mean backing up settings. The bridge pins
/// RetroArch to this tree at launch and the daemon's readiness check verifies the
/// same files, so both resolve it here.
pub fn retro_save_root() -> PathBuf {
    retro_save_root_from(
        std::env::var_os(RETRO_SAVE_ROOT_ENV).map(PathBuf::from),
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// [`retro_save_root`] split from the environment so the override-then-XDG chain
/// is unit-testable without mutating process environment from a parallel test.
pub fn retro_save_root_from(
    configured: Option<PathBuf>,
    data_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PathBuf {
    if let Some(configured) = configured.filter(|path| !path.as_os_str().is_empty()) {
        return configured;
    }
    data_home
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| home.map(|path| path.join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("hearthdeck/retroarch")
}

/// RetroArch's `system_directory`: where BIOS files live.
pub fn retro_system_dir(save_root: &Path) -> PathBuf {
    save_root.join("system")
}

/// The user's own cores directory: cores no distro package provides. The bridge
/// allowlists it beside the distro cores directory.
pub fn retro_cores_dir(save_root: &Path) -> PathBuf {
    save_root.join("cores")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{DEFAULT_ROMM_DATA_ROOT, retro_save_root_from, romm_data_root};

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

    #[test]
    fn retro_save_root_lets_the_configured_root_win() {
        // A box pointing the tree straight at a cloud-sync folder: the override
        // is used as-is, with no `hearthdeck/retroarch` suffix.
        assert_eq!(
            retro_save_root_from(
                Some(PathBuf::from("/home/user/pCloudDrive/Hearthdeck")),
                Some(PathBuf::from("/home/user/.local/share")),
                Some(PathBuf::from("/home/user")),
            ),
            PathBuf::from("/home/user/pCloudDrive/Hearthdeck")
        );
        // Otherwise the XDG data home, not the config directory.
        assert_eq!(
            retro_save_root_from(
                None,
                Some(PathBuf::from("/data/home")),
                Some(PathBuf::from("/home/user")),
            ),
            PathBuf::from("/data/home/hearthdeck/retroarch")
        );
        // An empty variable counts as unset, and without XDG_DATA_HOME the
        // spec's `$HOME/.local/share` fallback applies.
        assert_eq!(
            retro_save_root_from(
                Some(PathBuf::new()),
                None,
                Some(PathBuf::from("/home/user")),
            ),
            PathBuf::from("/home/user/.local/share/hearthdeck/retroarch")
        );
    }
}
