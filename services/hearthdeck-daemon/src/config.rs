use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_address: SocketAddr,
    pub local_admin_address: SocketAddr,
    pub database_path: PathBuf,
    pub bridge_socket_path: PathBuf,
    pub romm: Option<RommConfig>,
    pub lan_enabled: bool,
    pub tls: Option<TlsConfig>,
}

#[derive(Clone, Debug)]
pub struct RommConfig {
    pub base_url: String,
    pub token: String,
}

#[derive(Clone, Debug)]
pub struct TlsConfig {
    pub certificate_path: PathBuf,
    pub private_key_path: PathBuf,
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
        let romm_url = env::var("HEARTHDECK_ROMM_URL")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let romm_token = env::var("HEARTHDECK_ROMM_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let romm = match (romm_url, romm_token) {
            (Some(base_url), Some(token)) => Some(RommConfig {
                base_url: base_url.trim_end_matches('/').to_owned(),
                token,
            }),
            (None, None) => None,
            _ => bail!("HEARTHDECK_ROMM_URL and HEARTHDECK_ROMM_TOKEN must be set together"),
        };

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
            romm,
            lan_enabled,
            tls,
        })
    }
}
