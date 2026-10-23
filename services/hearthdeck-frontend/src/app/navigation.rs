//! What a navigation event means: stepping, confirming and Back, which is
//! one global contract and is unwound here and nowhere else.

use cosmic::Application;
use cosmic::app::Task;
use cosmic::widget::text_input;

use crate::app::animation::ScrollTarget;
use crate::app_group::Section;
use crate::providers::filter::cycle;
use crate::screens::dashboard;
use crate::screens::library::focus::{Chrome, Confirm, Direction, Outcome};
use crate::screens::library::{focus, grid, header, sidebar};
use crate::subscriptions::gamepad::GamepadEvent;
use crate::widgets::top_bar;

use super::{App, Message, Page, unfocus, widget_focus};

impl App {
    /// Acts on one navigation event.
    ///
    /// The pad and the keyboard both arrive here, so there is one place that
    /// knows what a step, a confirm and Back mean - and no control that decides
    /// it for itself.
    pub(super) fn navigate(&mut self, event: GamepadEvent) -> Task<Message> {
        // The details screen owns the whole surface while it is up: it moves
        // its own cursor and runs its own actions, and the chords belong to the
        // library behind it - switching section under a game's details would
        // change what that screen describes.
        if self.page == Page::Details {
            return match event {
                GamepadEvent::MoveUp => self.step_details(Direction::Up),
                GamepadEvent::MoveDown => self.step_details(Direction::Down),
                GamepadEvent::MoveLeft => self.step_details(Direction::Left),
                GamepadEvent::MoveRight => self.step_details(Direction::Right),
                GamepadEvent::Confirm => self.confirm_details(),
                GamepadEvent::Back => self.back(),
                GamepadEvent::Search
                | GamepadEvent::ContextMenu
                | GamepadEvent::PrevGroup
                | GamepadEvent::NextGroup
                | GamepadEvent::PrevTab
                | GamepadEvent::NextTab
                | GamepadEvent::FilterPanel
                | GamepadEvent::Keyboard => Task::none(),
            };
        }
        match event {
            GamepadEvent::MoveUp => self.step_focus(Direction::Up),
            GamepadEvent::MoveDown => self.step_focus(Direction::Down),
            GamepadEvent::MoveLeft => self.step_focus(Direction::Left),
            GamepadEvent::MoveRight => self.step_focus(Direction::Right),
            GamepadEvent::Confirm => self.confirm_focus(),
            GamepadEvent::Back => self.back(),
            // Search puts the caret in the search box - which, off the library,
            // means showing it first, the same way the bar's Search button does.
            GamepadEvent::Search => self.update(Message::OpenSearch),
            // Start+Select is the keyboard. The search box is the text field the
            // pad can reach on its own, so the chord puts the caret there rather
            // than raising a keyboard with nothing to type into - and once the
            // box has it, the keyboard takes the pad (see the gamepad arm of
            // `update`).
            GamepadEvent::Keyboard => self.update(Message::OpenSearch),
            GamepadEvent::ContextMenu => self.context_action(),
            // The shoulder buttons switch section and the triggers switch tab.
            // They are chords rather than places on the screen, so they wrap
            // where a step stops at the end of the grid.
            GamepadEvent::PrevGroup => self.switch_section(-1),
            GamepadEvent::NextGroup => self.switch_section(1),
            GamepadEvent::PrevTab => self.switch_tab(-1),
            GamepadEvent::NextTab => self.switch_tab(1),
            // The drawer is a surface over the grid rather than a place in it,
            // so it has its own button instead of being walked to.
            GamepadEvent::FilterPanel => self.update(Message::ToggleFilterPanel),
        }
    }

