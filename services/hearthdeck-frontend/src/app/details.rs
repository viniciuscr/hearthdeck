//! The details screen's side of the shell: what the presses there perform.
//!
//! [`crate::screens::details`] owns the screen's state and its view; this
//! block turns its intents into daemon work and back. It is an `impl App`
//! block beside the shell rather than inside `app/mod.rs`, the split
//! [`crate::app::animation`] takes, so that file keeps the state declaration
//! and the view composition. A child module of `app` sees `App`'s private
//! fields, so the split is a move, not a widening of visibility.

use cosmic::Element;
use cosmic::app::Task;
use cosmic::iced::widget::stack;
use cosmic::widget::{container, space};
use tracing::warn;

use crate::icon_cache::entry_icon_handle;
use crate::providers::daemon::{
    CollectionItem, DaemonClient, FAVORITES_COLLECTION, PLAY_LATER_COLLECTION,
};
use crate::screens::details::{Details, Effect, Flag, FlagWrite, Message as DetailsMessage, focus};
use crate::screens::library::focus::Direction;
use crate::style::DETAILS_HERO_RASTER;

use super::{App, Message, Page, view_launch_overlay, widget_focus};

impl App {
    /// Opens the details screen for one console grid tile.
    ///
    /// The screen is seeded from the tile rather than opened blank: the title,
    /// cover and the files the game collapsed are already in memory, and the
    /// daemon's read fills the rest in behind it.
    pub(super) fn open_details(&mut self, index: usize) -> Task<Message> {
        let Some(entry) = self.visible.get(index).cloned() else {
            return Task::none();
        };
        let Some(rom_id) = entry
            .id
            .strip_prefix("romm:")
            .and_then(|id| id.parse::<i64>().ok())
        else {
            warn!(item_id = %entry.id, "details requested for a non-RomM entry");
            return Task::none();
        };
        // The details screen's hero is a wide backdrop, not a card: it takes the
        // cover in its own orientation, so a landscape box is never turned here.
        let cover = entry_icon_handle(
            &entry.icon,
            DETAILS_HERO_RASTER,
            crate::icon_cache::Orientation::AsIs,
        );
        // Read from the scope the grid showed this entry on, which is the scope
        // on screen at the moment details open.
        let versions = self
            .console
            .versions(&self.console_scope(), &entry.id)
            .to_vec();
        self.details = Some(Details::new(
            rom_id,
            entry.name.clone(),
            Some(cover),
            versions,
            index,
        ));
        self.switch_page(Page::Details);
        Task::batch([
            self.focus_details(focus::Focus::Play),
            details_read_task(self.daemon_client.clone(), rom_id),
        ])
    }

    /// Leaves the details screen for the grid it was opened from, putting the
    /// cursor back on the tile the user came in through.
    ///
    /// The state itself outlives the screen: the fade-out draws the page it is
    /// leaving, so it is dropped with that fade in
    /// [`App::expire_page_fade`](super::App::expire_page_fade).
    pub(super) fn close_details(&mut self) -> Task<Message> {
        let Some(details) = self.details.as_ref() else {
            return Task::none();
        };
        let origin = details.origin_index;
        self.switch_page(Page::Library);
        let last = self.visible.len().saturating_sub(1);
        self.cursors.tile = origin.min(last);
        Task::batch([self.focus_task(), self.scroll_to_tile(self.cursors.tile)])
    }

    pub(super) fn update_details(&mut self, message: DetailsMessage) -> Task<Message> {
        if let DetailsMessage::Loaded { rom_id, result } = &message {
            if let Err(error) = &**result {
                warn!(rom_id, %error, "details read failed");
            }
            // The fade retains the old screen, but late reads cannot repaint it.
            if self.page != Page::Details {
                return Task::none();
            }
        }
        let Some(details) = self.details.as_mut() else {
            return Task::none();
        };
        let effect = details.update(message);
        match effect {
            Effect::Nothing => Task::none(),
            Effect::Close => self.close_details(),
            Effect::Launch { item_id, title } => self.launch_managed(item_id, title),
            Effect::WriteFlag(write) => flag_write_task(self.daemon_client.clone(), write),
            Effect::ReloadCollections => self.load_collections(),
            Effect::Focus(target) => self.focus_details(target),
            Effect::Read { rom_id, focus } => Task::batch([
                self.focus_details(focus),
                details_read_task(self.daemon_client.clone(), rom_id),
            ]),
        }
    }

