use std::error::Error;
use std::io;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixDatagram as StdUnixDatagram, UnixStream as StdUnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;
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
use cosmic::widget::{column, container, list, list_column, text};
use futures_util::SinkExt;
use hearthdeck_protocol::{BridgeRequest, BridgeResponse};
use tokio::net::UnixDatagram;
use tracing::{error, info, warn};

use crate::input;
use crate::shortcut;
use crate::state::{Effect, Event, OverlayState, Status};

const APP_ID: &str = "io.github.viniciuscr.hearthdeck.Overlay";
const TOGGLE_MESSAGE: &[u8] = b"toggle";
const INPUT_RELEASE_DELAY: Duration = Duration::from_millis(300);

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
    send_toggle_to(&socket_path())?;
    Ok(())
}

fn send_toggle_to(path: &Path) -> io::Result<usize> {
    StdUnixDatagram::unbound()?.send_to(TOGGLE_MESSAGE, path)
}

fn is_toggle_message(message: &[u8]) -> bool {
    message == TOGGLE_MESSAGE
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
    NavigateUp,
    NavigateDown,
    Activate,
    ResumeApp,
    CloseApp,
    /// Reported by the background task started for `CloseApp`, once
    /// `stop_active_application` returns. `Err` carries a display-formatted
    /// message (not the original `io::Error`) since `Message` must be
    /// `Clone` for iced/cosmic's event loop, and `io::Error` isn't.
    CloseFinished(Result<(), String>),
    HideAfterClose,
}

/// The menu's entries, in on-screen order - also the order gamepad D-pad
/// navigation cycles through. A plain array (not a widget-per-variant match)
/// so navigation logic (index math) and rendering (map to a list row) both
/// stay generic over "however many entries there are" instead of needing to
/// special-case each one.
const MENU_ITEMS: [(&str, Message); 2] = [
    ("Resume", Message::ResumeApp),
    ("Close App", Message::CloseApp),
];
const CLOSE_FAILED_ITEMS: [(&str, Message); 2] = [
    ("Retry Close", Message::CloseApp),
    ("Resume", Message::ResumeApp),
];

