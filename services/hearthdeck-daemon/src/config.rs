use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_address: SocketAddr,
    pub local_admin_address: SocketAddr,
    pub database_path: PathBuf,
    pub bridge_socket_path: PathBuf,
    pub lan_enabled: bool,
    pub tls: Option<TlsConfig>,
    /// Host roots of the RomM deployment, so the daemon reads roms and artwork
    /// off disk instead of downloading them through the API.
    pub romm: RommPaths,
}

#[derive(Clone, Debug)]
pub struct TlsConfig {
    pub certificate_path: PathBuf,
    pub private_key_path: PathBuf,
}

/// Host roots of the RomM deployment on this machine.
///
/// RomM reports every path relative to its own base paths (`LIBRARY_BASE_PATH`,
/// `RESOURCES_BASE_PATH`; both `/romm/...` inside its container), so those
/// paths are useless to a client on its own. All the daemon needs is the host
/// directory each mount was bound to. RomM's compose file sits at the root of
/// its data directory, beside the `library/` and `resources/` mounts, so that
/// directory is derived from `ROMM_COMPOSE_FILE` and the roots follow from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RommPaths {
    /// Host path of RomM's library (`LIBRARY_BASE_PATH`), holding the ROMs.
    pub library_root: PathBuf,
    /// Host path of RomM's resources (`RESOURCES_BASE_PATH`), holding covers.
    pub resources_root: PathBuf,
}

/// Where the packaged `romm.service` expects the deployment by default (its
/// unit hardcodes `Environment=ROMM_COMPOSE_FILE=...` at this path).
const DEFAULT_ROMM_DATA_ROOT: &str = "/mnt/external/romM";

impl RommPaths {
    /// Resolves the host roots from the deployment layout.
    ///
    /// `compose_file` is `ROMM_COMPOSE_FILE`; the two explicit roots are the
    /// escape hatch for a deployment that mounts them somewhere other than
    /// beside the compose file.
    pub fn resolve(
        compose_file: Option<PathBuf>,
        library_root: Option<PathBuf>,
        resources_root: Option<PathBuf>,
    ) -> Self {
        let data_root = compose_file
            .as_deref()
            .and_then(Path::parent)
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_ROMM_DATA_ROOT));
        Self {
            library_root: library_root.unwrap_or_else(|| data_root.join("library")),
            resources_root: resources_root.unwrap_or_else(|| data_root.join("resources")),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let project_dirs = ProjectDirs::from("dev", "hearthdeck", "hearthdeck")
            .context("could not determine Hearthdeck data directories")?;
        let data_dir = project_dirs.data_local_dir();
        let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("runtime"));

        let lan_enabled = env::var("HEARTHDECK_LAN_ENABLED")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let default_bind = if lan_enabled {
            "0.0.0.0:38400"
        } else {
            "127.0.0.1:38400"
        };
        let bind_address: SocketAddr = env::var("HEARTHDECK_BIND_ADDRESS")
            .unwrap_or_else(|_| default_bind.to_owned())
            .parse()
            .context("HEARTHDECK_BIND_ADDRESS must be a valid socket address")?;

        if !lan_enabled && !bind_address.ip().is_loopback() {
            bail!("non-loopback bind requires HEARTHDECK_LAN_ENABLED=true")
        }
        let local_admin_address: SocketAddr = env::var("HEARTHDECK_LOCAL_ADMIN_ADDRESS")
            .unwrap_or_else(|_| "127.0.0.1:38401".to_owned())
            .parse()
            .context("HEARTHDECK_LOCAL_ADMIN_ADDRESS must be a valid socket address")?;
        if !local_admin_address.ip().is_loopback() {
            bail!("HEARTHDECK_LOCAL_ADMIN_ADDRESS must bind to loopback")
        }
        let tls = if lan_enabled {
            let certificate_path = env::var_os("HEARTHDECK_TLS_CERT_PATH")
                .map(PathBuf::from)
                .context("HEARTHDECK_TLS_CERT_PATH is required when LAN access is enabled")?;
            let private_key_path = env::var_os("HEARTHDECK_TLS_KEY_PATH")
                .map(PathBuf::from)
                .context("HEARTHDECK_TLS_KEY_PATH is required when LAN access is enabled")?;
            Some(TlsConfig {
                certificate_path,
                private_key_path,
            })
        } else {
            None
        };
        // RomM reports paths relative to its own container base paths, so the
        // daemon resolves the host roots those mounts were bound to itself.
        let romm = RommPaths::resolve(
            env::var_os("ROMM_COMPOSE_FILE").map(PathBuf::from),
            env::var_os("HEARTHDECK_ROMM_LIBRARY_ROOT")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
            env::var_os("HEARTHDECK_ROMM_RESOURCES_ROOT")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
        );
        Ok(Self {
            bind_address,
            local_admin_address,
            database_path: env::var_os("HEARTHDECK_DATABASE_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|| data_dir.join("hearthdeck.db")),
            bridge_socket_path: env::var_os("HEARTHDECK_BRIDGE_SOCKET")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| runtime_dir.join("hearthdeck/bridge.sock")),
            lan_enabled,
            tls,
            romm,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_ROMM_DATA_ROOT, RommPaths};
    use std::path::PathBuf;

    #[test]
    fn romm_roots_derive_from_the_compose_file_directory() {
        let paths = RommPaths::resolve(
            Some(PathBuf::from("/mnt/external/romM/podman-compose.yaml")),
            None,
            None,
        );
        assert_eq!(
            paths.library_root,
            PathBuf::from("/mnt/external/romM/library")
        );
        assert_eq!(
            paths.resources_root,
            PathBuf::from("/mnt/external/romM/resources")
        );
    }

    #[test]
    fn romm_roots_fall_back_to_the_packaged_default() {
        let paths = RommPaths::resolve(None, None, None);
        assert_eq!(
            paths.library_root,
            PathBuf::from(DEFAULT_ROMM_DATA_ROOT).join("library")
        );
        assert_eq!(
            paths.resources_root,
            PathBuf::from(DEFAULT_ROMM_DATA_ROOT).join("resources")
        );
    }

    #[test]
    fn explicit_romm_roots_win_over_the_derived_ones() {
        let paths = RommPaths::resolve(
            Some(PathBuf::from("/srv/romm/podman-compose.yaml")),
            Some(PathBuf::from("/data/roms")),
            Some(PathBuf::from("/data/art")),
        );
        assert_eq!(paths.library_root, PathBuf::from("/data/roms"));
        assert_eq!(paths.resources_root, PathBuf::from("/data/art"));
    }

    #[test]
    fn a_compose_file_with_no_directory_still_resolves_roots() {
        let paths = RommPaths::resolve(Some(PathBuf::from("podman-compose.yaml")), None, None);
        assert_eq!(
            paths.library_root,
            PathBuf::from(DEFAULT_ROMM_DATA_ROOT).join("library")
        );
    }
}
