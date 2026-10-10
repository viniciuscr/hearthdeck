//! The library grid: the catalog as tiles.
//!
//! Rewritten from the inherited grid. The grid is virtualized: only the rows
//! inside the viewport, plus [`GRID_OVERSCAN_ROWS`] on either side, become
//! widgets, and the rest of the list is reserved with spacers of exactly the
//! missing height. A library of thousands of games therefore lays out a
//! screenful per frame instead of a thousand tiles, while the scrollable keeps
//! the list's true extent.
//!
//! Every row is laid out at [`Metrics::row_height`], and the console grid's
//! paging measures the loaded list in those same rows (see
//! [`crate::screens::library::console`]), so what the grid draws and where the
//! "the end of the list is on screen" question is answered cannot disagree.

use std::ops::Range;
use std::sync::{Arc, LazyLock};

use cosmic::Element;
use cosmic::desktop::DesktopEntryData;
use cosmic::iced::Length;
use cosmic::iced::alignment::Vertical;
use cosmic::iced::widget::scrollable::{AbsoluteOffset, scroll_to};
use cosmic::theme;
use cosmic::widget::{Id, column, container, row, scrollable, space};

use crate::app::Message;
use crate::style::{GRID_OVERSCAN_ROWS, grid_columns, grid_gap, tile_height, tile_width};
use crate::widgets::application::{Selection, app_tile};

/// The grid's scrollable. Held here because it is the grid's own widget, and
/// this module is what knows when a new list has to be drawn from the top.
static GRID_SCROLLABLE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("library-grid"));

/// How the grid lays a row out for one window width.
///
/// The grid draws with these numbers and the paging math measures the loaded
/// list in them, so they are computed once, here, instead of being derived
/// again wherever a row height is needed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub columns: usize,
    pub tile_width: f32,
    pub tile_height: f32,
    /// The gap between two tiles on a row, which is also the gap between rows.
    pub gap: f32,
    /// The pitch of one row: a tile plus the gap beneath it. Every row is laid
    /// out at exactly this height, which is what makes the spacers reserving
    /// the unbuilt rows exact.
    pub row_height: f32,
}

/// Measures the grid for `window_width`.
#[must_use]
pub fn metrics(window_width: f32) -> Metrics {
    let columns = grid_columns(false).max(1);
    let tile_width = tile_width(window_width, columns);
    let tile_height = tile_height(tile_width, false);
    let gap = grid_gap(window_width, columns);
    Metrics {
        columns,
        tile_width,
        tile_height,
        gap,
        row_height: tile_height + gap,
    }
}

impl Metrics {
    /// The number of rows `entries` are laid out in.
    #[must_use]
    pub fn rows(self, entries: usize) -> usize {
        entries.div_ceil(self.columns)
    }

    /// The height `entries` occupy once laid out as rows. This is the
    /// scrollable's content, and what paging measures the viewport against.
    #[must_use]
    pub fn content_height(self, entries: usize) -> f32 {
        self.rows(entries) as f32 * self.row_height
    }

    /// The rows to build for a viewport scrolled to `offset`.
    ///
    /// These are the rows the viewport shows, plus [`GRID_OVERSCAN_ROWS`] of
    /// margin at each end, so a small scroll never reveals a row that has not
    /// been built yet.
    ///
    /// The offset is clamped to the furthest the list can scroll, exactly as the
    /// renderer clamps the position it draws from. That keeps the rows built and
    /// the rows on screen the same set even if a caller hands over an offset
    /// from a longer list than the one being drawn.
    #[must_use]
    pub fn row_window(self, entries: usize, offset: f32, viewport_height: f32) -> Range<usize> {
        let total_rows = self.rows(entries);
        if total_rows == 0 {
            return 0..0;
        }
        let row_height = self.row_height.max(1.0);
        let furthest = (self.content_height(entries) - viewport_height).max(0.0);
        let first = (offset.clamp(0.0, furthest) / row_height).floor() as usize;
        let visible = (viewport_height.max(0.0) / row_height).ceil() as usize;
        let end = first
            .saturating_add(visible.max(1))
            .saturating_add(GRID_OVERSCAN_ROWS)
            .min(total_rows);
        first.saturating_sub(GRID_OVERSCAN_ROWS).min(end)..end
    }

