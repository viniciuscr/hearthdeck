//! Input-compatibility profiles, applied through the `hearthdeck-input`
//! service's control socket.
//!
//! A launch sets the profile its target needs (a desktop app, or RetroArch
//! with its on-screen keyboard) and every stop resets it to `native`.

use anyhow::{Context, Result};
use hearthdeck_protocol::{InputMode, InputProfile, paths};
use tokio::net::UnixDatagram;
use tokio::process::Command;
use tracing::warn;

const INPUT_SERVICE: &str = "hearthdeck-input.service";

pub(crate) async fn set_input_profile(profile: InputProfile, retro_osk: bool) -> Result<()> {
    set_session_input_profile(profile, retro_osk, None).await
}

pub(crate) async fn set_session_input_profile(
    profile: InputProfile,
    retro_osk: bool,
    application_id: Option<&str>,
) -> Result<()> {
    let Some(socket_path) = paths::input_socket_path() else {
        anyhow::ensure!(
            profile == InputProfile::Native && !retro_osk,
            "XDG_RUNTIME_DIR is not set"
        );
        return Ok(());
    };
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

    let mode = InputMode { profile, retro_osk };
    let app_ids = if retro_osk {
        [
            "retroarch",
            "org.libretro.RetroArch",
            "com.libretro.RetroArch",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    } else {
        application_id
            .into_iter()
            .map(|id| id.strip_suffix(".desktop").unwrap_or(id).to_owned())
            .collect()
    };
    let command =
        serde_json::to_vec(&hearthdeck_protocol::input::SessionContext { mode, app_ids })?;
    let socket = UnixDatagram::unbound()?;
    match socket.send_to(&command, &socket_path).await {
        Ok(_) => Ok(()),
        Err(error) if profile == InputProfile::Native => {
            warn!(%error, "could not reset input compatibility profile");
            Ok(())
        }
        Err(error) => Err(error).context("could not enable input compatibility profile"),
    }
}
