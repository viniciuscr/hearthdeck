use std::{env, path::PathBuf};

use anyhow::{Context, Result, bail};
use hearthdeck_protocol::{BridgeRequest, BridgeResponse};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

#[derive(Clone, Debug)]
pub struct ManagedSession {
    pub application_id: String,
    pub id: String,
}

pub async fn active_session() -> Result<Option<ManagedSession>, String> {
    let response = request(BridgeRequest::ActiveApplicationSession)
        .await
        .map_err(|error| error.to_string())?;
    match response {
        BridgeResponse::ApplicationSession { session } => {
            Ok(session.map(|session| ManagedSession {
                application_id: session.application_id,
                id: session.id,
            }))
        }
        BridgeResponse::Error { message, .. } => Err(message),
        _ => Err("bridge returned an unexpected active-session response".to_owned()),
    }
}

pub async fn stop_active_session() -> Result<(), String> {
    let Some(session) = active_session().await? else {
        return Ok(());
    };
    let response = request(BridgeRequest::StopApplicationSession {
        session_id: session.id,
    })
    .await
    .map_err(|error| error.to_string())?;
    match response {
        BridgeResponse::StopAccepted { .. } => Ok(()),
        BridgeResponse::Error { message, .. } => Err(message),
        _ => Err("bridge returned an unexpected stop-session response".to_owned()),
    }
}

async fn request(request: BridgeRequest) -> Result<BridgeResponse> {
    let socket_path = bridge_socket_path()?;
    let stream = UnixStream::connect(&socket_path)
        .await
        .with_context(|| format!("bridge unavailable at {}", socket_path.display()))?;
    let (reader, mut writer) = stream.into_split();
    writer
        .write_all(serde_json::to_string(&request)?.as_bytes())
        .await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    if line.is_empty() {
        bail!("bridge closed connection without a response")
    }
    Ok(serde_json::from_str(&line)?)
}

fn bridge_socket_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("HEARTHDECK_BRIDGE_SOCKET") {
        return Ok(PathBuf::from(path));
    }
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .context("XDG_RUNTIME_DIR is required for the Hearthdeck overlay")?;
    Ok(PathBuf::from(runtime).join("hearthdeck/bridge.sock"))
}