struct Overlay {
    core: Core,
    window_id: window::Id,
    state: OverlayState,
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
            state: OverlayState::default(),
        };
        (app, Task::none())
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            input::subscription(self.state.visible).map(|event| match event {
                input::GamepadEvent::ToggleOverlay => Message::ToggleOverlay,
                input::GamepadEvent::NavigateUp => Message::NavigateUp,
                input::GamepadEvent::NavigateDown => Message::NavigateDown,
                input::GamepadEvent::Activate => Message::Activate,
            }),
            toggle_subscription().map(|()| Message::ToggleOverlay),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ToggleOverlay => {
                let effect = self.state.update(Event::Toggle);
                self.apply_effect(effect)
            }
            Message::NavigateUp | Message::NavigateDown => {
                let event = if matches!(message, Message::NavigateUp) {
                    Event::NavigateUp
                } else {
                    Event::NavigateDown
                };
                let effect = self.state.update(event);
                self.apply_effect(effect)
            }
            Message::Activate => {
                let effect = self.state.update(Event::Activate);
                self.apply_effect(effect)
            }
            Message::ResumeApp => {
                let effect = self.state.update(Event::Resume);
                self.apply_effect(effect)
            }
            Message::CloseApp => {
                let effect = self.state.update(Event::Close);
                self.apply_effect(effect)
            }
            Message::CloseFinished(result) => match result {
                Ok(()) => {
                    let effect = self.state.update(Event::CloseSucceeded);
                    self.apply_effect(effect)
                }
                Err(err) => {
                    error!(%err, "hearthdeck-overlay: failed to stop active application");
                    let effect = self.state.update(Event::CloseFailed);
                    self.apply_effect(effect)
                }
            },
            Message::HideAfterClose => {
                let effect = self.state.update(Event::HideAfterClose);
                self.apply_effect(effect)
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

        let menu: Element<'_, Message> = match self.state.status {
            Status::Menu => {
                let mut items = list_column();
                for (index, (label, message)) in MENU_ITEMS.iter().enumerate() {
                    items = items.add(
                        list::button(text::body(*label))
                            .on_press(message.clone())
                            .selected(index == self.state.selected),
                    );
                }
                column(vec![text::title2("Hearthdeck").into(), items.into()])
            }
            // No menu while closing: the pending stop_active_application
            // call is already committed to whatever was active when
            // CloseApp was pressed, and navigating/activating here
            // wouldn't affect it either way.
            Status::Closing => column(vec![text::title2("Closing app\u{2026}").into()]),
            Status::CloseFailed => {
                let mut items = list_column();
                for (index, (label, message)) in CLOSE_FAILED_ITEMS.iter().enumerate() {
                    items = items.add(
                        list::button(text::body(*label))
                            .on_press(message.clone())
                            .selected(index == self.state.selected),
                    );
                }
                column(vec![
                    text::title2("Could not close app").into(),
                    text::body("The app is still running.").into(),
                    items.into(),
                ])
            }
        }
        .spacing(12)
        .align_x(Alignment::Center)
        .into();

        // A fixed-width card - matching the look of COSMIC's own launcher/
        // dialog popups (rounded corners, a subtle border and drop shadow,
        // via the theme's own `Dialog` container preset) - centered over a
        // full-screen dim scrim, rather than the menu floating edge-to-edge
        // with nothing but the scrim behind it.
        let card = container(menu)
            .width(Length::Fixed(420.0))
            .padding(20)
            .class(cosmic::theme::Container::Dialog(true));

        container(card)
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
                    Ok(length) if is_toggle_message(&buffer[..length]) => {
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
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no application session is active",
        ));
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
    fn apply_effect(&mut self, effect: Effect) -> Task<Message> {
        match effect {
            Effect::None => Task::none(),
            Effect::Show => self.show(),
            Effect::Hide => self.hide(),
            Effect::StopApplication => cosmic::task::future(async {
                Message::CloseFinished(
                    tokio::task::spawn_blocking(stop_active_application)
                        .await
                        .map_err(|err| err.to_string())
                        .and_then(|result| result.map_err(|err| err.to_string())),
                )
            }),
            Effect::DelayHide => cosmic::task::future(async {
                tokio::time::sleep(INPUT_RELEASE_DELAY).await;
                Message::HideAfterClose
            }),
        }
    }

    fn show(&mut self) -> Task<Message> {
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
        destroy_layer_surface(self.window_id)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixDatagram, UnixListener};

    use super::{TOGGLE_MESSAGE, is_toggle_message, send_toggle_to, stop_active_application};

    #[test]
    fn toggle_command_reaches_the_overlay_control_socket() {
        let socket_path = std::env::temp_dir().join(format!(
            "hearthdeck-overlay-control-test-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&socket_path);
        let socket = UnixDatagram::bind(&socket_path).expect("bind overlay control socket");

        assert_eq!(
            send_toggle_to(&socket_path).expect("send toggle command"),
            TOGGLE_MESSAGE.len()
        );
        let mut received = [0; 16];
        let length = socket.recv(&mut received).expect("receive toggle command");
        assert!(is_toggle_message(&received[..length]));
        assert!(!is_toggle_message(b"invalid"));

        let _ = std::fs::remove_file(socket_path);
    }

    #[test]
    fn overlay_dependency_and_service_wiring_stay_pinned() {
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("dc1cf9f00cbe2902a52166492654bb9fee8a73d1"));

        let service = include_str!("../../../packaging/arch/hearthdeck-overlay.service");
        assert!(service.contains("ExecStart=/usr/lib/hearthdeck/hearthdeck-overlay"));
        assert!(service.contains("Restart=always"));

        let package = include_str!("../../../packaging/arch/PKGBUILD");
        assert!(package.contains(
            "install -Dm755 services/target/release/hearthdeck-overlay \"$pkgdir/usr/lib/hearthdeck/hearthdeck-overlay\""
        ));
    }

    #[test]
    fn stops_the_session_the_bridge_reports_as_active_and_rejects_missing_sessions() {
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

        let listener = UnixListener::bind(&socket_path).expect("bind test socket");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept test connection");
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            reader.read_line(&mut line).expect("read test request");
            assert!(line.contains("active_application_session"));
            let mut stream = &stream;
            stream
                .write_all(b"{\"type\":\"application_session\",\"session\":null}\n")
                .expect("write test response");
        });

        let error = stop_active_application().expect_err("missing session must fail close");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        server.join().expect("test bridge thread should not panic");
        // SAFETY: no other test in this crate touches this env var.
        unsafe {
            std::env::remove_var("HEARTHDECK_BRIDGE_SOCKET");
        }
        let _ = std::fs::remove_file(&socket_path);
    }
}
