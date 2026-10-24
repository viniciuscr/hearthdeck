//! The dashboard's rails and the cursor that walks them and the top bar.

use cosmic::app::Task;
use cosmic::{Application, theme};

use crate::app::animation::ScrollTarget;
use crate::app_group::Section;
use crate::fl;
use crate::providers::daemon::NEW_ARRIVALS_COLLECTION;
use crate::screens::dashboard;
use crate::screens::dashboard::{Card, Shelf};
use crate::screens::library::focus::Direction;
use crate::style::{
    DASHBOARD_RAIL_TILES, WINDOW_HEIGHT, content_horizontal_padding, dashboard_console_tile_size,
    dashboard_tile_size,
};
use crate::widgets::top_bar;

use super::{
    App, Message, Page, dashboard_card_id, launch_card_message, launch_console_message, navigate,
    widget_focus,
};

impl App {
    /// The dashboard's rails: the daemon's collections, in the order it composes
    /// them, with a consoles rail at the top when RomM is configured.
    ///
    /// Drawn from the collections rather than from a list of known shelves, so a
    /// collection nobody wrote frontend code for - which is what a feature's own
    /// categories, and so the AI's, are - appears on the dashboard without a
    /// change here. With nothing composed yet the library still gets a rail,
    /// because a blank front door is worse than a short one.
    pub(super) fn dashboard_shelves(&self) -> Vec<Shelf> {
        let spacing = theme::spacing();
        let size = dashboard_tile_size(
            self.window_width,
            content_horizontal_padding(),
            spacing.space_l,
        );
        let mut shelves = Vec::new();

        // The banner first: what has just arrived on Stremio, drawn over the
        // rails rather than beside them. It is a row like any other - the pad
        // walks its slides with left and right - so it is built here with them,
        // and the page draws it as the banner because of what kind of row it is.
        if let Some(cards) = self.banner_cards(size) {
            let title = self
                .collections
                .iter()
                .find(|collection| collection.slug == NEW_ARRIVALS_COLLECTION)
                .and_then(|collection| collection.name.clone())
                .unwrap_or_else(|| NEW_ARRIVALS_COLLECTION.to_owned());
            shelves.push(Shelf::banner(title, cards));
        }

        // Consoles rail: shows all RomM platforms with their artwork.
        if !self.console_platforms.is_empty() {
            let console_size = dashboard_console_tile_size(
                self.window_width,
                content_horizontal_padding(),
                spacing.space_l,
            );
            let cards: Vec<Card> = self
                .console_platforms
                .iter()
                .take(DASHBOARD_RAIL_TILES)
                .enumerate()
                .map(|(index, platform)| {
                    Card::new(
                        dashboard_card_id("consoles", index),
                        platform.label.clone(),
                        crate::icon_cache::reference_handle(
                            platform.artwork.as_deref(),
                            console_size as u32,
                            // A platform's logo is drawn square on the card, so
                            // there is nothing to turn.
                            crate::icon_cache::Orientation::AsIs,
                        ),
                        Some(launch_console_message(platform.id)),
                    )
                })
                .collect();
            shelves.push(Shelf::new(true, fl!("consoles"), cards));
        }

        // Collection rails from the daemon. The banner's own collection is drawn
        // as the banner, not again as a rail of posters beneath it.
        shelves.extend(
            self.collections
                .iter()
                .filter(|collection| collection.slug != NEW_ARRIVALS_COLLECTION)
                .filter(|collection| !collection.items.is_empty())
                .map(|collection| {
                    let cards = collection
                        .items
                        .iter()
                        .take(DASHBOARD_RAIL_TILES)
                        .enumerate()
                        .map(|(index, item)| {
                            Card::new(
                                dashboard_card_id(&collection.slug, index),
                                item.name.clone(),
                                crate::icon_cache::reference_handle(
                                    item.icon.as_deref(),
                                    size as u32,
                                    // A rail card is a portrait poster: a
                                    // horizontal box is turned to fill it.
                                    crate::icon_cache::Orientation::Portrait,
                                ),
                                Some(launch_card_message(&item.item_id, &item.name)),
                            )
                        })
                        .collect();
                    Shelf::new(false, dashboard::rail_title(collection), cards)
                }),
        );

        if shelves.is_empty() {
            return self.library_shelf(size);
        }
        shelves
    }

