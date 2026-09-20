//! The menu card: rows of actions in a dialog card over a dimming scrim.
//!
//! This is a new menu, built to look and behave like the quick menu in
//! `hearthdeck-overlay`'s `overlay.rs`: one table of rows rendered as a
//! `list_column` of list buttons under a heading, that column inside a
//! `Container::Dialog` card, and the card centered over a full-window scrim
//! which dismisses the menu when a press reaches it.
//!
//! The module owns both halves of "a menu": [`State`] is *open on what, with
//! which row highlighted* plus the navigation over it, and [`Menu`] is the card
//! it draws. The rows themselves come from the caller — the library opens one of
//! these as an entry's context menu, deriving the rows from that entry — so a
//! menu stays a menu and not a catalog.

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::core::text::Wrapping;
use cosmic::iced::{Alignment, Length};
use cosmic::theme;
use cosmic::widget::{
    Id, autosize::autosize, column, container, list, list::list_column, mouse_area, scrollable,
    text,
};

use crate::style::{MENU_CARD_WIDTH, MENU_MAX_HEIGHT, TEXT_BODY, TEXT_HEADER, menu_scrim};

/// One row of a [`Menu`]: what it reads and the message activating it sends.
pub struct Row<Message> {
    pub label: String,
    pub message: Message,
}

impl<Message> Row<Message> {
    pub fn new(label: impl Into<String>, message: Message) -> Self {
        Self {
            label: label.into(),
            message,
        }
    }
}

/// Which entry the menu is open on, and which of its rows is highlighted.
///
/// `target` is opaque here: it is the caller's own id for whatever the menu
/// belongs to (for the library grid, the index of the entry whose actions these
/// are). Keeping it beside the highlight gives "is a menu open" and "which row
/// is highlighted" one home, instead of a flag that could disagree with a target
/// stored somewhere else.
///
/// Unlike `hearthdeck-overlay`'s `OverlayState`, there is no event/effect pair
/// here: nothing a menu does has a side effect beyond the message a row already
/// carries, so the callers drive these methods directly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    target: Option<usize>,
    selected: usize,
}

impl State {
    /// Whether a menu is open.
    pub fn is_open(self) -> bool {
        self.target.is_some()
    }

    /// The target the open menu belongs to.
    pub fn target(self) -> Option<usize> {
        self.target
    }

    /// The highlighted row.
    pub fn selected(self) -> usize {
        self.selected
    }

    /// Open on `target` with its first row highlighted, or close a menu that is
    /// already open: the one button that opens the menu also closes it.
    pub fn toggle(&mut self, target: usize) {
        if self.is_open() {
            self.close();
        } else {
            self.open(target);
        }
    }

    /// Open on `target` with its first row highlighted.
    pub fn open(&mut self, target: usize) {
        self.target = Some(target);
        self.selected = 0;
    }

    /// Close, forgetting the highlighted row.
    pub fn close(&mut self) {
        self.target = None;
        self.selected = 0;
    }

    /// Move the highlight by `delta` through a menu of `rows` rows, wrapping at
    /// both ends so the D-pad can circle the whole list either way.
    pub fn move_selection(&mut self, delta: i32, rows: usize) {
        if rows == 0 {
            return;
        }
        let current = self.selected.min(rows - 1) as i32;
        self.selected = (current + delta).rem_euclid(rows as i32) as usize;
    }
}

/// A menu card with its scrim. Build with [`Menu::new`], then convert into an
/// [`Element`] and stack it over the surface it belongs to.
#[must_use]
pub struct Menu<'a, Message> {
    title: &'a str,
    rows: Vec<Row<Message>>,
    selected: usize,
    on_dismiss: Message,
}

impl<'a, Message: Clone + 'static> Menu<'a, Message> {
    /// A card titled `title` listing `rows`, with the row at `selected`
    /// highlighted, and `on_dismiss` sent when a press lands on the scrim
    /// instead of on a row.
    pub fn new(
        title: &'a str,
        rows: Vec<Row<Message>>,
        selected: usize,
        on_dismiss: Message,
    ) -> Self {
        Self {
            title,
            rows,
            selected,
            on_dismiss,
        }
    }

    /// The card and its scrim, ready to be stacked over a page.
    pub fn into_element(self) -> Element<'a, Message> {
        let Self {
            title,
            rows,
            selected,
            on_dismiss,
        } = self;
        let Spacing {
            space_s, space_m, ..
        } = theme::spacing();

        let mut items = list_column();
        for (index, row) in rows.into_iter().enumerate() {
            items = items.add(
                list::button(text::body(row.label).size(TEXT_BODY))
                    .on_press(row.message)
                    .selected(index == selected),
            );
        }

        // The list is capped rather than the card, so an entry with many rows
        // scrolls instead of pushing the card off the screen.
        let list = autosize(
            container(scrollable(items.into_element())).padding(1),
            Id::new("action-menu-autosize"),
        )
        .max_height(MENU_MAX_HEIGHT);

        let card = container(
            column![
                text::title3(title)
                    .size(TEXT_HEADER)
                    .wrapping(Wrapping::Word),
                list,
            ]
            .spacing(space_s)
            .align_x(Alignment::Center),
        )
        .width(Length::Fixed(MENU_CARD_WIDTH))
        .padding(space_m)
        .class(theme::Container::Dialog(true));

        // The scrim dims the page and dismisses the menu. `MouseArea` forwards a
        // press to its content first, so a press on a row activates that row and
        // never reaches this handler.
        mouse_area(
            container(card)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .class(theme::Container::Custom(Box::new(menu_scrim))),
        )
        .on_press(on_dismiss)
        .into()
    }
}

impl<'a, Message: Clone + 'static> From<Menu<'a, Message>> for Element<'a, Message> {
    fn from(menu: Menu<'a, Message>) -> Self {
        menu.into_element()
    }
}

#[cfg(test)]
mod tests {
    use super::State;

    #[test]
    fn the_same_button_opens_and_closes_the_menu() {
        let mut state = State::default();

        assert!(!state.is_open());
        state.toggle(3);
        assert_eq!(state.target(), Some(3));
        state.toggle(3);
        assert!(!state.is_open());
        assert_eq!(state.target(), None);
    }

    #[test]
    fn opening_resets_the_highlight_and_closing_forgets_it() {
        let mut state = State::default();

        state.open(1);
        state.move_selection(2, 4);
        assert_eq!(state.selected(), 2);

        state.open(1);
        assert_eq!(state.selected(), 0);

        state.move_selection(1, 4);
        state.close();
        assert_eq!(state.selected(), 0);
        assert!(!state.is_open());
    }

    #[test]
    fn navigation_wraps_at_both_ends() {
        let mut state = State::default();
        state.open(0);

        state.move_selection(-1, 4);
        assert_eq!(state.selected(), 3);
        state.move_selection(1, 4);
        assert_eq!(state.selected(), 0);
        state.move_selection(7, 4);
        assert_eq!(state.selected(), 3);
    }

    #[test]
    fn a_highlight_left_past_the_end_is_pulled_back_on_the_next_move() {
        let mut state = State::default();
        state.open(0);
        state.move_selection(5, 2);
        assert_eq!(state.selected(), 1);

        // The menu shrunk under the highlight (rows depend on state), so the
        // next move has to clamp before it steps.
        state.move_selection(1, 1);
        assert_eq!(state.selected(), 0);
    }

    #[test]
    fn navigating_a_menu_with_no_rows_is_inert() {
        let mut state = State::default();
        state.open(0);

        state.move_selection(1, 0);

        assert_eq!(state.selected(), 0);
    }
}
