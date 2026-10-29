use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use hearthdeck_protocol::{BridgeRequest, BridgeResponse};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

// Replacing an active session can include systemd's default 90-second stop grace.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

pub async fn request(socket_path: &Path, request: BridgeRequest) -> Result<BridgeResponse> {
    tokio::time::timeout(REQUEST_TIMEOUT, async {
        let stream = UnixStream::connect(socket_path)
            .await
            .context("bridge unavailable")?;
        let (reader, mut writer) = stream.into_split();
        let payload = serde_json::to_vec(&request)?;
        writer.write_all(&payload).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;

        let mut reader = BufReader::new(reader.take(u64::try_from(MAX_RESPONSE_BYTES)? + 1));
        let mut line = Vec::new();
        reader.read_until(b'\n', &mut line).await?;
        ensure!(
            line.len() <= MAX_RESPONSE_BYTES,
            "bridge response exceeds {MAX_RESPONSE_BYTES} bytes"
        );
        if line.is_empty() {
            bail!("bridge closed connection without a response")
        }
        ensure!(line.last() == Some(&b'\n'), "incomplete bridge response");
        serde_json::from_slice(&line).context("invalid bridge response")
    })
    .await
    .with_context(|| {
        format!(
            "bridge request timed out after {} seconds",
            REQUEST_TIMEOUT.as_secs()
        )
    })?
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::net::UnixListener;

    use super::*;

    const RESPONSE_LIMIT: usize = 16 * 1024 * 1024;

    async fn respond(response: Vec<u8>) -> Result<BridgeResponse> {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bridge.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = async {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .unwrap();
            assert!(matches!(
                serde_json::from_str(&line).unwrap(),
                BridgeRequest::Health
            ));
            stream.write_all(&response).await.unwrap();
        };
        let (response, ()) = tokio::join!(request(&path, BridgeRequest::Health), server);
        response
    }

    fn health_response(length: usize) -> Vec<u8> {
        let mut response = b"{\"type\":\"health\",\"version\":\"".to_vec();
        response.resize(length - b"\"}\n".len(), b'x');
        response.extend_from_slice(b"\"}\n");
        assert_eq!(response.len(), length);
        response
    }

    #[tokio::test]
    async fn accepts_a_response_at_the_size_limit() {
        let response = respond(health_response(RESPONSE_LIMIT)).await.unwrap();
        let BridgeResponse::Health { version } = response else {
            panic!("expected a health response");
        };
        assert_eq!(
            version.len(),
            RESPONSE_LIMIT - b"{\"type\":\"health\",\"version\":\"\"}\n".len()
        );
    }

    #[tokio::test]
    async fn rejects_a_response_above_the_size_limit() {
        let Err(error) = respond(health_response(RESPONSE_LIMIT + 1)).await else {
            panic!("accepted an oversized bridge response");
        };
        assert!(error.to_string().contains("exceeds"), "{error:#}");
    }

    #[tokio::test]
    async fn rejects_an_unterminated_response() {
        let error = respond(b"{\"type\":\"health\",\"version\":\"1\"}".to_vec())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("incomplete"), "{error:#}");
    }

    #[tokio::test]
    async fn rejects_an_empty_or_invalid_response() {
        for response in [Vec::new(), b"not-json\n".to_vec()] {
            assert!(respond(response.clone()).await.is_err(), "{response:?}");
        }
    }

    #[tokio::test]
    async fn finishes_reading_at_the_newline_or_size_limit_without_waiting_for_eof() {
        for response in [
            b"{\"type\":\"health\",\"version\":\"1\"}\n".to_vec(),
            vec![b'x'; RESPONSE_LIMIT + 1],
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("bridge.sock");
            let listener = UnixListener::bind(&path).unwrap();
            let client = tokio::spawn(async move { request(&path, BridgeRequest::Health).await });
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .unwrap();
            stream.write_all(&response).await.unwrap();
            let result = tokio::time::timeout(Duration::from_secs(5), client)
                .await
                .expect("must not wait for the bridge to close the connection")
                .unwrap();
            if response.len() > RESPONSE_LIMIT {
                let error = result.unwrap_err();
                assert!(error.to_string().contains("exceeds"), "{error:#}");
            } else {
                assert!(matches!(
                    result.unwrap(),
                    BridgeResponse::Health { version } if version == "1"
                ));
            }
        }
    }

    #[tokio::test]
    async fn a_stalled_bridge_does_not_wait_forever() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bridge.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let client = tokio::spawn(async move { request(&path, BridgeRequest::Health).await });
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(matches!(
            serde_json::from_str(&line).unwrap(),
            BridgeRequest::Health
        ));

        // Freeze after socket I/O so Tokio cannot auto-advance during the handshake.
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(120)).await;
        let error = tokio::time::timeout(Duration::from_secs(1), client)
            .await
            .expect("bridge call must finish when its deadline expires")
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error:#}");
    }
}
