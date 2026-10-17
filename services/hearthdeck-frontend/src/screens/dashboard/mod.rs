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
use cosmic::iced::{ContentFit, Length};
use cosmic::theme;
use cosmic::widget::{Id, column, container, icon, scrollable};

use crate::app::Message;
use crate::fl;
use crate::providers::daemon::{
    CONTINUE_WATCHING_COLLECTION, Collection, FAVORITES_COLLECTION, LAST_PLAYED_COLLECTION,
    PLAY_LATER_COLLECTION,
};
use crate::style::{
    DASHBOARD_GAME_ASPECT, content_horizontal_padding, dashboard_console_tile_size,
    dashboard_tile_size, sidebar_width,
};
use crate::widgets::application::{LABEL_HEIGHT, Selection, TileLabel, app_label, app_tile};
use crate::widgets::rail::rail;
use crate::widgets::top_bar;

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
    /// What pressing the card does. Every rail the dashboard builds attaches a
    /// launch: the card's id goes to the daemon untouched and what that names
    /// is the daemon's answer to give - an unknown id comes back as a failed
    /// launch, not silence. `None` is only a card with nothing attached: still
    /// drawn, dim at rest, with the cursor ring but no effect when pressed.
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

/// The page: the shared top bar, then every shelf as a rail, stacked and
/// scrolled.
///
/// `on_navigate` belongs to the bar, which every top-level destination wears. The
/// two scroll callbacks belong to the shell, because only it can remember an
/// offset: a rail needs its own, and the page the focused rail's place in the
/// stack, to know whether the cursor would leave the view.
#[must_use]
pub fn view<'a>(
    shelves: &[Shelf],
    cursor: focus::Cursor,
    bar: &top_bar::TopBar<'_>,
    window_width: f32,
    on_navigate: fn(top_bar::Destination) -> Message,
    on_rail_scroll: impl Fn(usize, f32) -> Message + Clone + 'a,
    on_page_scroll: impl Fn(f32) -> Message + 'a,
) -> Element<'a, Message> {
    let Spacing {
        space_l, space_xl, ..
    } = theme::spacing();
    let padding = content_horizontal_padding();
    // The rails' card counts, which `card_selection` clamps the cursor against -
    // the same shape the pad's movement walks. Computed once so every card asks
    // the question against the same rails.
    let rail_lengths: Vec<usize> = shelves.iter().map(|shelf| shelf.cards.len()).collect();

    let rails: Vec<Element<'a, Message>> = shelves
        .iter()
        .enumerate()
        .map(|(index, shelf)| {
            let (size, aspect, fit) = shelf_metrics(shelf, window_width);
            let scroll = on_rail_scroll.clone();
            let rail_element: Element<'a, Message> =
                rail(rail_id(index), shelf.title.clone(), size)
                    .items(shelf.cards.iter().enumerate().map(|(card_index, card)| {
                        // The one shared tile. The rails differ from the grid
                        // only in geometry, in having no context menu, and in
                        // where the name is read: below the rail rather than on
                        // the art. The card under the cursor is marked here,
                        // which is what gives an inert movie card its ring.
                        app_tile(
                            card.id.clone(),
                            &card.label,
                            card.icon.clone(),
                            size * aspect,
                            size,
                            fit,
                            1,
                            None,
                            card.message.clone(),
                            None,
                            TileLabel::Elsewhere,
                            card_selection(cursor, &rail_lengths, index, card_index),
                        )
                    }))
                    .on_scroll(move |viewport| scroll(index, viewport.absolute_offset().x))
                    .into();
            column![
                rail_element,
                app_label(focused_label(cursor, &rail_lengths, index, &shelf.cards)),
            ]
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
        container(top_bar::view(bar, on_navigate)).padding([space_l, padding, 0, padding]),
        page,
    ]
    .width(Length::Fill)
    .height(Length::Fill)
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

/// Whether the card at `(rail_index, card_index)` is the one the dashboard
/// cursor is on.
///
/// A pure function of the cursor and the rails, so which card the ring lands on
/// is decided - and tested - without a compositor. The cursor is confirmed
/// against the rails first (`focus::confirm`), so a cursor left past the end of
/// a rail that shrank reads as the card it clamps onto, and a cursor on an empty
/// dashboard selects nothing at all.
#[must_use]
pub fn card_selection(
    cursor: focus::Cursor,
    rails: &[usize],
    rail_index: usize,
    card_index: usize,
) -> Selection {
    if focus::confirm(cursor, rails) == Some((rail_index, card_index)) {
        Selection::Selected
    } else {
        Selection::Unselected
    }
}

/// The name read out below one rail: the label of the card the cursor is on
/// in it, if the cursor is on this rail at all.
///
/// Clamped through [`focus::confirm`] the same way [`card_selection`] is, so
/// the name below the rail and the ring on the card cannot disagree.
#[must_use]
pub fn focused_label<'a>(
    cursor: focus::Cursor,
    rails: &[usize],
    rail_index: usize,
    cards: &'a [Card],
) -> Option<&'a str> {
    match focus::confirm(cursor, rails) {
        Some((rail, card_index)) if rail == rail_index => {
            cards.get(card_index).map(|card| card.label.as_str())
        }
        _ => None,
    }
}

