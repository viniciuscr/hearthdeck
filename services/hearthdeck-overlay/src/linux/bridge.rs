use std::{
    env,
    io::{BufRead, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use hearthdeck_protocol::{BridgeRequest, BridgeResponse};

#[derive(Clone, Debug)]
pub struct ManagedSession {
    pub application_id: String,
    pub id: String,
}

pub fn active_session() -> Result<Option<ManagedSession>> {
    match request(BridgeRequest::ActiveApplicationSession)? {
        BridgeResponse::ApplicationSession { session } => {
            Ok(session.map(|session| ManagedSession {
                application_id: session.application_id,
                id: session.id,
            }))
        }
        BridgeResponse::Error { message, .. } => bail!(message),
        _ => bail!("bridge returned an unexpected active-session response"),
    }
}

pub fn stop_active_session() -> Result<()> {
    let Some(session) = active_session()? else {
        return Ok(());
    };
    match request(BridgeRequest::StopApplicationSession {
        session_id: session.id,
    })? {
        BridgeResponse::StopAccepted { .. } => Ok(()),
        BridgeResponse::Error { message, .. } => bail!(message),
        _ => bail!("bridge returned an unexpected stop-session response"),
    }
}

fn request(request: BridgeRequest) -> Result<BridgeResponse> {
    let socket_path = bridge_socket_path()?;
    let mut stream = UnixStream::connect(&socket_path)
        .with_context(|| format!("bridge unavailable at {}", socket_path.display()))?;
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    stream.set_write_timeout(Some(Duration::from_millis(100)))?;
    stream.write_all(serde_json::to_string(&request)?.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut response = String::new();
    std::io::BufReader::new(stream).read_line(&mut response)?;
    if response.is_empty() {
        bail!("bridge closed connection without a response")
    }
    Ok(serde_json::from_str(&response)?)
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
