use std::error::Error;
use std::io;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixDatagram as StdUnixDatagram, UnixStream as StdUnixStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use cosmic::Element;
use cosmic::app::{Core, Settings, Task};
use cosmic::cctk::sctk::shell::wlr_layer::Layer;
use cosmic::iced::futures::channel::mpsc::Sender;
use cosmic::iced::platform_specific::runtime::wayland::layer_surface::SctkLayerSurfaceSettings;
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    Anchor, KeyboardInteractivity, destroy_layer_surface,
};
use cosmic::iced::runtime::core::layout::Limits;
use cosmic::iced::runtime::platform_specific::wayland::CornerRadius;
use cosmic::iced::runtime::platform_specific::wayland::layer_surface::IcedMargin;
use cosmic::iced::stream;
use cosmic::iced::{Alignment, Color, Length, Subscription, window};
use cosmic::surface::action::{LiveSettings, app_layer_shell};
use cosmic::widget::{button, column, container, text};
use futures_util::SinkExt;
use hearthdeck_protocol::{BridgeRequest, BridgeResponse};
use tokio::net::UnixDatagram;
use tracing::{error, info, warn};

use crate::input;
use crate::shortcut;

const APP_ID: &str = "io.github.viniciuscr.hearthdeck.Overlay";
const TOGGLE_MESSAGE: &[u8] = b"toggle";

pub fn main() -> Result<(), Box<dyn Error>> {
    match std::env::args().nth(1).as_deref() {
        None => run().map_err(Into::into),
        Some("--toggle") => toggle().map_err(Into::into),
        Some("--install-shortcut") => shortcut::install(),
        Some(argument) => Err(format!("unknown argument: {argument}").into()),
    }
}

fn run() -> cosmic::iced::Result {
    init_logging();
    info!("hearthdeck-overlay ({APP_ID})");

    cosmic::app::run::<Overlay>(
        Settings::default()
            .no_main_window(true)
            .exit_on_close(false),
        (),
    )
}

fn toggle() -> io::Result<()> {
    match send_toggle() {
        Ok(()) => Ok(()),
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            let status = Command::new("systemctl")
                .args(["--user", "start", "hearthdeck-overlay.service"])
                .status()?;
            if !status.success() {
                return Err(io::Error::other(
                    "failed to start hearthdeck-overlay.service",
                ));
            }

            for _ in 0..20 {
                thread::sleep(Duration::from_millis(100));
                if send_toggle().is_ok() {
                    return Ok(());
                }
            }
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "hearthdeck-overlay control socket did not become available",
            ))
        }
        Err(err) => Err(err),
    }
}

fn send_toggle() -> io::Result<()> {
    StdUnixDatagram::unbound()?.send_to(TOGGLE_MESSAGE, socket_path())?;
    Ok(())
}

fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("hearthdeck-overlay.sock")
}

fn init_logging() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("warn,{}=debug", env!("CARGO_CRATE_NAME"))));

    tracing_subscriber::registry()
        .with(fmt::layer().with_target(false))
        .with(filter)
        .init();
}

#[derive(Debug, Clone)]
enum Message {
    ToggleOverlay,
    ResumeApp,
    CloseApp,
}

struct Overlay {
    core: Core,
    window_id: window::Id,
    visible: bool,
    // Guards against a single "Close App" press somehow resulting in more
    // than one in-flight stop_active_application() call - each call asks the
    // bridge "what's active *right now*?", so a second call starting before
    // the first finishes could end up closing a completely different app
    // once the first one's stop has already removed it from the bridge's
    // tracking. Shared with the spawned closing thread via Arc since Overlay
    // itself only lives on the iced event loop's thread.
    closing: Arc<AtomicBool>,
}

