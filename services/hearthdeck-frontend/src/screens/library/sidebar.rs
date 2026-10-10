//! The library's fixed left navigation: the app header, the section list and the
//! storage footer.
//!
//! Extracted from `app.rs` as the first slice of the screen migration. It owns
//! only its view; the section state and the "select a section" message still
//! live in [`crate::app`], which is where navigation is routed.

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::Alignment;
use cosmic::iced::Length;
use cosmic::iced::alignment::Vertical;
use cosmic::theme;
use cosmic::widget::{Id, button, column, container, icon, row, space, text};

use crate::app_group::Section;
use crate::fl;
use crate::style::{
    ICON_BODY, ICON_LARGE, SIDEBAR_ACCENT_BAR_WIDTH, TEXT_BODY, TEXT_CAPTION, TEXT_HEADER,
    TEXT_LARGE, accent_bar, section_button_class, sidebar_accent_bar_height, sidebar_header_height,
    sidebar_item_height, sidebar_width,
};
use crate::ui;

/// State the sidebar draws from.
pub struct Sidebar<'a> {
    /// The section the grid is showing, drawn as the active item.
    pub current: Section,
    pub user_name: &'a str,
    pub disk_free: &'a str,
    /// The app logo, drawn in the header.
    pub app_icon: icon::Handle,
    pub window_width: f32,
}

#[must_use]
pub fn view<'a, Message: Clone + 'a>(
    sidebar: Sidebar<'a>,
    on_select: impl Fn(Section) -> Message + 'a,
) -> Element<'a, Message> {
    let Spacing {
        space_xs,
        space_m,
        space_l,
        space_xxs,
        ..
    } = theme::spacing();
    let current = sidebar.current;

    let section_button = |section: Section| {
        let is_active = current == section;
        let label = container(
            row![
                ui::icon(section.icon_name(), ICON_BODY),
                text(section.name()).size(TEXT_LARGE),
            ]
            .spacing(space_m)
            .align_y(Alignment::Center),
        )
        .align_y(Vertical::Center)
        .width(Length::Fill)
        .padding([0, space_l]);

        let content = if is_active {
            row![
                label,
                container(space::horizontal().width(Length::Fixed(SIDEBAR_ACCENT_BAR_WIDTH)))
                    .width(Length::Fixed(SIDEBAR_ACCENT_BAR_WIDTH))
                    .height(Length::Fixed(sidebar_accent_bar_height()))
                    .class(theme::Container::Custom(Box::new(accent_bar))),
            ]
            .align_y(Alignment::Center)
        } else {
            row![label]
        };

        button::custom(
            container(content)
                .align_y(Vertical::Center)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .height(Length::Fixed(sidebar_item_height()))
        .width(Length::Fill)
        .class(section_button_class(is_active))
        .id(section_id(section))
        .on_press(on_select(section))
    };

    let header = container(
        row![
            icon::icon(sidebar.app_icon).size(ICON_LARGE),
            container(text(sidebar.user_name).size(TEXT_HEADER))
                .align_y(Vertical::Center)
                .width(Length::Fill),
        ]
        .spacing(space_m)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(sidebar_header_height()))
    .align_y(Vertical::Center)
    .padding([0, space_l]);

    let storage = container(
        row![
            ui::icon("drive-harddisk-solidstate-symbolic", ICON_BODY),
            column![
                text::caption(fl!("storage-available")).size(TEXT_CAPTION),
                text::body(sidebar.disk_free).size(TEXT_BODY),
            ]
            .spacing(space_xxs),
        ]
        .spacing(space_xs)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([space_m, space_l, space_l, space_l]);

    container(
        column![
            header,
            section_button(Section::PcGames),
            section_button(Section::ConsoleGames),
            section_button(Section::Applications),
            space::vertical().height(Length::Fill),
            storage,
        ]
        .spacing(space_xs),
    )
    .width(Length::Fixed(sidebar_width(sidebar.window_width)))
    .height(Length::Fill)
    // Inset the navigation items from the window edge and the divider so the
    // selected chip reads as a rounded pill instead of a full-bleed bar.
    .padding([0, space_xs, space_m, space_xs])
    .into()
}

/// The widget id of a sidebar entry.
///
/// The navigation addresses the sections through these, so the ids and the
/// order the sidebar draws them in have to stay the same list: `Section::ALL`,
/// whose indices they are keyed on.
pub fn section_id(section: Section) -> Id {
    Id::new(format!("section-{}", section.index()))
}
