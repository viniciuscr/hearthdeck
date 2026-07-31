use std::{
    env,
    io::{BufRead, ErrorKind, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use tracing::warn;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlayCommand {
    Toggle,
    Show,
    Hide,
}

pub struct ControlListener {
    listener: UnixListener,
}

impl ControlListener {
    pub fn bind() -> Result<Self> {
        let path = socket_path()?;
        let parent = path
            .parent()
            .context("overlay control socket has no parent")?;
        std::fs::create_dir_all(parent)?;
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener })
    }

    pub fn drain(&self) -> Vec<OverlayCommand> {
        let mut commands = Vec::new();
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => match read_command(stream) {
                    Ok(command) => commands.push(command),
                    Err(error) => warn!(%error, "overlay control command rejected"),
                },
                Err(error) if error.kind() == ErrorKind::WouldBlock => return commands,
                Err(error) => {
                    warn!(%error, "overlay control socket accept failed");
                    return commands;
                }
            }
        }
    }
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
    let mut stream = UnixStream::connect(&path)
        .with_context(|| format!("overlay service unavailable at {}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    stream.write_all(command.as_str().as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut response = String::new();
    std::io::BufReader::new(stream).read_line(&mut response)?;
    if response.trim() != "ok" {
        bail!("overlay rejected command: {}", response.trim())
    }
    Ok(())
}

fn read_command(mut stream: UnixStream) -> Result<OverlayCommand> {
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    let mut line = String::new();
    std::io::BufReader::new(stream.try_clone()?).read_line(&mut line)?;
    if line.len() > 32 {
        bail!("overlay command exceeds 32 bytes")
    }
    let command = parse(line.trim())?;
    stream.write_all(b"ok\n")?;
    stream.flush()?;
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