    /// Moves the cursor one step, and the renderer's focus with it.
    pub(super) fn step_focus(&mut self, direction: Direction) -> Task<Message> {
        // The menu is drawn over whichever page is behind it, so it takes the
        // step before the page does.
        if self.menu_is_up() {
            return self.move_menu(direction);
        }
        // The top bar is the chrome both sibling pages wear, and while it holds
        // the cursor it takes every step: sideways along its buttons, and Down
        // off it and into the content below.
        if self.bar_cursor.is_some() {
            return self.step_bar(direction);
        }
        if self.page == Page::Dashboard {
            return self.step_dashboard(direction);
        }
        // The settings screen's rows are not walkable yet - they are controls
        // the pad has no cursor for - so any step lands on the one row it can
        // walk, its bar, rather than moving a cursor over the library the
        // screen does not show.
        if self.page == Page::Settings {
            self.bar_cursor = Some(top_bar::Destination::Settings.index());
            return self.focus_bar_button();
        }
        // A step belongs to the content: a D-pad press takes the cursor back
        // from the keyboard's ring rather than walking along the chrome.
        self.cursors.ring = None;
        match focus::step(self.cursors, direction, self.layout()) {
            Outcome::Tile { tile, column } => {
                self.cursors.tile = tile;
                self.cursors.column = column;
                let focus = self.focus_task();
                let scroll = self.scroll_to_tile(tile);
                Task::batch([focus, scroll])
            }
            // Inside the drawer the row is the control, so the cursor moves
            // between rows and nothing is repainted: the drawer draws its own
            // cursor, because a row is a container and cannot hold focus.
            Outcome::FilterRow(row) => {
                self.cursors.filter_row = row;
                Task::none()
            }
            // Sideways on a row is that row's own control: the value steps and
            // the cursor stays where the user is looking.
            Outcome::FilterValue { row, delta } => self.cycle_facet(row, delta),
            Outcome::Stayed => Task::none(),
        }
    }

    /// Acts on whatever holds the cursor: a ring stop, a drawer row, or a tile.
    pub(super) fn confirm_focus(&mut self) -> Task<Message> {
        // The menu is the innermost surface, on either page.
        if self.menu_is_up() {
            return self.activate_menu_row();
        }
        // The top bar's button under the cursor, on whichever page wears the
        // bar - the same action a click on it asks for.
        if let Some(index) = self.bar_cursor {
            return self.activate_bar(index);
        }
        if self.page == Page::Dashboard {
            return self.confirm_dashboard();
        }
        // The settings screen's rows are not walkable yet, so there is nothing
        // on it to confirm - and above all nothing that may be read as the
        // library's tiles, which this screen does not show.
        if self.page == Page::Settings {
            return Task::none();
        }
        match focus::confirm(self.cursors, self.layout()) {
            Confirm::Tile(index) => self.update(Message::Activate(index)),
            // A pad whose left and right do not work still has to be able to
            // change a facet, so confirming a row steps it forward.
            Confirm::Filter { row } => self.cycle_facet(row, 1),
            Confirm::Chrome(chrome) => self.activate_chrome(chrome),
            Confirm::Nothing => Task::none(),
        }
    }

    /// Acts on a chrome stop the keyboard's ring is on.
    ///
    /// These are the actions the pad's own buttons ask for, so a keyboard
    /// reaches exactly what a controller does and the two cannot drift apart.
    pub(super) fn activate_chrome(&mut self, chrome: Chrome) -> Task<Message> {
        match chrome {
            Chrome::Section(index) => match Section::ALL.get(index) {
                Some(section) => self.update(Message::SelectSection(*section)),
                None => Task::none(),
            },
            // 0 is the section's "all" tab; what follows it are its groups,
            // which is the numbering the header and the ring share.
            Chrome::Tab(index) => self.update(Message::SelectGroup(index.checked_sub(1))),
            Chrome::Search => self.focus_search(),
            Chrome::FilterButton => self.update(Message::ToggleFilterPanel),
        }
    }

    /// Steps the facet drawn on `row` through its values, which is what the
    /// drawer's own arrows do.
    pub(super) fn cycle_facet(&mut self, row: usize, delta: i32) -> Task<Message> {
        let Some(facet) = self.facets.get(row) else {
            return Task::none();
        };
        let current = self.facet_selection.get(&facet.id).map(String::as_str);
        let value = cycle(facet, current, delta);
        // The same message the drawer's arrows send, so a pad-driven selection
        // and a clicked one cannot drift apart.
        self.update(Message::SelectFacet {
            facet: facet.id.clone(),
            value,
        })
    }

