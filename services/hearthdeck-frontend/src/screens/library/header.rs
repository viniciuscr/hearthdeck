//! The library's header: the section/group title, the search box, and the group
//! tab strip.
//!
//! Tabs are read-only: a title, a search field and a row of category buttons.
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
use cosmic::widget::{Id, button, column, container, row, scrollable, space, text, text_input};

use crate::app::Message;
use crate::app_group::{AppGroup, Section, SectionUi};
use crate::fl;
use crate::style::{
    FILTER_BUTTON_MIN_WIDTH, ICON_BODY, ICON_SEARCH, SEARCH_WIDTH, TEXT_BODY, TEXT_HEADER,
    TEXT_TITLE, accent_bar, content_width, control_button_class, filter_button_height, passthrough,
    search_icon_padding, tab_button_class, tab_height, tab_underline_height, tab_width,
};
use crate::text_entry::Field;
use crate::ui;

static SEARCH_ID: LazyLock<Id> = LazyLock::new(|| Id::new("search"));
static STRIP_ID: LazyLock<Id> = LazyLock::new(|| Id::new("tab-strip"));
static SEARCH_PLACEHOLDER: LazyLock<String> = LazyLock::new(|| fl!("search-placeholder"));

/// The widget id of the search box, which the navigation puts the cursor on.
pub fn search_id() -> Id {
    SEARCH_ID.clone()
}

/// The widget id of the tab strip, which is what a scroll to it addresses.
#[must_use]
pub fn tab_strip_id() -> Id {
    STRIP_ID.clone()
}

/// The widget id of a tab: 0 is the section's "all" tab, and `1 + i` is its
/// `i`th group - the same numbering the navigation's cursor uses.
pub fn tab_id(index: usize) -> Id {
    Id::new(format!("tab-{index}"))
}

/// The strip position of a group's tab, which is the numbering [`tab_id`] and
/// the navigation's cursor share: 0 is the section's "all" tab, `1 + i` its
/// `i`th group.
#[must_use]
pub fn tab_index(group: Option<usize>) -> usize {
    group.map_or(0, |index| index + 1)
}

/// The width each tab is laid out at, in strip order: the section's "all" tab
/// first, then its groups.
///
/// [`tab_width`] is an estimate, because the strip is not measured until it is
/// drawn - the same approximation [`tab_viewport`] makes from the other side.
/// The follow only needs to know roughly where a tab sits, and the scrollable
/// clamps what it is handed.
#[must_use]
pub fn tab_widths(section: Section, groups: &[AppGroup]) -> Vec<f32> {
    std::iter::once(section.all_name())
        .chain(groups.iter().map(AppGroup::name))
        .map(|label| tab_width(&label))
        .collect()
}

/// The horizontal offset that brings tab `index` into the strip's view, or
/// `None` when it is already there and the strip should be left alone.
///
/// The strip follows the rule the dashboard's rails follow: it moves only when
/// the tab the selection lands on would otherwise leave the visible width, and
/// then it pins that tab to the nearest edge, so the tabs slide under a
/// selection that stays where the user is looking. Clamped to the strip's own
/// extent, so the last tab stops at the end instead of scrolling past it.
#[must_use]
pub fn tab_scroll_target(widths: &[f32], index: usize, offset: f32, viewport: f32) -> Option<f32> {
    let gap = tab_gap();
    let start = widths.iter().take(index).sum::<f32>() + index as f32 * gap;
    let end = start + *widths.get(index)?;
    let target = if start < offset {
        // It left through the leading edge.
        start
    } else if end > offset + viewport {
        // It is leaving through the trailing edge.
        end - viewport
    } else {
        return None;
    };
    let content = widths.iter().sum::<f32>() + widths.len().saturating_sub(1) as f32 * gap;
    Some(target.clamp(0.0, (content - viewport).max(0.0)))
}

/// The visible width of the tab strip.
///
/// The strip shares its row with the filter button when the section offers one,
/// and sits inside the content column's horizontal padding - so, like
/// [`crate::screens::dashboard::rail_viewport`], this is derived from the window
/// rather than measured. The scrollable clamps what it is handed, so being a few
/// pixels out costs a little over-scroll rather than a tab the pad cannot reach.
#[must_use]
pub fn tab_viewport(window_width: f32, show_filter: bool) -> f32 {
    let filter = if show_filter {
        FILTER_BUTTON_MIN_WIDTH + tab_gap()
    } else {
        0.0
    };
    (content_width(window_width) - filter).max(1.0)
}

