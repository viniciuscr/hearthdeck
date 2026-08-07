use std::time::Duration;

use cosmic::app::{Core, Settings, Task};
use cosmic::iced::widget::{button, column, container, row, scrollable, text, Space};
use cosmic::iced::{self, Background, Border, Color, Length, Subscription, alignment, time};
use cosmic::{ApplicationExt, Element, executor};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let settings = Settings::default()
        .size(cosmic::iced::Size::new(1365.0, 768.0))
        .client_decorations(false);
    cosmic::app::run::<LibraryApp>(settings, ())?;
    Ok(())
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    SelectGame(usize),
}

#[derive(Debug, Clone, Copy)]
struct Game {
    title: &'static str,
    color: Color,
}

struct LibraryApp {
    core: Core,
    selected_game: usize,
    tick_phase: f32,
}

impl cosmic::Application for LibraryApp {
    type Executor = executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = "com.hearthdeck.v2";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: ()) -> (Self, Task<Self::Message>) {
        let mut app = Self {
            core,
            selected_game: 0,
            tick_phase: 0.0,
        };
        let tasks = Task::batch(vec![app.update_title(), app.enter_fullscreen()]);
        (app, tasks)
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        time::every(Duration::from_millis(16)).map(|_| Message::Tick)
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::Tick => {
                self.tick_phase = (self.tick_phase + 0.1) % std::f32::consts::TAU;
            }
            Message::SelectGame(index) => self.selected_game = index,
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let root = row![
            sidebar(),
            container(main_panel(self.selected_game, self.tick_phase))
                .width(Length::Fill)
                .height(Length::Fill)
                .padding([36, 44])
        ]
        .height(Length::Fill);

        container(root)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(rgb(0.06, 0.06, 0.06))),
                text_color: Some(Color::WHITE),
                ..container::Style::default()
            })
            .into()
    }
}

impl LibraryApp
where
    Self: cosmic::Application,
{
    fn update_title(&mut self) -> Task<Message> {
        let title = String::from("Full library");
        self.set_header_title(title.clone());
        self.set_window_title(title)
    }

    fn enter_fullscreen(&self) -> Task<Message> {
        self.core.main_window_id().map_or_else(Task::none, |id| {
            iced::window::set_mode(id, iced::window::Mode::Fullscreen)
        })
    }
}

fn sidebar<'a>() -> Element<'a, Message> {
    let nav_item = |label: &'static str, count: &'static str, selected: bool| {
        let bg = if selected {
            rgb(0.19, 0.19, 0.19)
        } else {
            Color::TRANSPARENT
        };
        container(row![
            container(
                row![
                    text(label).size(14),
                    Space::new().width(Length::Fill),
                    text(count).size(14)
                ]
                .align_y(alignment::Vertical::Center),
            )
            .width(Length::Fill),
            container(Space::new().width(3).height(Length::Fill)).style(move |_| container::Style {
                background: Some(Background::Color(if selected {
                    rgb(0.10, 0.65, 0.18)
                } else {
                    Color::TRANSPARENT
                })),
                ..container::Style::default()
            })
        ])
        .padding([10, 12])
        .style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            text_color: Some(rgb(0.90, 0.90, 0.90)),
            ..container::Style::default()
        })
    };

    let divider = || {
        container(Space::new().height(1))
            .width(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(rgb(0.16, 0.16, 0.16))),
                ..container::Style::default()
            })
    };

    let body = column![
        row![
            container(text("●").size(18)).padding([0, 0, 0, 2]),
            text("StormYeti").size(16)
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center),
        Space::new().height(16),
        search_box(),
        Space::new().height(18),
        nav_item("Games", "16", false),
        nav_item("Apps", "6", false),
        nav_item("Groups", "2", false),
        nav_item("Full library", "2", true),
        Space::new().height(8),
        divider(),
        Space::new().height(8),
        container(column![text("Manage").size(14), text("Queue").size(14), text("Update").size(14)].spacing(10))
            .padding([8, 12]),
        Space::new().height(Length::Fill),
        divider(),
        Space::new().height(14),
        container(
            row![
                column![text("All Storage").size(12), text("300 GB free").size(14)]
                    .spacing(4)
                    .align_x(alignment::Horizontal::Left),
                Space::new().width(Length::Fill),
                container(text("25%").size(12))
                    .padding([10, 8])
                    .style(|_| container::Style {
                        border: Border {
                            color: rgb(0.25, 0.25, 0.25),
                            width: 2.0,
                            radius: 999.0.into(),
                        },
                        ..container::Style::default()
                    })
            ]
        )
    ]
    .spacing(8);

    container(body)
        .width(312)
        .height(Length::Fill)
        .padding([28, 20])
        .style(|_| container::Style {
            background: Some(Background::Color(rgb(0.08, 0.08, 0.08))),
            border: Border {
                color: rgb(0.17, 0.17, 0.17),
                width: 1.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn search_box<'a>() -> Element<'a, Message> {
    container(text("Search").size(15))
        .width(Length::Fill)
        .padding([10, 14])
        .style(|_| container::Style {
            background: Some(Background::Color(rgb(0.13, 0.13, 0.13))),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 18.0.into(),
            },
            text_color: Some(rgb(0.88, 0.88, 0.88)),
            ..container::Style::default()
        })
        .into()
}

fn main_panel<'a>(selected_game: usize, tick_phase: f32) -> Element<'a, Message> {
    let tabs = row![
        tab("All games", true),
        tab("Owned games", false),
        tab("Xbox Game Pass", false),
        tab("EA Play", false),
        tab("Xbox Live Gold", false),
        tab("Owned apps", false),
    ]
    .spacing(24);

    let controls = row![
        fake_sort(),
        Space::new().width(Length::Fill),
        text("400 GAMES").size(15),
        fake_filters()
    ]
    .spacing(16)
    .align_y(alignment::Vertical::Center);

    let grid = games_grid(selected_game, tick_phase);

    column![
        text("Full library").size(24),
        Space::new().height(24),
        tabs,
        Space::new().height(22),
        controls,
        Space::new().height(16),
        scrollable(grid).height(Length::Fill),
    ]
    .into()
}

fn tab<'a>(label: &'static str, selected: bool) -> Element<'a, Message> {
    let underline = container(Space::new().height(4))
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(if selected {
                rgb(0.10, 0.65, 0.18)
            } else {
                Color::TRANSPARENT
            })),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 2.0.into(),
            },
            ..container::Style::default()
        });

    column![text(label).size(15), Space::new().height(6), underline]
        .spacing(2)
        .into()
}

