//! Where the dashboard's cursor is, and where a step from there goes.
//!
//! The dashboard is a stack of rails, which is a different shape from the
//! library's grid: every rail is one row of cards and each row is as long as
//! its own rail, where the grid's rows are all equally wide. So the rail
//! movement is its own set of rules rather than the grid's rules bent to fit -
//! what the two do share is the shape: a step is a function of the cursor and
//! of what exists, so it is testable without a compositor.

use crate::screens::library::focus::Direction;

/// Where the pad is on the dashboard: which rail, and which card in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    /// The rail the pad is on.
    pub rail: usize,
    /// The card within that rail.
    pub card: usize,
    /// The card the pad last moved sideways to, which is the card a vertical
    /// step comes back to when the rail it lands on is too short to hold it.
    ///
    /// Kept beside the card rather than derived from it, because the card a
    /// short rail clamps into is not the card the user was on: without this,
    /// stepping down out of a long rail and back up would drag them to the
    /// short rail's last card instead of returning them where they were.
    pub remembered: usize,
}

/// Where a step from `cursor` goes.
///
/// Left and right walk the rail the cursor is on and stop at its ends: the pad
/// owns the content, so a step off an edge does nothing rather than jumping
/// somewhere the user was not looking. Up and down move between rails and keep
/// the card the cursor came from, bounded by the rail it lands on.
///
/// A rail with no cards is not somewhere to be, so a vertical step passes over
/// it: an empty shelf holds nothing to look at.
pub fn step(cursor: Cursor, direction: Direction, rails: &[usize]) -> Cursor {
    let cursor = clamp(cursor, rails);
    match direction {
        Direction::Left => {
            let card = cursor.card.saturating_sub(1);
            Cursor {
                card,
                remembered: card,
                ..cursor
            }
        }
        Direction::Right => {
            let last = cards_in(rails, cursor.rail).saturating_sub(1);
            let card = (cursor.card + 1).min(last);
            Cursor {
                card,
                remembered: card,
                ..cursor
            }
        }
        Direction::Up => shift_rail(cursor, rails, -1),
        Direction::Down => shift_rail(cursor, rails, 1),
    }
}

/// The card the cursor is on, as `(rail, card)`, or `None` when it is on nothing
/// at all - which is what an empty dashboard amounts to.
pub fn confirm(cursor: Cursor, rails: &[usize]) -> Option<(usize, usize)> {
    let cursor = clamp(cursor, rails);
    (cursor.card < cards_in(rails, cursor.rail)).then_some((cursor.rail, cursor.card))
}

/// Keeps the cursor on a rail and a card that exist: a rail can lose its cards
/// when the shelf behind it empties, and the list can lose the rail entirely.
pub fn clamp(cursor: Cursor, rails: &[usize]) -> Cursor {
    let rail = if cards_in(rails, cursor.rail) > 0 {
        cursor.rail
    } else {
        // The rail is empty, or gone, or was never there: the first rail that
        // has something in it, and the first rail at all when none has.
        rails.iter().position(|cards| *cards > 0).unwrap_or(0)
    };
    Cursor {
        rail,
        card: cursor.card.min(cards_in(rails, rail).saturating_sub(1)),
        // What the user remembers is not something the layout can invalidate.
        remembered: cursor.remembered,
    }
}

/// How many cards the rail at `index` holds, or none when there is no such rail.
fn cards_in(rails: &[usize], index: usize) -> usize {
    rails.get(index).copied().unwrap_or(0)
}

/// Moves to the nearest rail in `direction` that has cards, bringing the card the
/// cursor came from and bounding it by what that rail holds. Stays put when there
/// is no such rail, so the ends of the dashboard are ends.
fn shift_rail(cursor: Cursor, rails: &[usize], delta: i32) -> Cursor {
    let mut rail = cursor.rail as i32 + delta;
    while rail >= 0 && (rail as usize) < rails.len() {
        let index = rail as usize;
        if rails[index] > 0 {
            return Cursor {
                rail: index,
                card: cursor.remembered.min(rails[index] - 1),
                remembered: cursor.remembered,
            };
        }
        rail += delta;
    }
    cursor
}

