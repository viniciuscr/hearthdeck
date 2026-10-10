//! Where the pad's cursor is, and where a step from there goes.
//!
//! **The D-pad owns the content.** It moves inside the grid, and inside whatever
//! surface is open over the grid - the filter drawer today, the context menu
//! next - and it never leaves for the chrome. A cursor that can wander onto a
//! heading or a tab is a cursor that gets lost between parts of the screen:
//! from the grid's first row, "up" should do nothing, not move the user to a tab
//! they were not looking at.
//!
//! The chrome is reached by its own buttons instead - the shoulders switch
//! section, the triggers switch tab, the search button puts the caret in the
//! search box, the select button opens the filter drawer - and, for a keyboard,
//! by the ring `Tab` walks.
//!
//! Every rule below is a function of the cursors and of what exists, so what the
//! pad does is testable without a compositor.

/// A directional step, from the D-pad or from the arrow keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// What the library has on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// How many sections the sidebar offers.
    pub sections: usize,
    /// How many tabs the section on screen offers, its "all" tab included.
    pub tabs: usize,
    /// Whether the header is showing its filter button.
    pub filter_button: bool,
    /// How many rows the filter drawer holds, whether or not it is open.
    pub facets: usize,
    /// Whether the filter drawer is open, which is what gives it the pad.
    pub drawer_open: bool,
    /// Whether the tile menu is open. It is drawn over the drawer as well as the
    /// grid, so while it is open it is the innermost surface and owns the pad.
    pub menu_open: bool,
    /// How many rows the open menu holds.
    pub menu_rows: usize,
    /// How many tiles the grid holds.
    pub tiles: usize,
    /// How many tiles are on one row of the grid.
    pub columns: usize,
}

/// Where the cursors are.
///
/// One value rather than four fields, because they move together: a step
/// produces a new set, and clamping keeps a set valid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursors {
    /// The tile the pad is on.
    pub tile: usize,
    /// The column the pad last moved sideways in, which is the column a
    /// vertical step comes back to.
    pub column: usize,
    /// The row of the filter drawer the pad is on, meaningful while it is open.
    pub filter_row: usize,
    /// The chrome stop the keyboard's ring is on, when it is on one. `None`
    /// means the content has the cursor, which is where the pad always leaves
    /// it.
    pub ring: Option<usize>,
}

/// What a directional step asks the shell to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The grid's cursor moved: the tile it is on, and the column it steps from
    /// from now on.
    Tile { tile: usize, column: usize },
    /// The cursor moved to another row of the open drawer.
    FilterRow(usize),
    /// The row under the cursor steps through its values. This is what left and
    /// right mean inside the drawer: the row is the control there, so its own
    /// arrows are the step.
    FilterValue { row: usize, delta: i32 },
    /// The open menu's highlight moved by this many rows. Which row it lands on
    /// stays with the menu's own state, which is the only thing that knows how
    /// many rows it has.
    MenuStep(i32),
    /// The cursor is at the edge of what it can reach, and stayed there.
    Stayed,
}

/// A control the keyboard's ring walks: the chrome the pad reaches by a
/// dedicated button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chrome {
    /// A section in the sidebar.
    Section(usize),
    /// The header's search box.
    Search,
    /// A tab of the section on screen: 0 is its "all" tab.
    Tab(usize),
    /// The header's filter button, which opens the filter drawer.
    FilterButton,
}

/// What the confirm button asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// Launch what the tile at this index names.
    Tile(usize),
    /// Step the facet drawn on this row forward.
    Filter { row: usize },
    /// The open menu's highlighted row. Which row that is stays with the menu's
    /// own state, so this names the menu rather than a row that could have moved
    /// between the question and the answer.
    Menu,
    /// A ring stop asks for its own control's action.
    Chrome(Chrome),
    /// There is nothing here to confirm.
    Nothing,
}