    /// The banner's slides: what the daemon resolved as having just arrived.
    ///
    /// This is the one collection the dashboard draws as a banner rather than as
    /// a rail. Its items carry the wide artwork and the title's own wordmark,
    /// which no other collection's items have, and the pad walks the slides as
    /// the page's first row.
    fn banner_cards(&self, size: f32) -> Option<Vec<Card>> {
        let collection = self
            .collections
            .iter()
            .find(|collection| collection.slug == NEW_ARRIVALS_COLLECTION)?;
        let cards: Vec<Card> = collection
            .items
            .iter()
            // A banner is artwork: an arrival with no art at all would be a grey
            // slide the size of the window, so it does not get a turn.
            .filter(|item| item.background.is_some() || item.icon.is_some())
            .enumerate()
            .map(|(index, item)| {
                // A slide with no wide art falls back to its poster rather than
                // to the generic mark: a banner of one still beats no banner.
                let background = crate::icon_cache::reference_handle(
                    item.background.as_deref().or(item.icon.as_deref()),
                    size as u32,
                    // The banner is a wide surface: it takes the art as it is.
                    crate::icon_cache::Orientation::AsIs,
                );
                Card::banner(
                    dashboard_card_id(NEW_ARRIVALS_COLLECTION, index),
                    item.name.clone(),
                    background,
                    item.logo.as_deref().map(|logo| {
                        crate::icon_cache::reference_handle(
                            Some(logo),
                            size as u32,
                            crate::icon_cache::Orientation::AsIs,
                        )
                    }),
                    item.description.clone(),
                    Some(launch_card_message(&item.item_id, &item.name)),
                )
            })
            .collect();
        (!cards.is_empty()).then_some(cards)
    }

    /// How many slides the banner has: none when the daemon has published no
    /// arrivals, which is what an unlinked or offline Stremio looks like - and
    /// then the dashboard is exactly the rails it was before.
    pub(super) fn dashboard_hero_len(&self) -> usize {
        self.collections
            .iter()
            .find(|collection| collection.slug == NEW_ARRIVALS_COLLECTION)
            .map_or(0, |collection| collection.items.len())
    }

    /// Moves the banner on to its next slide, when the pad is not on it.
    ///
    /// The shell asks on a timer; whether anything happens is decided here, from
    /// the same rule the subscription is gated by, so a tick that arrives after
    /// the pad moved onto the banner cannot move it out from under them.
    pub(super) fn rotate_hero(&mut self) -> Task<Message> {
        let hero_len = self.dashboard_hero_len();
        if !dashboard::hero::rotates(hero_len, self.dashboard_cursor) {
            return Task::none();
        }
        self.hero_index = dashboard::hero::rotate(self.hero_index, hero_len);
        Task::none()
    }

    /// The library as a single rail, for when the daemon has composed nothing yet
    /// - a fresh install, or a fetch that has not landed.
    pub(super) fn library_shelf(&self, size: f32) -> Vec<Shelf> {
        let cards: Vec<Card> = self
            .catalog
            .iter()
            .filter(|entry| Section::PcGames.matches(entry))
            .take(DASHBOARD_RAIL_TILES)
            .enumerate()
            .map(|(index, entry)| {
                Card::new(
                    dashboard_card_id("library", index),
                    entry.name.clone(),
                    crate::icon_cache::entry_icon_handle(
                        &entry.icon,
                        size as u32,
                        crate::icon_cache::Orientation::Portrait,
                    ),
                    Some(launch_card_message(&entry.id, &entry.name)),
                )
            })
            .collect();
        if cards.is_empty() {
            return Vec::new();
        }
        vec![Shelf::new(false, fl!("all-games"), cards)]
    }

    /// How many cards each rail holds, which is the shape the rail navigation
    /// walks.
    pub(super) fn dashboard_rail_lengths(&self) -> Vec<usize> {
        self.dashboard_shelves()
            .iter()
            .map(|shelf| shelf.cards.len())
            .collect()
    }

