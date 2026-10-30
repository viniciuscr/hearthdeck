//! The console grid's loaded pages, cached per scope.
//!
//! A console is served a page at a time, so the console grid is not one list
//! that arrives whole: every scope - a console tab narrowed by the search term
//! and the facet selection - holds a grow-only log of the pages fetched for it,
//! plus the cursor that extends it. This module is those logs: it decides *what*
//! to ask for, and [`crate::app`] performs the request.
//!
//! The pages of one scope are never mixed with another's. A scope is a value
//! ([`ConsoleScope`]) keyed into a map rather than one log every tab pours pages
//! into, which is what makes switching tabs draw the tab that was switched to -
//! its own cached pages the moment the tab changes, nothing at all while its
//! first page is out - instead of leaving the last tab's games on the grid until
//! the answer arrives. Opening console B never clears console A's pages, so
//! switching back is instant - though the cache is bounded ([`ConsoleScopes`])
//! and keeps only the searches used most recently warm beside the tabs. A tab
//! is never dropped.
//!
//! Within one scope the log keeps what it has until the replacement arrives, so
//! a reload does not blank the grid: the list stays until the first page of the
//! new load lands, then becomes that page. A page carrying a generation the log
//! has since replaced is dropped rather than merged into a list it no longer
//! describes: a slow answer for a load the user superseded must not paint onto
//! what they are looking at.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use crate::library_item::LibraryItem;

use crate::providers::daemon::RetroRomVersion;
use crate::screens::library::grid::Metrics;

/// How many scopes the background prefetch may have in flight at once, so
/// warming the tab strip does not open every console's covers at the same time.
const MAX_PREFETCH_INFLIGHT: usize = 3;

/// How many narrowed scopes - a tab narrowed by a search or a facet selection -
/// the cache keeps beside the tabs. Tabs are bounded by the strip and are never
/// dropped; a narrowed scope is a query, and only the most recently used
/// queries stay warm, so a long session of searching cannot grow the cache
/// without bound.
const MAX_NARROWED_SCOPES: usize = 8;

/// Which console grid a page belongs to: the platform tab it is filed under,
/// plus everything the source narrows it by. Two scopes never share pages,
/// however close they are - "SNES" and "SNES searched for zelda" are different
/// lists holding different records.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConsoleScope {
    /// The RomM platform id of the tab in scope; `None` is the "all consoles"
    /// tab, which is also what a tab with no platform behind it falls back to.
    pub platform: Option<i64>,
    /// The search term, as the source applies it.
    pub search: String,
    /// The facet selection, as the source applies it.
    pub facets: BTreeMap<String, String>,
}

impl ConsoleScope {
    /// One tab with no search and no filters on it: what a plain tab switch
    /// opens on, and what the background prefetch warms.
    #[must_use]
    pub fn plain(platform: Option<i64>) -> Self {
        Self {
            platform,
            search: String::new(),
            facets: BTreeMap::new(),
        }
    }

    /// Whether this is a plain tab rather than a query. Tabs are bounded by
    /// the strip and never leave the cache; narrowed scopes are what it drops
    /// when it is full.
    fn is_plain(&self) -> bool {
        self.search.is_empty() && self.facets.is_empty()
    }
}

/// One page of a source's records: the items, the source's own total for the
/// whole scope, and the offset the page starts at.
#[derive(Debug, Clone)]
pub struct Page {
    pub entries: Vec<Arc<LibraryItem>>,
    /// The version lists of the multi-file entries on this page, keyed by
    /// entry id, kept separate from the grid's display items.
    pub versions: BTreeMap<String, Vec<RetroRomVersion>>,
    pub total: u64,
    pub offset: u32,
}

/// The pages loaded for one scope.
#[derive(Debug, Default)]
pub struct ConsoleListing {
    entries: Vec<Arc<LibraryItem>>,
    /// Version lists of the multi-file entries loaded so far, keyed by entry
    /// id. The details screen reads them rather than re-deriving them from its
    /// own detail payload, so its disc picker works the moment it opens.
    versions: BTreeMap<String, Vec<RetroRomVersion>>,
    /// The offset to ask for next, or `None` when the scope is fully loaded.
    next_offset: Option<u32>,
    /// The generation of the request in flight, if there is one.
    pending: Option<u64>,
    /// Bumped by every first-page load, so a page from before it can be told
    /// apart from a page that still describes the scope on screen.
    generation: u64,
    /// Whether the scope has ever answered with a page. An empty console is an
    /// answer: its scope is loaded and must not be asked again.
    loaded: bool,
    /// When the scope was last asked for or written, in [`ConsoleScopes`]'s
    /// clock. A full cache drops the narrowed scope carrying the oldest of
    /// these.
    used: u64,
}

