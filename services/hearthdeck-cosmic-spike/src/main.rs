#[cfg(target_os = "linux")]
mod app {
    use cosmic::Element;
    use cosmic::app::{Core, Settings, Task};
    use cosmic::iced::{Alignment, Length};
    use cosmic::widget::{column, container, list, list_column, row, text};

    const APP_ID: &str = "io.github.viniciuscr.hearthdeck.CosmicSpike";

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Screen {
        Dashboard,
        FullLibrary,
    }

    #[derive(Clone, Debug)]
    enum Message {
        OpenFullLibrary,
        BackToDashboard,
    }

    pub struct CosmicSpike {
        core: Core,
        screen: Screen,
    }

    impl cosmic::Application for CosmicSpike {
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
            (
                Self {
                    core,
                    screen: Screen::Dashboard,
                },
                Task::none(),
            )
        }

        fn update(&mut self, message: Message) -> Task<Message> {
            self.screen = transition(self.screen, &message);
            Task::none()
        }

        fn view(&self) -> Element<Message> {
            match self.screen {
                Screen::Dashboard => dashboard_view(),
                Screen::FullLibrary => full_library_view(),
            }
        }
    }

    fn transition(screen: Screen, message: &Message) -> Screen {
        match (screen, message) {
            (_, Message::OpenFullLibrary) => Screen::FullLibrary,
            (_, Message::BackToDashboard) => Screen::Dashboard,
        }
    }

    fn dashboard_view() -> Element<'static, Message> {
        // ponytail: placeholder "dashboard icon" text; swap to a real icon asset once visual baseline is approved.
        let menu = list_column().add(
            list::button(text::title2("[ Dashboard ]"))
                .on_press(Message::OpenFullLibrary)
                .selected(true),
        );

        container(
            column(vec![
                text::title2("Hearthdeck Cosmic Spike").into(),
                text::body("Open Full Library").into(),
                menu.into(),
            ])
            .spacing(24)
            .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(36)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .into()
    }

    fn full_library_view() -> Element<'static, Message> {
        let hero_row = row(vec![
            library_tile("Featured"),
            library_tile("Recently Added"),
            library_tile("Collections"),
        ])
        .spacing(16);

        let categories = list_column()
            .add(list::button(text::body("Games")).selected(true))
            .add(list::button(text::body("Apps")))
            .add(list::button(text::body("History")));

        container(
            column(vec![
                list::button(text::body("< Dashboard"))
                    .on_press(Message::BackToDashboard)
                    .into(),
                text::title2("Full Library").into(),
                text::body("Bigger text + bigger covers, simple responsive layout.").into(),
                hero_row.into(),
                categories.into(),
            ])
            .spacing(20),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(32)
        .into()
    }

    fn library_tile(title: &'static str) -> Element<'static, Message> {
        container(text::title2(title))
            .width(Length::FillPortion(1))
            .height(Length::Fixed(220.0))
            .padding(16)
            .class(cosmic::theme::Container::Dialog(true))
            .into()
    }

    pub fn run() -> cosmic::iced::Result {
        cosmic::app::run::<CosmicSpike>(Settings::default(), ())
    }

    #[cfg(test)]
    mod tests {
        use super::{Message, Screen, transition};

        #[test]
        fn dashboard_flow_switches_between_screens() {
            assert_eq!(
                transition(Screen::Dashboard, &Message::OpenFullLibrary),
                Screen::FullLibrary
            );
            assert_eq!(
                transition(Screen::FullLibrary, &Message::BackToDashboard),
                Screen::Dashboard
            );
        }
    }
}

#[cfg(target_os = "linux")]
fn main() -> cosmic::iced::Result {
    app::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("hearthdeck-cosmic-spike is supported only on Linux Wayland sessions");
}