    /// The offset that brings `row` fully into view, or `None` when it is
    /// already there and the viewport should be left alone.
    ///
    /// The viewport does not track the cursor proportionally: it moves only when
    /// the row the cursor is on would otherwise leave the visible area, and then
    /// it pins that row to the nearest edge. A grid that slid forward on every
    /// step would read as the list moving rather than the selection doing so.
    ///
    /// A row leaving through the bottom pins to the bottom edge, so stepping
    /// down keeps it just inside the viewport; a row that left through the top
    /// pins to the top, so stepping back up does the same. The result is clamped
    /// to the furthest the list can scroll - the same clamp [`Self::row_window`]
    /// applies when it decides which rows to build - so the offset returned here
    /// and the rows that get built cannot disagree.
    #[must_use]
    pub fn scroll_to_row(
        self,
        entries: usize,
        row: usize,
        offset: f32,
        viewport_height: f32,
    ) -> Option<f32> {
        let total_rows = self.rows(entries);
        let row_height = self.row_height.max(1.0);
        if total_rows <= 1 {
            return None;
        }
        let visible = viewport_height.max(0.0) / row_height;
        if visible >= total_rows as f32 {
            return None;
        }
        let row = row as f32;
        // The viewport's top edge in rows, as the renderer has it: fractional,
        // because a pixel offset rarely lands on a row boundary.
        let top = offset.max(0.0) / row_height;
        if row >= top && row + 1.0 <= top + visible {
            // The row is already fully inside the viewport.
            return None;
        }
        let target = if row + 1.0 > top + visible {
            (row + 1.0 - visible).max(0.0)
        } else {
            row
        };
        let furthest = (self.content_height(entries) - viewport_height).max(0.0);
        Some((target * row_height).clamp(0.0, furthest))
    }
}

/// The widget id of a tile, so the controller's cursor can address it.
///
/// Keyed on the entry rather than on its position, so a tile keeps its identity
/// while the grid around it changes: entries load, filters narrow, sections
/// swap.
pub fn tile_id(entry: &DesktopEntryData) -> Id {
    Id::new(format!("tile-{}", entry.id))
}

/// Scrolls the grid to an absolute vertical offset.
///
/// The grid only scrolls vertically, so the horizontal offset stays where it is.
pub fn scroll_to_offset(offset: f32) -> cosmic::app::Task<Message> {
    scroll_to(
        GRID_SCROLLABLE_ID.clone(),
        AbsoluteOffset {
            x: Some(0.0),
            y: Some(offset),
        },
    )
}

/// Sends the grid back to the top, for when the list it shows is replaced: a new
/// list is read from its first row, not from wherever the list before it
/// happened to be scrolled to.
pub fn scroll_to_top() -> cosmic::app::Task<Message> {
    scroll_to_offset(0.0)
}

/// The catalog to draw and the window to lay it out for.
pub struct Grid<'a> {
    pub entries: &'a [Arc<DesktopEntryData>],
    pub window_width: f32,
    /// Where the grid is scrolled to, so only the rows it shows are built.
    pub scroll_offset: f32,
    /// The viewport's height, measured by the scrollable itself.
    pub viewport_height: f32,
}