impl ConsoleListing {
    /// The records loaded so far, in the source's own order.
    #[must_use]
    pub fn entries(&self) -> &[Arc<LibraryItem>] {
        &self.entries
    }

    /// The files a console entry collapsed, its own file first. The details
    /// screen's disc picker reads them from here rather than re-deriving them
    /// from its own detail payload, so it works the moment the screen opens.
    #[must_use]
    pub fn versions(&self, entry_id: &str) -> &[RetroRomVersion] {
        self.versions.get(entry_id).map_or(&[], Vec::as_slice)
    }

    /// Begins a load of the page at `offset`, returning the generation to tag
    /// the request with, or `None` when the request must not be made.
    ///
    /// Offset 0 opens a new generation, which abandons a page still in flight -
    /// [`Self::apply`] drops it when it lands. A later page is refused while one
    /// is already in flight, so paging cannot stack requests up.
    pub fn start(&mut self, offset: u32) -> Option<u64> {
        if offset > 0 && self.pending.is_some() {
            return None;
        }
        if offset == 0 {
            self.generation += 1;
            // Nothing to extend until the new load reports its own total.
            self.next_offset = None;
        }
        self.pending = Some(self.generation);
        Some(self.generation)
    }

    /// Merges an arriving page and reports whether it belonged to the load in
    /// flight. A page from a generation that has since been replaced is dropped.
    ///
    /// A first page replaces the log; a later one is appended to it, so a record
    /// already drawn keeps its position as further pages land.
    pub fn apply(&mut self, generation: u64, page: Page) -> bool {
        if generation != self.generation {
            return false;
        }
        self.pending = None;
        self.loaded = true;
        if page.offset == 0 {
            self.entries.clear();
            self.versions.clear();
        }
        for (entry_id, versions) in page.versions {
            self.versions.insert(entry_id, versions);
        }
        // The cursor is where this page ends, measured from the source's own
        // numbering rather than from the log, so a short page cannot make the
        // log believe it holds more than it does.
        let loaded = page.offset as usize + page.entries.len();
        let more = !page.entries.is_empty() && loaded < page.total as usize;
        self.entries.extend(page.entries);
        self.next_offset = more.then_some(loaded as u32);
        true
    }

    /// Records that the load tagged `generation` came back without a page, so
    /// the log stops waiting for it. Reports whether that load was the one in
    /// flight; a failure from a replaced generation concerns a load the user
    /// has already superseded, and is ignored.
    pub fn failed(&mut self, generation: u64) -> bool {
        if generation != self.generation {
            return false;
        }
        self.pending = None;
        true
    }

    /// The offset to load next, if the viewport has reached the end of what is
    /// loaded and the scope has more to give.
    ///
    /// The list is measured in the grid's own rows ([`Metrics`]), so "the end is
    /// on screen" is exact. The next page is requested while the last row is
    /// still visible - one row early - which is early enough that reaching the
    /// bottom is not a wait.
    #[must_use]
    pub fn extend_offset(
        &self,
        scroll_offset: f32,
        viewport_height: f32,
        metrics: Metrics,
    ) -> Option<u32> {
        let next = self.next_offset?;
        if self.pending.is_some() {
            return None;
        }
        let content_height = metrics.content_height(self.entries.len());
        (scroll_offset + viewport_height >= content_height - metrics.row_height).then_some(next)
    }
}

/// What showing a scope asks of [`crate::app`].
#[derive(Debug, PartialEq, Eq)]
pub enum Ensure {
    /// The scope answers this query already: draw the pages it holds.
    Cached,
    /// Its first page is on the way; the grid repaints when it lands.
    Loading,
    /// The scope has never answered: this is the page-0 request to make, tagged
    /// with the generation its answer must carry.
    Requested(u64),
}