impl cosmic::Application for Overlay {
    type Executor = cosmic::executor::single::Executor;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: ()) -> (Self, Task<Message>) {
        let app = Overlay {
            core,
            window_id: window::Id::unique(),
            visible: false,
            closing: Arc::new(AtomicBool::new(false)),
        };
        (app, Task::none())
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            input::subscription().map(|input::GuideButtonPressed| Message::ToggleOverlay),
            toggle_subscription().map(|()| Message::ToggleOverlay),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ToggleOverlay => {
                if self.visible {
                    self.hide()
                } else {
                    self.show()
                }
            }
            Message::ResumeApp => self.hide(),
            Message::CloseApp => {
                let hide = self.hide();
                if self.closing.swap(true, Ordering::SeqCst) {
                    // A close is already running; ignore this extra press
                    // instead of asking the bridge "what's active *now*?"
                    // and closing whatever that happens to be next.
                    return hide;
                }
                let closing = Arc::clone(&self.closing);
                thread::spawn(move || {
                    if let Err(err) = stop_active_application() {
                        error!(
                            ?err,
                            "hearthdeck-overlay: failed to stop active application"
                        );
                    }
                    closing.store(false, Ordering::SeqCst);
                });
                hide
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        unreachable!("no main window - Settings::no_main_window(true)")
    }

    fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        if id != self.window_id {
            return text("").into();
        }

        let menu = column(vec![
            text::title2("Hearthdeck").into(),
            button::suggested("Resume")
                .on_press(Message::ResumeApp)
                .into(),
            button::destructive("Close App")
                .on_press(Message::CloseApp)
                .into(),
        ])
        .spacing(12)
        .align_x(Alignment::Center);

        container(menu)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(|_theme| container::Style {
                background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.6).into()),
                ..Default::default()
            })
            .into()
    }
}

fn toggle_subscription() -> Subscription<()> {
    Subscription::run(|| {
        stream::channel(8, |mut output: Sender<()>| async move {
            let path = socket_path();
            if let Err(err) = std::fs::remove_file(&path)
                && err.kind() != io::ErrorKind::NotFound
            {
                error!(?err, path = ?path, "hearthdeck-overlay: failed to remove stale socket");
                return;
            }

            let socket = match UnixDatagram::bind(&path) {
                Ok(socket) => socket,
                Err(err) => {
                    error!(?err, path = ?path, "hearthdeck-overlay: failed to bind control socket");
                    return;
                }
            };
            let mut buffer = [0; 16];

            loop {
                match socket.recv(&mut buffer).await {
                    Ok(length) if &buffer[..length] == TOGGLE_MESSAGE => {
                        if output.send(()).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) => warn!("hearthdeck-overlay: ignored invalid control message"),
                    Err(err) => {
                        error!(?err, "hearthdeck-overlay: control socket read failed");
                        break;
                    }
                }
            }

            let _ = std::fs::remove_file(path);
        })
    })
}

/// Path to the bridge's Unix control socket. Mirrors
/// `hearthdeck-bridge::bridge_socket_path()`'s primary branch; skips its
/// `ProjectDirs` fallback (needs the `directories` crate) since that only
/// matters when `XDG_RUNTIME_DIR` is unset, which doesn't happen in a real
/// systemd user session. Keep in sync if the bridge's default ever changes.
fn bridge_socket_path() -> PathBuf {
    std::env::var_os("HEARTHDECK_BRIDGE_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("hearthdeck/bridge.sock")
        })
}