    /// Moves the details cursor one row or one control sideways.
    pub(super) fn step_details(&mut self, direction: Direction) -> Task<Message> {
        let Some(details) = self.details.as_ref() else {
            return Task::none();
        };
        match focus::step(details.layout(), details.cursor(), direction) {
            Some(target) => self.focus_details(target),
            // At the edge of a row a step does nothing at all.
            None => Task::none(),
        }
    }

    /// Acts on the details control the cursor is on - the same action its
    /// button sends, so the pad and the pointer cannot drift apart.
    pub(super) fn confirm_details(&mut self) -> Task<Message> {
        let Some(details) = self.details.as_ref() else {
            return Task::none();
        };
        match details.confirm_message() {
            Some(message) => self.update_details(message),
            None => Task::none(),
        }
    }

    /// Moves the details cursor onto a control that exists, and the renderer's
    /// focus with it.
    fn focus_details(&mut self, target: focus::Focus) -> Task<Message> {
        let Some(details) = self.details.as_mut() else {
            return Task::none();
        };
        details.move_cursor(target);
        let cursor = details.cursor();
        widget_focus(focus::widget_id(cursor))
    }

    /// The details screen, drawn as the whole window.
    pub(super) fn view_details(&self) -> Element<'_, Message> {
        let screen: Element<'_, Message> = match self.details.as_ref() {
            Some(details) => {
                crate::screens::details::view(details, self.window_width).map(Message::Details)
            }
            None => container(space::horizontal()).into(),
        };
        // A launch takes over the screen it was started from: the game is
        // starting, so the details page is no longer what the user is acting
        // on. The layer exists only for a launch in flight or a failure, so
        // seeing the game's details draws the details and nothing else.
        if let Some(overlay) = self.launch_state.overlay() {
            stack![screen, view_launch_overlay(overlay)].into()
        } else {
            screen
        }
    }
}

/// The daemon's detail read for one file, or its failure.
fn details_read_task(client: DaemonClient, rom_id: i64) -> Task<Message> {
    Task::perform(
        async move {
            client
                .retro_rom_details(rom_id)
                .await
                .map_err(|error| error.to_string())
        },
        move |result| {
            cosmic::Action::App(Message::Details(DetailsMessage::Loaded {
                rom_id,
                result: Box::new(result),
            }))
        },
    )
}

/// Writes one flag toggle, and the collection membership that goes with it.
///
/// The flag and the collection answer two different consumers: RomM draws the
/// button's state from the flag, and the dashboard's rails are built from the
/// collection. One press is one logical write, so it is settled as one.
fn flag_write_task(client: DaemonClient, write: FlagWrite) -> Task<Message> {
    let FlagWrite {
        rom_id,
        title,
        flag,
        value,
        previous,
    } = write;
    let collection = match flag {
        Flag::Favorite => FAVORITES_COLLECTION,
        Flag::Backlog => PLAY_LATER_COLLECTION,
    };
    Task::perform(
        async move {
            let written = match flag {
                Flag::Favorite => client.set_retro_favorite(rom_id, value).await,
                Flag::Backlog => client.set_retro_backlogged(rom_id, value).await,
            };
            written.map_err(|error| error.to_string())?;
            let item = CollectionItem {
                item_id: format!("romm:{rom_id}"),
                name: title,
                icon: None,
                background: None,
                logo: None,
                description: None,
                played: None,
            };
            let membership = if value {
                client.add_collection_item(collection, &item).await
            } else {
                client
                    .remove_collection_item(collection, &item.item_id)
                    .await
            };
            membership.map_err(|error| error.to_string())?;
            Ok(value)
        },
        move |result| {
            cosmic::Action::App(Message::Details(DetailsMessage::FlagWritten {
                rom_id,
                flag,
                previous,
                result,
            }))
        },
    )
}