    /// The X button: what the user can do with what the cursor is on.
    pub(super) fn context_action(&mut self) -> Task<Message> {
        // An open menu takes the button back: it is the innermost surface, so
        // the press closes it before it reaches anything behind it.
        if self.menu.is_open() {
            self.menu.close();
            return Task::none();
        }
        // The menu belongs to the library grid: on a sibling page it would open
        // over a tile the screen does not show, and from then on the pad would
        // walk that invisible menu instead of the page in front of it.
        if self.page != Page::Library {
            return Task::none();
        }
        // Inside the drawer, X is the one thing a drawer cannot do for itself:
        // clear every facet without travelling to a button for it. The clear
        // action is deliberately not a cursor row, so the cursor never has to
        // walk to it.
        if self.filter_open {
            return self.update(Message::ClearFilters);
        }
        // A console game's own screen answers "what can I do with this": Run,
        // favorite, play later and every file of a multi-disc game live there,
        // so X opens it rather than stacking a menu of the same actions.
        if self.cursors.tile < self.visible.len() {
            let index = self.cursors.tile;
            if self.visible[index].id.starts_with("romm:") {
                return self.update(Message::OpenDetails(index));
            }
            return self.update(Message::TileContext(index));
        }
        Task::none()
    }

    /// Back: what leaves the screen, innermost surface first.
    ///
    /// This is the whole global Back contract. Back that behaves differently
    /// depending on where the user is would be worse than no Back at all, so no
    /// screen and no drawer handles it for itself - they are unwound here, in
    /// the order the surfaces stack, ending on the Dashboard.
    pub(super) fn back(&mut self) -> Task<Message> {
        // A text field owns the caret, so the keyboard is up over everything:
        // Back dismisses the input and the keyboard first, keeps the screen
        // open, and hands the pad back to it. Nothing else can be reached
        // while the keyboard has the pad, so this is the whole dismissal.
        if self.text_focused {
            self.text_focused = false;
            return unfocus();
        }
        // A modal dialog sits over every surface, so Back closes it first.
        if self.romm.sign_in.is_some() {
            return self.update(Message::Romm(
                crate::screens::settings::romm::Message::SignInClosed,
            ));
        }
        // A failed launch is a message to read, so it is the innermost surface.
        if self.launch_state.error().is_some() {
            return self.update(Message::DismissLaunch);
        }
        // A launch in flight is not something to back out of.
        if self.launch_state.is_visible() {
            return Task::none();
        }
        // The menu is drawn over everything there is, so it is the surface Back
        // reaches first.
        if self.menu.is_open() {
            self.menu.close();
            return Task::none();
        }
        // The drawer is a surface of its own: it closes before Back reaches the
        // grid behind it, and the pad goes back to the tile it left - which the
        // cursors kept, because the drawer has a row of its own rather than
        // being a place in the grid.
        if self.filter_open {
            return self.update(Message::CloseFilterPanel);
        }
        // The details screen is a place of its own, over the grid it was opened
        // from: Back leaves it for that grid rather than past it, and closes
        // its disc picker first - a surface stacked over the page.
        if self.page == Page::Details {
            if self
                .details
                .as_ref()
                .is_some_and(|details| details.disc_picker)
            {
                return self.update(Message::DetailsToggleDiscPicker);
            }
            return self.update(Message::CloseDetails);
        }
        // The top bar is chrome rather than a surface over anything: Back steps
        // off it into the content the way it steps out of the library's ring.
        // The settings screen has no content cursor below its bar, so Back there
        // leaves the page itself, as it already did.
        if self.bar_cursor.is_some() && self.page == Page::Dashboard {
            self.bar_cursor = None;
            return self.focus_dashboard_card();
        }
        // The ring is the keyboard's surface: Back steps out of it into the
        // content, the same way it leaves the drawer.
        if self.cursors.ring.is_some() {
            self.cursors.ring = None;
            return self.focus_task();
        }
        // With nothing else open, Back leaves the library and settings alike for
        // the dashboard - the one page with nothing behind it.
        if self.page != Page::Dashboard {
            self.switch_page(Page::Dashboard);
            self.cursors.ring = None;
            self.bar_cursor = None;
            let rails = self.dashboard_rail_lengths();
            self.dashboard_cursor = dashboard::focus::clamp(self.dashboard_cursor, &rails);
            return Task::batch([self.focus_dashboard_card(), self.refresh_dashboard()]);
        }
        self.focus_task()
    }