/// Where a step from `cursors` goes.
pub fn step(cursors: Cursors, direction: Direction, layout: Layout) -> Outcome {
    // The menu is drawn over everything, the filter drawer included, so it is
    // the innermost surface there is and it owns the pad while it is open.
    if layout.menu_open && layout.menu_rows > 0 {
        return match direction {
            // A menu is a list of actions, so both vertical directions walk it
            // and wrap: the D-pad reaches every row from either end.
            Direction::Up => Outcome::MenuStep(-1),
            Direction::Down => Outcome::MenuStep(1),
            // A row is one whole action, so there is nothing beside it to step
            // to; sideways leaves the highlight where the user is looking.
            Direction::Left | Direction::Right => Outcome::Stayed,
        };
    }
    // An open drawer is a surface over the grid: it owns the pad, and the grid
    // behind it is not what the user is looking at.
    if layout.drawer_open && layout.facets > 0 {
        let row = cursors.filter_row.min(layout.facets - 1);
        return match direction {
            // Rows wrap, so a long list of facets stays walkable in one
            // direction instead of stopping at an end.
            Direction::Up => Outcome::FilterRow((row + layout.facets - 1) % layout.facets),
            Direction::Down => Outcome::FilterRow((row + 1) % layout.facets),
            Direction::Left => Outcome::FilterValue { row, delta: -1 },
            Direction::Right => Outcome::FilterValue { row, delta: 1 },
        };
    }
    match step_tile(
        cursors.tile,
        cursors.column,
        direction,
        layout.tiles,
        layout.columns,
    ) {
        Some((tile, column)) => Outcome::Tile { tile, column },
        None => Outcome::Stayed,
    }
}

/// Where a step inside the grid lands, or `None` when the cursor is already at
/// the edge of what it can reach.
///
/// The pad does not leave the grid for the chrome, so a step off an edge does
/// nothing at all rather than moving the user somewhere they were not looking.
///
/// `column` is the column the cursor came down. It is remembered across vertical
/// moves, so a short last row - or a row that ends before the column the user is
/// in - does not drag the cursor to the first column, and the next vertical step
/// returns to the column they were actually in.
pub fn step_tile(
    tile: usize,
    column: usize,
    direction: Direction,
    tiles: usize,
    columns: usize,
) -> Option<(usize, usize)> {
    if tiles == 0 {
        return None;
    }
    let columns = columns.max(1);
    let last = tiles - 1;
    let tile = tile.min(last);
    let column = column.min(columns - 1);
    let row = tile / columns;
    let rows = tiles.div_ceil(columns);
    let in_column = tile % columns;
    match direction {
        Direction::Left if in_column > 0 => Some((tile - 1, in_column - 1)),
        Direction::Right if in_column + 1 < columns && tile < last => {
            Some((tile + 1, in_column + 1))
        }
        Direction::Up if row > 0 => {
            // Up the column the cursor came down, not the column a short row
            // clamped it into. The previous row's end bounds it.
            let target = (row - 1) * columns + column;
            Some((target.min(row * columns - 1), column))
        }
        Direction::Down if row + 1 < rows => {
            // Into the next row at the same column, or at the end of it when
            // that row is the short one. The column is kept either way, so the
            // step after that still comes back to it.
            let target = (row + 1) * columns + column;
            Some((target.min(last), column))
        }
        _ => None,
    }
}

/// How many stops the keyboard's ring has: one per section, the search box, one
/// per tab, and the filter button when the section offers filtering.
pub fn ring_len(layout: Layout) -> usize {
    layout.sections + 1 + layout.tabs + usize::from(layout.filter_button)
}

/// What the ring's stop at `index` is, or `None` past its last one.
pub fn ring_at(index: usize, layout: Layout) -> Option<Chrome> {
    if index < layout.sections {
        return Some(Chrome::Section(index));
    }
    let index = index - layout.sections;
    if index == 0 {
        return Some(Chrome::Search);
    }
    let index = index - 1;
    if index < layout.tabs {
        return Some(Chrome::Tab(index));
    }
    (index == layout.tabs && layout.filter_button).then_some(Chrome::FilterButton)
}

