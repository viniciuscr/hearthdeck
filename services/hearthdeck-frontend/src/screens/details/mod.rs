//! The console game's details screen: its state.
//!
//! The screen is seeded from the grid tile that opened it — the title, cover
//! and version list are already in memory — so it never opens blank, then
//! filled in by the daemon's per-game read. `info` staying `None` is a normal,
//! temporary state: the seed alone is a complete screen for the fields it
//! carries.
//!
//! Everything here is state and pure decision: what the page shows, what a
//! press changes, and how an answer settles. The daemon work is performed by
//! [`crate::app`], which turns the intents below into tasks.

pub mod focus;
mod view;

use std::path::PathBuf;

use cosmic::desktop::fde::IconSource;
use cosmic::widget::icon;

pub use view::view;

use crate::fl;
use crate::icon_cache::entry_icon_handle;
use crate::providers::daemon::{RetroDetails, RetroGameDetails, RetroRomVersion};
use crate::style::{DETAILS_HERO_RASTER, DETAILS_SHOT_RASTER};
use crate::system_status::human_size;

/// Which RomM per-user flag a write answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Favorite,
    Backlog,
}

/// A flag toggle, ready for the shell to write it to the daemon and to the
/// collection the dashboard's rail is fed by.
#[derive(Clone, Debug)]
pub struct FlagWrite {
    /// The file being shown, which is what RomM scopes the flag to.
    pub rom_id: i64,
    pub title: String,
    pub flag: Flag,
    /// The state the press asked for.
    pub value: bool,
    /// The state before the press, which a failure puts back.
    pub previous: Option<bool>,
}

/// What settling a flag write means for the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settled {
    /// An answer for a file the screen has moved past: it no longer describes
    /// what is on screen, so it is dropped.
    Dropped,
    /// The daemon agreed; the shown state is now its state.
    Applied,
    /// The write failed: the previous state is back, and the screen says why.
    Reverted { reason: String },
}

/// What the page's big image is.
///
/// The handles are compared by identity — two handles from the icon cache are
/// the same decoded image — so `PartialEq` is written by hand.
#[derive(Clone, Copy, Debug)]
pub enum Hero<'a> {
    /// A screenshot in its own 16:9 shape: the default big image, whichever
    /// one of the game's shots is picked.
    Screenshot(&'a icon::Handle),
    /// The game's cover, shown only when it has no screenshots — contained,
    /// so an odd-shaped poster is not cropped.
    Cover(&'a icon::Handle),
}

impl PartialEq for Hero<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Screenshot(this), Self::Screenshot(that))
            | (Self::Cover(this), Self::Cover(that)) => std::ptr::eq(*this, *that),
            (Self::Screenshot(_), Self::Cover(_)) | (Self::Cover(_), Self::Screenshot(_)) => false,
        }
    }
}

impl Eq for Hero<'_> {}

/// State of one game's details screen.
pub struct Details {
    /// RomM id of the file being shown: the game's representative, or the disc
    /// the user picked from the picker. Everything the buttons act on — launch,
    /// favorite, play later — is scoped to this id.
    pub rom_id: i64,
    pub title: String,
    /// Every file of the game, the one Play targets first. Only populated for
    /// games RomM collapsed from several files; empty means single-file.
    pub versions: Vec<RetroRomVersion>,
    /// Grid index the screen was opened from, so Back can put the cursor back
    /// on the tile the user came from.
    pub origin_index: usize,
    /// Whether the disc picker overlay is open.
    pub disc_picker: bool,
    /// Cover handle recycled from the grid tile, the fallback art for a game
    /// with no screenshots.
    cover: Option<icon::Handle>,
    /// Full-size cover, once it has been downloaded.
    hero_cover: Option<icon::Handle>,
    /// Screenshot thumbnails, in RomM's order.
    screenshots: Vec<icon::Handle>,
    /// Screenshot the user picked as the big image. `None` is the first one.
    hero_shot: Option<usize>,
    /// Whether the game is in the user's RomM favorites. `None` means the
    /// screen does not know yet (nothing has been read or written for this
    /// file), which the button treats as "off" and a press turns into "add".
    favorite: Option<bool>,
    /// RomM's per-user "play later" flag, same unknown-state handling.
    backlogged: Option<bool>,
    /// A favorite or play-later write is in flight: stops a second press from
    /// racing the first.
    state_pending: bool,
    /// The daemon's detail read. `None` while it is still in flight.
    info: Option<RetroGameDetails>,
    /// Failure text from that read, shown in place of the detail rows.
    error: Option<String>,
    /// Failure text from the last flag write, shown until the next press.
    write_error: Option<String>,
    /// The cursor.
    cursor: focus::Focus,
}