/// Sends one JSON-lines request to the bridge and reads its response.
/// Mirrors `hearthdeck-daemon::bridge::request`, using a blocking
/// `UnixStream` instead of tokio's since this runs on a plain spawned
/// thread, not inside an async runtime.
fn bridge_request(request: &BridgeRequest) -> io::Result<BridgeResponse> {
    let mut stream = StdUnixStream::connect(bridge_socket_path())?;
    let payload = serde_json::to_string(request).map_err(io::Error::other)?;
    stream.write_all(payload.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    if line.is_empty() {
        return Err(io::Error::other(
            "bridge closed connection without a response",
        ));
    }
    serde_json::from_str(&line).map_err(io::Error::other)
}

/// Asks the bridge which application session it is currently tracking as
/// active, then asks it to stop that session.
///
/// ponytail: this reuses the daemon's own session-stop path (see
/// hearthdeck-bridge/src/platform.rs::stop_application) instead of
/// reinventing app-liveness/focus detection here. The bridge already tracks
/// exactly one thing as "the active session" - the most recent launch that's
/// still running, no window-matching heuristics involved - and stops it via
/// `systemctl --user stop`, which is cgroup-based and tears down the whole
/// process tree regardless of whether the app implements any Wayland close
/// protocol, is nested in Gamescope, or ignores synthetic key presses.
/// Earlier attempts here (COSMIC toplevel-manager close, then synthetic
/// Alt+F4) both depended on details Hearthdeck doesn't control; this doesn't.
fn stop_active_application() -> io::Result<()> {
    let BridgeResponse::ApplicationSession { session } =
        bridge_request(&BridgeRequest::ActiveApplicationSession)?
    else {
        return Err(io::Error::other("bridge rejected active-session lookup"));
    };
    let Some(session) = session else {
        warn!("hearthdeck-overlay: no application session is active");
        return Ok(());
    };

    let response = bridge_request(&BridgeRequest::StopApplicationSession {
        session_id: session.id.clone(),
    })?;
    if !matches!(response, BridgeResponse::StopAccepted { .. }) {
        return Err(io::Error::other("bridge rejected application-session stop"));
    }
    info!(
        session_id = %session.id,
        application_id = %session.application_id,
        "hearthdeck-overlay: stopped active application session"
    );
    Ok(())
}

impl Overlay {
    fn show(&mut self) -> Task<Message> {
        self.visible = true;
        cosmic::surface::surface_task(app_layer_shell(
            |_app: &Overlay| LiveSettings {
                padding: Some(IcedMargin::default()),
                corners: Some(CornerRadius::default()),
                blur: Some(false),
            },
            move |app: &mut Overlay| SctkLayerSurfaceSettings {
                id: app.window_id,
                layer: Layer::Overlay,
                keyboard_interactivity: KeyboardInteractivity::Exclusive,
                anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                namespace: "hearthdeck-overlay".into(),
                size: None,
                size_limits: Limits::NONE,
                exclusive_zone: -1,
                ..Default::default()
            },
            None,
        ))
    }

    fn hide(&mut self) -> Task<Message> {
        self.visible = false;
        destroy_layer_surface(self.window_id)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    use super::stop_active_application;

    #[test]
    fn stops_the_session_the_bridge_reports_as_active() {
        let socket_path = std::env::temp_dir().join(format!(
            "hearthdeck-overlay-test-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).expect("bind test socket");
        // SAFETY: no other test in this crate touches this env var.
        unsafe {
            std::env::set_var("HEARTHDECK_BRIDGE_SOCKET", &socket_path);
        }

        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (stream, _) = listener.accept().expect("accept test connection");
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                reader.read_line(&mut line).expect("read test request");

                let response = if line.contains("active_application_session") {
                    r#"{"type":"application_session","session":{"id":"abc","source_id":"desktop-apps","application_id":"foo","state":"running"}}"#
                } else {
                    assert!(
                        line.contains(r#""session_id":"abc""#),
                        "expected a stop request for the reported session, got: {line}"
                    );
                    r#"{"type":"stop_accepted","session_id":"abc"}"#
                };
                let mut stream = &stream;
                stream
                    .write_all(response.as_bytes())
                    .expect("write test response");
                stream.write_all(b"\n").expect("write test response");
            }
        });

        stop_active_application().expect("stop_active_application should succeed");
        server.join().expect("test bridge thread should not panic");
        let _ = std::fs::remove_file(&socket_path);
    }
}
