//! The library's header: the section/group title, the search box, and the group
//! tab strip.
//!
//! Rewritten from the inherited `view_main_content` header. Deliberately gone:
//! the group **rename** controls (edit button, inline input, virtual-keyboard
//! round-trip) and the **draggable/reorderable** tab strip. What is left is a
//! title, a search field and a row of tabs.
//!
//! Tied to [`crate::app::Message`] — the new app's own message — not made
//! generic, because it emits several distinct messages and threading a closure
//! per message reads worse than naming them here.

use std::sync::LazyLock;

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::Alignment;
use cosmic::iced::Length;
use cosmic::iced::alignment::Vertical;
use cosmic::theme::{self, TextInput};
use cosmic::widget::{Id, button, column, container, row, space, text, text_input};

use crate::app::Message;
use crate::app_group::{AppGroup, Section};
use crate::fl;
use crate::style::{
    FILTER_BUTTON_MIN_WIDTH, ICON_BODY, ICON_SEARCH, SEARCH_WIDTH, TEXT_BODY, TEXT_HEADER,
    TEXT_TITLE, accent_bar, control_button_class, filter_button_height, passthrough,
    search_icon_padding, tab_button_class, tab_height, tab_underline_height, tab_width,
};
use crate::ui;

static SEARCH_ID: LazyLock<Id> = LazyLock::new(|| Id::new("search"));
static SEARCH_PLACEHOLDER: LazyLock<String> = LazyLock::new(|| fl!("search-placeholder"));

/// The widget id of the search box, which the navigation puts the cursor on.
pub fn search_id() -> Id {
    SEARCH_ID.clone()
}

/// The widget id of a tab: 0 is the section's "all" tab, and `1 + i` is its
/// `i`th group - the same numbering the navigation's cursor uses.
pub fn tab_id(index: usize) -> Id {
    Id::new(format!("tab-{index}"))
}

/// The widget id of the filter button, which opens the filter drawer.
pub fn filter_button_id() -> Id {
    Id::new("filter-button")
}

/// What the header draws from.
pub struct Header<'a> {
    pub section: Section,
    /// The focused group, if the section is not showing "all apps".
    pub selected_group: Option<usize>,
    pub groups: &'a [AppGroup],
    pub search_value: &'a str,
    /// How many facets are narrowed, for the filter button's badge.
    pub filter_active: usize,
    /// Whether the section offers any facets to filter by.
    pub show_filter: bool,
}

#[must_use]
pub fn view(header: Header<'_>) -> Element<'_, Message> {
    let Spacing {
        space_none,
        space_s,
        space_m,
        space_l,
        ..
    } = theme::spacing();

    let title = header
        .selected_group
        .and_then(|index| header.groups.get(index))
        .map_or_else(|| header.section.name(), AppGroup::name);

    let top_bar = row![
        container(text(title).size(TEXT_TITLE)).align_y(Vertical::Center),
        space::horizontal().width(Length::FillPortion(1)),
        container(
            text_input(SEARCH_PLACEHOLDER.as_str(), header.search_value)
                .on_input(Message::InputChanged)
                .style(TextInput::Search)
                .width(Length::Fixed(SEARCH_WIDTH))
                .size(TEXT_HEADER)
                .padding([space_s, space_m])
                .leading_icon(
                    container(ui::icon("system-search-symbolic", ICON_SEARCH))
                        .padding(search_icon_padding())
                        .into(),
                )
                .id(SEARCH_ID.clone()),
        )
        .align_y(Vertical::Center),
    ]
    .align_y(Alignment::Center)
    .spacing(space_s);

    // One tab builder for the locked "all apps" tab and every group tab. No
    // drag: the tabs are plain buttons.
    let tab = |index: usize, label: String, is_active: bool, on_press: Message| {
        let width = tab_width(&label);
        let tab_btn = button::custom(
            container(text::body(label).size(TEXT_BODY))
                .align_x(Alignment::Center)
                .align_y(Vertical::Center)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding([space_none, space_m]),
        )
        .width(Length::Shrink)
        .height(Length::Fill)
        .class(tab_button_class(is_active))
        .id(tab_id(index))
        .on_press(on_press);

        let underline = if is_active {
            container(space::horizontal().width(Length::Fixed(1.0)))
                .width(Length::Fill)
                .height(Length::Fixed(tab_underline_height()))
                .class(theme::Container::Custom(Box::new(accent_bar)))
        } else {
            container(space::horizontal())
                .width(Length::Fill)
                .height(Length::Fixed(tab_underline_height()))
        };

        column![tab_btn, underline]
            .width(Length::Shrink)
            .height(Length::Fixed(tab_height()))
            .max_width(width)
            .align_x(Alignment::Center)
    };

    let mut tabs = row![tab(
        0,
        header.section.all_name(),
        header.selected_group.is_none(),
        Message::SelectGroup(None),
    )]
    .spacing(space_m)
    .align_y(Alignment::Center);
    for (index, group) in header.groups.iter().enumerate() {
        tabs = tabs.push(tab(
            index + 1,
            group.name(),
            header.selected_group == Some(index),
            Message::SelectGroup(Some(index)),
        ));
    }

    // The filter button sits at the end of the tab row, on the same baseline, so
    // it reads as one more control over this grid. Tinted while any facet is
    // narrowed, so the grid's state is legible without opening the sidebar.
    if header.show_filter {
        let label = if header.filter_active > 0 {
            fl!("filter-count", count = header.filter_active)
        } else {
            fl!("filter")
        };
        let filter_button = button::custom(
            container(
                row![
                    ui::icon("view-filter-symbolic", ICON_BODY),
                    text::body(label).size(TEXT_BODY),
                ]
                .spacing(space_s)
                .align_y(Alignment::Center),
            )
            .align_x(Alignment::Center)
            .align_y(Vertical::Center)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding([space_none, space_m])
            // The control class paints the label; keep this container from
            // overriding it with the default foreground.
            .class(theme::Container::Custom(Box::new(passthrough))),
        )
        .height(Length::Fixed(filter_button_height()))
        .width(Length::Fixed(FILTER_BUTTON_MIN_WIDTH))
        .class(control_button_class(header.filter_active > 0))
        .id(filter_button_id())
        .on_press(Message::ToggleFilterPanel);

        tabs = tabs
            .push(space::horizontal().width(Length::Fill))
            .push(filter_button);
    }

    column![
        container(top_bar).padding([space_l, 0, 0, 0]),
        // Keep the tab strip visually attached to its title rather than a full
        // gap's distance below it.
        space::vertical().height(space_m),
        container(tabs).padding([0, 0, space_s, 0]),
    ]
    .width(Length::Fill)
    .into()
}
