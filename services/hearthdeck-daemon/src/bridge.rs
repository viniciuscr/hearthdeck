use std::path::Path;

use anyhow::{Context, Result, bail};
use hearthdeck_protocol::{BridgeRequest, BridgeResponse};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

pub async fn request(socket_path: &Path, request: BridgeRequest) -> Result<BridgeResponse> {
    let stream = UnixStream::connect(socket_path)
        .await
        .context("bridge unavailable")?;
    let (reader, mut writer) = stream.into_split();
    let payload = serde_json::to_string(&request)?;
    writer.write_all(payload.as_bytes()).await?;
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
