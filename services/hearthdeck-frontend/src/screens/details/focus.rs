//! Where the details screen's cursor is, and where a step from there goes.
//!
//! The screen is a small, fixed set of rows rather than a free grid: the
//! screenshot strip, then the action line along the bottom. An open disc
//! picker covers the page and replaces the rows entirely, so a cursor behind
//! it would be on a control the user cannot see.
//!
//! Movement is a pure function of [`Layout`] - what the screen currently
//! offers - and of the cursor, exactly like [`crate::screens::library::focus`]:
//! the pad never depends on the renderer having reported focus back, so
//! Confirm always acts on the control the user can see is highlighted.

use cosmic::widget::Id;

pub use crate::screens::library::focus::Direction;

/// Which control the details screen's cursor is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    /// The Play button: launches the file the screen is pointed at.
    #[default]
    Play,
    Favorite,
    /// Opens the disc picker. Only present for a game RomM collapsed out of
    /// several files.
    Disc,
    PlayLater,
    /// Leaves the screen. Sits in the action line so that line holds every
    /// control the screen has, instead of a lone back button elsewhere.
    Back,
    /// One of the game's screenshots, which becomes the big image when picked.
    Screenshot(usize),
    /// One row of the disc picker, whose rows replace the page's while it is
    /// open.
    DiscChoice(usize),
}

/// The action line, left to right. Disc selection only exists for a game with
/// more than one file.
#[must_use]
pub fn actions(has_discs: bool) -> &'static [Focus] {
    const MULTI_FILE: &[Focus] = &[
        Focus::Back,
        Focus::Play,
        Focus::Favorite,
        Focus::Disc,
        Focus::PlayLater,
    ];
    const SINGLE_FILE: &[Focus] = &[Focus::Back, Focus::Play, Focus::Favorite, Focus::PlayLater];
    if has_discs { MULTI_FILE } else { SINGLE_FILE }
}

/// What the screen currently offers, which is all the movement rules read.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    /// How many screenshots the strip holds.
    pub shots: usize,
    /// How many files the disc picker offers. Below two, there is no picker.
    pub discs: usize,
    /// Whether the disc picker is open, which replaces the page's rows.
    pub picker_open: bool,
}

/// The focus rows, top to bottom: the screenshot strip, then the action line.
/// A row that would be empty is left out, so a step never lands on nothing.
///
/// The open disc picker replaces the rows entirely.
#[must_use]
pub fn rows(layout: Layout) -> Vec<Vec<Focus>> {
    if layout.picker_open {
        return (0..layout.discs)
            .map(|index| vec![Focus::DiscChoice(index)])
            .collect();
    }
    let mut rows = Vec::new();
    if layout.shots > 0 {
        rows.push((0..layout.shots).map(Focus::Screenshot).collect());
    }
    rows.push(actions(layout.discs > 1).to_vec());
    rows
}

/// The cursor, corrected when the row it pointed at is gone: Play if it is
/// still there, otherwise the first control that exists. Without this the pad
/// would have nothing left to move onto.
#[must_use]
pub fn clamp(layout: Layout, focus: Focus) -> Focus {
    let rows = rows(layout);
    if rows.iter().any(|row| row.contains(&focus)) {
        return focus;
    }
    if rows.iter().any(|row| row.contains(&Focus::Play)) {
        return Focus::Play;
    }
    rows.first()
        .and_then(|row| row.first().copied())
        .unwrap_or_default()
}

/// Where a step from `focus` lands, or `None` when the cursor is already at
/// the edge of what it can reach.
///
/// The pad stays inside the row it is on when stepping sideways, and clamps to
/// the shorter row's last control when stepping up or down - a row that ends
/// early does not drag the cursor to its first column. A step off either edge
/// does nothing at all.
#[must_use]
pub fn step(layout: Layout, focus: Focus, direction: Direction) -> Option<Focus> {
    let rows = rows(layout);
    let row_index = rows.iter().position(|row| row.contains(&focus))?;
    let column = rows[row_index]
        .iter()
        .position(|candidate| *candidate == focus)?;
    match direction {
        Direction::Left | Direction::Right => {
            let delta = if direction == Direction::Right { 1 } else { -1 };
            let next = column as i32 + delta;
            usize::try_from(next)
                .ok()
                .and_then(|next| rows[row_index].get(next))
                .copied()
        }
        Direction::Up | Direction::Down => {
            let delta = if direction == Direction::Down { 1 } else { -1 };
            let next = row_index as i32 + delta;
            let row = usize::try_from(next).ok().and_then(|next| rows.get(next))?;
            row.get(column.min(row.len().saturating_sub(1))).copied()
        }
    }
}

