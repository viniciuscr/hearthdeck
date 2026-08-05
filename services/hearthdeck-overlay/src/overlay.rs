use std::error::Error;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixDatagram as StdUnixDatagram;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use cosmic::Element;
use cosmic::app::{Core, Settings, Task};
use cosmic::cctk::sctk::shell::wlr_layer::Layer;
use cosmic::iced::platform_specific::runtime::wayland::layer_surface::SctkLayerSurfaceSettings;
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    Anchor, KeyboardInteractivity, destroy_layer_surface,
};
use cosmic::iced::runtime::core::layout::Limits;
use cosmic::iced::runtime::platform_specific::wayland::CornerRadius;
use cosmic::iced::runtime::platform_specific::wayland::layer_surface::IcedMargin;
use cosmic::iced::stream;
use cosmic::iced::futures::channel::mpsc::Sender;
use cosmic::iced::{Alignment, Color, Length, Subscription, window};
use cosmic::surface::action::{LiveSettings, app_layer_shell};
use cosmic::widget::{button, column, container, text};
use cosmic_client_toolkit::toplevel_info::{ToplevelInfoHandler, ToplevelInfoState};
use futures_util::SinkExt;
use hearthdeck_protocol::{BridgeRequest, BridgeResponse};
use smithay_client_toolkit as sctk;
use sctk::{
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
};
use tokio::net::UnixDatagram;
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::wl_output,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1;
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

#[derive(Debug)]
struct VisibleApp {
    app_id: String,
    title: String,
    identifier: String,
}

struct ToplevelProbe {
    output_state: OutputState,
    registry_state: RegistryState,
    toplevel_info_state: ToplevelInfoState,
}

impl ProvidesRegistryState for ToplevelProbe {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    sctk::registry_handlers!(OutputState);
}

impl OutputHandler for ToplevelProbe {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl ToplevelInfoHandler for ToplevelProbe {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevel_info_state
    }

    fn new_toplevel(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _toplevel: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    ) {
    }

    fn update_toplevel(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _toplevel: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    ) {
    }

    fn toplevel_closed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _toplevel: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    ) {
    }
}

cosmic_client_toolkit::delegate_toplevel_info!(ToplevelProbe);
sctk::delegate_output!(ToplevelProbe);
sctk::delegate_registry!(ToplevelProbe);

struct Overlay {
    core: Core,
    window_id: window::Id,
    visible: bool,
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
                thread::spawn(|| {
                    if let Err(err) = close_active_app() {
                        error!(?err, "hearthdeck-overlay: failed to stop active application");
                    }
                });
                self.hide()
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

fn close_active_app() -> io::Result<()> {
    let visible_app = visible_active_toplevel()?;
    let Some(visible_app) = visible_app else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no visible app is active",
        ));
    };
    let response = bridge_request(BridgeRequest::ActiveApplicationSession)?;
    let BridgeResponse::ApplicationSession { session: Some(session) } = response else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no managed application session is active",
        ));
    };
    if !visible_app_matches_session(&visible_app, &session) {
        return Err(io::Error::other(format!(
            "focused app does not match managed session (visible_app_id={}, visible_title={}, session_source={}, session_application_id={})",
            visible_app.app_id,
            visible_app.title,
            session.source_id,
            session.application_id
        )));
    }
    info!(
        app_id = %visible_app.app_id,
        title = %visible_app.title,
        identifier = %visible_app.identifier,
        session_id = %session.id,
        "hearthdeck-overlay: stopping visible active session"
    );
    match bridge_request(BridgeRequest::StopApplicationSession {
        session_id: session.id,
    })? {
        BridgeResponse::StopAccepted { .. } => Ok(()),
        BridgeResponse::Error { message, .. } => Err(io::Error::other(message)),
        response => Err(io::Error::other(format!(
            "unexpected bridge response: {response:?}"
        ))),
    }
}

fn visible_active_toplevel() -> io::Result<Option<VisibleApp>> {
    let conn = Connection::connect_to_env().map_err(io::Error::other)?;
    let (globals, mut event_queue) = registry_queue_init(&conn).map_err(io::Error::other)?;
    let qh = event_queue.handle();
    let registry_state = RegistryState::new(&globals);
    let toplevel_info_state = ToplevelInfoState::new(&registry_state, &qh);
    let mut probe = ToplevelProbe {
        output_state: OutputState::new(&globals, &qh),
        registry_state,
        toplevel_info_state,
    };
    event_queue
        .roundtrip(&mut probe)
        .map_err(io::Error::other)?;
    let active = probe
        .toplevel_info_state
        .toplevels()
        .next()
        .map(|toplevel| VisibleApp {
            app_id: toplevel.app_id.clone(),
            title: toplevel.title.clone(),
            identifier: toplevel.identifier.clone(),
        });
    Ok(active)
}

fn visible_app_matches_session(visible_app: &VisibleApp, session: &hearthdeck_protocol::ApplicationSession) -> bool {
        let visible_app_id = visible_app.app_id.trim().to_ascii_lowercase();
        let visible_title = visible_app.title.trim().to_ascii_lowercase();
        let session_application_id = session.application_id.trim().to_ascii_lowercase();
        match session.source_id.as_str() {
            "desktop-apps" => visible_app_id == session_application_id,
            "retroarch" => {
                visible_app_id == "retroarch"
                    || visible_title.contains("retroarch")
                    || visible_title.contains(&session_application_id)
            }
            "heroic" => {
                visible_app_id == "heroic"
                    || visible_app_id == "gamescope"
                    || visible_title.contains("heroic")
                    || visible_title.contains(&session_application_id)
            }
            _ => visible_app_id == session_application_id || visible_title.contains(&session_application_id),
        }
}

fn bridge_request(request: BridgeRequest) -> io::Result<BridgeResponse> {
    let mut stream = StdUnixStream::connect(bridge_socket_path())?;
    let payload = serde_json::to_string(&request).map_err(io::Error::other)?;
    stream.write_all(payload.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.shutdown(Shutdown::Write)?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    if response.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "bridge closed connection without a response",
        ));
    }
    serde_json::from_str(&response).map_err(io::Error::other)
}

fn bridge_socket_path() -> PathBuf {
    std::env::var_os("HEARTHDECK_BRIDGE_SOCKET")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("hearthdeck/bridge.sock")
        })
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
