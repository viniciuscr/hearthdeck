//! Input-compatibility profiles, applied through the `hearthdeck-input`
//! service's control socket.
//!
//! A launch sets the profile its target needs (a desktop app, or RetroArch
//! with its on-screen keyboard) and every stop resets it to `native`.

use std::{env, path::PathBuf};

use anyhow::{Context, Result};
use hearthdeck_protocol::InputProfile;
use tokio::net::UnixDatagram;
use tokio::process::Command;
use tracing::warn;

const INPUT_SERVICE: &str = "hearthdeck-input.service";
const INPUT_SOCKET: &str = "hearthdeck-input.sock";

pub(crate) async fn set_input_profile(profile: InputProfile, retro_osk: bool) -> Result<()> {
    let Some(runtime_directory) = env::var_os("XDG_RUNTIME_DIR") else {
        anyhow::ensure!(
            profile == InputProfile::Native && !retro_osk,
            "XDG_RUNTIME_DIR is not set"
        );
        return Ok(());
    };
    let socket_path = PathBuf::from(runtime_directory).join(INPUT_SOCKET);
    let input_service_required = profile == InputProfile::Desktop || retro_osk;

    if input_service_required && !socket_path.exists() {
        let status = Command::new("systemctl")
            .args(["--user", "start", INPUT_SERVICE])
            .status()
            .await
            .context("could not start input compatibility service")?;
        anyhow::ensure!(
            status.success(),
            "input compatibility service failed to start"
        );
        for _ in 0..20 {
            if socket_path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    if !socket_path.exists() && !input_service_required {
        return Ok(());
    }
    anyhow::ensure!(
        socket_path.exists(),
        "input compatibility service is unavailable"
    );

    let command = match (profile, retro_osk) {
        (InputProfile::Native, false) => b"native".as_slice(),
        (InputProfile::Desktop, false) => b"desktop".as_slice(),
        (InputProfile::Native, true) => b"retro-native".as_slice(),
        (InputProfile::Desktop, true) => b"retro-desktop".as_slice(),
    };
    let socket = UnixDatagram::unbound()?;
    match socket.send_to(command, &socket_path).await {
        Ok(_) => Ok(()),
        Err(error) if profile == InputProfile::Native => {
            warn!(%error, "could not reset input compatibility profile");
            Ok(())
        }
        Err(error) => Err(error).context("could not enable input compatibility profile"),
    }
}