fn fake_sort<'a>() -> Element<'a, Message> {
    container(text("Sort by date acquired").size(15))
        .padding([10, 14])
        .width(305)
        .style(|_| container::Style {
            background: Some(Background::Color(rgb(0.20, 0.20, 0.20))),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn fake_filters<'a>() -> Element<'a, Message> {
    container(text("Filters").size(15))
        .padding([10, 18])
        .width(150)
        .style(|_| container::Style {
            background: Some(Background::Color(rgb(0.20, 0.20, 0.20))),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn games_grid<'a>(selected_game: usize, tick_phase: f32) -> Element<'a, Message> {
    let games = [
        Game {
            title: "Halo Infinite",
            color: rgb(0.26, 0.44, 0.58),
        },
        Game {
            title: "Opus",
            color: rgb(0.41, 0.35, 0.64),
        },
        Game {
            title: "Sea of Thieves",
            color: rgb(0.05, 0.31, 0.26),
        },
        Game {
            title: "Assassin's Creed",
            color: rgb(0.18, 0.18, 0.18),
        },
        Game {
            title: "It Takes Two",
            color: rgb(0.56, 0.44, 0.26),
        },
        Game {
            title: "Fuga",
            color: rgb(0.62, 0.35, 0.28),
        },
        Game {
            title: "Tinykin",
            color: rgb(0.48, 0.28, 0.56),
        },
        Game {
            title: "Forza Horizon",
            color: rgb(0.33, 0.49, 0.61),
        },
        Game {
            title: "Unravel Two",
            color: rgb(0.50, 0.33, 0.33),
        },
        Game {
            title: "Dead by Daylight",
            color: rgb(0.20, 0.24, 0.28),
        },
        Game {
            title: "Madden 23",
            color: rgb(0.34, 0.52, 0.60),
        },
        Game {
            title: "Rocket Arena",
            color: rgb(0.57, 0.39, 0.60),
        },
        Game {
            title: "Ori",
            color: rgb(0.25, 0.28, 0.60),
        },
        Game {
            title: "Flight Simulator",
            color: rgb(0.34, 0.49, 0.61),
        },
        Game {
            title: "Unravel",
            color: rgb(0.43, 0.37, 0.42),
        },
        Game {
            title: "Battlefront II",
            color: rgb(0.15, 0.22, 0.36),
        },
        Game {
            title: "Sea of Solitude",
            color: rgb(0.17, 0.49, 0.59),
        },
        Game {
            title: "Ark",
            color: rgb(0.40, 0.40, 0.35),
        },
    ];

    let mut rows = column!().spacing(12);
    for (row_idx, chunk) in games.chunks(6).enumerate() {
        let mut tiles = row!().spacing(12);
        for (col_idx, game) in chunk.iter().enumerate() {
            let index = row_idx * 6 + col_idx;
            tiles = tiles.push(game_tile(*game, index == selected_game, tick_phase, index));
        }
        rows = rows.push(tiles);
    }
    rows.into()
}

fn game_tile<'a>(game: Game, selected: bool, tick_phase: f32, index: usize) -> Element<'a, Message> {
    let pulse = if selected {
        (tick_phase.sin() + 1.0) * 0.5
    } else {
        0.0
    };
    let border_color = if selected {
        Color::from_rgba(0.12, 0.85, 0.20, 0.75 + pulse * 0.25)
    } else {
        rgb(0.10, 0.10, 0.10)
    };
    let border_width = if selected { 2.0 + pulse * 1.5 } else { 1.0 };

    let tile = container(
        column![
            Space::new().height(Length::Fill),
            container(text(game.title).size(10))
                .width(Length::Fill)
                .padding([10, 8])
                .style(|_| container::Style {
                    background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.55))),
                    text_color: Some(Color::WHITE),
                    ..container::Style::default()
                })
        ],
    )
    .width(150)
    .height(214)
    .style(move |_| container::Style {
        background: Some(Background::Color(game.color)),
        border: Border {
            color: border_color,
            width: border_width,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    });

    button(tile)
        .padding(0)
        .on_press(Message::SelectGame(index))
        .into()
}

const fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color::from_rgb(r, g, b)
}
