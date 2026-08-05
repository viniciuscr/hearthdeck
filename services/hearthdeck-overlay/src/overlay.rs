use std::error::Error;
use std::io;
use std::os::unix::net::UnixDatagram as StdUnixDatagram;
use std::path::PathBuf;
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
use cosmic::widget::{button, column, container, text};
use cosmic_client_toolkit::{
    cosmic_protocols::{
        toplevel_info::v1::client::zcosmic_toplevel_handle_v1,
        toplevel_management::v1::client::zcosmic_toplevel_manager_v1,
    },
    toplevel_info::{ToplevelInfoHandler, ToplevelInfoState},
    toplevel_management::{ToplevelManagerHandler, ToplevelManagerState},
};
use futures_util::SinkExt;
use sctk::{
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
};
use smithay_client_toolkit as sctk;
use tokio::net::UnixDatagram;
use tracing::{error, info, warn};
use wayland_client::{
    Connection, QueueHandle, WEnum, globals::registry_queue_init, protocol::wl_output,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1;

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

struct ToplevelProbe {
    output_state: OutputState,
    registry_state: RegistryState,
    toplevel_info_state: ToplevelInfoState,
    toplevel_manager_state: ToplevelManagerState,
    close_supported: bool,
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

impl ToplevelManagerHandler for ToplevelProbe {
    fn toplevel_manager_state(&mut self) -> &mut ToplevelManagerState {
        &mut self.toplevel_manager_state
    }

    fn capabilities(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        capabilities: Vec<
            WEnum<zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1>,
        >,
    ) {
        self.close_supported = close_is_supported(&capabilities);
    }
}

cosmic_client_toolkit::delegate_toplevel_info!(ToplevelProbe);
cosmic_client_toolkit::delegate_toplevel_manager!(ToplevelProbe);
sctk::delegate_output!(ToplevelProbe);
sctk::delegate_registry!(ToplevelProbe);

struct Overlay {
    core: Core,
    window_id: window::Id,
    visible: bool,
    // ponytail: identifier of the app that was focused right before we grabbed
    // keyboard input. The overlay's own layer surface uses
    // KeyboardInteractivity::Exclusive, which clears the underlying toplevel's
    // Activated state as soon as the overlay opens - so "Close App" can no
    // longer find it by activation state at click time. Capture it up front.
    active_before_overlay: Option<String>,
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
            active_before_overlay: None,
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
                let Some(identifier) = self.active_before_overlay.clone() else {
                    warn!("hearthdeck-overlay: no app was focused before the overlay opened");
                    return self.hide();
                };
                thread::spawn(move || {
                    if let Err(err) = close_toplevel(&identifier) {
                        error!(
                            ?err,
                            "hearthdeck-overlay: failed to close active application"
                        );
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

fn probe_toplevels() -> io::Result<(Connection, ToplevelProbe)> {
    let conn = Connection::connect_to_env().map_err(io::Error::other)?;
    let (globals, mut event_queue) = registry_queue_init(&conn).map_err(io::Error::other)?;
    let qh = event_queue.handle();
    let registry_state = RegistryState::new(&globals);
    let toplevel_info_state =
        ToplevelInfoState::try_new(&registry_state, &qh).ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "toplevel list is unavailable")
        })?;
    let toplevel_manager_state =
        ToplevelManagerState::try_new(&registry_state, &qh).ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "toplevel close is unavailable")
        })?;
    let mut probe = ToplevelProbe {
        output_state: OutputState::new(&globals, &qh),
        registry_state,
        toplevel_info_state,
        toplevel_manager_state,
        close_supported: false,
    };
    event_queue
        .roundtrip(&mut probe)
        .map_err(io::Error::other)?;
    Ok((conn, probe))
}

/// Identifier of the single activated toplevel, if any. Meant to be called
/// before the overlay grabs keyboard input - see `Overlay::active_before_overlay`.
fn activated_toplevel_identifier() -> io::Result<Option<String>> {
    let (_conn, probe) = probe_toplevels()?;
    let mut activated = probe.toplevel_info_state.toplevels().filter(|toplevel| {
        toplevel
            .state
            .contains(&zcosmic_toplevel_handle_v1::State::Activated)
    });
    let Some(active) = activated.next() else {
        return Ok(None);
    };
    if activated.next().is_some() {
        return Err(io::Error::other("more than one toplevel is activated"));
    }
    Ok(Some(active.identifier.clone()))
}

/// Requests that the compositor close the toplevel matching `identifier`.
/// Matching by identifier (not activation state) because by the time this
/// runs, the overlay's own keyboard grab has already cleared Activated on the
/// underlying toplevel.
fn close_toplevel(identifier: &str) -> io::Result<()> {
    let (conn, probe) = probe_toplevels()?;
    if !probe.close_supported {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "compositor does not support closing toplevels",
        ));
    }
    let target = probe
        .toplevel_info_state
        .toplevels()
        .find(|toplevel| toplevel.identifier == identifier)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "tracked toplevel is no longer open",
            )
        })?;
    let toplevel = target
        .cosmic_toplevel
        .clone()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "toplevel cannot be closed"))?;
    let app_id = target.app_id.clone();
    let title = target.title.clone();

    probe.toplevel_manager_state.manager.close(&toplevel);
    conn.flush().map_err(io::Error::other)?;
    info!(
        %app_id,
        %title,
        %identifier,
        "hearthdeck-overlay: requested close for tracked toplevel"
    );
    Ok(())
}

fn close_is_supported(
    capabilities: &[WEnum<
        zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1,
    >],
) -> bool {
    capabilities.iter().any(|capability| {
        matches!(
            capability,
            WEnum::Value(
                zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1::Close
            )
        )
    })
}

impl Overlay {
    fn show(&mut self) -> Task<Message> {
        // Capture identity before we grab keyboard input below - once the
        // overlay has exclusive keyboard interactivity, the compositor clears
        // the underlying toplevel's Activated state and it can no longer be
        // found that way.
        self.active_before_overlay = match activated_toplevel_identifier() {
            Ok(identifier) => identifier,
            Err(err) => {
                warn!(
                    ?err,
                    "hearthdeck-overlay: failed to read activated toplevel"
                );
                None
            }
        };
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
    use super::{WEnum, close_is_supported, zcosmic_toplevel_manager_v1};

    #[test]
    fn requires_the_close_capability() {
        use zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1::Close;

        assert!(close_is_supported(&[WEnum::Value(Close)]));
        assert!(!close_is_supported(&[WEnum::Unknown(0)]));
    }
}