/// How tall each rail is in order - its title, its cards, and the name line
/// below them - which is what the page scroll measures against to keep the
/// focused one on screen.
#[must_use]
pub fn rail_heights(shelves: &[Shelf], window_width: f32) -> Vec<f32> {
    let Spacing { space_m, .. } = theme::spacing();
    shelves
        .iter()
        .map(|shelf| {
            let (size, _, _) = shelf_metrics(shelf, window_width);
            RAIL_TITLE_HEIGHT + f32::from(space_m) + size + LABEL_HEIGHT
        })
        .collect()
}

/// The gap between two rails on the page.
#[must_use]
pub fn rail_gap() -> f32 {
    f32::from(theme::spacing().space_xl)
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

#[cfg(test)]
mod tests {
    use super::{
        Card, RAIL_TITLE_HEIGHT, Shelf, card_selection, focus, focused_label, rail_heights,
        shelf_metrics,
    };
    use crate::widgets::application::{LABEL_HEIGHT, Selection};
    use cosmic::theme;
    use cosmic::widget::{Id, icon};

    /// A cursor on a rail and a card in it.
    fn on(rail: usize, card: usize) -> focus::Cursor {
        focus::Cursor {
            rail,
            card,
            remembered: card,
        }
    }

    /// A card named after its label, with no icon or press to speak of.
    fn card(label: &str) -> Card {
        Card::new(
            Id::new(label.to_string()),
            label,
            icon::from_raster_pixels(4, 4, vec![0u8; 64]),
            None,
        )
    }

    /// Three rails: four cards, one card, three cards.
    const RAILS: [usize; 3] = [4, 1, 3];

    #[test]
    fn the_name_below_a_rail_is_the_card_the_cursor_is_on() {
        let cards = [card("Half-Life 2"), card("Portal"), card("Hades")];
        assert_eq!(focused_label(on(1, 1), &[2, 3], 1, &cards), Some("Portal"));
    }

    #[test]
    fn a_rail_the_cursor_is_not_in_reads_no_name() {
        let cards = [card("Half-Life 2"), card("Portal")];
        assert_eq!(focused_label(on(1, 0), &[2, 3], 0, &cards), None);
    }

    #[test]
    fn a_cursor_past_the_end_names_the_card_it_clamps_to() {
        let cards = [card("Half-Life 2"), card("Portal"), card("Hades")];
        assert_eq!(focused_label(on(1, 9), &[2, 3], 1, &cards), Some("Hades"));
    }

    #[test]
    fn an_empty_dashboard_reads_no_name() {
        let cards = [card("Half-Life 2")];
        assert_eq!(focused_label(on(0, 0), &[0, 0], 0, &cards), None);
    }

    // The page scroll measures a rail by `rail_heights`, so the name line
    // below its cards has to fit inside that measurement: under-measure the
    // rail and the scroll stops with the focused name off the bottom of the
    // screen.
    #[test]
    fn a_rail_reserves_room_for_the_name_line_below_its_cards() {
        let shelves = vec![Shelf::new(true, "Consoles", vec![card("Hades")])];
        let heights = rail_heights(&shelves, 1200.0);
        let (size, _, _) = shelf_metrics(&shelves[0], 1200.0);

        assert_eq!(
            heights[0],
            RAIL_TITLE_HEIGHT + f32::from(theme::spacing().space_m) + size + LABEL_HEIGHT,
            "a rail is measured as exactly what its column stacks: the title, the gap under it, \
             its cards, and the name line below them",
        );
    }

    #[test]
    fn the_card_under_the_cursor_is_selected_and_its_neighbours_are_not() {
        // Rail 2 holds three cards, so card 1 is a card that exists.
        let cursor = on(2, 1);
        assert_eq!(card_selection(cursor, &RAILS, 2, 1), Selection::Selected);

        // Sideways neighbours, the rail above and the rail below.
        assert_eq!(card_selection(cursor, &RAILS, 2, 0), Selection::Unselected);
        assert_eq!(card_selection(cursor, &RAILS, 2, 2), Selection::Unselected);
        assert_eq!(card_selection(cursor, &RAILS, 1, 1), Selection::Unselected);
        assert_eq!(card_selection(cursor, &RAILS, 0, 1), Selection::Unselected);
    }

    #[test]
    fn no_card_is_selected_when_there_is_nowhere_to_be() {
        // An empty dashboard has no card to be on.
        let empty: [usize; 2] = [0, 0];
        assert_eq!(
            card_selection(on(0, 0), &empty, 0, 0),
            Selection::Unselected
        );

        // A cursor left past the end of a rail that shrank reads as the card it
        // clamps onto rather than selecting nothing.
        assert_eq!(
            card_selection(on(0, 9), &[3], 0, 2),
            Selection::Selected,
            "the cursor clamps to the last card that exists"
        );
        assert_eq!(
            card_selection(on(0, 9), &[3], 0, 9),
            Selection::Unselected,
            "the card the cursor named does not exist"
        );
    }
}
