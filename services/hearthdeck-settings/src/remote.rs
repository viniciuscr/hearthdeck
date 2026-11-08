//! The daemon on another machine, over the ssh you already trust.
//!
//! This is `podman-remote`'s model, and it is the light one: the daemon never
//! stops binding loopback, and an `ssh -L` tunnel makes this window look local
//! from the other side. ssh is both the transport and the authentication — your
//! key, your user — so there is no certificate to trust, no LAN mode to enable,
//! and no new secret to keep.
//!
//! `BatchMode=yes` is the whole security posture in one flag: keys or an agent,
//! never a password prompt. A window has nobody to ask, and a prompt it cannot
//! show would hang the first read instead of failing.

use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The daemon's own ports, on the far side.
const REMOTE_API_PORT: u16 = 38400;
const REMOTE_ADMIN_PORT: u16 = 38401;
/// How long the forwards get to come up before ssh is declared useless.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// A live tunnel, and where it points.
pub struct Tunnel {
    child: Child,
    api: String,
    admin_port: u16,
}

impl Tunnel {
    /// Opens the tunnel and waits until both forwards accept, so the first read
    /// does not race the handshake.
    pub fn open(target: &str) -> Result<Self, String> {
        let api_port = free_port()?;
        let admin_port = free_port()?;
        let mut child = Command::new("ssh")
            .args(arguments(target, api_port, admin_port))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("ssh could not be started: {error}"))?;

        let deadline = Instant::now() + CONNECT_TIMEOUT;
        while Instant::now() < deadline {
            if accepts(api_port) && accepts(admin_port) {
                return Ok(Self {
                    child,
                    api: format!(
                        "http://{}",
                        SocketAddr::from((Ipv4Addr::LOCALHOST, api_port))
                    ),
                    admin_port,
                });
            }
            // ssh gave up already - a wrong host, a key it does not have, or a
            // RemoteForward the far side refused.
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!(
                    "ssh to {target} failed ({})",
                    match status.code() {
                        Some(code) => format!("exit {code}"),
                        None => "killed".to_string(),
                    }
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        let mut message = String::new();
        if let Some(stderr) = child.stderr.as_mut() {
            let _ = stderr.read_to_string(&mut message);
        }
        let _ = child.kill();
        Err(match message.trim() {
            "" => format!("ssh to {target} did not open the tunnel in time"),
            detail => format!("ssh to {target}: {detail}"),
        })
    }

    /// The tunneled API, as the daemon client wants it.
    pub fn api(&self) -> &str {
        &self.api
    }

    /// The tunneled admin listener, where pairing happens.
    pub fn admin_port(&self) -> u16 {
        self.admin_port
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // `ssh -N` holds the forwards until it is told to stop; nothing else
        // ends it, because it has no session to close.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The ssh command line: no session, two forwards, and no way to be asked for a
/// password.
fn arguments(target: &str, api_port: u16, admin_port: u16) -> Vec<String> {
    let forward = |port: u16, remote: u16| format!("127.0.0.1:{port}:127.0.0.1:{remote}");
    vec![
        "-N".to_string(),
        "-T".to_string(),
        "-o".to_string(),
        "ExitOnForwardFailure=yes".to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=10".to_string(),
        "-L".to_string(),
        forward(api_port, REMOTE_API_PORT),
        "-L".to_string(),
        forward(admin_port, REMOTE_ADMIN_PORT),
        target.to_string(),
    ]
}

/// A port nobody is using. It is released before ssh claims it, which is a race
/// only if something else takes it in the microseconds between.
fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| format!("no local port for the tunnel: {error}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("no local port for the tunnel: {error}"))
}

fn accepts(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        Duration::from_millis(200),
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::{REMOTE_ADMIN_PORT, REMOTE_API_PORT, arguments, free_port};

    #[test]
    fn the_command_line_forwards_both_ports_and_can_never_prompt() {
        let args = arguments("deck@tvbox", 41000, 41001);

        assert!(args.contains(&"-N".to_string()), "no session is opened");
        assert!(
            args.contains(&"BatchMode=yes".to_string()),
            "a key or an agent, never a password prompt"
        );
        assert!(
            args.contains(&"ExitOnForwardFailure=yes".to_string()),
            "a forward that cannot be made fails the tunnel"
        );
        assert!(args.contains(&format!("127.0.0.1:41000:127.0.0.1:{REMOTE_API_PORT}")));
        assert!(args.contains(&format!("127.0.0.1:41001:127.0.0.1:{REMOTE_ADMIN_PORT}")));
        assert_eq!(args.last().map(String::as_str), Some("deck@tvbox"));
    }

    #[test]
    fn two_tunnels_do_not_ask_for_the_same_port() {
        let first = free_port().expect("a port");
        let second = free_port().expect("a port");
        assert_ne!(first, second);
    }
}