/// The ring's next stop, or `None` to hand the cursor back to the content.
///
/// The ring leaves the content at one end and returns to it past the other, so
/// a keyboard can walk the whole chrome and come back to where it was.
pub fn ring_step(ring: Option<usize>, backwards: bool, layout: Layout) -> Option<usize> {
    let stops = ring_len(layout);
    if stops == 0 {
        return None;
    }
    let Some(index) = ring else {
        // Entering the ring keeps the direction, so walking backwards lands on
        // the last thing the chrome holds.
        return Some(if backwards { stops - 1 } else { 0 });
    };
    let index = index.min(stops - 1);
    if backwards {
        return index.checked_sub(1);
    }
    (index + 1 < stops).then_some(index + 1)
}

/// What the confirm button asks for.
pub fn confirm(cursors: Cursors, layout: Layout) -> Confirm {
    // The menu is the innermost surface, so it is what Confirm acts on.
    if layout.menu_open && layout.menu_rows > 0 {
        return Confirm::Menu;
    }
    // The ring is only ever on the chrome, so a ring stop confirms that
    // control's own action.
    if let Some(chrome) = cursors.ring.and_then(|index| ring_at(index, layout)) {
        return Confirm::Chrome(chrome);
    }
    if layout.drawer_open && layout.facets > 0 {
        // A pad whose left and right do not work still has to be able to change
        // a facet, so confirming a row steps it forward.
        return Confirm::Filter {
            row: cursors.filter_row.min(layout.facets - 1),
        };
    }
    if cursors.tile < layout.tiles {
        return Confirm::Tile(cursors.tile);
    }
    Confirm::Nothing
}

