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

pub mod backdrop;
pub mod focus;

use std::sync::LazyLock;

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::widget::scrollable::{AbsoluteOffset, scroll_to};
use cosmic::iced::{ContentFit, Length};
use cosmic::theme;
use cosmic::widget::{Id, column, container, icon, scrollable, text};

use crate::fl;
use crate::providers::daemon::{
    CONTINUE_WATCHING_COLLECTION, Collection, FAVORITES_COLLECTION, LAST_PLAYED_COLLECTION,
    NEW_ARRIVALS_COLLECTION, PLAY_LATER_COLLECTION,
};
use crate::style::{
    DASHBOARD_GAME_ASPECT, content_horizontal_padding, dashboard_console_tile_size,
    dashboard_tile_size,
};
use crate::widgets::application::{LABEL_HEIGHT, Selection, TileLabel, app_label, app_tile};
use crate::widgets::rail::rail;
use crate::widgets::top_bar;

/// The page's own scrollable. Held here because the page is its widget, and this
/// module is what knows the page scrolls vertically and nothing else.
static PAGE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("dashboard"));

#[derive(Clone, Debug)]
pub enum Message {
    CardPressed { id: String, title: String },
    ConsolePressed(i64),
    DestinationPressed(top_bar::Destination),
    RailScrolled { rail: usize, offset: f32 },
    PageScrolled(f32),
}

impl Message {
    #[must_use]
    pub fn card_pressed(id: &str, title: &str) -> Self {
        Self::CardPressed {
            id: id.to_owned(),
            title: title.to_owned(),
        }
    }
}

/// What one card shows and what pressing it does.
#[derive(Clone)]
pub struct Card {
    /// The widget id, so the pad can be pointed at this card.
    pub id: Id,
    pub label: String,
    /// The card's own art on the rail: its portrait poster.
    pub icon: icon::Handle,
    /// The title's wide art, when the feed gave it one: what the window's
    /// background shows while the pad is on this card.
    pub background: Option<icon::Handle>,
    /// Whether the card wears no focus ring of its own. Its art fills the
    /// window behind the page while the pad is on it - the picture is how the
    /// pad's place is shown - so a border around the tile would frame it twice.
    pub no_border: bool,
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
            background: None,
            no_border: false,
            message,
        }
    }

    /// The card's wide art: the window's background while the pad is on it.
    ///
    /// A card with art of that size announces the pad's place by the picture
    /// changing, so it is the card's own flag to wear no focus ring.
    #[must_use]
    pub fn with_background(mut self, background: Option<icon::Handle>) -> Self {
        self.no_border = background.is_some();
        self.background = background;
        self
    }

    /// The selection as the tile wears it: a card that wears no ring still is
    /// the one the pad is on, so its label and its press are unchanged.
    #[must_use]
    pub fn selection(&self, selection: Selection) -> Selection {
        if self.no_border && selection.is_selected() {
            Selection::SelectedBare
        } else {
            selection
        }
    }
}

/// The card shape that determines a rail's layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailKind {
    Games,
    Consoles,
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
    #[must_use]
    pub fn kind(&self) -> RailKind {
        if self.consoles {
            RailKind::Consoles
        } else {
            RailKind::Games
        }
    }

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
        NEW_ARRIVALS_COLLECTION => fl!("new-arrivals"),
        _ => collection
            .name
            .clone()
            .unwrap_or_else(|| collection.slug.clone()),
    }
}