    /// Puts the caret in the search box, which is what the pad's search button
    /// and the ring's stop at it both ask for.
    ///
    /// The cursor itself stays where it is: the box takes the *caret*, not the
    /// pad, so leaving it again puts the user back on the tile they were on.
    pub(super) fn focus_search(&mut self) -> Task<Message> {
        self.cursors.ring = None;
        self.bar_cursor = None;
        // A text field is about to own the caret, so the keyboard is coming up
        // and the pad belongs to it. `on_focus` only reports a click; a caret
        // the shell places through a task has to be recorded here, or the pad
        // would keep driving the screen under the keyboard.
        self.text_focused = true;
        text_input::focus(header::search_id())
    }

    /// Switches section, wrapping at either end.
    pub(super) fn switch_section(&mut self, delta: i32) -> Task<Message> {
        let sections = Section::ALL.len() as i32;
        let index = (self.cur_section.index() as i32 + delta).rem_euclid(sections);
        match Section::ALL.get(index as usize) {
            Some(section) => self.update(Message::SelectSection(*section)),
            None => Task::none(),
        }
    }

    /// Switches tab within the section on screen, wrapping over that section's
    /// "all" tab and its groups.
    pub(super) fn switch_tab(&mut self, delta: i32) -> Task<Message> {
        let tabs = self.config.sections.get(self.cur_section).len() as i32 + 1;
        let current = self.cur_group.map_or(0, |group| group as i32 + 1);
        let next = (current + delta).rem_euclid(tabs);
        self.update(Message::SelectGroup(
            next.checked_sub(1).map(|group| group as usize),
        ))
    }

    /// Scrolls the tab strip so the tab at `index` is on screen.
    ///
    /// The strip follows the selection the way a dashboard rail follows its
    /// card: it moves only when the tab the selection lands on would leave the
    /// visible width, and then pins it to the nearest edge. Where it is *going*
    /// is recorded before the animation starts, because a scroll the shell
    /// drives does not come back as an `on_scroll` event - the same reason the
    /// dashboard remembers each rail's offset.
    pub(super) fn reveal_tab(&mut self, index: usize) -> Task<Message> {
        let widths =
            header::tab_widths(self.cur_section, self.config.sections.get(self.cur_section));
        let viewport = header::tab_viewport(self.window_width, !self.facets.is_empty());
        let from = self.tabs_offset;
        let Some(offset) = header::tab_scroll_target(&widths, index, from, viewport) else {
            return Task::none();
        };
        self.tabs_offset = offset;
        self.animate_scroll_to(ScrollTarget::Tabs, from, offset)
    }

    /// Puts the tab strip back at its start, for when the section above it
    /// changes and the strip holds a different set of tabs altogether.
    pub(super) fn reset_tab_strip(&mut self) -> Task<Message> {
        let from = self.tabs_offset;
        self.tabs_offset = 0.0;
        self.animate_scroll_to(ScrollTarget::Tabs, from, 0.0)
    }

