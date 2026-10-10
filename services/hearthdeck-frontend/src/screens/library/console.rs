//! The console grid's loaded pages.
//!
//! A console is served a page at a time, so the console grid is not one list
//! that arrives whole: it is a grow-only log of the pages fetched so far, plus
//! the cursor that extends it. This module is that log, and it knows nothing
//! about the daemon or the screen - it decides *what* to ask for, and
//! [`crate::app`] performs the request.
//!
//! The pages of one scope are never interleaved. Changing scope - a tab, a
//! facet, a search term - starts a new generation, and a page carrying an older
//! generation is dropped rather than merged into a list it no longer describes:
//! a slow answer for a console the user has left must not paint onto the one
//! they are looking at.
//!
//! The log keeps what it has until the replacement arrives, so a reload does not
//! blank the grid: the list on screen stays until the first page of the new
//! scope lands, then becomes that page.

use std::sync::Arc;

use cosmic::desktop::DesktopEntryData;

use crate::screens::library::grid::Metrics;

/// One page of a source's records: the items, the source's own total for the
/// whole scope, and the offset the page starts at.
#[derive(Debug, Clone)]
pub struct Page {
    pub entries: Vec<Arc<DesktopEntryData>>,
    pub total: u64,
    pub offset: u32,
}

/// The pages loaded for the console scope on screen.
#[derive(Debug, Default)]
pub struct ConsoleListing {
    entries: Vec<Arc<DesktopEntryData>>,
    /// The offset to ask for next, or `None` when the scope is fully loaded.
    next_offset: Option<u32>,
    /// The generation of the request in flight, if there is one.
    pending: Option<u64>,
    /// Bumped by every first-page load, so a page from before it can be told
    /// apart from a page that still describes the scope on screen.
    generation: u64,
}

impl ConsoleListing {
    /// The records loaded so far, in the source's own order.
    #[must_use]
    pub fn entries(&self) -> &[Arc<DesktopEntryData>] {
        &self.entries
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
            // Nothing to extend until the new scope reports its own total.
            self.next_offset = None;
        }
        self.pending = Some(self.generation);
        Some(self.generation)
    }

    /// Merges an arriving page and reports whether it belonged to the scope on
    /// screen. A page from a generation that has since been replaced is dropped.
    ///
    /// A first page replaces the log; a later one is appended to it, so a record
    /// already drawn keeps its position as further pages land.
    pub fn apply(&mut self, generation: u64, page: Page) -> bool {
        if generation != self.generation {
            return false;
        }
        self.pending = None;
        if page.offset == 0 {
            self.entries.clear();
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
    /// flight; a failure from a replaced generation concerns a scope the user
    /// has already left, and is ignored.
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cosmic::desktop::DesktopEntryData;

    use super::{ConsoleListing, Page};
    use crate::screens::library::grid::{Metrics, metrics};

    /// A page starting at `offset`, holding `count` records of `total`.
    fn page(offset: u32, count: usize, total: u64) -> Page {
        Page {
            entries: (0..count)
                .map(|index| entry(offset as usize + index))
                .collect(),
            total,
            offset,
        }
    }

    fn entry(index: usize) -> Arc<DesktopEntryData> {
        Arc::new(DesktopEntryData {
            id: format!("romm:{index}"),
            name: format!("Game {index}"),
            ..DesktopEntryData::default()
        })
    }

    /// Loads one page the way [`crate::app`] does, and reports whether it was
    /// accepted as the scope on screen.
    fn load(listing: &mut ConsoleListing, page: Page) -> bool {
        let generation = listing.start(page.offset).expect("the load is allowed");
        listing.apply(generation, page)
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
    fn a_page_from_a_scope_the_user_has_left_is_dropped() {
        let mut listing = ConsoleListing::default();
        let abandoned = listing.start(0).expect("the load is allowed");
        load(&mut listing, page(0, 60, 100));

        // The user changes scope while the first load is still out; the answer
        // to the load they abandoned arrives after the new one.
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
    fn a_failure_from_a_replaced_scope_is_ignored() {
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
}