/// The page: the shared top bar, then every shelf as a rail, stacked and
/// scrolled.
///
/// Scroll reports are mapped by the shell, which remembers each offset.
#[must_use]
pub fn view<'a>(
    shelves: &[Shelf],
    cursor: focus::Cursor,
    bar: &top_bar::TopBar<'_>,
    window_width: f32,
    window_height: f32,
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
            let (size, aspect, fit) = shelf_metrics(shelf.kind(), window_width);
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
                            card.selection(card_selection(
                                cursor,
                                &rail_lengths,
                                index,
                                card_index,
                            )),
                        )
                    }))
                    .on_scroll(move |viewport| Message::RailScrolled {
                        rail: index,
                        offset: viewport.absolute_offset().x,
                    })
                    .into();
            column![
                rail_element,
                app_label(focused_label(cursor, &rail_lengths, index, &shelf.cards)),
            ]
            .into()
        })
        .collect();

    // The blank space at the top of the page, where the banner was: kept clear
    // so the focused title's art has a window onto the page above its rail.
    let front = front_space(
        shelves.first().map(Shelf::kind),
        window_width,
        window_height,
    );
    let mut rows: Vec<Element<'a, Message>> = Vec::with_capacity(rails.len() + 2);
    rows.push(
        container(text(""))
            .width(Length::Fill)
            .height(Length::Fixed(front))
            .into(),
    );
    rows.extend(rails);
    // The blank space at the end of the page, below the last rail: the room
    // the follow needs for that rail to rise to the top of the view.
    rows.push(
        container(text(""))
            .width(Length::Fill)
            .height(Length::Fixed(tail_space(
                shelves.last().map(Shelf::kind),
                window_width,
                window_height,
            )))
            .into(),
    );

    let page = scrollable(
        column(rows)
            .spacing(space_xl)
            .padding([space_l, padding, space_xl, padding]),
    )
    .id(PAGE_ID.clone())
    .on_scroll(move |viewport| Message::PageScrolled(viewport.absolute_offset().y))
    .width(Length::Fill)
    .height(Length::Fill)
    .scrollbar_width(0)
    .scroller_width(0);

    column![
        container(top_bar::view(bar, Message::DestinationPressed))
            .padding([space_l, padding, 0, padding]),
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
fn shelf_metrics(kind: RailKind, window_width: f32) -> (f32, f32, ContentFit) {
    let Spacing { space_l, .. } = theme::spacing();
    let padding = content_horizontal_padding();
    match kind {
        RailKind::Consoles => (
            dashboard_console_tile_size(window_width, padding, space_l),
            1.0,
            ContentFit::Contain,
        ),
        RailKind::Games => (
            dashboard_tile_size(window_width, padding, space_l),
            DASHBOARD_GAME_ASPECT,
            ContentFit::Cover,
        ),
    }
}

/// A card's width and the gap beside it, without constructing its artwork.
#[must_use]
pub fn card_pitch(kind: RailKind, window_width: f32) -> (f32, f32) {
    let (size, aspect, _) = shelf_metrics(kind, window_width);
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
pub fn rail_heights(kinds: &[RailKind], window_width: f32, window_height: f32) -> Vec<f32> {
    let mut heights: Vec<f32> = kinds
        .iter()
        .map(|&kind| rail_height(kind, window_width))
        .collect();
    // The blank space at the top of the page belongs to the first rail: folded
    // into its height - with the page's own top padding and the gap under the
    // blank - the heights measure the very offsets the renderer scrolls, and
    // the page's follow pins "the first rail" into view by pinning the blank
    // space's top - which is the page's own top. That is the snap the first
    // rail gets: the rail at the bottom of the screen, with the whole window
    // onto the art above it, however the window is sized.
    if let Some(first) = heights.first_mut() {
        let Spacing { space_l, .. } = theme::spacing();
        *first += front_space(kinds.first().copied(), window_width, window_height)
            + rail_gap()
            + f32::from(space_l);
    }
    heights
}

/// One rail's height on the page: its title, its cards, and the name line.
fn rail_height(kind: RailKind, window_width: f32) -> f32 {
    let Spacing { space_m, .. } = theme::spacing();
    let (size, _, _) = shelf_metrics(kind, window_width);
    RAIL_TITLE_HEIGHT + f32::from(space_m) + size + LABEL_HEIGHT
}

/// The blank space at the top of the dashboard, where the banner was.
///
/// It is whatever of the page's first screenful the first rail leaves unused,
/// so the rail under it and the window onto the art above it are both on screen
/// when the page sits at its top. A rail taller than that screenful leaves no
/// space, which is right: the focused card is what has to stay visible.
fn front_space(first: Option<RailKind>, window_width: f32, window_height: f32) -> f32 {
    let Spacing { space_l, .. } = theme::spacing();
    let first = first.map_or(0.0, |kind| rail_height(kind, window_width));
    let page_viewport = window_height - top_bar::height();
    (page_viewport - f32::from(space_l) - rail_gap() - first).max(0.0)
}

/// The blank space at the end of the dashboard, below the last rail.
///
/// The mirror of [`front_space`]: the follow's snap brings a rail below the
/// fold to the top of the view, so the page has to extend far enough for the
/// last one to make that scroll - its top at the top of the view, with the gap
/// and the page's own bottom padding filling out the screenful under it.
fn tail_space(last: Option<RailKind>, window_width: f32, window_height: f32) -> f32 {
    let Spacing { space_xl, .. } = theme::spacing();
    let last = last.map_or(0.0, |kind| rail_height(kind, window_width));
    let page_viewport = window_height - top_bar::height();
    (page_viewport - rail_gap() - f32::from(space_xl) - last).max(0.0)
}

/// What the page's content measures end to end: the rails and the gaps between
/// them, the blank at each end, and the page's own padding - which is what the
/// page's scroll is clamped against.
#[must_use]
pub fn page_extent(kinds: &[RailKind], window_width: f32, window_height: f32) -> f32 {
    let Spacing { space_xl, .. } = theme::spacing();
    let heights = rail_heights(kinds, window_width, window_height);
    let stack: f32 = heights.iter().sum::<f32>() + heights.len() as f32 * rail_gap();
    stack + tail_space(kinds.last().copied(), window_width, window_height) + f32::from(space_xl)
}

/// The gap between two rails on the page.
#[must_use]
pub fn rail_gap() -> f32 {
    f32::from(theme::spacing().space_xl)
}

/// Scrolls the page to a vertical offset, leaving the rails where they are
/// horizontally.
pub fn scroll_to_page<M: 'static>(offset: f32) -> cosmic::app::Task<M> {
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
/// approximation. The dashboard is the whole window - it has no sidebar - so the
/// rail gets the page's padded content width. The scrollable clamps what it is
/// handed, so being a few pixels out costs a few pixels of over-scroll rather
/// than a card the pad cannot reach.
#[must_use]
pub fn rail_viewport(window_width: f32) -> f32 {
    (window_width - 2.0 * f32::from(content_horizontal_padding())).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::{
        Card, RAIL_TITLE_HEIGHT, RailKind, Shelf, card_pitch, card_selection, focus, focused_label,
        page_extent, rail_heights, rail_viewport, shelf_metrics,
    };
    use crate::style::content_horizontal_padding;
    use crate::widgets::application::{LABEL_HEIGHT, Selection};
    use cosmic::theme;

    #[test]
    fn rail_geometry_needs_only_card_kind_and_window_size() {
        let spacing = theme::spacing();
        let gap = f32::from(spacing.space_l);
        for width in [640.0, 1200.0, 1920.0] {
            let padding = content_horizontal_padding();
            let game_size = crate::style::dashboard_tile_size(width, padding, spacing.space_l);
            let console_size =
                crate::style::dashboard_console_tile_size(width, padding, spacing.space_l);
            assert_eq!(
                card_pitch(RailKind::Games, width),
                (game_size * crate::style::DASHBOARD_GAME_ASPECT, gap)
            );
            assert_eq!(card_pitch(RailKind::Consoles, width), (console_size, gap));
            for height in [400.0, 1080.0] {
                assert!(rail_heights(&[], width, height).is_empty());
                let heights = rail_heights(&[RailKind::Games, RailKind::Consoles], width, height);
                let console_height =
                    RAIL_TITLE_HEIGHT + f32::from(spacing.space_m) + console_size + LABEL_HEIGHT;
                assert_eq!(heights[1], console_height);
                let game_height =
                    RAIL_TITLE_HEIGHT + f32::from(spacing.space_m) + game_size + LABEL_HEIGHT;
                let viewport = height - crate::widgets::top_bar::height();
                let front = (viewport - gap - super::rail_gap() - game_height).max(0.0);
                assert_eq!(
                    heights[0],
                    game_height + front + super::rail_gap() + gap,
                    "the first rail carries the blank above it, the gap under it \
                     and the page's top padding",
                );
            }
        }
    }
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
        let shelves = [
            Shelf::new(true, "Consoles", vec![card("Hades")]),
            Shelf::new(false, "Recently Played", vec![card("Half-Life 2")]),
        ];
        let kinds: Vec<_> = shelves.iter().map(Shelf::kind).collect();
        let heights = rail_heights(&kinds, 1200.0, crate::style::WINDOW_HEIGHT);
        let (size, _, _) = shelf_metrics(shelves[1].kind(), 1200.0);

        assert_eq!(
            heights[1],
            RAIL_TITLE_HEIGHT + f32::from(theme::spacing().space_m) + size + LABEL_HEIGHT,
            "a rail is measured as exactly what its column stacks: the title, the gap under it, \
             its cards, and the name line below them",
        );
    }

    /// The blank space at the top of the page is kept for the art, and rides on
    /// the first rail's measurement - so the page's follow pins the first rail
    /// into view by pinning the page's own top, which is the snap it gets.
    #[test]
    fn the_first_rail_carries_the_blank_space_that_keeps_the_art_visible() {
        let shelves = [
            Shelf::new(false, "New on Stremio", vec![card("Wicker")]),
            Shelf::new(false, "Recently Played", vec![card("Half-Life 2")]),
        ];
        // The window is the display's own size under the kiosk's compositor,
        // so the blank space is whatever of *that* screenful the rail leaves
        // unused - a television is nothing like the size the app opens at.
        for window_height in [crate::style::WINDOW_HEIGHT, 1080.0] {
            let kinds: Vec<_> = shelves.iter().map(Shelf::kind).collect();
            let heights = rail_heights(&kinds, 1200.0, window_height);
            let plain = RAIL_TITLE_HEIGHT
                + f32::from(theme::spacing().space_m)
                + shelf_metrics(shelves[1].kind(), 1200.0).0
                + LABEL_HEIGHT;

            assert_eq!(
                heights[1], plain,
                "the other rails are measured as themselves"
            );
            assert!(
                heights[0] > plain,
                "the blank space rides on the first: {heights:?}"
            );
            assert_eq!(
                heights[0],
                window_height - crate::widgets::top_bar::height(),
                "the blank space is whatever of the first screenful the rail leaves unused: \
                 {heights:?}"
            );
        }
    }

    /// Every rail can make the snap's scroll: a rail below the fold brings its
    /// top to the top of the view, and the page extends far enough for the last
    /// one to make that scroll rather than stopping short with the rails above
    /// still on screen.
    #[test]
    fn every_rail_can_reach_the_top_of_the_view() {
        for window_height in [crate::style::WINDOW_HEIGHT, 1080.0] {
            for kinds in [
                vec![RailKind::Games],
                vec![RailKind::Games, RailKind::Games],
                vec![RailKind::Games, RailKind::Consoles, RailKind::Games],
            ] {
                let heights = rail_heights(&kinds, 1200.0, window_height);
                let gap = super::rail_gap();
                let extent = page_extent(&kinds, 1200.0, window_height);
                let viewport = window_height - crate::widgets::top_bar::height();
                for index in 1..heights.len() {
                    let top: f32 = heights.iter().take(index).sum::<f32>() + index as f32 * gap;
                    let landed =
                        focus::page_scroll_target(&heights, index, gap, 0.0, viewport, extent)
                            .expect("the page moves to it");
                    assert!(
                        (landed - top).abs() < 1.0,
                        "rail {index} of {kinds:?} lands at the top of the view ({top}): {landed}",
                    );
                }
            }
        }
    }

    /// A card whose art fills the window behind the page wears no ring of its
    /// own: the picture changing is how the pad's place is shown.
    #[test]
    fn a_card_whose_art_fills_the_window_wears_no_ring() {
        let art =
            card("Wicker").with_background(Some(icon::from_raster_pixels(4, 4, vec![0u8; 64])));
        assert_eq!(art.selection(Selection::Selected), Selection::SelectedBare);
        assert_eq!(art.selection(Selection::Unselected), Selection::Unselected);

        let plain = card("Half-Life 2");
        assert_eq!(plain.selection(Selection::Selected), Selection::Selected);
    }

    #[test]
    fn a_rail_gets_the_dashboards_whole_padded_width() {
        // The dashboard is the whole window and has no sidebar, so a rail is
        // only inset by the page's own horizontal padding. Measuring it as if a
        // sidebar were there made every rail scroll before its last visible
        // card had left the screen.
        let window_width = 1200.0;
        assert_eq!(
            rail_viewport(window_width),
            window_width - 2.0 * f32::from(content_horizontal_padding()),
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
