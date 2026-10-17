//! Styles used only by the application tile.
//!
//! These dress the tile that [`super::app_tile`] builds and live next to it
//! rather than in the shared design system. What stays tile-specific here is the
//! source badge and the label scrim. The shared primitives (surface radius,
//! artwork fitting, card surface, text scale) and the button classes are
//! imported, not copied.
//!
//! The tile draws every surface's card - the library grid, the dashboard's
//! rails - so this scrim and its [`TEXT_TILE_LABEL`] are the whole label
//! treatment a card has: [`super::app_label`] reads the name off the artwork
//! at the same size, for surfaces that have the room for it.

use cosmic::Theme;
use cosmic::iced::core::{Background, Border, Color, Shadow};
use cosmic::widget::container;

use crate::style::{TEXT_SCALE, card_surface, surface_radius};

/// Tile label text.
///
/// Read at TV distance, and it rides on the cover art rather than on its own
/// strip below it.
pub const TEXT_TILE_LABEL: f32 = 24.0 * TEXT_SCALE;

/// Line height of the name text, as a multiple of [`TEXT_TILE_LABEL`].
///
/// Pinned here and handed to the text, so [`LABEL_HEIGHT`] is derived from the
/// same number the renderer draws with: a band figured from any other line
/// height is how a wrapped name outgrows its rail.
pub const LABEL_LINE_HEIGHT: f32 = 1.4;

/// How many lines of a name the band holds before the ellipsis cuts it
/// ([`super::app_label`]).
pub const LABEL_MAX_LINES: usize = 2;

/// Height of the name line a surface keeps below its cards
/// ([`super::app_label`]).
///
/// Room for the [`LABEL_MAX_LINES`] wrapped lines a long name may reach at
/// [`LABEL_LINE_HEIGHT`], plus the air around them - half a text size. Most
/// names are one line at a rail's width, but a longer one must not push what
/// is below it out of where the page scroll thinks it is. Fixed rather than
/// measured - only the renderer measures text - so the layout cannot shift as
/// the cursor walks between rails.
pub const LABEL_HEIGHT: f32 = TEXT_TILE_LABEL * (LABEL_LINE_HEIGHT * LABEL_MAX_LINES as f32 + 0.5);

/// Size of the source badge overlaid on the tile artwork corner.
pub const SOURCE_BADGE: f32 = 28.0;

/// Background for the source badge overlaid on tile artwork. A card-like
/// surface, so it shares the same radius and colors as the tile instead of
/// borrowing a different preset.
pub fn source_badge(theme: &Theme) -> container::Style {
    card_surface(theme)
}

/// Translucent scrim behind a tile's label, drawn over the bottom of the cover
/// art.
///
/// The label sits on artwork, not on the page, so it needs a scrim to stay
/// readable. The scrim is the theme's background at partial opacity and the text
/// its `on_bg_color`, so it follows the palette under both themes while the art
/// shows through. Only the bottom corners follow the tile radius; the top edge
/// meets the artwork and stays square.
pub fn tile_label_overlay(theme: &Theme) -> container::Style {
    let radius = surface_radius(theme);
    let t = theme.cosmic();
    let mut background: Color = t.bg_color().into();
    background.a = 0.5;
    container::Style {
        text_color: Some(t.on_bg_color().into()),
        icon_color: Some(t.on_bg_color().into()),
        background: Some(Background::Color(background)),
        border: Border {
            radius: [0.0, 0.0, radius[2], radius[3]].into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        shadow: Shadow::default(),
        snap: false,
    }
}