/// The space between two tabs, which the strip's row also puts between itself
/// and the filter button.
fn tab_gap() -> f32 {
    f32::from(theme::spacing().space_m)
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
                // The keyboard follows the caret, so a focused box means the
                // pad belongs to the keyboard until it is dismissed. Enter is
                // how the field accepts what was typed; the shell decides what
                // that means for a search box.
                .on_focus(Message::TextFieldFocused(Field::Search))
                .on_unfocus(Message::TextFieldBlurred(Field::Search))
                .on_submit(|_| Message::TextFieldSubmitted(Field::Search))
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

    // The strip is one horizontal scrollable: a section with more tabs than the
    // window is wide still reaches every one of them, and the follow
    // (`App::reveal_tab`) keeps the selected tab on screen. No scrollbar - the
    // accent underline already says where the user is.
    let strip = scrollable::horizontal(tabs)
        .id(tab_strip_id())
        .width(Length::Fill)
        .height(Length::Fixed(tab_height()))
        .scrollbar_width(0)
        .scroller_width(0)
        .on_scroll(|viewport| Message::TabsScrolled(viewport.absolute_offset().x));

    let mut controls = row![strip]
        .width(Length::Fill)
        .spacing(space_m)
        .align_y(Alignment::Center);

    // The filter button sits at the end of the tab row, on the same baseline, so
    // it reads as one more control over this grid. It stays put while the tabs
    // scroll under it. Tinted while any facet is narrowed, so the grid's state is
    // legible without opening the sidebar.
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

        controls = controls.push(filter_button);
    }

    column![
        container(top_bar).padding([space_l, 0, 0, 0]),
        // Keep the tab strip visually attached to its title rather than a full
        // gap's distance below it.
        space::vertical().height(space_m),
        container(controls).padding([0, 0, space_s, 0]),
    ]
    .width(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::{tab_scroll_target, tab_viewport, tab_widths};
    use crate::app_group::{AppGroup, FilterType, GroupSource, Section, SectionUi};
    use crate::style::{FILTER_BUTTON_MIN_WIDTH, content_width, tab_width};

    /// The gap the strip's row puts between two tabs, which the follow measures
    /// with.
    fn gap() -> f32 {
        f32::from(cosmic::theme::spacing().space_m)
    }

    /// Eight tabs of one width each, so a test's arithmetic is the strip's.
    fn widths(count: usize) -> Vec<f32> {
        vec![120.0; count]
    }

    /// Where tab `index` starts, which every assertion about an edge is stated
    /// against.
    fn start(index: usize) -> f32 {
        index as f32 * (120.0 + gap())
    }

    fn group(name: &str) -> AppGroup {
        AppGroup {
            name: name.to_owned(),
            icon: "folder-symbolic".to_owned(),
            filter: FilterType::AppIds(Vec::new()),
            source: GroupSource::Local,
        }
    }

    #[test]
    fn a_tab_inside_the_strip_drags_nothing() {
        // The second tab sits well inside a 400px strip: moving the row for it
        // would be the row moving rather than the selection.
        assert_eq!(tab_scroll_target(&widths(8), 1, 0.0, 400.0), None);
    }

    #[test]
    fn a_tab_leaving_the_trailing_edge_is_pinned_to_it() {
        let target =
            tab_scroll_target(&widths(8), 3, 0.0, 400.0).expect("the fourth tab leaves the strip");

        // Its trailing edge comes to rest on the strip's trailing edge, and the
        // tab before it is still inside.
        assert!(
            (target - (start(3) + 120.0 - 400.0)).abs() < 0.01,
            "{target}"
        );
        assert!(start(2) >= target, "the tab before it stays visible");
    }

    #[test]
    fn a_tab_leaving_the_leading_edge_is_pinned_to_it() {
        let target =
            tab_scroll_target(&widths(8), 2, 500.0, 400.0).expect("the third tab left the strip");

        assert!((target - start(2)).abs() < 0.01, "{target}");
    }

    #[test]
    fn the_strip_stops_at_its_last_tab() {
        // Pinning the eighth tab to the trailing edge would scroll past the end
        // of the row; it stops with that tab flush against it instead.
        let target =
            tab_scroll_target(&widths(8), 7, 0.0, 400.0).expect("the last tab is off screen");
        let content = 8.0 * 120.0 + 7.0 * gap();

        assert!((target - (content - 400.0)).abs() < 0.01, "{target}");
        assert!(
            start(7) + 120.0 <= target + 400.0 + 0.01,
            "the last tab fits"
        );
    }

    #[test]
    fn a_strip_shorter_than_its_viewport_never_scrolls() {
        // Three tabs fit in this viewport, so even the last one has nowhere to
        // go.
        assert_eq!(tab_scroll_target(&widths(3), 2, 0.0, 700.0), None);
    }

    #[test]
    fn the_filter_button_takes_its_width_from_the_strip() {
        let window_width = 1200.0;
        let with_button = tab_viewport(window_width, true);

        assert_eq!(
            tab_viewport(window_width, false),
            content_width(window_width)
        );
        assert_eq!(
            with_button,
            content_width(window_width) - FILTER_BUTTON_MIN_WIDTH - gap(),
            "the filter button is pinned beside the strip, not over it"
        );
    }

    #[test]
    fn the_widths_follow_the_strips_order() {
        let groups = [group("Steam"), group("GOG")];
        let widths = tab_widths(Section::PcGames, &groups);

        assert_eq!(widths.len(), 3, "the section's all tab, then its groups");
        assert_eq!(widths[0], tab_width(&Section::PcGames.all_name()));
        assert_eq!(widths[1], tab_width("Steam"));
        assert_eq!(widths[2], tab_width("GOG"));
    }
}
