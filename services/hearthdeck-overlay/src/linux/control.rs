use std::{env, os::unix::fs::PermissionsExt, path::PathBuf};

use anyhow::{Context, Result, bail};
use cosmic::iced::{Subscription, futures::SinkExt, stream};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
};
use tracing::warn;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlayCommand {
    Toggle,
    Show,
    Hide,
}

pub fn command_from_args() -> Result<Option<OverlayCommand>> {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    let Some(argument) = arguments.next() else {
        return Ok(None);
    };
    if arguments.next().is_some() {
        bail!("expected one overlay command: toggle, show, or hide")
    }
    parse(argument.to_string_lossy().as_ref()).map(Some)
}

pub fn send(command: OverlayCommand) -> Result<()> {
    let path = socket_path()?;
    let mut stream = std::os::unix::net::UnixStream::connect(&path)
        .with_context(|| format!("overlay service unavailable at {}", path.display()))?;
    use std::io::{BufRead, Write};
    stream.write_all(command.as_str().as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut response = String::new();
    let mut reader = std::io::BufReader::new(stream);
    reader.read_line(&mut response)?;
    if response.trim() != "ok" {
        bail!("overlay rejected command: {}", response.trim())
    }
    Ok(())
}

pub fn subscription() -> Subscription<OverlayCommand> {
    Subscription::run(messages)
}

fn messages() -> impl cosmic::iced::futures::Stream<Item = OverlayCommand> {
    stream::channel(8, |mut output| async move {
        let listener = match bind_listener().await {
            Ok(listener) => listener,
            Err(error) => {
                warn!(%error, "could not create overlay control socket");
                return;
            }
        };
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    warn!(%error, "overlay control socket accept failed");
                    continue;
                }
            };
            let command = match read_command(stream).await {
                Ok(command) => command,
                Err(error) => {
                    warn!(%error, "overlay control command rejected");
                    continue;
                }
            };
            if output.send(command).await.is_err() {
                return;
            }
        }
    })
}

async fn bind_listener() -> Result<UnixListener> {
    let path = socket_path()?;
    let parent = path
        .parent()
        .context("overlay control socket has no parent")?;
    tokio::fs::create_dir_all(parent).await?;
    if path.exists() {
        tokio::fs::remove_file(&path).await?;
    }
    let listener = UnixListener::bind(&path)?;
    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await?;
    Ok(listener)
}

async fn read_command(stream: UnixStream) -> Result<OverlayCommand> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let command = parse(line.trim())?;
    writer.write_all(b"ok\n").await?;
    writer.flush().await?;
    Ok(command)
}

fn parse(value: &str) -> Result<OverlayCommand> {
    match value {
        "toggle" => Ok(OverlayCommand::Toggle),
        "show" => Ok(OverlayCommand::Show),
        "hide" => Ok(OverlayCommand::Hide),
        _ => bail!("expected overlay command: toggle, show, or hide"),
    }
}

impl OverlayCommand {
    fn as_str(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::Show => "show",
            Self::Hide => "hide",
        }
    }
}

fn socket_path() -> Result<PathBuf> {
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .context("XDG_RUNTIME_DIR is required for the Hearthdeck overlay")?;
    Ok(PathBuf::from(runtime).join("hearthdeck/overlay.sock"))
}

#[cfg(test)]
mod tests {
    use super::{OverlayCommand, parse};

    #[test]
    fn parses_only_known_control_commands() {
        assert_eq!(parse("toggle").unwrap(), OverlayCommand::Toggle);
        assert_eq!(parse("show").unwrap(), OverlayCommand::Show);
        assert_eq!(parse("hide").unwrap(), OverlayCommand::Hide);
        assert!(parse("stop").is_err());
    }
}
