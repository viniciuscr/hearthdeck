//! Styles used only by the application grid tile.
//!
//! These dress the tile that [`super::app_tile`] builds and live next to it
//! rather than in the shared design system. What stays tile-specific here is the
//! source badge, the label scrim and the fitted-artwork helper. The shared
//! primitives (surface radius, artwork fitting, card surface, text scale) and
//! the button classes are imported, not copied.
//!
//! The rail draws its own label scrim and size from [`crate::style`]'s
//! `rail_label_overlay`/`TEXT_RAIL_LABEL`; the two are deliberately different
//! sizes, so the tile's pair is named for the tile.

use cosmic::Element;
use cosmic::Theme;
use cosmic::iced::core::{Background, Border, Color, Shadow};
use cosmic::iced::{ContentFit, Length};
use cosmic::widget::{container, icon};

use crate::style::{TEXT_SCALE, artwork_fit, card_surface, surface_radius};

/// Tile label text.
///
/// Larger than the rail's `TEXT_RAIL_LABEL`: a grid tile is read at TV distance
/// and its label rides on the cover art rather than on its own strip.
pub const TEXT_TILE_LABEL: f32 = 24.0 * TEXT_SCALE;

/// Size of the source badge overlaid on the tile artwork corner.
pub const SOURCE_BADGE: f32 = 28.0;

/// Artwork scaled to *fit* inside its bounds, whole: the image is resized to the
/// largest size that still fits and centred, so a cover whose aspect ratio does
/// not match the tile is neither stretched nor cropped to match it. This is what
/// grid tiles use — box art comes in too many shapes for a cover-crop to be
/// trusted with the title art.
pub fn artwork_contained<'a, M: 'a>(
    handle: &icon::Handle,
    width: Length,
    height: Length,
) -> Element<'a, M> {
    artwork_fit(handle, ContentFit::Contain, width, height)
}

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
