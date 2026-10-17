//! The name of the card a rail's cursor sits on, read out below the rail.
//!
//! The tile draws a truncated name on its artwork ([`super::TileLabel`]), all a
//! 240-pixel card can carry. The dashboard has the whole rail's width under the
//! cards - room a long name never gets on a cover - so its tiles are nameless
//! and the name is read out here instead: the card the cursor is on, and an
//! empty line on every other rail. The band is [`LABEL_HEIGHT`] tall whether or
//! not a name is showing, so a rail never reflows as the cursor walks between
//! rails.

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::iced::alignment::Vertical;
use cosmic::iced::core::text::{Alignment, Ellipsize, EllipsizeHeightLimit, Wrapping};
use cosmic::widget::{container, text};

use super::style::{LABEL_HEIGHT, LABEL_LINE_HEIGHT, LABEL_MAX_LINES, TEXT_TILE_LABEL};

/// The name line kept below a rail: the full name of the card the cursor is
/// on, or an empty line when the cursor is on another rail.
#[must_use]
pub fn app_label<'a, Message: Clone + 'a>(name: Option<&str>) -> Element<'a, Message> {
    // The widget owns its text: the dashboard builds its rails from shelves
    // that live only for this draw, so the name line may not borrow them.
    let name = text(name.unwrap_or_default().to_owned())
        .size(TEXT_TILE_LABEL)
        .line_height(LABEL_LINE_HEIGHT)
        .width(Length::Fill)
        // Centered on the text itself: a container's `align_x` never moves a
        // fill-width child, which is how the name came out flush left.
        .align_x(Alignment::Center)
        // Word boundaries, falling back to glyphs: one unbreakable name must
        // not run past the band it is laid out in.
        .wrapping(Wrapping::WordOrGlyph)
        // A name past two wrapped lines ends in an ellipsis at the last line
        // the band holds, never spills below it.
        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(LABEL_MAX_LINES)));

    // Centered under the rail, in a band of a fixed height: the name must
    // never change where its rail ends, or the page scroll would measure a
    // different dashboard than the one on screen.
    container(name)
        .width(Length::Fill)
        .height(Length::Fixed(LABEL_HEIGHT))
        .align_y(Vertical::Center)
        .into()
}

#[cfg(test)]
mod tests {
    use super::app_label;
    use crate::widgets::application::style::{
        LABEL_HEIGHT, LABEL_LINE_HEIGHT, LABEL_MAX_LINES, TEXT_TILE_LABEL,
    };
    use cosmic::iced::{Length, Size};

    // The band is a layout constant, not a rendered measurement: whether or
    // not a name is in it must not change how tall the rail below it is.
    #[test]
    fn a_name_line_declares_its_fixed_height_with_or_without_a_name() {
        for name in [Some("Portal"), None] {
            let line = app_label::<()>(name);

            assert_eq!(
                line.as_widget().size(),
                Size::new(Length::Fill, Length::Fixed(LABEL_HEIGHT)),
                "name={name:?}",
            );
        }
    }

    // The band must have room for every line a name may wrap to - as many as
    // the ellipsis lets it reach (LABEL_MAX_LINES, the same cap the name text
    // ellipsizes at) at the pinned line height - or the name grows past where
    // the page scroll thinks the rail ends.
    #[test]
    fn the_name_band_has_room_for_every_line_a_name_may_wrap_to() {
        let wrapped = TEXT_TILE_LABEL * LABEL_LINE_HEIGHT * LABEL_MAX_LINES as f32;

        assert!(
            LABEL_HEIGHT >= wrapped,
            "a name wrapped to {LABEL_MAX_LINES} lines ({wrapped} px) must fit in the {LABEL_HEIGHT} px band",
        );
    }
}
