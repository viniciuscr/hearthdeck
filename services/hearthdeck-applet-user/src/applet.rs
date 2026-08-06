use cosmic::Element;
use cosmic::app::{Core, Task};

const APP_ID: &str = "io.github.viniciuscr.hearthdeck.AppletUser";

pub struct UserApplet {
    core: Core,
    username: String,
}

// No interactive actions needed: this applet is a read-only text label.
#[derive(Clone, Debug)]
pub enum Message {}

impl cosmic::Application for UserApplet {
    type Executor = cosmic::SingleThreadExecutor;
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
        let username = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_else(|_| "user".to_owned());
        (Self { core, username }, Task::none())
    }

    fn update(&mut self, _message: Message) -> Task<Message> {
        Task::none()
    }

    fn view(&self) -> Element<Message> {
        // `applet.text` auto-scales with panel size (XS→XL).
        // `autosize_window` wraps it in the sizing container cosmic-panel
        // expects so it gets the right dimensions for the slot it occupies.
        self.core
            .applet
            .autosize_window(self.core.applet.text(self.username.as_str()))
            .into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

pub fn run() -> cosmic::iced::Result {
    cosmic::applet::run::<UserApplet>(())
}
