// Bare-minimum quick-menu overlay for the "COSMIC (Test)" session (see
// packaging/arch/cosmic-test-session). A full-screen, semi-transparent
// wlr-layer-shell surface, toggled by a gamepad's Guide/Mode button (see
// input.rs), listing session actions - starting with "Close App".
//
// UNTESTED against a real Linux/Wayland/wgpu toolchain: written without
// access to one. The layer-shell wiring below mirrors real, shipped code
// read directly from cosmic-launcher's own src/app.rs (its
// `create_dummy_layer_surface`/`show`/`hide` functions) and cosmic-comp's
// own src/lib.rs (kiosk_child exit handling), not guessed from memory - but
// libcosmic's git-main API has no stable release and does change, so the
// first real step is `cargo build --manifest-path services/Cargo.toml -p
// hearthdeck-overlay` on the actual target hardware, expecting to fix
// compile errors from whatever the pinned commit's API actually looks like
// today. The `view_window` widget tree is the section most likely to need
// adjusting; see libcosmic's own `examples/application` and `examples/menu`
// for current widget builder syntax if it doesn't compile as written.
mod input;

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
use cosmic::iced::{Alignment, Color, Length, Subscription, window};
use cosmic::surface::action::{LiveSettings, app_layer_shell};
use cosmic::widget::{button, column, container, text};
use tracing::{error, info, warn};

const APP_ID: &str = "io.github.viniciuscr.hearthdeck.Overlay";

fn main() -> cosmic::iced::Result {
    init_logging();
    info!("hearthdeck-overlay ({APP_ID})");

    cosmic::app::run::<Overlay>(
        Settings::default()
            .no_main_window(true)
            .exit_on_close(false),
        (),
    )
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
        input::subscription().map(|input::GuideButtonPressed| Message::ToggleOverlay)
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
                close_hearthdeck();
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

/// Sends SIGTERM to the Hearthdeck process, matched by exact
/// `/proc/<pid>/comm` name - it's cosmic-comp's kiosk_command child, not a
/// systemd unit this could `systemctl stop`. cosmic-comp itself ends its
/// whole event loop once this child exits (confirmed by reading
/// cosmic-comp's own src/lib.rs kiosk_child handling: it sets
/// `should_stop = true` and exits with the child's exit code), so this is
/// also, therefore, this bare session's only way back to the greeter.
fn close_hearthdeck() {
    match find_hearthdeck_pid() {
        Some(pid) => {
            if let Err(err) = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM) {
                error!(?err, "hearthdeck-overlay: failed to signal Hearthdeck");
            }
        }
        None => warn!("hearthdeck-overlay: could not find a running Hearthdeck process"),
    }
}

fn find_hearthdeck_pid() -> Option<nix::unistd::Pid> {
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(pid) = name.parse::<i32>() else {
            continue;
        };
        let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) else {
            continue;
        };
        if comm.trim() == "hearthdeck" {
            return Some(nix::unistd::Pid::from_raw(pid));
        }
    }
    None
}
