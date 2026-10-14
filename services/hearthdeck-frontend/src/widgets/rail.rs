//! A shared, single-line horizontal rail of cards.
//!
//! The dashboard shows several of these (consoles, recently played, favourites),
//! and every one must look and behave identically: a title, uniform-size cards,
//! horizontal scrolling, and no visible scrollbar. This module owns those rules
//! once so no surface re-implements (or diverges from) them.
//!
//! The cards themselves are not built here: every surface builds its tiles from
//! the one shared tile, [`crate::widgets::application::app_tile`], and hands the
//! resulting [`Element`]s to [`Rail::item`]. This module is only the row, its
//! title and its scroll.
//!
//! This is a *composed* widget, not a custom one: it is built entirely from
//! existing widgets, has no internal state, and needs no custom layout or event
//! handling. iced's own guidance for that case is to "leverage the Elm
//! Architecture directly" rather than implement [`Widget`] or (deprecated)
//! `Component` — so this exposes builders that convert into an [`Element`],
//! following the same shape as libcosmic's `ListColumn`:
//! a `#[must_use]` struct, a lowercase constructor, `into_element()`, and a
//! `From` impl so callers can just `.into()`.
//!
//! [`Widget`]: cosmic::iced::Widget

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::Length;
use cosmic::iced::widget::scrollable::Viewport;
use cosmic::theme;
use cosmic::widget::{Id, column, container, row, scrollable, space, text};

use crate::style::TEXT_HEADER;

/// A titled row of cards. Scrolls horizontally inside a single line and hides
/// its scrollbar; cards never wrap onto a second row.
#[must_use]
pub struct Rail<'a, Message> {
    id: Id,
    title: String,
    item_size: f32,
    items: Vec<Element<'a, Message>>,
    empty: Option<Element<'a, Message>>,
    on_scroll: Option<Box<dyn Fn(Viewport) -> Message + 'a>>,
}

/// Creates an empty [`Rail`] with a fixed card size.
pub fn rail<'a, Message: 'static>(
    id: Id,
    title: impl Into<String>,
    item_size: f32,
) -> Rail<'a, Message> {
    Rail {
        id,
        title: title.into(),
        item_size,
        items: Vec::new(),
        empty: None,
        on_scroll: None,
    }
}

impl<'a, Message: Clone + 'static> Rail<'a, Message> {
    /// Appends one card, built by [`crate::widgets::application::app_tile`].
    #[inline]
    pub fn item(mut self, item: impl Into<Element<'a, Message>>) -> Self {
        self.items.push(item.into());
        self
    }

    /// Appends every card in `items`.
    #[inline]
    pub fn items(mut self, items: impl IntoIterator<Item = Element<'a, Message>>) -> Self {
        self.items.extend(items);
        self
    }

    /// Content shown in place of the cards when the rail is empty.
    #[inline]
    pub fn empty(mut self, empty: impl Into<Element<'a, Message>>) -> Self {
        self.empty = Some(empty.into());
        self
    }

    /// Reports the rail's scroll [`Viewport`] whenever it changes. Callers use
    /// this to learn the current horizontal offset and the visible width so
    /// focus navigation can keep the selected card in view without pinning it
    /// to an edge.
    #[inline]
    pub fn on_scroll(mut self, f: impl Fn(Viewport) -> Message + 'a) -> Self {
        self.on_scroll = Some(Box::new(f));
        self
    }

    #[must_use]
    pub fn into_element(self) -> Element<'a, Message> {
        let Spacing {
            space_l, space_m, ..
        } = theme::spacing();

        let body: Element<'a, Message> = if self.items.is_empty() {
            self.empty.unwrap_or_else(|| {
                container(space::horizontal())
                    .height(Length::Fixed(self.item_size))
                    .into()
            })
        } else {
            let mut scrollable = scrollable::horizontal(row(self.items).spacing(space_l))
                .id(self.id)
                .width(Length::Fill)
                .height(Length::Fixed(self.item_size))
                .scrollbar_width(0)
                .scroller_width(0);
            if let Some(on_scroll) = self.on_scroll {
                scrollable = scrollable.on_scroll(on_scroll);
            }
            scrollable.into()
        };

        column![
            text::title3(self.title).size(TEXT_HEADER),
            container(body).padding([space_m, 0, 0, 0]),
        ]
        .into()
    }
}

impl<'a, Message: Clone + 'static> From<Rail<'a, Message>> for Element<'a, Message> {
    fn from(rail: Rail<'a, Message>) -> Self {
        rail.into_element()
    }
}