/// Widget id of a focus target. Stable for the page's lifetime, which is what
/// lets the renderer's accent ring and [`Focus`] name the same control.
#[must_use]
pub fn widget_id(focus: Focus) -> Id {
    match focus {
        Focus::Play => Id::new("details-play"),
        Focus::Favorite => Id::new("details-favorite"),
        Focus::Disc => Id::new("details-disc"),
        Focus::PlayLater => Id::new("details-play-later"),
        Focus::Back => Id::new("details-back"),
        Focus::Screenshot(index) => Id::new(format!("details-screenshot-{index}")),
        Focus::DiscChoice(index) => Id::new(format!("details-disc-{index}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Direction, Focus, Layout, actions, clamp, rows, step};

    fn with_shots(shots: usize) -> Layout {
        Layout {
            shots,
            discs: 1,
            picker_open: false,
        }
    }

    fn multi_file() -> Layout {
        Layout {
            shots: 2,
            discs: 3,
            picker_open: false,
        }
    }

    #[test]
    fn the_action_line_drops_disc_selection_for_single_file_games() {
        assert_eq!(
            actions(false),
            &[Focus::Back, Focus::Play, Focus::Favorite, Focus::PlayLater],
        );
        assert_eq!(
            actions(true),
            &[
                Focus::Back,
                Focus::Play,
                Focus::Favorite,
                Focus::Disc,
                Focus::PlayLater,
            ],
        );
    }

    #[test]
    fn a_row_that_would_be_empty_is_left_out() {
        let rows = rows(with_shots(0));
        assert_eq!(rows.len(), 1, "no screenshots means only the action line");
        assert!(rows[0].contains(&Focus::Play), "rows: {rows:?}");
    }

    #[test]
    fn the_open_disc_picker_replaces_the_pages_rows() {
        let layout = Layout {
            picker_open: true,
            discs: 3,
            ..with_shots(2)
        };
        assert_eq!(
            rows(layout),
            [
                vec![Focus::DiscChoice(0)],
                vec![Focus::DiscChoice(1)],
                vec![Focus::DiscChoice(2)],
            ],
        );
    }

    #[test]
    fn disc_choices_follow_the_vertical_list_and_stop_at_its_edges() {
        let layout = Layout {
            picker_open: true,
            ..multi_file()
        };
        assert_eq!(
            step(layout, Focus::DiscChoice(0), Direction::Down),
            Some(Focus::DiscChoice(1))
        );
        assert_eq!(
            step(layout, Focus::DiscChoice(1), Direction::Up),
            Some(Focus::DiscChoice(0))
        );
        assert_eq!(
            step(layout, Focus::DiscChoice(1), Direction::Down),
            Some(Focus::DiscChoice(2))
        );
        assert_eq!(step(layout, Focus::DiscChoice(0), Direction::Up), None);
        assert_eq!(step(layout, Focus::DiscChoice(2), Direction::Down), None);
        assert_eq!(step(layout, Focus::DiscChoice(1), Direction::Left), None);
        assert_eq!(step(layout, Focus::DiscChoice(1), Direction::Right), None);
    }

    #[test]
    fn navigation_walks_rows_without_leaving_them() {
        let layout = multi_file();
        // Sideways stays on the strip and stops at its ends.
        assert_eq!(
            step(layout, Focus::Screenshot(0), Direction::Right),
            Some(Focus::Screenshot(1)),
        );
        assert_eq!(step(layout, Focus::Screenshot(0), Direction::Left), None);
        assert_eq!(step(layout, Focus::Screenshot(1), Direction::Right), None);
        // Down lands on the same column of the action line ...
        assert_eq!(
            step(layout, Focus::Screenshot(1), Direction::Down),
            Some(Focus::Play),
        );
        // ... and a shorter row clamps to its last control.
        assert_eq!(
            step(layout, Focus::PlayLater, Direction::Up),
            Some(Focus::Screenshot(1)),
        );
        // A step off the top or bottom edge does nothing.
        assert_eq!(step(layout, Focus::Screenshot(1), Direction::Up), None);
        assert_eq!(step(layout, Focus::PlayLater, Direction::Down), None);
        assert_eq!(
            step(
                Layout {
                    picker_open: true,
                    discs: 3,
                    ..multi_file()
                },
                Focus::DiscChoice(2),
                Direction::Up
            ),
            Some(Focus::DiscChoice(1)),
        );
    }

    #[test]
    fn a_vertical_step_keeps_the_column_it_came_from() {
        let layout = multi_file();
        // Play sits at column 1 of the action line; the strip has a column 1.
        assert_eq!(
            step(layout, Focus::Play, Direction::Up),
            Some(Focus::Screenshot(1)),
        );
    }

    #[test]
    fn the_cursor_recovers_when_its_row_disappears() {
        // A screenshot that failed to download leaves the cursor pointing at
        // nothing; it falls back to Play rather than highlighting nothing.
        let focus = clamp(with_shots(0), Focus::Screenshot(3));
        assert_eq!(focus, Focus::Play);
        // With no Play at all, the first control that exists is the fallback.
        let layout = Layout {
            picker_open: true,
            discs: 2,
            ..with_shots(0)
        };
        assert_eq!(clamp(layout, Focus::Play), Focus::DiscChoice(0));
        // A cursor that still exists is left alone.
        assert_eq!(
            clamp(multi_file(), Focus::Screenshot(1)),
            Focus::Screenshot(1)
        );
    }
}