/// The horizontal offset that brings card `index` into a rail's view, or `None`
/// when it is already there and the row should be left alone.
///
/// A rail does not scroll with every step: it moves only when the card the
/// cursor is on would otherwise leave the visible width, and then it pins that
/// card to the nearest edge, so a row of posters slides under a highlight that
/// stays where the user is looking. The result is clamped to the rail's own
/// extent, so the last card stops at the end instead of scrolling past it.
///
/// `card` is a card's width and `gap` the space between two of them, which
/// together are the pitch a card sits at.
#[must_use]
pub fn rail_scroll_target(
    index: usize,
    count: usize,
    offset: f32,
    viewport: f32,
    card: f32,
    gap: f32,
) -> Option<f32> {
    let pitch = card + gap;
    let start = index as f32 * pitch;
    let end = start + card;
    let target = if start < offset {
        // It left through the leading edge.
        start
    } else if end > offset + viewport {
        // It is leaving through the trailing edge.
        end - viewport
    } else {
        return None;
    };
    let content = count as f32 * pitch - gap;
    Some(target.clamp(0.0, (content - viewport).max(0.0)))
}

/// The vertical offset that brings rail `index` into the page's view, or `None`
/// when it is already there.
///
/// The page is a stack of rails of unequal height, and the rule is the rails'
/// own: it moves only when the rail the cursor is on would otherwise leave the
/// visible height, and then it pins that rail to the nearest edge. Clamped to the
/// page's own extent, so the last rail stops at the end instead of scrolling past
/// it.
///
/// `rail_heights` is each rail's height in order and `gap` the space between two
/// of them, which together are the stack's geometry.
#[must_use]
pub fn page_scroll_target(
    rail_heights: &[f32],
    index: usize,
    gap: f32,
    offset: f32,
    viewport: f32,
) -> Option<f32> {
    let height = *rail_heights.get(index)?;
    let top = rail_top(rail_heights, index, gap);
    let bottom = top + height;
    let target = if top < offset {
        // It left through the top.
        top
    } else if bottom > offset + viewport {
        // It is leaving through the bottom.
        bottom - viewport
    } else {
        return None;
    };
    let content = rail_top(rail_heights, rail_heights.len(), gap) - gap;
    Some(target.clamp(0.0, (content - viewport).max(0.0)))
}

/// Where rail `index` starts on the page: the rails above it, and the gaps
/// between them. One past the last rail is the end of the stack.
fn rail_top(rail_heights: &[f32], index: usize, gap: f32) -> f32 {
    let above: f32 = rail_heights.iter().take(index).sum();
    above + index as f32 * gap
}

#[cfg(test)]
mod tests {
    use super::{Cursor, clamp, confirm, page_scroll_target, rail_scroll_target, step};
    use crate::screens::library::focus::Direction;

    /// Three rails: four cards, one card, three cards.
    const RAILS: [usize; 3] = [4, 1, 3];

    /// A pad sitting on one card, having arrived there sideways - so the card it
    /// came from is that one.
    fn on(rail: usize, card: usize) -> Cursor {
        Cursor {
            rail,
            card,
            remembered: card,
        }
    }

    #[test]
    fn a_horizontal_step_walks_the_rail_and_stops_at_its_ends() {
        assert_eq!(step(on(0, 1), Direction::Right, &RAILS), on(0, 2));
        assert_eq!(step(on(0, 1), Direction::Left, &RAILS), on(0, 0));

        // The ends of a rail are ends: a step off one does not wrap, and it does
        // not leave for another rail either.
        assert_eq!(step(on(0, 0), Direction::Left, &RAILS), on(0, 0));
        assert_eq!(step(on(0, 3), Direction::Right, &RAILS), on(0, 3));
    }

    /// A vertical step returns to the card the user came down from, so a short
    /// rail below does not swallow their place in a long one.
    #[test]
    fn a_vertical_step_moves_between_rails_and_keeps_the_card() {
        // Down onto the one-card rail: the card is bounded by what it holds.
        let landed = step(on(0, 2), Direction::Down, &RAILS);
        assert_eq!(
            landed,
            Cursor {
                rail: 1,
                card: 0,
                remembered: 2
            }
        );

        // And back up returns to the card it came down from, not to the card the
        // short rail clamped it into.
        assert_eq!(step(landed, Direction::Up, &RAILS), on(0, 2));

        // The ends of the stack are ends.
        assert_eq!(step(on(0, 0), Direction::Up, &RAILS), on(0, 0));
        assert_eq!(step(on(2, 0), Direction::Down, &RAILS), on(2, 0));
    }