/// Every scope's log, plus the queue that warms them in the background.
///
/// The map is the whole point: a tab switch reads the scope it lands on and
/// nothing else, and a scope's pages stay warm for the next time it is opened.
/// The warmth is bounded: tabs are few and never dropped, and only the
/// [`MAX_NARROWED_SCOPES`] narrowed scopes used most recently are kept, so a
/// long session of searching cannot grow the cache without bound.
#[derive(Debug, Default)]
pub struct ConsoleScopes {
    scopes: BTreeMap<ConsoleScope, ConsoleListing>,
    /// The scopes the background prefetch still owes a first page, in the order
    /// they were queued.
    prefetch: VecDeque<ConsoleScope>,
    /// The counter [`Self::touch`] stamps scopes with, so a full cache can
    /// drop the one that has gone longest without being used.
    clock: u64,
}

impl ConsoleScopes {
    /// Forget data from the previous server without accepting its in-flight pages.
    pub fn invalidate(&mut self) {
        self.prefetch.clear();
        for log in self.scopes.values_mut() {
            *log = ConsoleListing {
                generation: log.generation + 1,
                ..ConsoleListing::default()
            };
        }
    }

    /// The records loaded for `scope` so far, in the source's own order. Empty
    /// for a scope that has never answered.
    #[must_use]
    pub fn entries(&self, scope: &ConsoleScope) -> &[Arc<LibraryItem>] {
        self.scopes.get(scope).map_or(&[], ConsoleListing::entries)
    }

    /// The files a scope's entry collapsed, its own file first.
    #[must_use]
    pub fn versions(&self, scope: &ConsoleScope, entry_id: &str) -> &[RetroRomVersion] {
        self.scopes
            .get(scope)
            .map_or(&[], |log| log.versions(entry_id))
    }

    /// What showing `scope` asks for: its cached pages, a first page already on
    /// its way, or the page-0 request to make.
    ///
    /// This is what makes opening a tab instant: a scope that has answered is
    /// drawn straight from its log and no request is made at all.
    pub fn ensure(&mut self, scope: &ConsoleScope) -> Ensure {
        self.touch(scope);
        if let Some(log) = self.scopes.get(scope) {
            if log.loaded {
                return Ensure::Cached;
            }
            if log.pending.is_some() {
                return Ensure::Loading;
            }
        }
        match self.begin(scope, 0) {
            Some(generation) => Ensure::Requested(generation),
            // A first page is never refused. If one ever were, the request the
            // log is waiting for would repaint when it lands.
            None => Ensure::Loading,
        }
    }

    /// Begins a load of `scope`'s page at `offset`, returning the generation to
    /// tag the request with, or `None` when the request must not be made.
    ///
    /// Offset 0 opens a new generation, which abandons a page still in flight -
    /// [`Self::apply`] drops it when it lands. A later page is refused while one
    /// is already in flight, so paging cannot stack requests up.
    pub fn begin(&mut self, scope: &ConsoleScope, offset: u32) -> Option<u64> {
        let generation = self.scopes.entry(scope.clone()).or_default().start(offset);
        self.touch(scope);
        self.evict();
        generation
    }

    /// Merges an arriving page into the log of the scope it describes and
    /// reports whether it was accepted. The page is logged whether or not that
    /// scope is on screen - a warm cache is what the next tab switch reads - but
    /// a page from a generation the log has replaced is dropped.
    pub fn apply(&mut self, scope: &ConsoleScope, generation: u64, page: Page) -> bool {
        self.scopes
            .entry(scope.clone())
            .or_default()
            .apply(generation, page)
    }

    /// Records that the load tagged `generation` came back without a page, so
    /// the log stops waiting for it. Reports whether that load was the one the
    /// scope had in flight; a failure from a replaced generation concerns a load
    /// the user has already superseded, and is ignored.
    pub fn failed(&mut self, scope: &ConsoleScope, generation: u64) -> bool {
        self.scopes
            .get_mut(scope)
            .is_some_and(|log| log.failed(generation))
    }

    /// The offset `scope` should load next, if its viewport has reached the end
    /// of what is loaded and the scope has more to give.
    #[must_use]
    pub fn extend_offset(
        &self,
        scope: &ConsoleScope,
        scroll_offset: f32,
        viewport_height: f32,
        metrics: Metrics,
    ) -> Option<u32> {
        self.scopes
            .get(scope)?
            .extend_offset(scroll_offset, viewport_height, metrics)
    }