    /// One stop along the chrome ring, which is how a keyboard reaches what the
    /// pad reaches by a button. Walking past an end hands the cursor back to the
    /// content, which is where the pad always leaves it.
    ///
    /// The ring is input, so it waits for the same ownership the pad's events do
    /// rather than moving the cursor of a screen a game is covering.
    ///
    /// What the ring walks is the page's own chrome: the library's sidebar,
    /// header and tabs on the library, and the top bar - the only chrome there
    /// is - on the sibling pages.
    pub(super) fn walk_ring(&mut self, backwards: bool) -> Task<Message> {
        if !self.input_ownership.frontend_has_control() {
            return Task::none();
        }
        if self.page == Page::Library {
            self.cursors.ring = focus::ring_step(self.cursors.ring, backwards, self.layout());
            let focus = self.focus_task();
            let stop = self
                .cursors
                .ring
                .and_then(|index| focus::ring_at(index, self.layout()));
            // Landing on the search box puts the caret in it, so the keyboard
            // comes up and takes the pad - the same state the pad's own search
            // button leaves behind. Any other stop hands focus to a button, so
            // there is no text field left with the caret.
            self.text_focused = stop == Some(Chrome::Search);
            // A tab the ring stops on is a tab the user is looking at, so the
            // strip follows it the way it follows a selection.
            let reveal = match stop {
                Some(Chrome::Tab(index)) => self.reveal_tab(index),
                _ => Task::none(),
            };
            return Task::batch([focus, reveal]);
        }
        self.bar_cursor = top_bar::ring_step(self.bar_cursor, backwards);
        match self.bar_cursor {
            Some(_) => self.focus_bar_button(),
            // Past either end the cursor goes back to the content - the rail
            // card it left, which the dashboard's own cursor kept.
            None if self.page == Page::Dashboard => self.focus_dashboard_card(),
            None => Task::none(),
        }
    }

    /// Points the renderer's focus at whatever holds the cursor, which is where
    /// a button's `focused` style flag - and so the design system's focus ring -
    /// comes from.
    ///
    /// The drawer is the exception: its rows are containers, and a container
    /// cannot hold focus, so they are drawn from the cursor itself (see
    /// `Filters::cursor`).
    pub(super) fn focus_task(&self) -> Task<Message> {
        if let Some(chrome) = self
            .cursors
            .ring
            .and_then(|index| focus::ring_at(index, self.layout()))
        {
            return match chrome {
                Chrome::Section(index) => Section::ALL.get(index).map_or(Task::none(), |section| {
                    widget_focus(sidebar::section_id(*section))
                }),
                Chrome::Tab(index) => widget_focus(header::tab_id(index)),
                Chrome::Search => text_input::focus(header::search_id()),
                Chrome::FilterButton => widget_focus(header::filter_button_id()),
            };
        }
        focus::focused_tile(self.cursors, self.layout())
            .and_then(|tile| self.visible.get(tile))
            .map_or(Task::none(), |entry| widget_focus(grid::tile_id(entry)))
    }

    /// Moves the grid so the cursor stays on screen.
    ///
    /// The viewport follows the cursor rather than the other way round: a step
    /// that would take the tile off the visible area scrolls the grid to put its
    /// row back inside, and a step within the viewport does not move the page at
    /// all. Without this the cursor walks off the bottom of the screen and the
    /// grid behind it never moves.
    ///
    /// The new offset is recorded before the scrollable reports it, because the
    /// grid is virtualized: the rows built are the rows around the offset, so a
    /// row not yet inside it does not exist as a widget - and `focus` on a widget
    /// that does not exist resolves to nothing. The scrollable confirms the
    /// offset on the next frame, through `GridScrolled`.
    pub(super) fn scroll_to_tile(&mut self, tile: usize) -> Task<Message> {
        let metrics = grid::metrics(self.window_width);
        let row = tile / metrics.columns.max(1);
        let from = self.grid_scroll_offset;
        let Some(offset) = metrics.scroll_to_row(
            self.visible.len(),
            row,
            self.grid_scroll_offset,
            self.grid_viewport_height,
        ) else {
            return Task::none();
        };
        // The grid is virtualized, so the offset it is *going* to is recorded now -
        // the rows around it are the ones that get built - while the scrollable is
        // eased there.
        self.grid_scroll_offset = offset;
        self.animate_scroll_to(ScrollTarget::Grid, from, offset)
    }
}
