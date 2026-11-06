//! The dashboard's rails and the cursor that walks them and the top bar.

use cosmic::app::Task;
use cosmic::widget::Id;
use cosmic::{Application, theme};

use crate::app::animation::ScrollTarget;
use crate::app_group::{Section, SectionUi};
use crate::fl;
use crate::providers::daemon::{Collection, NEW_ARRIVALS_COLLECTION};
use crate::screens::dashboard;
use crate::screens::dashboard::{Card, Shelf};
use crate::screens::library::focus::Direction;
use crate::style::{
    DASHBOARD_RAIL_TILES, content_horizontal_padding, dashboard_console_tile_size,
    dashboard_tile_size,
};
use crate::widgets::top_bar;

use super::{App, Message, Page, dashboard_card_id, navigate, widget_focus};

/// One dashboard rail, named by what it is drawn from.
enum Rail<'a> {
    Collection(&'a Collection),
    Consoles,
    Library,
}

impl Rail<'_> {
    fn kind(&self) -> dashboard::RailKind {
        match self {
            Self::Consoles => dashboard::RailKind::Consoles,
            Self::Collection(_) | Self::Library => dashboard::RailKind::Games,
        }
    }

    fn card_id(&self, index: usize) -> Id {
        let name = match self {
            Self::Collection(collection) => &collection.slug,
            Self::Consoles => "consoles",
            Self::Library => "library",
        };
        dashboard_card_id(name, index)
    }
}

impl App {
    pub(super) fn update_dashboard(&mut self, message: dashboard::Message) -> Task<Message> {
        match message {
            dashboard::Message::CardPressed { id, title } => self.launch_managed(id, title),
            dashboard::Message::ConsolePressed(platform_id) => {
                self.switch_page(Page::Library);
                self.bar_cursor = None;
                let group = self
                    .tabs
                    .console_games
                    .iter()
                    .position(|group| group.romm_platform_id() == Some(platform_id));
                self.apply_section_with_group(Section::ConsoleGames, group)
            }
            dashboard::Message::DestinationPressed(destination) => {
                self.update(navigate(destination))
            }
            dashboard::Message::RailScrolled { rail, offset } => {
                // The next pad step measures from the target, not an intermediate animation frame.
                if !self.scrolls.is_moving(ScrollTarget::Rail(rail)) {
                    self.set_rail_offset(rail, offset);
                }
                Task::none()
            }
            dashboard::Message::PageScrolled(offset) => {
                if !self.scrolls.is_moving(ScrollTarget::Page) {
                    self.page_offset = offset;
                }
                Task::none()
            }
        }
    }