    /// Queues every tab that has no page yet, so the whole strip warms up in the
    /// background and any console opens from cache. `platforms` is in the order
    /// the tabs stand, the "all consoles" tab first: it is the landing view.
    pub fn queue_prefetch(&mut self, platforms: impl IntoIterator<Item = Option<i64>>) {
        for platform in platforms {
            let scope = ConsoleScope::plain(platform);
            let owed = self
                .scopes
                .get(&scope)
                .is_none_or(|log| !log.loaded && log.pending.is_none());
            if owed && !self.prefetch.contains(&scope) {
                self.prefetch.push_back(scope);
            }
        }
    }

    /// Hands out queued prefetches up to [`MAX_PREFETCH_INFLIGHT`] in flight,
    /// each with the generation its answer must carry. The caller performs them;
    /// calling again after a page lands is what walks the queue on.
    pub fn next_prefetch(&mut self) -> Vec<(ConsoleScope, u64)> {
        let mut ready = Vec::new();
        while self.inflight() < MAX_PREFETCH_INFLIGHT {
            let Some(scope) = self.prefetch.pop_front() else {
                break;
            };
            // Loaded or loading since it was queued: nothing left to do.
            if self
                .scopes
                .get(&scope)
                .is_some_and(|log| log.loaded || log.pending.is_some())
            {
                continue;
            }
            if let Some(generation) = self.begin(&scope, 0) {
                ready.push((scope, generation));
            }
        }
        ready
    }

    /// How many scopes are waiting for a page right now.
    fn inflight(&self) -> usize {
        self.scopes
            .values()
            .filter(|log| log.pending.is_some())
            .count()
    }

    /// Marks `scope` as just used, so a full cache drops some other narrowed
    /// scope first: the one that has gone longest without being asked for is
    /// the one to go. Paging counts too, since it goes through
    /// [`Self::begin`].
    fn touch(&mut self, scope: &ConsoleScope) {
        self.clock += 1;
        let clock = self.clock;
        if let Some(log) = self.scopes.get_mut(scope) {
            log.used = clock;
        }
    }

    /// Drops the least recently used narrowed scopes until no more than
    /// [`MAX_NARROWED_SCOPES`] are left. Tabs are never dropped - the strip
    /// bounds them - and neither is a scope with a page in flight: its answer
    /// still has to land in the log.
    fn evict(&mut self) {
        while self.narrowed() > MAX_NARROWED_SCOPES {
            let Some(oldest) = self
                .scopes
                .iter()
                .filter(|(scope, log)| !scope.is_plain() && log.pending.is_none())
                .min_by_key(|(_, log)| log.used)
                .map(|(scope, _)| scope.clone())
            else {
                // Every narrowed scope is still loading: none may be dropped.
                break;
            };
            self.scopes.remove(&oldest);
        }
    }