    /// An empty shelf is not somewhere to be, so a vertical step passes over it
    /// rather than landing on nothing.
    #[test]
    fn a_vertical_step_skips_a_rail_with_no_cards() {
        let rails = [2, 0, 2];
        assert_eq!(step(on(0, 1), Direction::Down, &rails), on(2, 1));
        assert_eq!(step(on(2, 1), Direction::Up, &rails), on(0, 1));

        // With everything empty there is nowhere to be, and nothing moves.
        let empty = [0, 0];
        assert_eq!(
            step(Cursor::default(), Direction::Down, &empty),
            Cursor::default()
        );
        assert_eq!(
            step(Cursor::default(), Direction::Up, &empty),
            Cursor::default()
        );
    }

    #[test]
    fn clamp_keeps_the_cursor_on_a_card_that_exists() {
        assert_eq!(clamp(on(0, 2), &RAILS), on(0, 2));

        // The rail emptied, so the cursor moves to the first that has cards.
        assert_eq!(
            clamp(on(0, 2), &[0, 2, 3]),
            Cursor {
                rail: 1,
                card: 1,
                remembered: 2
            }
        );

        // The list lost rails, and the cursor's card is past the end of what is
        // left.
        assert_eq!(
            clamp(on(2, 2), &[2]),
            Cursor {
                rail: 0,
                card: 1,
                remembered: 2
            }
        );

        // Nothing at all: the cursor sits on the first rail, which is empty, and
        // every read of it tolerates that. What the user remembers is kept, as
        // it is for the grid's column - the layout cannot invalidate it.
        assert_eq!(
            clamp(on(5, 5), &[]),
            Cursor {
                rail: 0,
                card: 0,
                remembered: 5
            }
        );
    }

    #[test]
    fn confirm_names_the_card_the_cursor_is_on() {
        assert_eq!(confirm(on(2, 1), &RAILS), Some((2, 1)));
        // A cursor clamped onto an empty dashboard confirms nothing.
        assert_eq!(confirm(on(0, 0), &[0, 0]), None);
    }

    /// A rail scrolls only once the cursor crosses one of its edges, and then it
    /// pins the card to that edge - so a row of posters slides under a highlight
    /// the user is looking at, rather than the row jumping on every step.
    #[test]
    fn a_rail_scrolls_only_once_the_cursor_crosses_an_edge() {
        // Ten cards 100 wide, 10 apart, in a 1000-wide view.
        let (count, card, gap, viewport) = (10, 100.0, 10.0, 1000.0);

        // The ninth is the first that leaves through the trailing edge, and the
        // row stops at the end of its own content rather than past it.
        assert_eq!(
            rail_scroll_target(9, count, 0.0, viewport, card, gap),
            Some(90.0)
        );
        // A card leaving through the leading edge pins to the start, not to the
        // trailing edge, so stepping back left keeps it just inside the view.
        assert_eq!(
            rail_scroll_target(0, count, 220.0, viewport, card, gap),
            Some(0.0)
        );
        // A card already in view leaves the row where it is.
        assert_eq!(rail_scroll_target(4, count, 0.0, viewport, card, gap), None);
        // And a rail short enough to fit whole never scrolls at all.
        assert_eq!(rail_scroll_target(2, 3, 0.0, viewport, card, gap), None);
    }

    /// The page scrolls only once the cursor leaves the rail it is on, and then
    /// it pins that rail to the nearest edge - so a stack of rails slides under a
    /// highlight rather than the page jumping on every step.
    #[test]
    fn the_page_scrolls_only_once_the_cursor_leaves_a_rail() {
        // Three rails 200 tall, 20 apart, in a 500-tall page: 640 of content, so
        // the furthest it scrolls is 140.
        let rails = [200.0, 200.0, 200.0];
        let (gap, viewport) = (20.0, 500.0);

        // The first two are inside the page as it opens (0..200 and 220..420).
        assert_eq!(page_scroll_target(&rails, 0, gap, 0.0, viewport), None);
        assert_eq!(page_scroll_target(&rails, 1, gap, 0.0, viewport), None);

        // The third leaves through the bottom (440..640), and the page stops at
        // the end of its own content rather than past it.
        assert_eq!(
            page_scroll_target(&rails, 2, gap, 0.0, viewport),
            Some(140.0)
        );

        // Stepping back up pins the rail to the top, not to the bottom, so it
        // stays just inside the view.
        assert_eq!(
            page_scroll_target(&rails, 0, gap, 140.0, viewport),
            Some(0.0)
        );
        // And a rail already in view leaves the page where it is.
        assert_eq!(page_scroll_target(&rails, 1, gap, 140.0, viewport), None);

        // A rail there is no such thing as has nowhere to scroll to.
        assert_eq!(page_scroll_target(&rails, 3, gap, 0.0, viewport), None);
    }
}
