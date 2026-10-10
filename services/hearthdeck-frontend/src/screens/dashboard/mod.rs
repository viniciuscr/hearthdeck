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
use cosmic::iced::{ContentFit, Length};
use cosmic::theme;
use cosmic::widget::{Id, column, icon, scrollable};

use crate::app::Message;
use crate::style::{
    DASHBOARD_GAME_ASPECT, content_horizontal_padding, dashboard_console_tile_size,
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
    pub message: Message,
}

impl Card {
    pub fn new(id: Id, label: impl Into<String>, icon: icon::Handle, message: Message) -> Self {
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

/// The page: every shelf as a rail, stacked and scrolled.
///
/// `on_rail_scroll` is handed each rail's own horizontal offset as it moves,
/// because only the shell can remember it - and a rail needs its current offset
/// to know whether the card the cursor lands on would leave the view.
#[must_use]
pub fn view<'a>(
    shelves: &[Shelf],
    window_width: f32,
    on_rail_scroll: impl Fn(usize, f32) -> Message + Clone + 'a,
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
                    rail_item(card.id.clone(), card.label.clone(), card.icon.clone(), size)
                        .aspect(aspect)
                        .fit(fit)
                        .on_press(card.message.clone())
                        .into_element()
                }))
                .on_scroll(move |viewport| scroll(index, viewport.absolute_offset().x))
                .into()
        })
        .collect();

    scrollable(
        column(rails)
            .spacing(space_xl)
            .padding([space_l, padding, space_xl, padding]),
    )
    .id(PAGE_ID.clone())
    .width(Length::Fill)
    .height(Length::Fill)
    .scrollbar_width(0)
    .scroller_width(0)
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