    /// How many narrowed scopes the cache holds.
    fn narrowed(&self) -> usize {
        self.scopes.keys().filter(|scope| !scope.is_plain()).count()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use crate::library_item::LibraryItem;

    use super::{
        ConsoleListing, ConsoleScope, ConsoleScopes, Ensure, MAX_NARROWED_SCOPES,
        MAX_PREFETCH_INFLIGHT, Page,
    };
    use crate::providers::daemon::RetroRomVersion;
    use crate::screens::library::grid::{Metrics, metrics};

    /// A page starting at `offset`, holding `count` records of `total`.
    fn page(offset: u32, count: usize, total: u64) -> Page {
        Page {
            entries: (0..count)
                .map(|index| entry(offset as usize + index))
                .collect(),
            versions: BTreeMap::new(),
            total,
            offset,
        }
    }

    fn entry(index: usize) -> Arc<LibraryItem> {
        Arc::new(LibraryItem {
            id: format!("romm:{index}"),
            name: format!("Game {index}"),
            ..LibraryItem::default()
        })
    }

    /// Loads one page the way [`crate::app`] does, and reports whether it was
    /// accepted as the load in flight.
    fn load(listing: &mut ConsoleListing, page: Page) -> bool {
        let generation = listing.start(page.offset).expect("the load is allowed");
        listing.apply(generation, page)
    }

    /// The scope of one console tab, with nothing narrowing it.
    fn scope(platform: i64) -> ConsoleScope {
        ConsoleScope::plain(Some(platform))
    }

    #[test]
    fn changing_server_drops_cached_games_and_rejects_old_pages() {
        let mut scopes = ConsoleScopes::default();
        let scope = scope(7);
        let old = scopes.begin(&scope, 0).unwrap();
        assert!(scopes.apply(&scope, old, page(0, 1, 2)));
        let pending = scopes.begin(&scope, 1).unwrap();
        scopes.invalidate();
        assert!(scopes.entries(&scope).is_empty());
        assert!(!scopes.apply(&scope, pending, page(1, 1, 2)));
        let Ensure::Requested(new) = scopes.ensure(&scope) else {
            panic!("the new server must be queried");
        };
        assert_ne!(old, new);
        assert!(!scopes.apply(&scope, old, page(0, 1, 1)));
        assert!(scopes.apply(&scope, new, page(0, 1, 1)));
    }

    /// The scope of one console tab narrowed by a search term.
    fn search(platform: i64, term: &str) -> ConsoleScope {
        ConsoleScope {
            platform: Some(platform),
            search: term.to_owned(),
            facets: BTreeMap::new(),
        }
    }

    /// Loads one page into a scope the way [`crate::app`] does.
    fn load_scope(scopes: &mut ConsoleScopes, scope: &ConsoleScope, page: Page) -> bool {
        let generation = scopes
            .begin(scope, page.offset)
            .expect("the load is allowed");
        scopes.apply(scope, generation, page)
    }

    /// The generation of the page-0 request [`ConsoleScopes::ensure`] asks for.
    fn request(scopes: &mut ConsoleScopes, scope: &ConsoleScope) -> u64 {
        match scopes.ensure(scope) {
            Ensure::Requested(generation) => generation,
            other => panic!("the scope should be requested, got {other:?}"),
        }
    }

    /// The metrics a 1200px window lays the grid out with.
    fn grid() -> Metrics {
        metrics(1200.0)
    }

    /// Scrolled to the very end of what is loaded, with one row on screen.
    fn at_the_end(listing: &ConsoleListing) -> Option<u32> {
        let metrics = grid();
        let content = metrics.content_height(listing.entries().len());
        listing.extend_offset(content, metrics.row_height, metrics)
    }

    #[test]
    fn a_first_page_replaces_the_log_and_points_at_the_page_after_it() {
        let mut listing = ConsoleListing::default();

        assert!(load(&mut listing, page(0, 60, 100)));
        assert_eq!(listing.entries().len(), 60);
        assert_eq!(at_the_end(&listing), Some(60));

        assert!(load(&mut listing, page(60, 40, 100)));
        assert_eq!(listing.entries().len(), 100);
        assert_eq!(at_the_end(&listing), None);
    }

    #[test]
    fn a_later_page_is_appended_rather_than_replacing_the_log() {
        let mut listing = ConsoleListing::default();
        load(&mut listing, page(0, 4, 8));
        load(&mut listing, page(4, 4, 8));

        let names: Vec<String> = listing
            .entries()
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        let expected: Vec<String> = (0..8).map(|index| format!("Game {index}")).collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn a_page_from_a_superseded_reload_is_dropped() {
        let mut listing = ConsoleListing::default();
        let abandoned = listing.start(0).expect("the load is allowed");
        load(&mut listing, page(0, 60, 100));

        // The scope is reloaded while the first load is still out; the answer to
        // the load that was abandoned arrives after the new one.
        assert!(!listing.apply(abandoned, page(60, 40, 100)));
        assert_eq!(listing.entries().len(), 60);
    }

    #[test]
    fn a_later_page_is_refused_while_one_is_in_flight() {
        let mut listing = ConsoleListing::default();
        let generation = listing.start(0).expect("the load is allowed");

        assert_eq!(
            listing.start(60),
            None,
            "two pages at once would interleave in arrival order"
        );

        listing.apply(generation, page(0, 60, 100));
        assert!(
            listing.start(60).is_some(),
            "the slot frees up once the page it was waiting for lands"
        );
    }

    /// A page that lands after a failure must not be applied twice or left
    /// waiting: one failure clears the scope for the next request.
    #[test]
    fn a_failed_page_stops_the_log_waiting_for_it() {
        let mut listing = ConsoleListing::default();
        let generation = listing.start(0).expect("the load is allowed");
        assert!(listing.failed(generation));

        assert!(listing.start(60).is_some());
    }

    #[test]
    fn a_failure_from_a_superseded_reload_is_ignored() {
        let mut listing = ConsoleListing::default();
        let abandoned = listing.start(0).expect("the load is allowed");
        listing.start(0).expect("a first page is never refused");

        assert!(!listing.failed(abandoned));
    }

    /// Reaching the bottom is what pulls the next page in - and only then: a
    /// viewport still showing rows above the end asks for nothing.
    #[test]
    fn the_next_page_is_asked_for_only_at_the_end_of_what_is_loaded() {
        let mut listing = ConsoleListing::default();
        load(&mut listing, page(0, 60, 500));

        let metrics = grid();
        let content = metrics.content_height(listing.entries().len());

        assert_eq!(
            listing.extend_offset(0.0, metrics.row_height, metrics),
            None
        );
        assert_eq!(
            listing.extend_offset(
                content - 2.0 * metrics.row_height,
                metrics.row_height,
                metrics
            ),
            Some(60)
        );
    }

    /// A source that stops returning records ends the scope, even if its total
    /// says otherwise. Without this, a page that comes back empty would be asked
    /// for again on every scroll, forever.
    #[test]
    fn an_empty_page_ends_the_scope() {
        let mut listing = ConsoleListing::default();
        load(&mut listing, page(0, 60, 500));
        load(&mut listing, page(60, 0, 500));

        assert_eq!(at_the_end(&listing), None);
    }

    /// The version lists the details screen reads are kept with the log and
    /// replaced with it: a later page adds its own without touching what is
    /// already loaded, and a new first page clears the scope it no longer
    /// describes.
    #[test]
    fn version_lists_are_kept_with_the_log_and_replaced_with_it() {
        let mut listing = ConsoleListing::default();
        let mut first = page(0, 1, 2);
        first.versions.insert(
            "romm:0".to_owned(),
            vec![RetroRomVersion {
                id: 0,
                title: "Disc 1".to_owned(),
                is_main_sibling: true,
            }],
        );
        load(&mut listing, first);
        assert_eq!(listing.versions("romm:0").len(), 1);
        assert!(listing.versions("romm:1").is_empty());

        let mut later = page(1, 1, 2);
        later.versions.insert(
            "romm:1".to_owned(),
            vec![RetroRomVersion {
                id: 1,
                title: "Disc 1".to_owned(),
                is_main_sibling: true,
            }],
        );
        load(&mut listing, later);
        assert_eq!(
            listing.versions("romm:0").len(),
            1,
            "a later page adds without clearing what came before it"
        );
        assert_eq!(listing.versions("romm:1").len(), 1);

        // A new first page is a new load: its version lists go with it.
        load(&mut listing, page(0, 1, 1));
        assert!(
            listing.versions("romm:0").is_empty(),
            "the version lists of the load the user left are gone"
        );
        assert!(listing.versions("romm:1").is_empty());
    }

    /// The regression the per-scope cache exists for: one tab's pages must never
    /// be readable as another tab's.
    #[test]
    fn a_page_for_one_scope_is_never_mixed_into_another() {
        let mut scopes = ConsoleScopes::default();
        let snes = scope(7);
        let playstation = scope(9);
        load_scope(&mut scopes, &snes, page(0, 2, 2));

        assert!(
            scopes.entries(&playstation).is_empty(),
            "the SNES pages leaked into the PlayStation scope"
        );

        assert!(load_scope(&mut scopes, &playstation, page(0, 1, 1)));
        let snes_ids: Vec<&str> = scopes
            .entries(&snes)
            .iter()
            .map(|entry| entry.id.as_str())
            .collect();
        assert_eq!(
            snes_ids,
            ["romm:0", "romm:1"],
            "the PlayStation page moved SNES"
        );
    }

    #[test]
    fn a_scope_that_has_answered_is_drawn_from_its_cache_without_a_request() {
        let mut scopes = ConsoleScopes::default();
        let snes = scope(7);
        let generation = request(&mut scopes, &snes);
        assert!(scopes.apply(&snes, generation, page(0, 1, 1)));

        assert_eq!(scopes.ensure(&snes), Ensure::Cached);
        assert_eq!(scopes.entries(&snes).len(), 1);
    }

    #[test]
    fn a_scope_with_its_first_page_still_out_is_not_requested_twice() {
        let mut scopes = ConsoleScopes::default();
        let snes = scope(7);
        let _ = request(&mut scopes, &snes);

        assert_eq!(scopes.ensure(&snes), Ensure::Loading);
    }

    #[test]
    fn a_console_with_no_games_is_loaded_and_never_reasked() {
        let mut scopes = ConsoleScopes::default();
        let snes = scope(7);
        let generation = request(&mut scopes, &snes);
        assert!(scopes.apply(&snes, generation, page(0, 0, 0)));

        assert_eq!(
            scopes.ensure(&snes),
            Ensure::Cached,
            "an empty console is an answer, not a reason to fetch forever"
        );
    }

    #[test]
    fn prefetch_warms_every_tab_under_one_cap() {
        let mut scopes = ConsoleScopes::default();
        scopes.queue_prefetch([None, Some(7), Some(9), Some(11)]);

        let first = scopes.next_prefetch();
        assert_eq!(
            first.len(),
            MAX_PREFETCH_INFLIGHT,
            "warming the strip opens at most {MAX_PREFETCH_INFLIGHT} covers at once"
        );

        // One page lands; only then does the queue hand out the next scope.
        let (scope, generation) = &first[0];
        assert!(scopes.apply(scope, *generation, page(0, 1, 1)));
        assert_eq!(scopes.next_prefetch().len(), 1);

        assert!(scopes.next_prefetch().is_empty(), "the queue is drained");
    }

    #[test]
    fn prefetch_leaves_out_tabs_that_already_answered_or_are_loading() {
        let mut scopes = ConsoleScopes::default();
        let snes = scope(7);
        load_scope(&mut scopes, &snes, page(0, 1, 1));
        let playstation = scope(9);
        let _ = scopes.ensure(&playstation);

        scopes.queue_prefetch([None, Some(7), Some(9)]);
        let queued: Vec<Option<i64>> = scopes
            .next_prefetch()
            .into_iter()
            .map(|(scope, _)| scope.platform)
            .collect();

        assert_eq!(queued, [None], "only the all-consoles tab is still owed");
    }

    #[test]
    fn the_cache_drops_the_oldest_search_when_it_is_full() {
        let mut scopes = ConsoleScopes::default();
        let searches: Vec<ConsoleScope> = (0..MAX_NARROWED_SCOPES + 2)
            .map(|index| search(7, &format!("q{index}")))
            .collect();
        for scope in &searches {
            load_scope(&mut scopes, scope, page(0, 1, 1));
        }

        assert!(
            scopes.entries(&searches[0]).is_empty(),
            "the oldest search was kept forever and the cache never stops growing"
        );
        assert_eq!(
            scopes.entries(&searches[MAX_NARROWED_SCOPES + 1]).len(),
            1,
            "the searches that fit are all still warm"
        );
    }

    #[test]
    fn a_tab_is_never_dropped_to_make_room_for_a_search() {
        let mut scopes = ConsoleScopes::default();
        let snes = scope(7);
        load_scope(&mut scopes, &snes, page(0, 1, 1));
        for index in 0..MAX_NARROWED_SCOPES + 1 {
            load_scope(&mut scopes, &search(7, &format!("q{index}")), page(0, 1, 1));
        }

        assert_eq!(
            scopes.ensure(&snes),
            Ensure::Cached,
            "a tab is bounded by the strip; the cache must never drop one"
        );
    }

    #[test]
    fn a_search_the_user_returns_to_is_kept_over_one_left_behind() {
        let mut scopes = ConsoleScopes::default();
        let warm = search(7, "zeld");
        load_scope(&mut scopes, &warm, page(0, 1, 1));
        for index in 0..MAX_NARROWED_SCOPES - 1 {
            load_scope(&mut scopes, &search(7, &format!("q{index}")), page(0, 1, 1));
        }
        // The first search is used again, making it the most recent of them.
        assert_eq!(scopes.ensure(&warm), Ensure::Cached);

        load_scope(&mut scopes, &search(7, "last"), page(0, 1, 1));

        assert_eq!(
            scopes.ensure(&warm),
            Ensure::Cached,
            "a search the user keeps coming back to must outlive one they do not"
        );
    }

    #[test]
    fn a_search_waiting_for_its_first_page_is_not_dropped() {
        let mut scopes = ConsoleScopes::default();
        let loading: Vec<ConsoleScope> = (0..MAX_NARROWED_SCOPES + 1)
            .map(|index| search(7, &format!("q{index}")))
            .collect();
        for scope in &loading {
            let _ = request(&mut scopes, scope);
        }

        // None of them has answered yet: dropping one now would drop the page
        // it is still waiting for.
        for scope in &loading {
            assert_eq!(scopes.ensure(scope), Ensure::Loading);
        }
    }
}