impl Details {
    /// A screen seeded from the grid tile that opens it.
    #[must_use]
    pub fn new(
        rom_id: i64,
        title: String,
        cover: Option<icon::Handle>,
        versions: Vec<RetroRomVersion>,
        origin_index: usize,
    ) -> Self {
        Self {
            rom_id,
            title,
            versions,
            origin_index,
            disc_picker: false,
            cover,
            hero_cover: None,
            screenshots: Vec::new(),
            hero_shot: None,
            favorite: None,
            backlogged: None,
            state_pending: false,
            info: None,
            error: None,
            write_error: None,
            cursor: focus::Focus::default(),
        }
    }

    /// The page's big image: the screenshot being previewed, or the game's
    /// first — a shot is the game's identity on this page, not the box art.
    #[must_use]
    pub fn hero(&self) -> Option<Hero<'_>> {
        match self.picked_shot().or_else(|| self.screenshots.first()) {
            Some(handle) => Some(Hero::Screenshot(handle)),
            None => self.cover_art().map(Hero::Cover),
        }
    }

    /// Art for the full-bleed backdrop: the same screenshot the big image
    /// shows. `None` when the game has none, which leaves the page a flat
    /// surface rather than a stretched, blurry poster.
    #[must_use]
    pub fn backdrop_art(&self) -> Option<&icon::Handle> {
        self.picked_shot().or_else(|| self.screenshots.first())
    }

    /// The picked screenshot, or the first one when nothing was picked.
    fn picked_shot(&self) -> Option<&icon::Handle> {
        self.hero_shot
            .and_then(|index| self.screenshots.get(index))
            .or_else(|| self.screenshots.first())
    }

    /// Poster art for a game with no screenshots: the large cover, otherwise
    /// the grid tile's own.
    fn cover_art(&self) -> Option<&icon::Handle> {
        self.hero_cover.as_ref().or(self.cover.as_ref())
    }

    /// Applies the daemon's detail read, or its failure.
    ///
    /// An answer for a file the screen has moved past is dropped rather than
    /// painted onto what the user picked in the meantime: RomM's per-user
    /// flags are scoped to the file, so a read for another one is wrong about
    /// exactly the fields it would settle. Reports whether it applied.
    pub fn load(&mut self, rom_id: i64, result: Result<RetroDetails, String>) -> bool {
        if self.rom_id != rom_id {
            return false;
        }
        match result {
            Ok(loaded) => {
                // Artwork is decoded here, once, rather than on every frame of
                // the strip. The daemon has already cached every path on disk.
                self.hero_cover = loaded
                    .hero
                    .as_deref()
                    .map(|path| details_handle(path, DETAILS_HERO_RASTER));
                self.screenshots = loaded
                    .screenshots
                    .iter()
                    .map(|path| details_handle(path, DETAILS_SHOT_RASTER))
                    .collect();
                // RomM's per-user state for this file, now known.
                self.favorite = Some(loaded.info.favorite);
                self.backlogged = Some(loaded.info.backlogged);
                self.info = Some(loaded.info);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        true
    }

    /// Shows one of the game's screenshots as the big image.
    pub fn select_shot(&mut self, index: usize) {
        if index < self.screenshots.len() {
            self.hero_shot = Some(index);
        }
    }

    /// Opens the disc picker, or closes it when it is already open, moving the
    /// cursor with the surface: into the picker on the file Play already
    /// targets, back to the Disc button on the way out.
    pub fn toggle_picker(&mut self) {
        if self.versions.len() < 2 {
            return;
        }
        self.disc_picker = !self.disc_picker;
        self.cursor = if self.disc_picker {
            let current = self
                .versions
                .iter()
                .position(|version| version.id == self.rom_id)
                .unwrap_or(0);
            focus::Focus::DiscChoice(current)
        } else {
            focus::Focus::Disc
        };
    }

    /// Points the screen at another file of the same game. Play, favorite and
    /// play later then apply to that file, which is how RomM models them: one
    /// `rom_user` row and one collection membership per file.
    ///
    /// The file's own user state is unknown until it is read, so it is cleared
    /// rather than inherited from the file the user came from.
    pub fn select_disc(&mut self, rom_id: i64) {
        self.disc_picker = false;
        if self.rom_id != rom_id {
            self.rom_id = rom_id;
            self.favorite = None;
            self.backlogged = None;
            self.write_error = None;
        }
        self.cursor = focus::Focus::Play;
    }

    /// What the screen currently offers the cursor, and where the cursor is —
    /// corrected when the row it pointed at is gone.
    #[must_use]
    pub fn layout(&self) -> focus::Layout {
        focus::Layout {
            shots: self.screenshots.len(),
            discs: self.versions.len(),
            picker_open: self.disc_picker,
        }
    }

    /// The cursor, corrected when the row it pointed at is gone.
    #[must_use]
    pub fn cursor(&self) -> focus::Focus {
        focus::clamp(self.layout(), self.cursor)
    }

    /// Moves the cursor onto a control that exists.
    pub fn move_cursor(&mut self, target: focus::Focus) {
        self.cursor = focus::clamp(self.layout(), target);
    }

    /// Adds or removes the current file from a RomM per-user flag, optimistically
    /// so the button answers the press. The write it asks the shell to perform
    /// settles it; a press while one is already in flight is ignored.
    pub fn toggle_flag(&mut self, flag: Flag) -> Option<FlagWrite> {
        if self.state_pending {
            return None;
        }
        let previous = self.flag_state(flag);
        let value = !previous.unwrap_or(false);
        self.set_flag_state(flag, Some(value));
        self.state_pending = true;
        self.write_error = None;
        Some(FlagWrite {
            rom_id: self.rom_id,
            title: self.title.clone(),
            flag,
            value,
            previous,
        })
    }

    /// Settles an optimistic flag write: the daemon's answer becomes the shown
    /// state, or the state it replaced when the write failed — with the reason
    /// on screen rather than a button claiming something RomM never stored.
    pub fn settle_flag(
        &mut self,
        rom_id: i64,
        flag: Flag,
        previous: Option<bool>,
        result: Result<bool, String>,
    ) -> Settled {
        if self.rom_id != rom_id {
            return Settled::Dropped;
        }
        self.state_pending = false;
        match result {
            Ok(state) => {
                self.set_flag_state(flag, Some(state));
                self.write_error = None;
                Settled::Applied
            }
            Err(reason) => {
                self.set_flag_state(flag, previous);
                self.write_error = Some(reason.clone());
                Settled::Reverted { reason }
            }
        }
    }

    /// The shown state of one flag.
    #[must_use]
    pub fn flag_state(&self, flag: Flag) -> Option<bool> {
        match flag {
            Flag::Favorite => self.favorite,
            Flag::Backlog => self.backlogged,
        }
    }

    /// The version Play launches, when the game has more than one file. `None`
    /// for single-file games, where the label would only repeat the title.
    #[must_use]
    pub fn current_version_label(&self) -> Option<&str> {
        if self.versions.len() < 2 {
            return None;
        }
        self.versions
            .iter()
            .find(|version| version.id == self.rom_id)
            .map(|version| version.title.as_str())
    }

    /// Short labels shown under the title: release year, platform, community
    /// score, genres, player count and age rating. Empty fields are skipped so
    /// a sparse game does not leave blank chips.
    pub fn chips(&self) -> Vec<String> {
        let Some(info) = self.info.as_ref() else {
            return Vec::new();
        };
        let mut chips = Vec::new();
        if let Some(year) = info.release_year {
            chips.push(year.to_string());
        }
        if let Some(platform) = info.platform_name.as_deref() {
            chips.push(platform.to_owned());
        }
        if let Some(rating) = info.average_rating {
            chips.push(fl!("community-rating", value = format!("{rating:.0}")));
        }
        chips.extend(info.genres.iter().cloned());
        if let Some(players) = info.player_count.as_deref() {
            chips.push(fl!("players-value", count = players));
        }
        chips.extend(info.age_ratings.iter().cloned());
        // Which file Play launches, when the game has more than one: the disc
        // picker sets it, and without it named here the choice would be
        // invisible on the screen itself.
        if let Some(label) = self.current_version_label() {
            chips.push(label.to_owned());
        }
        chips
    }

    /// The labelled fact rows below the actions: who made it, how it plays,
    /// when it shipped, and the file itself. Anything RomM did not send is
    /// omitted rather than shown empty.
    pub fn facts(&self) -> Vec<(String, String)> {
        let Some(info) = self.info.as_ref() else {
            return Vec::new();
        };
        let mut facts: Vec<(String, String)> = Vec::new();
        let mut push = |label: String, value: String| {
            if !value.is_empty() {
                facts.push((label, value));
            }
        };
        // RomM merges studios and publishers into one list, so the label does
        // not claim to be the developer specifically.
        push(fl!("companies"), info.companies.join(", "));
        push(fl!("play-modes"), info.game_modes.join(", "));
        push(
            fl!("players"),
            info.player_count.clone().unwrap_or_default(),
        );
        push(
            fl!("released"),
            info.release_date.clone().unwrap_or_default(),
        );
        // RomM's own region metadata is often empty where the region tag parsed
        // from the file name is not, so fall back to the tags.
        let regions = if info.regions.is_empty() {
            &info.tags
        } else {
            &info.regions
        };
        push(fl!("regions"), regions.join(", "));
        push(fl!("languages"), info.languages.join(", "));
        push(
            fl!("file-size"),
            info.file_size_bytes.map(human_size).unwrap_or_default(),
        );
        facts
    }

    /// The summary "About" block, when RomM sent one.
    #[must_use]
    pub fn summary(&self) -> Option<&str> {
        self.info.as_ref().and_then(|info| info.summary.as_deref())
    }

    /// Failure from the daemon's detail read, shown in place of the rows.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Failure from the last flag write, shown until the next press.
    #[must_use]
    pub fn write_error(&self) -> Option<&str> {
        self.write_error.as_deref()
    }

    /// The shown state of the favorite flag.
    #[must_use]
    pub fn favorite(&self) -> Option<bool> {
        self.favorite
    }

    /// The shown state of the play-later flag.
    #[must_use]
    pub fn backlogged(&self) -> Option<bool> {
        self.backlogged
    }

    fn set_flag_state(&mut self, flag: Flag, state: Option<bool>) {
        match flag {
            Flag::Favorite => self.favorite = state,
            Flag::Backlog => self.backlogged = state,
        }
    }
}

/// Resolves an artwork path the daemon has already cached on disk into a
/// rasterized icon handle. Building a handle is then a local read rather than
/// a download.
fn details_handle(path: &str, size: u32) -> icon::Handle {
    entry_icon_handle(
        &IconSource::Path(PathBuf::from(path)),
        size,
        crate::icon_cache::Orientation::AsIs,
    )
}

#[cfg(test)]
mod tests {
    use cosmic::widget::icon;

    use super::{Details, Flag, Hero, Settled, focus};
    use crate::providers::daemon::{RetroDetails, RetroGameDetails, RetroRomVersion};

    fn handle() -> icon::Handle {
        icon::from_path(std::path::PathBuf::from("/tmp/none.png"))
    }

    fn info() -> RetroGameDetails {
        // Every field is optional on the wire, so an empty object is a legal
        // answer and the test fills in only what it is about.
        serde_json::from_str("{}").expect("an empty detail payload is legal")
    }

    fn loaded(info: RetroGameDetails) -> RetroDetails {
        RetroDetails {
            info,
            hero: None,
            screenshots: Vec::new(),
        }
    }

    fn versions() -> Vec<RetroRomVersion> {
        vec![
            RetroRomVersion {
                id: 1,
                title: "Disc 1".to_owned(),
                is_main_sibling: true,
            },
            RetroRomVersion {
                id: 2,
                title: "Disc 2".to_owned(),
                is_main_sibling: false,
            },
        ]
    }

    fn seeded() -> Details {
        Details::new(
            1,
            "Metal Gear Solid".to_owned(),
            Some(handle()),
            versions(),
            7,
        )
    }

    fn shot_details(count: usize) -> Details {
        let mut details = Details::new(1, "Game".to_owned(), Some(handle()), Vec::new(), 0);
        // Paths, as the daemon serves them; the screen turns them into handles.
        let screenshots = (0..count)
            .map(|index| format!("/tmp/shot-{index}.png"))
            .collect();
        details.load(
            1,
            Ok(RetroDetails {
                info: info(),
                hero: None,
                screenshots,
            }),
        );
        details
    }

    #[test]
    fn the_big_image_is_the_first_screenshot_not_the_cover() {
        let details = shot_details(3);
        assert!(
            matches!(details.hero(), Some(Hero::Screenshot(_))),
            "the big image should be a screenshot",
        );
        // With no screenshots at all, the cover is the only art there is.
        let cover_only = Details::new(1, "Game".to_owned(), Some(handle()), Vec::new(), 0);
        assert!(
            matches!(cover_only.hero(), Some(Hero::Cover(_))),
            "a game without shots falls back to the cover",
        );
    }

    #[test]
    fn picking_a_screenshot_moves_the_big_image_to_it() {
        let mut details = shot_details(3);
        details.select_shot(2);
        assert_eq!(
            details.hero_shot,
            Some(2),
            "the third shot should be picked"
        );
        // A pick past the end of the strip is ignored rather than blanking.
        details.select_shot(9);
        assert_eq!(
            details.hero_shot,
            Some(2),
            "an out-of-range pick changes nothing"
        );
    }

    #[test]
    fn a_picked_screenshot_is_what_the_backdrop_shows_too() {
        let mut details = shot_details(2);
        assert!(
            details.backdrop_art().is_some(),
            "the first shot is the default"
        );
        details.select_shot(1);
        let hero = match details.hero() {
            Some(Hero::Screenshot(handle)) => Some(handle),
            _ => None,
        };
        let backdrop = details.backdrop_art();
        assert!(
            hero.zip(backdrop)
                .is_some_and(|(left, right)| std::ptr::eq(left, right)),
            "the backdrop and the big image show the same shot",
        );
    }

    #[test]
    fn the_disc_picker_moves_the_cursor_with_the_surface() {
        let mut details = seeded();
        assert_eq!(details.cursor(), focus::Focus::Play);
        details.toggle_picker();
        assert!(details.disc_picker);
        assert_eq!(details.cursor(), focus::Focus::DiscChoice(0));
        details.toggle_picker();
        assert!(!details.disc_picker);
        assert_eq!(details.cursor(), focus::Focus::Disc);
    }

    #[test]
    fn a_single_file_game_has_no_picker_to_open() {
        let mut details = Details::new(1, "Game".to_owned(), None, Vec::new(), 0);
        details.toggle_picker();
        assert!(
            !details.disc_picker,
            "a single-file game cannot open the picker"
        );
    }

    #[test]
    fn picking_a_disc_clears_the_flags_it_could_not_have_known() {
        let mut details = seeded();
        details.load(1, Ok(loaded(info())));
        assert_eq!(details.favorite, Some(false));
        details.favorite = Some(true);
        details.select_disc(2);
        assert_eq!(details.rom_id, 2, "the screen points at the picked file");
        assert_eq!(details.favorite, None, "the new file's state is unknown");
        assert_eq!(details.cursor(), focus::Focus::Play);
    }

    #[test]
    fn a_toggle_is_optimistic_and_a_second_press_waits_for_the_first() {
        let mut details = seeded();
        let write = details
            .toggle_flag(Flag::Favorite)
            .expect("the press writes");
        assert!(write.value, "unknown state toggles to on");
        assert_eq!(write.previous, None);
        assert_eq!(details.favorite, Some(true), "the button answers the press");
        assert!(
            details.toggle_flag(Flag::Favorite).is_none(),
            "a second press must not race the write in flight",
        );
    }

    #[test]
    fn a_failed_write_puts_the_previous_state_back() {
        let mut details = seeded();
        let write = details
            .toggle_flag(Flag::Backlog)
            .expect("the press writes");
        let settled = details.settle_flag(
            write.rom_id,
            write.flag,
            write.previous,
            Err("daemon is down".to_owned()),
        );
        assert_eq!(
            settled,
            Settled::Reverted {
                reason: "daemon is down".to_owned()
            }
        );
        assert_eq!(details.backlogged, None, "the optimistic state is put back");
        assert_eq!(details.write_error(), Some("daemon is down"));
    }

    #[test]
    fn an_answer_for_a_file_the_screen_left_is_dropped() {
        let mut details = seeded();
        details.toggle_flag(Flag::Favorite);
        details.select_disc(2);
        let settled = details.settle_flag(1, Flag::Favorite, None, Ok(true));
        assert_eq!(settled, Settled::Dropped);
        assert_eq!(details.favorite, None, "a stale answer changes nothing");
    }

    #[test]
    fn a_detail_read_for_another_file_does_not_paint() {
        let mut details = seeded();
        assert!(!details.load(99, Ok(loaded(info()))));
        assert!(details.info.is_none(), "the stale read paints nothing");
        assert!(details.load(1, Ok(loaded(info()))));
    }

    #[test]
    fn chips_skip_fields_romm_did_not_send() {
        let mut details = shot_details(0);
        let mut info = info();
        info.release_year = Some(1998);
        info.genres = vec!["Tactical".to_owned()];
        details.load(1, Ok(loaded(info)));
        assert_eq!(
            details.chips(),
            vec!["1998".to_owned(), "Tactical".to_owned()]
        );
    }

    #[test]
    fn the_version_play_launches_is_named_only_for_multi_file_games() {
        let mut details = seeded();
        assert_eq!(details.current_version_label(), Some("Disc 1"));
        details.select_disc(2);
        assert_eq!(details.current_version_label(), Some("Disc 2"));
        let single = Details::new(1, "Game".to_owned(), None, versions()[..1].to_vec(), 0);
        assert_eq!(single.current_version_label(), None);
    }

    #[test]
    fn facts_fall_back_to_the_file_tags_for_regions() {
        let mut details = shot_details(0);
        let mut info = info();
        info.tags = vec!["NA".to_owned()];
        info.file_size_bytes = Some(1024);
        details.load(1, Ok(loaded(info)));
        let facts = details.facts();
        assert!(
            facts
                .iter()
                .any(|(label, value)| label == "Regions" && value == "NA"),
            "a tag parsed from the file name is the region: {facts:?}",
        );
        assert!(
            facts.iter().any(|(_, value)| value == "1.0 KB"),
            "the file size is human-readable: {facts:?}",
        );
    }
}
