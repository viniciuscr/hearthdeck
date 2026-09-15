//! A shared, single-line horizontal rail of media cards.
//!
//! The dashboard shows several of these (consoles, recently played, favorites),
//! and every one must look and behave identically: a title, uniform-size cards,
//! horizontal scrolling, and no visible scrollbar. This module owns those rules
//! once so no surface re-implements (or diverges from) them.
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
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::widget::scrollable::Viewport;
use cosmic::iced::{Alignment, ContentFit, Length};
use cosmic::theme;
use cosmic::widget::{Id, button, column, container, icon, row, scrollable, space, text};

use crate::style::{
    TEXT_HEADER, TEXT_TILE_LABEL, artwork_fit, tile_button_class, tile_label_overlay,
};

/// One card in a [`Rail`]: the same height as every other card in the rail and,
/// by default, square. Only the artwork fit and the aspect ratio vary — game
/// covers are cropped to fill, console art is contained so its pixel-art edges
/// are not cut off, and screenshots keep their own 16:9 shape.
#[must_use]
pub struct RailItem<Message> {
    id: Id,
    label: String,
    handle: icon::Handle,
    size: f32,
    aspect: f32,
    fit: ContentFit,
    on_press: Option<Message>,
}

/// Creates a [`RailItem`] for the given entry.
pub fn rail_item<Message: 'static>(
    id: Id,
    label: impl Into<String>,
    handle: icon::Handle,
    size: f32,
) -> RailItem<Message> {
    RailItem {
        id,
        label: label.into(),
        handle,
        size,
        aspect: 1.0,
        fit: ContentFit::Cover,
        on_press: None,
    }
}

impl<Message: Clone + 'static> RailItem<Message> {
    /// Sets how the artwork fills the card. Defaults to [`ContentFit::Cover`].
    #[inline]
    pub fn fit(mut self, fit: ContentFit) -> Self {
        self.fit = fit;
        self
    }

    /// Sets the card's width as a multiple of `size`, which stays the card's
    /// height. Defaults to `1.0`, a square card; landscape art such as
    /// screenshots passes its own ratio so it is not cropped to a square.
    #[inline]
    pub fn aspect(mut self, aspect: f32) -> Self {
        self.aspect = aspect;
        self
    }

    #[inline]
    pub fn on_press(mut self, message: Message) -> Self {
        self.on_press = Some(message);
        self
    }

    #[must_use]
    pub fn into_element<'a>(self) -> Element<'a, Message> {
        button::custom(
            container(
                column![
                    artwork_fit(&self.handle, self.fit, Length::Fill, Length::Fill),
                    container(text(self.label).size(TEXT_TILE_LABEL).width(Length::Fill))
                        .padding([2, 6])
                        .width(Length::Fill)
                        .height(Length::Shrink)
                        .class(cosmic::theme::Container::Custom(Box::new(
                            tile_label_overlay
                        ))),
                ]
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Bottom),
        )
        .id(self.id)
        .width(Length::Fixed(self.size * self.aspect))
        .height(Length::Fixed(self.size))
        .class(tile_button_class(false))
        .padding(0)
        .on_press_maybe(self.on_press)
        .into()
    }
}

impl<'a, Message: Clone + 'static> From<RailItem<Message>> for Element<'a, Message> {
    fn from(item: RailItem<Message>) -> Self {
        item.into_element()
    }
}

/// A titled row of [`RailItem`]s. Scrolls horizontally inside a single line and
/// hides its scrollbar; cards never wrap onto a second row.
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
    /// Appends one card.
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