/// Keeps every cursor on something that exists: entries load, facets come and
/// go, and the chrome the ring walks shrinks with them.
pub fn clamp(cursors: Cursors, layout: Layout) -> Cursors {
    let rows = layout.facets.max(1);
    let stops = ring_len(layout);
    Cursors {
        tile: cursors.tile.min(layout.tiles.saturating_sub(1)),
        // The column is what the user remembers, not something the layout can
        // invalidate.
        column: cursors.column,
        filter_row: cursors.filter_row.min(rows - 1),
        ring: cursors
            .ring
            .map(|index| index.min(stops.saturating_sub(1)))
            .filter(|_| stops > 0),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Chrome, Confirm, Cursors, Direction, Layout, Outcome, clamp, confirm, ring_at, ring_len,
        ring_step, step, step_tile,
    };

    /// A library with three sections, two tabs, a filter button and twelve tiles
    /// in rows of four. The drawer is closed, but holds three rows.
    fn library() -> Layout {
        Layout {
            sections: 3,
            tabs: 2,
            filter_button: true,
            facets: 3,
            drawer_open: false,
            menu_open: false,
            menu_rows: 0,
            tiles: 12,
            columns: 4,
        }
    }

    /// The cursors of a pad sitting on one tile, with the column it came down
    /// being that tile's own - the state a pad that stepped there sideways is in.
    ///
    /// The tests' grids are four columns wide, so the tile's column is its index
    /// modulo four.
    fn on_tile(tile: usize) -> Cursors {
        Cursors {
            tile,
            column: tile % 4,
            ..Cursors::default()
        }
    }

    /// Where a grid step lands, as the cursors the shell would hold afterwards.
    fn landed_on(outcome: Outcome) -> Cursors {
        match outcome {
            Outcome::Tile { tile, column } => Cursors {
                tile,
                column,
                ..Cursors::default()
            },
            other => panic!("expected a grid step, got {other:?}"),
        }
    }

    /// Where a grid step lands, or `None` when it did nothing at all.
    fn moved(cursors: Cursors, direction: Direction, layout: Layout) -> Option<(usize, usize)> {
        match step(cursors, direction, layout) {
            Outcome::Tile { tile, column } => Some((tile, column)),
            Outcome::FilterRow(_)
            | Outcome::FilterValue { .. }
            | Outcome::MenuStep(_)
            | Outcome::Stayed => None,
        }
    }

    #[test]
    fn a_step_walks_the_grid_and_stops_at_its_edges() {
        let layout = library();

        assert_eq!(moved(on_tile(5), Direction::Right, layout), Some((6, 2)));
        assert_eq!(moved(on_tile(5), Direction::Left, layout), Some((4, 0)));
        assert_eq!(moved(on_tile(5), Direction::Down, layout), Some((9, 1)));
        assert_eq!(moved(on_tile(5), Direction::Up, layout), Some((1, 1)));

        // The left-hand column does not step out of the grid: the pad stays in
        // the content instead of wandering onto the sidebar.
        assert_eq!(moved(on_tile(4), Direction::Left, layout), None);
        // Nor does the end of a row, nor the last tile, nor the first row.
        assert_eq!(moved(on_tile(3), Direction::Right, layout), None);
        assert_eq!(moved(on_tile(11), Direction::Right, layout), None);
        assert_eq!(moved(on_tile(11), Direction::Down, layout), None);
        assert_eq!(moved(on_tile(2), Direction::Up, layout), None);
    }

    /// Up and down return to the column the user came down, which is what makes
    /// a grid with a short last row predictable.
    #[test]
    fn a_vertical_step_comes_back_to_the_column_the_cursor_came_down() {
        // Seven tiles in rows of four: the last row holds three.
        let layout = Layout {
            tiles: 7,
            ..library()
        };

        // Down from the fourth column, where that row ends early. The step
        // itself is what carries the remembered column, so the state after it is
        // built from its own outcome rather than assumed.
        let landed = landed_on(step(on_tile(3), Direction::Down, layout));
        assert_eq!(
            (landed.tile, landed.column),
            (6, 3),
            "the short row's end, with the column kept"
        );

        // Back up, and the cursor returns to the column it came down - not to
        // the column the short row clamped it into.
        assert_eq!(moved(landed, Direction::Up, layout), Some((3, 3)));

        // The same holds for a row holding a single tile.
        let one_tile_row = Layout {
            tiles: 5,
            ..library()
        };
        let landed = landed_on(step(on_tile(3), Direction::Down, one_tile_row));
        assert_eq!((landed.tile, landed.column), (4, 3));
        assert_eq!(moved(landed, Direction::Up, one_tile_row), Some((3, 3)));
    }

    #[test]
    fn the_drawer_owns_the_pad_while_it_is_open() {
        let layout = Layout {
            drawer_open: true,
            ..library()
        };

        // A step in the grid belongs to the drawer while it is up.
        assert_eq!(
            step(on_tile(7), Direction::Down, layout),
            Outcome::FilterRow(1)
        );
        assert_eq!(
            step(on_tile(7), Direction::Up, layout),
            Outcome::FilterRow(2),
            "and its rows wrap"
        );
        assert_eq!(
            step(on_tile(7), Direction::Right, layout),
            Outcome::FilterValue { row: 0, delta: 1 }
        );
        assert_eq!(
            step(on_tile(7), Direction::Left, layout),
            Outcome::FilterValue { row: 0, delta: -1 }
        );
    }

    #[test]
    fn a_drawer_with_no_rows_leaves_the_pad_in_the_grid() {
        let layout = Layout {
            drawer_open: true,
            facets: 0,
            ..library()
        };

        assert_eq!(
            step(on_tile(0), Direction::Down, layout),
            Outcome::Tile { tile: 4, column: 0 }
        );
    }

    #[test]
    fn the_ring_walks_the_chrome_and_returns_to_the_content() {
        let layout = library();
        assert_eq!(ring_len(layout), 3 + 1 + 2 + 1);

        let stops: Vec<Chrome> = (0..ring_len(layout))
            .filter_map(|index| ring_at(index, layout))
            .collect();
        assert_eq!(
            stops,
            [
                Chrome::Section(0),
                Chrome::Section(1),
                Chrome::Section(2),
                Chrome::Search,
                Chrome::Tab(0),
                Chrome::Tab(1),
                Chrome::FilterButton,
            ]
        );
        assert_eq!(ring_at(ring_len(layout), layout), None);

        // Entering, walking, and leaving at the far end - which is what hands
        // the cursor back to the content.
        assert_eq!(ring_step(None, false, layout), Some(0));
        assert_eq!(ring_step(Some(0), false, layout), Some(1));
        assert_eq!(ring_step(Some(5), false, layout), Some(6));
        assert_eq!(ring_step(Some(6), false, layout), None);
        // And backwards, which is how it leaves the other way.
        assert_eq!(ring_step(None, true, layout), Some(6));
        assert_eq!(ring_step(Some(1), true, layout), Some(0));
        assert_eq!(ring_step(Some(0), true, layout), None);

        // A section with no filter button has one stop fewer.
        let unfiltered = Layout {
            filter_button: false,
            ..library()
        };
        assert_eq!(ring_len(unfiltered), 6);
        assert_eq!(ring_at(6, unfiltered), None);
    }

    #[test]
    fn confirm_acts_on_whatever_holds_the_cursor() {
        let layout = library();

        assert_eq!(confirm(on_tile(7), layout), Confirm::Tile(7));
        // A cursor left past the last tile confirms nothing.
        assert_eq!(confirm(on_tile(12), layout), Confirm::Nothing);

        // The drawer's row, and the ring's stop, both take precedence over the
        // tile they happen to be drawn over.
        let open = Layout {
            drawer_open: true,
            ..library()
        };
        assert_eq!(confirm(on_tile(7), open), Confirm::Filter { row: 0 });
        let ringed = Cursors {
            ring: Some(4),
            ..on_tile(7)
        };
        assert_eq!(confirm(ringed, layout), Confirm::Chrome(Chrome::Tab(0)));
    }

    #[test]
    fn clamp_keeps_every_cursor_on_something_that_exists() {
        let layout = library();

        let cursors = Cursors {
            tile: 11,
            column: 3,
            filter_row: 2,
            ring: Some(6),
        };
        assert_eq!(clamp(cursors, layout), cursors);

        // A shorter list, fewer facets and a section that lost its filter button
        // each pull their own cursor back, and none of them touch the column.
        let shrunken = Layout {
            tabs: 1,
            filter_button: false,
            facets: 1,
            tiles: 3,
            ..library()
        };
        assert_eq!(
            clamp(cursors, shrunken),
            Cursors {
                tile: 2,
                column: 3,
                filter_row: 0,
                ring: Some(4),
            }
        );
    }

    #[test]
    fn an_empty_grid_has_nowhere_to_step() {
        assert_eq!(step_tile(0, 0, Direction::Down, 0, 4), None);
    }

    /// An open menu owns the pad: the grid behind it does not move, and Confirm
    /// acts on the menu's row rather than on the tile the cursor is still over.
    #[test]
    fn the_menu_owns_the_pad_while_it_is_open() {
        let layout = Layout {
            menu_open: true,
            menu_rows: 3,
            ..library()
        };

        assert_eq!(
            step(on_tile(7), Direction::Down, layout),
            Outcome::MenuStep(1)
        );
        assert_eq!(
            step(on_tile(7), Direction::Up, layout),
            Outcome::MenuStep(-1)
        );
        // A row is one whole action, so sideways has nowhere to go.
        assert_eq!(step(on_tile(7), Direction::Right, layout), Outcome::Stayed);
        assert_eq!(confirm(on_tile(7), layout), Confirm::Menu);

        // It is the innermost surface there is, so it wins over an open drawer.
        let over_the_drawer = Layout {
            drawer_open: true,
            ..layout
        };
        assert_eq!(
            step(on_tile(7), Direction::Down, over_the_drawer),
            Outcome::MenuStep(1)
        );
        assert_eq!(confirm(on_tile(7), over_the_drawer), Confirm::Menu);
    }

    /// A menu with nothing in it is not a surface: it must not hold the pad
    /// hostage, or a menu that lost its rows would leave the grid unreachable.
    #[test]
    fn a_menu_with_no_rows_leaves_the_pad_where_it_was() {
        let layout = Layout {
            menu_open: true,
            menu_rows: 0,
            ..library()
        };

        assert_eq!(
            step(on_tile(0), Direction::Down, layout),
            Outcome::Tile { tile: 4, column: 0 }
        );
        assert_eq!(confirm(on_tile(7), layout), Confirm::Tile(7));
    }
}
