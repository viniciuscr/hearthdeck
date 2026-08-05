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
use evdev::{EventType, InputEvent, KeyCode};
use futures_util::SinkExt;
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
                let hide = self.hide();
                thread::spawn(|| {
                    if let Err(err) = send_close_shortcut() {
                        error!(
                            ?err,
                            "hearthdeck-overlay: failed to send close shortcut"
                        );
                    }
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

/// Simulates an Alt+F4 keypress via a synthetic uinput keyboard, using
/// whatever app currently has focus - i.e. the app the overlay was covering,
/// once its own keyboard grab is released by `hide()`.
///
/// ponytail: COSMIC's native toplevel-close protocol (zcosmic_toplevel_manager_v1)
/// looked like the "correct" fix and passed every check we could run, but
/// didn't actually close anything on real hardware - likely because many
/// apps/games don't wire up a close-request handler for that protocol event,
/// while Alt+F4 is a close shortcut every desktop app and window manager
/// already honors. Simulating the keypress is the boring, reliable path.
/// Upgrade to a protocol-based close if/when we confirm which apps ignore it
/// and need something gentler than Alt+F4's forced-quit semantics.
fn send_close_shortcut() -> io::Result<()> {
    // Give the compositor a moment to return keyboard focus to the
    // now-uncovered app after hide() destroys the overlay's layer surface.
    thread::sleep(Duration::from_millis(200));

    let mut keys = evdev::AttributeSet::<KeyCode>::new();
    keys.insert(KeyCode::KEY_LEFTALT);
    keys.insert(KeyCode::KEY_F4);

    let mut device = evdev::uinput::VirtualDevice::builder()?
        .name("hearthdeck-overlay-close-shortcut")
        .with_keys(&keys)?
        .build()?;

    // Give the compositor time to notice the new input device before we
    // emit events on it.
    thread::sleep(Duration::from_millis(100));

    device.emit(&[
        InputEvent::new(EventType::KEY.0, KeyCode::KEY_LEFTALT.0, 1),
        InputEvent::new(EventType::KEY.0, KeyCode::KEY_F4.0, 1),
    ])?;
    thread::sleep(Duration::from_millis(50));
    device.emit(&[
        InputEvent::new(EventType::KEY.0, KeyCode::KEY_F4.0, 0),
        InputEvent::new(EventType::KEY.0, KeyCode::KEY_LEFTALT.0, 0),
    ])?;

    info!("hearthdeck-overlay: sent Alt+F4 close shortcut");
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
