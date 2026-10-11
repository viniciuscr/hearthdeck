//! The bar across the top of a top-level destination.
//!
//! The dashboard and the settings screen are siblings, and they wear the same
//! chrome: who is signed in on the left, the destinations centered, and the
//! system status on the right. It lives here rather than in either screen so the
//! two cannot drift apart - and so the next thing added to it is added once.

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::{Alignment, Length};
use cosmic::theme;
use cosmic::widget::{button, column, icon, row, space, text};

use crate::app::Message;
use crate::style::{ICON_BODY, ICON_LARGE, TEXT_CAPTION, TEXT_HEADER, nav_button_class};

/// A destination the top bar opens.
///
/// Every screen the app has, plus the search box's own way in. The screen
/// drawing the bar is the one that reads as current; search is a way *into* the
/// library rather than a screen, so it never does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Dashboard,
    Library,
    Search,
    Settings,
}

impl Destination {
    /// All of them, left to right, which is also the order they are drawn.
    const ALL: [Self; 4] = [Self::Dashboard, Self::Library, Self::Search, Self::Settings];

    /// The icon a button for this destination wears.
    fn icon_name(self) -> &'static str {
        match self {
            Self::Dashboard => "go-home-symbolic",
            Self::Library => "view-grid-symbolic",
            Self::Search => "system-search-symbolic",
            Self::Settings => "preferences-system-symbolic",
        }
    }
}

/// The bar's contents.
pub struct TopBar {
    /// The destination showing, drawn as the current button.
    pub current: Destination,
    pub user_name: String,
    /// The app logo, drawn beside the name.
    pub app_icon: icon::Handle,
}

/// The bar, with `on_navigate` called for whichever destination is pressed.
#[must_use]
pub fn view<'a>(bar: &TopBar, on_navigate: fn(Destination) -> Message) -> Element<'a, Message> {
    let Spacing {
        space_xxs,
        space_xs,
        space_s,
        space_l,
        space_xl,
        space_xxl,
        ..
    } = theme::spacing();
    let button_size = f32::from(space_xl);

    let profile = row![
        icon::icon(bar.app_icon.clone()).size(ICON_LARGE),
        column![
            text::body(bar.user_name.clone()).size(TEXT_HEADER),
            text::caption("Hearthdeck").size(TEXT_CAPTION),
        ]
        .spacing(space_xxs),
    ]
    .spacing(space_s)
    .align_y(Alignment::Center)
    .width(Length::FillPortion(1));

    // One builder for every destination, so the buttons cannot drift apart.
    let nav = |destination: Destination| {
        button::custom(icon::icon(icon::from_name(destination.icon_name()).into()).size(ICON_BODY))
            .width(Length::Fixed(button_size))
            .height(Length::Fixed(button_size))
            .class(nav_button_class(destination == bar.current))
            .on_press(on_navigate(destination))
    };

    let navigation = Destination::ALL.into_iter().fold(
        row![].spacing(space_xs).align_y(Alignment::Center),
        |navigation, destination| navigation.push(nav(destination)),
    );

    row![
        profile,
        navigation,
        // Where the system status goes: wifi and bluetooth, which nothing reads
        // yet. The space is reserved rather than filled with something untrue.
        space::horizontal().width(Length::FillPortion(1)),
    ]
    .spacing(space_l)
    .align_y(Alignment::Center)
    .height(Length::Fixed(f32::from(space_xxl)))
    .into()
}

/// The height the bar takes above a page, which is what that page's viewport is
/// the window's height less.
#[must_use]
pub fn height() -> f32 {
    let Spacing {
        space_l, space_xxl, ..
    } = theme::spacing();
    f32::from(space_l) + f32::from(space_xxl)
}
