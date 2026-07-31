mod bridge;
mod control;
mod input;

use cosmic::{Core, executor, iced, prelude::*, widget};
use iced::{Alignment, Length, Subscription};

use crate::linux::{bridge::ManagedSession, control::OverlayCommand, input::GamepadAction};

const OVERLAY_NAMESPACE: &str = "io.github.viniciuscr.hearthdeck.overlay";

pub fn run() -> anyhow::Result<()> {
    hearthdeck_observability::init("hearthdeck-overlay", "hearthdeck_overlay=info");
    match control::command_from_args()? {
        Some(command) => control::send(command),
        None => {
            cosmic::app::run::<Overlay>(cosmic::app::Settings::default().no_main_window(true), ())
                .map_err(Into::into)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Selection {
    Resume,
    CloseGame,
}

impl Selection {
    fn toggle(self) -> Self {
        match self {
            Self::Resume => Self::CloseGame,
            Self::CloseGame => Self::Resume,
        }
    }
}

#[derive(Clone, Debug)]
enum Message {
    Control(OverlayCommand),
    Gamepad(GamepadAction),
    SessionLoaded(Result<Option<ManagedSession>, String>),
    SessionStopped(Result<(), String>),
    Resume,
    CloseGame,
}

struct Overlay {
    core: Core,
    visible: bool,
    selection: Selection,
    active_session: Option<ManagedSession>,
    message: Option<String>,
}

impl cosmic::Application for Overlay {
    type Executor = executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = OVERLAY_NAMESPACE;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, cosmic::app::Task<Self::Message>) {
        let layer = cosmic::surface::action::app_layer_shell(
            |_| cosmic::surface::action::LiveSettings::default(),
            |_| {
                use cosmic::iced::platform_specific::{
                    runtime::wayland::layer_surface::SctkLayerSurfaceSettings,
                    shell::commands::layer_surface::{Anchor, KeyboardInteractivity, Layer},
                };

                SctkLayerSurfaceSettings {
                    id: iced::window::Id::unique(),
                    layer: Layer::Overlay,
                    anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                    keyboard_interactivity: KeyboardInteractivity::None,
                    // The overlay is controller-only for this slice. Keeping the
                    // Wayland input region empty makes mouse and touch click through.
                    input_zone: Some(Vec::new()),
                    namespace: OVERLAY_NAMESPACE.to_owned(),
                    size: Some((None, None)),
                    exclusive_zone: 0,
                    ..Default::default()
                }
            },
            Some(Box::new(|overlay: &Overlay| {
                overlay.overlay_view().map(cosmic::Action::App)
            })),
        );
        (
            Self {
                core,
                visible: false,
                selection: Selection::Resume,
                active_session: None,
                message: None,
            },
            cosmic::task::message(cosmic::Action::Cosmic(cosmic::app::Action::Surface(layer))),
        )
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::batch([
            control::subscription().map(Message::Control),
            input::subscription().map(Message::Gamepad),
        ])
    }

    fn update(&mut self, message: Self::Message) -> cosmic::app::Task<Self::Message> {
        match message {
            Message::Control(command) => self.handle_command(command),
            Message::Gamepad(action) => self.handle_gamepad(action),
            Message::SessionLoaded(result) => {
                match result {
                    Ok(session) => self.active_session = session,
                    Err(error) => self.message = Some(error),
                }
                cosmic::app::Task::none()
            }
            Message::SessionStopped(result) => {
                if let Err(error) = result {
                    self.message = Some(error);
                }
                cosmic::app::Task::none()
            }
            Message::Resume => self.hide(),
            Message::CloseGame => {
                let task = self.hide();
                cosmic::app::Task::batch([
                    task,
                    cosmic::app::Task::perform(
                        bridge::stop_active_session(),
                        Message::SessionStopped,
                    ),
                ])
            }
        }
    }

    fn view(&self) -> cosmic::Element<'_, Self::Message> {
        widget::text::body("").into()
    }
}

impl Overlay {
    fn overlay_view(&self) -> cosmic::Element<'_, Message> {
        if !self.visible {
            return widget::container(widget::text::body(""))
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        }

        let session_title = self
            .active_session
            .as_ref()
            .map_or("No managed game is active", |session| {
                session.application_id.as_str()
            });
        let feedback = self
            .message
            .as_deref()
            .unwrap_or("Guide / PS: close overlay");
        let resume = match self.selection {
            Selection::Resume => widget::button::suggested("Resume game"),
            Selection::CloseGame => widget::button::text("Resume game"),
        }
        .on_press(Message::Resume);
        let close_game = match self.selection {
            Selection::Resume => widget::button::text("Close game"),
            Selection::CloseGame => widget::button::suggested("Close game"),
        }
        .on_press(Message::CloseGame);
        let panel = widget::container(
            widget::column::with_capacity(5)
                .push(widget::text::body("HEARTHDECK OVERLAY"))
                .push(widget::text::body(session_title))
                .push(widget::text::body(feedback))
                .push(
                    widget::row::with_capacity(2)
                        .push(resume)
                        .push(close_game)
                        .spacing(16),
                )
                .push(widget::text::body("D-pad: choose   A: confirm   B: resume"))
                .spacing(16)
                .width(Length::Shrink),
        )
        .padding(32)
        .width(Length::Shrink);

        widget::container(panel)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .into()
    }

    fn handle_command(&mut self, command: OverlayCommand) -> cosmic::app::Task<Message> {
        match command {
            OverlayCommand::Toggle => {
                if self.visible {
                    self.hide()
                } else {
                    self.show()
                }
            }
            OverlayCommand::Show => self.show(),
            OverlayCommand::Hide => self.hide(),
        }
    }

    fn handle_gamepad(&mut self, action: GamepadAction) -> cosmic::app::Task<Message> {
        match action {
            GamepadAction::Toggle => self.handle_command(OverlayCommand::Toggle),
            GamepadAction::Hide => self.hide(),
            GamepadAction::ToggleSelection => {
                self.selection = self.selection.toggle();
                cosmic::app::Task::none()
            }
            GamepadAction::Activate => match self.selection {
                Selection::Resume => self.update(Message::Resume),
                Selection::CloseGame => self.update(Message::CloseGame),
            },
        }
    }

    fn show(&mut self) -> cosmic::app::Task<Message> {
        self.visible = true;
        self.message = None;
        input::set_visible(true);
        cosmic::app::Task::perform(bridge::active_session(), Message::SessionLoaded)
    }

    fn hide(&mut self) -> cosmic::app::Task<Message> {
        self.visible = false;
        self.active_session = None;
        input::set_visible(false);
        cosmic::app::Task::none()
    }
}