    /// Steps the dashboard's cursor, and keeps the card it lands on on screen.
    pub(super) fn step_dashboard(&mut self, direction: Direction) -> Task<Message> {
        let rails = self.dashboard_rail_lengths();
        let hero_len = self.dashboard_hero_len();
        let before = self.dashboard_cursor;
        // The banner is the first row and its cards are slides, so the pad
        // stepping onto it keeps the slide it was showing, and stepping along it
        // moves the slide with the pad. The rail maths below is the same either
        // way: a slide is a card that happens to fill the window.
        let (cursor, slide) = dashboard::hero::stepped(
            before,
            dashboard::focus::step(self.dashboard_cursor, direction, &rails),
            self.hero_index,
            hero_len,
        );
        self.dashboard_cursor = cursor;
        self.hero_index = slide;
        if self.dashboard_cursor != before {
            return self.focus_dashboard_card();
        }
        // Up off the top of the stack of rails is the top bar, the row above
        // them - which is where the pad comes to choose a destination.
        if direction == Direction::Up {
            self.bar_cursor = Some(top_bar::Destination::Dashboard.index());
            return self.focus_bar_button();
        }
        Task::none()
    }

    /// Steps the cursor along the top bar.
    ///
    /// The bar is one row of buttons: sideways walks it and stops at its ends,
    /// up off it does nothing, and down leaves it for the content below - the
    /// rails on the dashboard, which the bar is the top of. The settings screen
    /// has no content cursor below its bar, so down does nothing there and the
    /// bar keeps the cursor.
    pub(super) fn step_bar(&mut self, direction: Direction) -> Task<Message> {
        let Some(index) = self.bar_cursor else {
            return Task::none();
        };
        match direction {
            Direction::Left | Direction::Right => {
                let delta = if direction == Direction::Left { -1 } else { 1 };
                if let Some(next) = top_bar::stepped(index, delta) {
                    self.bar_cursor = Some(next);
                }
            }
            Direction::Down if self.page == Page::Dashboard => {
                self.bar_cursor = None;
                // The first rail, at the card the cursor remembers - the same
                // rule a vertical step between rails follows. The banner is a
                // row of slides rather than of cards, so arriving on it keeps the
                // slide it is showing.
                let remembered = self.dashboard_cursor.remembered;
                let (cursor, slide) = dashboard::hero::landed_from_above(
                    dashboard::focus::clamp(
                        dashboard::focus::Cursor {
                            rail: 0,
                            card: remembered,
                            remembered,
                        },
                        &self.dashboard_rail_lengths(),
                    ),
                    self.hero_index,
                    self.dashboard_hero_len(),
                );
                self.dashboard_cursor = cursor;
                self.hero_index = slide;
                return self.focus_dashboard_card();
            }
            Direction::Up | Direction::Down => {}
        }
        self.focus_bar_button()
    }

    /// Acts on the top bar's button at `index`, which is what both the pad's
    /// Confirm and the keyboard's Enter ask for - the same action a click on
    /// the button asks for.
    pub(super) fn activate_bar(&mut self, index: usize) -> Task<Message> {
        match top_bar::Destination::at(index) {
            Some(destination) => self.update(navigate(destination)),
            None => Task::none(),
        }
    }

    /// Points the renderer's focus at the top bar button the cursor is on,
    /// which is where its focus ring comes from.
    pub(super) fn focus_bar_button(&self) -> Task<Message> {
        let Some(current) = self.page.bar_destination() else {
            return Task::none();
        };
        self.bar_cursor
            .and_then(top_bar::Destination::at)
            .map_or(Task::none(), |destination| {
                widget_focus(top_bar::button_id(current, destination))
            })
    }

    /// Runs the card the dashboard's cursor is on.
    pub(super) fn confirm_dashboard(&mut self) -> Task<Message> {
        let rails = self.dashboard_rail_lengths();
        let message =
            dashboard::focus::confirm(self.dashboard_cursor, &rails).and_then(|(rail, card)| {
                self.dashboard_shelves()
                    .get(rail)
                    .and_then(|shelf| shelf.cards.get(card))
                    .and_then(|card| card.message.clone())
            });
        match message {
            Some(message) => self.update(message),
            None => Task::none(),
        }
    }

