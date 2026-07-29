mod platform;

use std::{
    env,
    os::unix::{fs::PermissionsExt, io::FromRawFd, net::UnixListener as StdUnixListener},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use hearthdeck_protocol::{BridgeErrorCode, BridgeRequest, BridgeResponse};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
};
use tracing::{Instrument, error, info, info_span, warn};

#[tokio::main]
async fn main() -> Result<()> {
    hearthdeck_observability::init("hearthdeck-bridge", "hearthdeck_bridge=info");
    let socket_path = bridge_socket_path()?;
    let (listener, socket_activated) = bridge_listener(&socket_path).await?;
    info!(socket = %socket_path.display(), socket_activated, "bridge listening");
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream)
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

async fn handle_connection(stream: UnixStream) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let response = match serde_json::from_str::<BridgeRequest>(&line) {
        Ok(request) => handle_request(request).await,
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

async fn handle_request(request: BridgeRequest) -> BridgeResponse {
    match request {
        BridgeRequest::Health => BridgeResponse::Health {
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        BridgeRequest::DiscoverApplications { source_id } => {
            match platform::discover_applications(&source_id).await {
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
        } => match platform::launch_application(&source_id, &application_id).await {
            Ok(()) => {
                info!(
                    source_id,
                    application_id, "registered application launch accepted"
                );
                BridgeResponse::LaunchAccepted {
                    source_id,
                    application_id,
                }
            }
            Err(error) => {
                warn!(source_id, application_id, %error, "registered application launch rejected");
                BridgeResponse::Error {
                    code: BridgeErrorCode::LaunchFailed,
                    message: error.to_string(),
                }
            }
        },
    }
}

fn bridge_socket_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("HEARTHDECK_BRIDGE_SOCKET") {
        return Ok(PathBuf::from(path));
    }
    let project_dirs = ProjectDirs::from("dev", "hearthdeck", "hearthdeck")
        .context("could not determine Hearthdeck data directories")?;
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| project_dirs.data_local_dir().join("runtime"));
    Ok(runtime_dir.join("hearthdeck/bridge.sock"))
}
