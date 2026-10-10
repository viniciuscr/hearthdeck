//! The dashboard: the page the shell opens on, a stack of rails.
//!
//! The rails themselves are [`crate::widgets::rail`] - a title and a scrolling
//! line of cards, one look for every shelf - so what this module owns is the
//! page they sit on and the shape of the cards the shell hands it. [`focus`]
//! owns how the pad walks them, and the scroll that keeps the cursor on screen.
//!
//! The shelves drawn so far are the ones the shell can already fill from what it
//! holds. Recently played, favourites, continue watching and provider health
//! each add a rail here as their data lands; none of them is a page of its own.

pub mod focus;

use std::sync::LazyLock;

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::widget::scrollable::{AbsoluteOffset, scroll_to};
use cosmic::iced::{Alignment, ContentFit, Length};
use cosmic::theme;
use cosmic::widget::{Id, button, column, container, icon, row, scrollable, space, text};

use crate::app::Message;
use crate::fl;
use crate::providers::daemon::{
    CONTINUE_WATCHING_COLLECTION, Collection, FAVORITES_COLLECTION, LAST_PLAYED_COLLECTION,
    PLAY_LATER_COLLECTION,
};
use crate::style::{
    DASHBOARD_GAME_ASPECT, ICON_BODY, ICON_LARGE, TEXT_CAPTION, TEXT_HEADER,
    content_horizontal_padding, dashboard_console_tile_size, dashboard_nav_button_class,
    dashboard_tile_size, sidebar_width,
};
use crate::widgets::rail::{rail, rail_item};

/// The page's own scrollable. Held here because the page is its widget, and this
/// module is what knows the page scrolls vertically and nothing else.
static PAGE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("dashboard"));

/// What one card shows and what pressing it does.
#[derive(Clone)]
pub struct Card {
    /// The widget id, so the pad can be pointed at this card.
    pub id: Id,
    pub label: String,
    pub icon: icon::Handle,
    /// What pressing the card does, or `None` when it has nowhere to go - an
    /// item the library does not hold. Such a card is still drawn, because the
    /// rail it is in is worth reading either way, but it does not look live
    /// while doing nothing.
    pub message: Option<Message>,
}

impl Card {
    pub fn new(
        id: Id,
        label: impl Into<String>,
        icon: icon::Handle,
        message: Option<Message>,
    ) -> Self {
        Self {
            id,
            label: label.into(),
            icon,
            message,
        }
    }
}

/// One rail: what it is called and the cards in it.
#[derive(Clone)]
pub struct Shelf {
    /// Whether the cards are console art, which is drawn at half the size,
    /// square, and contained so its pixel edges are not cropped. Every other
    /// shelf is a row of portrait game covers, cropped to fill.
    pub consoles: bool,
    pub title: String,
    pub cards: Vec<Card>,
}

impl Shelf {
    pub fn new(consoles: bool, title: impl Into<String>, cards: Vec<Card>) -> Self {
        Self {
            consoles,
            title: title.into(),
            cards,
        }
    }
}

/// The widget id of a rail, which is what a scroll to it addresses.
#[must_use]
pub fn rail_id(index: usize) -> Id {
    Id::new(format!("dashboard-rail-{index}"))
}

/// What a rail is called.
///
/// This client knows the slugs it ships with and shows their translated labels.
/// Anything else uses the name the daemon sent with the collection, because
/// nobody has translated a rail that did not exist when this client shipped -
/// which is exactly what a feature's own categories, and so the AI's, are. A
/// slug with neither falls back to itself rather than to a blank heading.
#[must_use]
pub fn rail_title(collection: &Collection) -> String {
    match collection.slug.as_str() {
        FAVORITES_COLLECTION => fl!("favorites"),
        PLAY_LATER_COLLECTION => fl!("play-later"),
        LAST_PLAYED_COLLECTION => fl!("recently-played"),
        CONTINUE_WATCHING_COLLECTION => fl!("continue-watching"),
        _ => collection
            .name
            .clone()
            .unwrap_or_else(|| collection.slug.clone()),
    }
}

/// The dashboard's top bar: who is signed in, and the destinations.
///
/// The dashboard is a sibling of the library rather than a page inside it, so it
/// carries its own chrome. The library's sidebar belongs to the library; drawing
/// it here would make the dashboard read as a section of it.
pub struct TopBar {
    pub user_name: String,
    pub app_icon: icon::Handle,
    pub on_home: Message,
    pub on_library: Message,
    pub on_search: Message,
}