    /// Points the renderer's focus at the card the dashboard's cursor is on, and
    /// scrolls its rail so that card stays on screen.
    ///
    /// A rail moves only once the cursor crosses one of its edges, which is the
    /// rule the grid follows and for the same reason: a row that slid on every
    /// step reads as the row moving rather than the selection.
    pub(super) fn focus_dashboard_card(&mut self) -> Task<Message> {
        let shelves = self.dashboard_shelves();
        let Some(shelf) = shelves.get(self.dashboard_cursor.rail) else {
            return Task::none();
        };
        let Some(card) = shelf.cards.get(self.dashboard_cursor.card) else {
            return Task::none();
        };
        let mut tasks = vec![widget_focus(card.id.clone())];

        // The rail follows the cursor sideways... unless the row is the banner,
        // which shows one slide at a time and so has nothing to scroll. The page
        // still follows it: the banner is a row of the page like any other.
        if !shelf.hero {
            let (card_width, gap) = dashboard::card_pitch(shelf, self.window_width);
            let rail = self.dashboard_cursor.rail;
            let rail_offset = self.rail_offsets.get(rail).copied().unwrap_or(0.0);
            if let Some(offset) = dashboard::focus::rail_scroll_target(
                self.dashboard_cursor.card,
                shelf.cards.len(),
                rail_offset,
                dashboard::rail_viewport(self.window_width),
                card_width,
                gap,
            ) {
                // Remember where the rail is going now, not where the renderer last
                // reported it: a scroll the shell drives does not come back as an
                // `on_scroll` event, and the next step has to measure against it.
                self.set_rail_offset(rail, offset);
                tasks.push(self.animate_scroll_to(ScrollTarget::Rail(rail), rail_offset, offset));
            }
        }

        // ...and the page follows it down the stack of rails, which is the same
        // rule a third time: rails of unequal height rather than cards of equal
        // width. The dashboard fills the content column, so the page's viewport
        // is the window's height.
        let heights = dashboard::rail_heights(&shelves, self.window_width);
        // The page sits below the top bar, so its viewport is the window's height
        // less the bar rather than the whole window.
        let page_viewport = WINDOW_HEIGHT - top_bar::height();
        if let Some(offset) = dashboard::focus::page_scroll_target(
            &heights,
            self.dashboard_cursor.rail,
            dashboard::rail_gap(),
            self.page_offset,
            page_viewport,
        ) {
            let from = self.page_offset;
            // The same reason the rail remembers its target: the page's next
            // follow step must measure against where this one is sending it.
            self.page_offset = offset;
            tasks.push(self.animate_scroll_to(ScrollTarget::Page, from, offset));
        }

        Task::batch(tasks)
    }

    /// Records where a rail has been sent, growing the list as rails appear.
    ///
    /// Both the shell's own follow scrolls and the rail's `on_scroll` reports go
    /// through here, so "where is this rail" has one writer and one shape.
    pub(super) fn set_rail_offset(&mut self, rail: usize, offset: f32) {
        if self.rail_offsets.len() <= rail {
            self.rail_offsets.resize(rail + 1, 0.0);
        }
        self.rail_offsets[rail] = offset;
    }

    /// Fetches the dashboard's collections, which are what its rails are made of.
    ///
    /// One request for all of them, in the order the daemon composes them, rather
    /// than a request per shelf: a collection nobody wrote frontend code for has
    /// to arrive the same way as one that did.
    pub(super) fn load_collections(&self) -> Task<Message> {
        self.load_collections_after(std::time::Duration::ZERO)
    }

    /// Fetches the dashboard's collections after `delay`.
    ///
    /// The delay exists for the write that produces them: publishing a source's
    /// rows is a network round trip on the daemon's side, so reading the
    /// collections straight away would find them still empty.
    pub(super) fn load_collections_after(&self, delay: std::time::Duration) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                tokio::time::sleep(delay).await;
                client
                    .list_collections()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::CollectionsLoaded(result)),
        )
    }
}
