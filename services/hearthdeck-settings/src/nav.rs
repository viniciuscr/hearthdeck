//! Where the cursor is, and what moving it does.
//!
//! The window is read-only, so nothing here changes a setting. What it owns is
//! the one thing the component kit does not provide: a cursor a d-pad can drive.
//! The model is a plain value with no gpui types in it, so the rules are tested
//! without a window - and so the pad transport that lands next drives the same
//! methods the keyboard already does.
//!
//! The rules follow `docs/product-foundations.md`: Up and Down reach every row
//! of the page, Left and Right move between pages, and nothing needs a pointer
//! to become reachable.

/// Which row of which page has the cursor.
///
/// Pages are a horizontal ring, rows are a vertical list that stops at both
/// ends. Switching page puts the cursor on that page's first row, so the next
/// Up or Down is always a step inside the page the user is looking at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Nav {
    /// Row count per page, in the order the pages are drawn.
    rows: Vec<usize>,
    page: usize,
    row: usize,
}

impl Nav {
    /// Re-reads the page sizes after the content changed - a refresh, or a
    /// failed read that emptied a page.
    pub fn resize(&mut self, rows: &[usize]) {
        self.rows.clear();
        self.rows.extend_from_slice(rows);
        if self.page >= self.rows.len() {
            // The page the cursor was on is gone: land on the last one's first
            // row rather than on whatever row index the old page happened to
            // leave behind.
            self.page = self.rows.len().saturating_sub(1);
            self.row = 0;
            return;
        }
        self.clamp();
    }

    pub fn page(&self) -> usize {
        self.page
    }

    pub fn row(&self) -> usize {
        self.row
    }

    pub fn up(&mut self) {
        self.row = self.row.saturating_sub(1);
    }

    pub fn down(&mut self) {
        if self.row + 1 < self.row_count() {
            self.row += 1;
        }
    }

    pub fn previous_page(&mut self) {
        self.turn(-1);
    }

    pub fn next_page(&mut self) {
        self.turn(1);
    }

    /// A pointer picked a page: the cursor follows it, on that page's first row.
    pub fn select_page(&mut self, page: usize) {
        if page < self.rows.len() {
            self.page = page;
            self.row = 0;
        }
    }

    /// A pointer picked a row: the cursor follows it, clamped to the rows that
    /// page actually has.
    pub fn select(&mut self, page: usize, row: usize) {
        if page >= self.rows.len() {
            return;
        }
        self.page = page;
        let rows = self.rows[page];
        self.row = if rows == 0 { 0 } else { row.min(rows - 1) };
    }

    fn turn(&mut self, delta: isize) {
        let pages = self.rows.len();
        if pages == 0 {
            return;
        }
        self.page = (self.page as isize + delta).rem_euclid(pages as isize) as usize;
        self.row = 0;
    }

    fn row_count(&self) -> usize {
        self.rows.get(self.page).copied().unwrap_or(0)
    }

    fn clamp(&mut self) {
        if self.row >= self.row_count() {
            self.row = self.row_count().saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Nav;

    fn nav(rows: &[usize]) -> Nav {
        let mut nav = Nav::default();
        nav.resize(rows);
        nav
    }

    #[test]
    fn up_and_down_stay_inside_the_page() {
        let mut nav = nav(&[3, 2]);

        nav.down();
        nav.down();
        nav.down();
        assert_eq!(
            (nav.page(), nav.row()),
            (0, 2),
            "Down stops at the last row"
        );

        nav.up();
        nav.up();
        nav.up();
        assert_eq!((nav.page(), nav.row()), (0, 0), "Up stops at the first row");
    }

    #[test]
    fn both_ends_of_the_page_ring_are_reachable() {
        let mut nav = nav(&[3, 2]);

        nav.next_page();
        assert_eq!((nav.page(), nav.row()), (1, 0));
        nav.next_page();
        assert_eq!(
            (nav.page(), nav.row()),
            (0, 0),
            "the last page leads to the first"
        );

        nav.previous_page();
        assert_eq!(
            (nav.page(), nav.row()),
            (1, 0),
            "the first page leads to the last"
        );
    }

    #[test]
    fn switching_page_puts_the_cursor_on_its_first_row() {
        let mut nav = nav(&[4, 2]);
        nav.down();
        nav.down();
        nav.down();

        nav.next_page();
        assert_eq!(nav.row(), 0);
        nav.next_page();
        assert_eq!(nav.row(), 0);
    }

    #[test]
    fn a_pointer_choice_moves_the_cursor_too() {
        let mut nav = nav(&[3, 2]);
        nav.down();
        nav.down();

        nav.select_page(1);
        assert_eq!((nav.page(), nav.row()), (1, 0));
    }

    #[test]
    fn a_pointer_choice_on_a_row_moves_the_cursor_there() {
        let mut nav = nav(&[3, 2]);

        nav.select(1, 1);
        assert_eq!((nav.page(), nav.row()), (1, 1));

        nav.select(0, 9);
        assert_eq!(
            (nav.page(), nav.row()),
            (0, 2),
            "a click past the last row lands on it"
        );
        nav.select(9, 0);
        assert_eq!(
            (nav.page(), nav.row()),
            (0, 2),
            "a click on no page changes nothing"
        );
    }

    #[test]
    fn a_shorter_page_pulls_the_cursor_back_inside() {
        let mut nav = nav(&[4, 2]);
        nav.down();
        nav.down();
        nav.down();
        assert_eq!(nav.row(), 3);

        nav.resize(&[2, 2]);
        assert_eq!(
            nav.row(),
            1,
            "the cursor cannot point past a row that is gone"
        );
        assert_eq!(nav.page(), 0);
        nav.up();
        assert_eq!(nav.row(), 0, "and it still moves from where it landed");
    }

    #[test]
    fn an_empty_page_has_a_cursor_at_its_only_position() {
        let mut nav = nav(&[0, 2]);
        nav.down();
        nav.up();
        assert_eq!((nav.page(), nav.row()), (0, 0));

        nav.next_page();
        assert_eq!((nav.page(), nav.row()), (1, 0));
        nav.down();
        assert_eq!(nav.row(), 1);
    }

    #[test]
    fn a_window_without_pages_does_not_move() {
        let mut nav = Nav::default();
        nav.resize(&[]);
        nav.down();
        nav.next_page();
        nav.select_page(0);
        assert_eq!((nav.page(), nav.row()), (0, 0));
    }

    #[test]
    fn a_page_that_disappears_takes_the_cursor_to_the_last_one() {
        let mut nav = nav(&[2, 3]);
        nav.select_page(1);
        nav.down();

        nav.resize(&[2]);
        assert_eq!((nav.page(), nav.row()), (0, 0));
    }
}