/// The page: the top bar, then every shelf as a rail, stacked and scrolled.
///
/// `on_rail_scroll` is handed each rail's own horizontal offset as it moves, and
/// `on_page_scroll` the page's vertical one, because only the shell can remember
/// them - a rail needs its own offset, and the page the focused rail's place in
/// the stack, to know whether the cursor would leave the view.
#[must_use]
pub fn view<'a>(
    shelves: &[Shelf],
    top_bar: TopBar,
    window_width: f32,
    on_rail_scroll: impl Fn(usize, f32) -> Message + Clone + 'a,
    on_page_scroll: impl Fn(f32) -> Message + 'a,
) -> Element<'a, Message> {
    let Spacing {
        space_l, space_xl, ..
    } = theme::spacing();
    let padding = content_horizontal_padding();

    let rails: Vec<Element<'a, Message>> = shelves
        .iter()
        .enumerate()
        .map(|(index, shelf)| {
            let (size, aspect, fit) = shelf_metrics(shelf, window_width);
            let scroll = on_rail_scroll.clone();
            rail(rail_id(index), shelf.title.clone(), size)
                .items(shelf.cards.iter().map(|card| {
                    let item =
                        rail_item(card.id.clone(), card.label.clone(), card.icon.clone(), size)
                            .aspect(aspect)
                            .fit(fit);
                    match card.message.clone() {
                        Some(message) => item.on_press(message),
                        None => item,
                    }
                    .into_element()
                }))
                .on_scroll(move |viewport| scroll(index, viewport.absolute_offset().x))
                .into()
        })
        .collect();

    let page = scrollable(
        column(rails)
            .spacing(space_xl)
            .padding([space_l, padding, space_xl, padding]),
    )
    .id(PAGE_ID.clone())
    .on_scroll(move |viewport| on_page_scroll(viewport.absolute_offset().y))
    .width(Length::Fill)
    .height(Length::Fill)
    .scrollbar_width(0)
    .scroller_width(0);

    column![
        container(top_bar_element(&top_bar, space_xl)).padding([space_l, padding, 0, padding]),
        page,
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// The top bar itself: the profile on the left, the destinations centered, and
/// the space the system status goes in on the right.
fn top_bar_element<'a>(bar: &TopBar, button_size: u16) -> Element<'a, Message> {
    let Spacing {
        space_xxs,
        space_xs,
        space_s,
        space_l,
        space_xxl,
        ..
    } = theme::spacing();
    let button_size = f32::from(button_size);

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
    let nav = |icon_name: &'static str, selected: bool, message: Message| {
        button::custom(icon::icon(icon::from_name(icon_name).into()).size(ICON_BODY))
            .width(Length::Fixed(button_size))
            .height(Length::Fixed(button_size))
            .class(dashboard_nav_button_class(selected))
            .on_press(message)
    };

    let navigation = row![
        // Home reads as the destination showing, because it is: this screen is
        // where the app opens.
        nav("go-home-symbolic", true, bar.on_home.clone()),
        nav("view-grid-symbolic", false, bar.on_library.clone()),
        nav("system-search-symbolic", false, bar.on_search.clone()),
    ]
    .spacing(space_xs)
    .align_y(Alignment::Center);

    row![
        profile,
        navigation,
        // Where the system status goes. It is not wired yet, so the space is
        // reserved rather than filled with something that is not true.
        space::horizontal().width(Length::FillPortion(1)),
    ]
    .spacing(space_l)
    .align_y(Alignment::Center)
    .height(Length::Fixed(f32::from(space_xxl)))
    .into()
}

/// The width, aspect and artwork fit one shelf's cards are laid out at.
///
/// One home for all three, so where a card is drawn and where the scroll thinks
/// it is cannot disagree.
fn shelf_metrics(shelf: &Shelf, window_width: f32) -> (f32, f32, ContentFit) {
    let Spacing { space_l, .. } = theme::spacing();
    let padding = content_horizontal_padding();
    if shelf.consoles {
        (
            dashboard_console_tile_size(window_width, padding, space_l),
            1.0,
            ContentFit::Contain,
        )
    } else {
        (
            dashboard_tile_size(window_width, padding, space_l),
            DASHBOARD_GAME_ASPECT,
            ContentFit::Cover,
        )
    }
}

/// The pitch a card of `shelf` sits at: its width, and the gap beside it.
#[must_use]
pub fn card_pitch(shelf: &Shelf, window_width: f32) -> (f32, f32) {
    let (size, aspect, _) = shelf_metrics(shelf, window_width);
    let Spacing { space_l, .. } = theme::spacing();
    (size * aspect, f32::from(space_l))
}

/// How tall each rail is in order, which is what the page scroll measures
/// against to keep the focused one on screen.
#[must_use]
pub fn rail_heights(shelves: &[Shelf], window_width: f32) -> Vec<f32> {
    let Spacing { space_m, .. } = theme::spacing();
    shelves
        .iter()
        .map(|shelf| {
            let (size, _, _) = shelf_metrics(shelf, window_width);
            RAIL_TITLE_HEIGHT + f32::from(space_m) + size
        })
        .collect()
}

/// The gap between two rails on the page.
#[must_use]
pub fn rail_gap() -> f32 {
    f32::from(theme::spacing().space_xl)
}

/// The height the top bar takes above the rails, which is what the page's viewport
/// is the window's height less.
#[must_use]
pub fn top_bar_height() -> f32 {
    let Spacing {
        space_l, space_xxl, ..
    } = theme::spacing();
    f32::from(space_l) + f32::from(space_xxl)
}

/// Scrolls the page to a vertical offset, leaving the rails where they are
/// horizontally.
pub fn scroll_to_page(offset: f32) -> cosmic::app::Task<Message> {
    scroll_to(
        PAGE_ID.clone(),
        AbsoluteOffset {
            x: None,
            y: Some(offset),
        },
    )
}

/// The height a rail's title takes above its cards.
///
/// The renderer measures the text itself, so this cannot be derived here - the
/// same reason the grid keeps an estimate of its header for its first frame. It
/// is deliberately generous: over-estimating scrolls slightly further than
/// needed, while under-estimating can leave the focused rail off screen.
const RAIL_TITLE_HEIGHT: f32 = 40.0;

/// The width a rail lays its cards out in, near enough to scroll one into view
/// with.
///
/// The scrollable would report its own width, but only once it has scrolled, and
/// a rail nobody has scrolled has none - so this is derived from the window the
/// same way [`dashboard_tile_size`] already derives a card, which is the same
/// approximation. The scrollable clamps what it is handed, so being a few pixels
/// out costs a few pixels of over-scroll rather than a card the pad cannot reach.
#[must_use]
pub fn rail_viewport(window_width: f32) -> f32 {
    (window_width - sidebar_width(window_width) - 2.0 * f32::from(content_horizontal_padding()))
        .max(1.0)
}