    /// The dashboard's rails: the daemon's collections, in the order it composes
    /// them, with a consoles rail closing the page when RomM is configured.
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
        self.dashboard_rails()
            .into_iter()
            .filter_map(|rail| match rail {
                Rail::Collection(collection) => Some(Self::collection_shelf(collection, size)),
                Rail::Consoles => Some(self.consoles_shelf()),
                Rail::Library => self.library_shelf(size),
            })
            .collect()
    }

    /// Which rails the dashboard has, in order, without building a card.
    ///
    /// What has just arrived on Stremio comes first, in the banner's old place:
    /// it is a collection rail like every other, drawn on top with the wide art
    /// that fills the window behind the page. Then the rest of the daemon's
    /// collections in the order it composes them, and the RomM consoles last -
    /// secondary navigation, so it closes the page rather than opening it.
    fn dashboard_rails(&self) -> Vec<Rail<'_>> {
        let (arrivals, rest): (Vec<_>, Vec<_>) = self
            .collections
            .iter()
            .filter(|collection| !collection.items.is_empty())
            .partition(|collection| collection.slug == NEW_ARRIVALS_COLLECTION);
        let mut rails: Vec<Rail<'_>> = arrivals.into_iter().map(Rail::Collection).collect();
        rails.extend(rest.into_iter().map(Rail::Collection));
        if !self.console_platforms.is_empty() {
            rails.push(Rail::Consoles);
        }
        if rails.is_empty() {
            rails.push(Rail::Library);
        }
        rails
    }

    /// Consoles rail: shows all RomM platforms with their artwork.
    fn consoles_shelf(&self) -> Shelf {
        let console_size = dashboard_console_tile_size(
            self.window_width,
            content_horizontal_padding(),
            theme::spacing().space_l,
        );
        let cards: Vec<Card> = self
            .console_platforms
            .iter()
            .take(DASHBOARD_RAIL_TILES)
            .enumerate()
            .map(|(index, platform)| {
                Card::new(
                    Rail::Consoles.card_id(index),
                    platform.label.clone(),
                    crate::icon_cache::reference_handle(
                        platform.artwork.as_deref(),
                        console_size as u32,
                        // A platform's logo is drawn square on the card, so
                        // there is nothing to turn.
                        crate::icon_cache::Orientation::AsIs,
                    ),
                    Some(dashboard::Message::ConsolePressed(platform.id)),
                )
            })
            .collect();
        Shelf::new(true, fl!("consoles"), cards)
    }

    /// One collection as a rail: its items as cards, in the order the daemon
    /// composes them. A card's own art is its portrait poster; its wide art
    /// rides along for the window's background to draw while the pad is on it.
    fn collection_shelf(collection: &Collection, size: f32) -> Shelf {
        let cards = collection
            .items
            .iter()
            .take(DASHBOARD_RAIL_TILES)
            .enumerate()
            .map(|(index, item)| {
                Card::new(
                    Rail::Collection(collection).card_id(index),
                    item.name.clone(),
                    crate::icon_cache::reference_handle(
                        item.icon.as_deref(),
                        size as u32,
                        // A rail card is a portrait poster: a horizontal box is
                        // turned to fill it.
                        crate::icon_cache::Orientation::Portrait,
                    ),
                    Some(dashboard::Message::card_pressed(&item.item_id, &item.name)),
                )
                .with_background(item.background.as_deref().map(|background| {
                    crate::icon_cache::reference_handle(
                        Some(background),
                        size as u32,
                        // The window's background is a wide surface: it takes
                        // the art as it is.
                        crate::icon_cache::Orientation::AsIs,
                    )
                }))
            })
            .collect();
        Shelf::new(false, dashboard::rail_title(collection), cards)
    }

    /// The library as a single rail, for when the daemon has composed nothing yet
    /// - a fresh install, or a fetch that has not landed.
    fn library_shelf(&self, size: f32) -> Option<Shelf> {
        let cards: Vec<Card> = self
            .catalog
            .entries()
            .iter()
            .filter(|entry| Section::PcGames.matches(entry))
            .take(DASHBOARD_RAIL_TILES)
            .enumerate()
            .map(|(index, entry)| {
                Card::new(
                    Rail::Library.card_id(index),
                    entry.name.clone(),
                    crate::icon_cache::entry_icon_handle(
                        &entry.icon,
                        size as u32,
                        crate::icon_cache::Orientation::Portrait,
                    ),
                    Some(dashboard::Message::card_pressed(&entry.id, &entry.name)),
                )
            })
            .collect();
        (!cards.is_empty()).then(|| Shelf::new(false, fl!("all-games"), cards))
    }

    /// How many cards each rail holds, which is the shape the rail navigation
    /// walks. Counted from the source data: every pad step asks, and building
    /// the cards would resolve every card's artwork to answer.
    pub(super) fn dashboard_rail_lengths(&self) -> Vec<usize> {
        self.dashboard_rails()
            .iter()
            .map(|rail| self.dashboard_rail_length(rail))
            .filter(|&cards| cards > 0)
            .collect()
    }

    fn dashboard_rail_length(&self, rail: &Rail<'_>) -> usize {
        let count = match rail {
            Rail::Collection(collection) => collection.items.len(),
            Rail::Consoles => self.console_platforms.len(),
            Rail::Library => self
                .catalog
                .entries()
                .iter()
                .filter(|entry| Section::PcGames.matches(entry))
                .count(),
        };
        count.min(DASHBOARD_RAIL_TILES)
    }

    fn dashboard_card_id_at(&self, rail: usize, card: usize) -> Option<Id> {
        let rails = self.dashboard_rails();
        let rail = rails.get(rail)?;
        (card < self.dashboard_rail_length(rail)).then(|| rail.card_id(card))
    }

    pub(super) fn dashboard_confirm_message(&self) -> Option<Message> {
        let lengths = self.dashboard_rail_lengths();
        let (rail, card) = dashboard::focus::confirm(self.dashboard_cursor, &lengths)?;
        let message = match self.dashboard_rails().get(rail)? {
            Rail::Collection(collection) => {
                let item = collection.items.get(card)?;
                Some(dashboard::Message::card_pressed(&item.item_id, &item.name))
            }
            Rail::Consoles => self
                .console_platforms
                .get(card)
                .map(|platform| dashboard::Message::ConsolePressed(platform.id)),
            Rail::Library => self
                .catalog
                .entries()
                .iter()
                .filter(|entry| Section::PcGames.matches(entry))
                .nth(card)
                .map(|entry| dashboard::Message::card_pressed(&entry.id, &entry.name)),
        }?;
        Some(Message::Dashboard(message))
    }

    /// Steps the dashboard's cursor, and keeps the card it lands on on screen.
    pub(super) fn step_dashboard(&mut self, direction: Direction) -> Task<Message> {
        let rails = self.dashboard_rail_lengths();
        let before = self.dashboard_cursor;
        self.dashboard_cursor = dashboard::focus::step(self.dashboard_cursor, direction, &rails);
        self.sync_backdrop();
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
                // rule a vertical step between rails follows.
                let remembered = self.dashboard_cursor.remembered;
                self.dashboard_cursor = dashboard::focus::clamp(
                    dashboard::focus::Cursor {
                        rail: 0,
                        card: remembered,
                        remembered,
                    },
                    &self.dashboard_rail_lengths(),
                );
                self.sync_backdrop();
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
        match self.dashboard_confirm_message() {
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
        let rails = self.dashboard_rails();
        let Some(source) = rails.get(self.dashboard_cursor.rail) else {
            return Task::none();
        };
        let count = self.dashboard_rail_length(source);
        if self.dashboard_cursor.card >= count {
            return Task::none();
        }
        let kind = source.kind();
        let kinds: Vec<_> = rails.iter().map(Rail::kind).collect();
        let mut tasks = vec![widget_focus(source.card_id(self.dashboard_cursor.card))];

        // The rail follows the cursor sideways: it moves only when the card
        // would leave the visible width, and then it pins that card to the
        // nearest edge.
        let (card_width, gap) = dashboard::card_pitch(kind, self.window_width);
        let rail = self.dashboard_cursor.rail;
        let rail_offset = self.rail_offsets.get(rail).copied().unwrap_or(0.0);
        if let Some(offset) = dashboard::focus::rail_scroll_target(
            self.dashboard_cursor.card,
            count,
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

        // ...and the page follows it down the stack of rails, which is the same
        // rule a third time: rails of unequal height rather than cards of equal
        // width. The dashboard fills the content column, so the page's viewport
        // is the window's height.
        let heights = dashboard::rail_heights(&kinds, self.window_width, self.window_height);
        // The page sits below the top bar, so its viewport is the window's height
        // less the bar rather than the whole window.
        let page_viewport = self.window_height - top_bar::height();
        if let Some(offset) = dashboard::focus::page_scroll_target(
            &heights,
            self.dashboard_cursor.rail,
            dashboard::rail_gap(),
            self.page_offset,
            page_viewport,
            dashboard::page_extent(&kinds, self.window_width, self.window_height),
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

    /// The card the pad is on, named by its own widget id: the one identity a
    /// reload cannot renumber.
    pub(super) fn dashboard_cursor_card(&self) -> Option<Id> {
        self.dashboard_card_id_at(self.dashboard_cursor.rail, self.dashboard_cursor.card)
    }

    /// Puts the pad back on the card it was on after the rails changed under
    /// it, and lands the page under it.
    ///
    /// Rails arrive late - the RomM consoles append themselves at the end once
    /// the server answers - and empty ones drop out, so a rail's number is not
    /// its identity: a rail that moves renumbers every rail past the pad. The
    /// card's own id names its collection and its place in it, so the pad goes
    /// back to the card it chose; a card that is gone leaves the cursor clamped
    /// to the new rails.
    pub(super) fn reland_dashboard(&mut self, was: Option<Id>) -> Task<Message> {
        if let Some(was) = was {
            for (rail, source) in self.dashboard_rails().iter().enumerate() {
                if let Some(card) = (0..self.dashboard_rail_length(source))
                    .find(|&card| source.card_id(card) == was)
                {
                    self.dashboard_cursor = dashboard::focus::Cursor {
                        rail,
                        card,
                        remembered: card,
                    };
                    self.sync_backdrop();
                    return self.focus_dashboard_card();
                }
            }
        }
        let rails = self.dashboard_rail_lengths();
        self.dashboard_cursor = dashboard::focus::clamp(self.dashboard_cursor, &rails);
        self.sync_backdrop();
        self.focus_dashboard_card()
    }

    /// Fetches the dashboard's collections, which are what its rails are made of.
    ///
    /// One request for all of them, in the order the daemon composes them, rather
    /// than a request per shelf: a collection nobody wrote frontend code for has
    /// to arrive the same way as one that did.
    pub(super) fn load_collections(&mut self) -> Task<Message> {
        self.load_collections_after(std::time::Duration::ZERO)
    }

    /// Fetches the dashboard's collections after `delay`.
    ///
    /// The delay exists for the write that produces them: publishing a source's
    /// rows is a network round trip on the daemon's side, so reading the
    /// collections straight away would find them still empty.
    pub(super) fn load_collections_after(&mut self, delay: std::time::Duration) -> Task<Message> {
        let request = self.collections_request.begin();
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                tokio::time::sleep(delay).await;
                client
                    .list_collections()
                    .await
                    .map_err(|error| error.to_string())
            },
            move |result| cosmic::Action::App(Message::CollectionsLoaded(request, result)),
        )
    }
}