#[must_use]
pub fn view(grid: Grid<'_>) -> Element<'_, Message> {
    let spacing = theme::spacing();
    let metrics = metrics(grid.window_width);
    let total_rows = metrics.rows(grid.entries.len());
    let window = metrics.row_window(grid.entries.len(), grid.scroll_offset, grid.viewport_height);

    let mut rows: Vec<Element<'_, Message>> = Vec::new();
    if window.start > 0 {
        rows.push(reserved_rows(window.start, metrics.row_height));
    }
    for row_index in window.clone() {
        let start = row_index * metrics.columns;
        let end = (start + metrics.columns).min(grid.entries.len());
        let mut children: Vec<Element<'_, Message>> = Vec::with_capacity(metrics.columns);
        for (column_index, entry) in grid.entries[start..end].iter().enumerate() {
            let index = start + column_index;
            let icon = crate::icon_cache::entry_icon_handle(&entry.icon, metrics.tile_width as u32);
            children.push(app_tile(
                tile_id(entry),
                &entry.name,
                icon,
                metrics.tile_width,
                metrics.tile_height,
                1,
                Message::TileContext(index),
                Some(Message::Activate(index)),
                None,
                Selection::Unselected,
            ));
        }
        // Keep every row `columns` wide so tiles line up in a grid rather than
        // stretching across the last row.
        for _ in children.len()..metrics.columns {
            children.push(space::horizontal().width(Length::Fill).into());
        }
        rows.push(
            container(row(children).spacing(metrics.gap))
                .width(Length::Fill)
                .height(Length::Fixed(metrics.row_height))
                .align_y(Vertical::Top)
                .into(),
        );
    }
    if window.end < total_rows {
        rows.push(reserved_rows(total_rows - window.end, metrics.row_height));
    }

    scrollable(
        column(rows)
            .width(Length::Fill)
            .padding([spacing.space_s, 0, spacing.space_xxl, 0]),
    )
    .id(GRID_SCROLLABLE_ID.clone())
    // Where the grid is scrolled to feeds two things: which rows to build, and
    // whether the console source has a page left to append.
    .on_scroll(|viewport| Message::GridScrolled {
        offset: viewport.absolute_offset().y,
        viewport_height: viewport.bounds().height,
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .scrollbar_width(0)
    .scroller_width(0)
    .into()
}

/// The height of `rows` rows that are not built this frame, so the scrollable
/// keeps the list's true extent without the tiles in it existing.
fn reserved_rows(rows: usize, row_height: f32) -> Element<'static, Message> {
    space::vertical()
        .height(Length::Fixed(rows as f32 * row_height))
        .into()
}

#[cfg(test)]
mod tests {
    use super::{GRID_OVERSCAN_ROWS, Metrics, metrics};

    /// A viewport `rows` rows tall. Deliberately not a whole number of rows:
    /// asking for a window that lands exactly on a row boundary is a question
    /// about floating-point rounding, not about the grid.
    fn viewport(metrics: Metrics, rows: f32) -> f32 {
        rows * metrics.row_height
    }

    #[test]
    fn a_row_is_a_tile_plus_the_gap_beneath_it() {
        let metrics = metrics(1200.0);
        assert_eq!(metrics.columns, crate::style::GRID_COLUMNS);
        assert!((metrics.row_height - (metrics.tile_height + metrics.gap)).abs() < f32::EPSILON);
    }

    #[test]
    fn a_row_count_rounds_the_last_row_up() {
        let metrics = metrics(1200.0);
        assert_eq!(metrics.rows(0), 0);
        assert_eq!(metrics.rows(1), 1);
        assert_eq!(metrics.rows(metrics.columns), 1);
        assert_eq!(metrics.rows(metrics.columns + 1), 2);

        let rows = 3;
        assert!(
            (metrics.content_height(metrics.columns * rows) - rows as f32 * metrics.row_height)
                .abs()
                < 0.01
        );
    }

    /// The grid builds what the viewport shows, plus a margin at each end, so a
    /// small scroll never lands on a row that has not been built - and never
    /// more than the viewport and that margin, so the work stays a screenful.
    #[test]
    fn the_built_rows_cover_the_viewport_and_stop_within_the_margin() {
        let metrics = metrics(1200.0);
        // Three and a half rows tall: four rows are needed to fill it.
        let viewport_height = viewport(metrics, 3.5);
        let visible = 4;
        let entries = metrics.columns * 100;

        let top = metrics.row_window(entries, 0.0, viewport_height);
        assert_eq!(top.start, 0, "nothing above the first row to reserve");
        assert!(
            (visible..=visible + GRID_OVERSCAN_ROWS).contains(&top.end),
            "built {} rows for a viewport of {visible}",
            top.end - top.start
        );

        let middle = metrics.row_window(entries, 50.0 * metrics.row_height, viewport_height);
        assert!(middle.start <= 50, "the row scrolled to was not built");
        assert!(
            middle.end >= 50 + visible,
            "the last row the viewport shows was not built"
        );
        assert!(middle.end - middle.start <= visible + 2 * GRID_OVERSCAN_ROWS);

        // The end of the list is the end of the window: nothing is built past it.
        let last = metrics.row_window(entries, 97.0 * metrics.row_height, viewport_height);
        assert_eq!(last.end, 100);
    }

    #[test]
    fn a_list_shorter_than_the_viewport_builds_every_row_of_it() {
        let metrics = metrics(1200.0);
        let viewport_height = viewport(metrics, 5.5);

        assert_eq!(metrics.row_window(0, 0.0, viewport_height), 0..0);
        assert_eq!(metrics.row_window(2, 0.0, viewport_height), 0..1);
        assert_eq!(
            metrics.row_window(metrics.columns * 2, 0.0, viewport_height),
            0..2
        );
    }

    /// An offset from a list that has since been replaced is read as the last
    /// position the new list can reach - the same clamp the renderer applies -
    /// so the rows built are the rows on screen instead of none.
    #[test]
    fn an_offset_past_the_end_reads_as_the_end_of_this_list() {
        let metrics = metrics(1200.0);
        let viewport_height = viewport(metrics, 3.5);
        let entries = metrics.columns * 5;

        let stale = metrics.row_window(entries, 10_000.0, viewport_height);

        // Five rows of content, three rows of viewport: the furthest the list
        // scrolls is row 2, so the window is every row it can show from there.
        assert_eq!(stale, 0..5);
    }

    /// The height a window of one row has to reserve is the row pitch, or the
    /// list's extent would drift from the number of rows it holds.
    #[test]
    fn a_reserved_row_is_one_row_pitch() {
        let metrics = metrics(1200.0);
        let reserved = super::reserved_rows(7, metrics.row_height);
        assert_eq!(
            reserved.as_widget().size().height,
            cosmic::iced::Length::Fixed(7.0 * metrics.row_height)
        );
    }

    /// A grid of 100px rows, four to a row, with no gap - so every expected
    /// offset below is a whole row and the arithmetic is readable.
    fn plain_grid() -> Metrics {
        Metrics {
            columns: 4,
            tile_width: 100.0,
            tile_height: 100.0,
            gap: 0.0,
            row_height: 100.0,
        }
    }

    /// The viewport follows the cursor, but only when it has to: a step that
    /// lands on a row the viewport already shows must not move the page, or the
    /// grid would slide under every press.
    #[test]
    fn the_viewport_follows_the_cursor_only_when_a_row_would_leave_it() {
        let grid = plain_grid();
        let entries = 4 * 10; // ten rows
        let viewport = 300.0; // three rows are visible

        for row in 0..3 {
            assert_eq!(
                grid.scroll_to_row(entries, row, 0.0, viewport),
                None,
                "row {row} is already on screen"
            );
        }

        // A row leaving through the bottom pins to the bottom edge, so stepping
        // down keeps it just inside the viewport rather than jumping it to the
        // top of the screen.
        assert_eq!(grid.scroll_to_row(entries, 3, 0.0, viewport), Some(100.0));
        assert_eq!(grid.scroll_to_row(entries, 4, 0.0, viewport), Some(200.0));

        // One leaving through the top pins to the top, so stepping back up does
        // not overshoot in the other direction.
        assert_eq!(grid.scroll_to_row(entries, 1, 200.0, viewport), Some(100.0));
        assert_eq!(grid.scroll_to_row(entries, 0, 200.0, viewport), Some(0.0));
    }

    #[test]
    fn a_list_that_fits_its_viewport_is_never_scrolled() {
        let grid = plain_grid();
        let entries = 4 * 2; // two rows, in a viewport three rows tall

        assert_eq!(grid.scroll_to_row(entries, 1, 0.0, 300.0), None);
        // And an empty grid has no row to follow.
        assert_eq!(grid.scroll_to_row(0, 0, 0.0, 300.0), None);
    }

    /// The last row cannot pin itself to the bottom edge without scrolling past
    /// the end of the list, so the offset stops where the list stops - the same
    /// clamp the rows-to-build window applies.
    #[test]
    fn following_the_cursor_stops_at_the_end_of_the_list() {
        let grid = plain_grid();
        let entries = 4 * 10; // 1000px of content
        let viewport = 300.0; // furthest scroll is 700px

        assert_eq!(grid.scroll_to_row(entries, 9, 0.0, viewport), Some(700.0));
    }
}
