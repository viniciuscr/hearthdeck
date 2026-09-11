//! Central design system for HearthDeck.
//!
//! Every visual constant and every custom widget style lives in this module,
//! so a single edit propagates through the whole UI. Views (`app.rs`,
//! `widgets/application.rs`) must never hardcode their own sizes, colors,
//! radii or style structs; they pull them from here instead.
//!
//! Theme-derived values (COSMIC's `space_*` spacing, `corner_radii`, accents
//! and colors) are used as-is so the app still follows the user's system
//! theme — they are already a centralized design-token system.

use cosmic::Element;
use cosmic::Theme;
use cosmic::iced::core::{Background, Border, Color, Shadow};
use cosmic::iced::{ContentFit, Length};
use cosmic::theme::Button;
use cosmic::widget::button::Catalog;
use cosmic::widget::{button, container, icon};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Typography
// ---------------------------------------------------------------------------

/// Global multiplier applied to every named text size below. COSMIC's own text
/// scaling from Settings still applies on top at render time.
const TEXT_SCALE: f32 = 1.3;

/// Page title.
pub const TEXT_TITLE: f32 = 40.0 * TEXT_SCALE;
/// Header text and text inputs (matches `text::text` default).
pub const TEXT_HEADER: f32 = 20.0 * TEXT_SCALE;
/// Emphasis text (sidebar sections, storage value).
pub const TEXT_LARGE: f32 = 16.0 * TEXT_SCALE;
/// Body text (matches `text::body`).
pub const TEXT_BODY: f32 = 14.0 * TEXT_SCALE;
/// Caption text (matches `text::caption`).
pub const TEXT_CAPTION: f32 = 12.0 * TEXT_SCALE;
/// Tile label text.
pub const TEXT_TILE_LABEL: f32 = 13.0 * TEXT_SCALE;

// ---------------------------------------------------------------------------
// Motion
//
// COSMIC ships no theme-level animation timing, so the frontend owns its own
// motion tokens here. Widgets take a duration as a parameter (the same shape as
// libcosmic's own `toggler`/`cards`); the values themselves live only here.
// ---------------------------------------------------------------------------

/// Duration of the fade-through between the Dashboard and the Library. Sits in
/// the 167-333 ms band that Fluent gives for page transitions.
pub const PAGE_TRANSITION_DURATION: Duration = Duration::from_millis(240);

// ---------------------------------------------------------------------------
// Window & layout (proportional — sizes computed from window_width)
// ---------------------------------------------------------------------------

/// Initial window size.
pub const WINDOW_WIDTH: f32 = 1200.0;
pub const WINDOW_HEIGHT: f32 = 690.0;

/// Sidebar width as a fraction of window width (Xbox: 23%).
pub const SIDEBAR_RATIO: f32 = 0.23;
/// Minimum sidebar width in pixels (prevents collapsing on tiny windows).
pub const SIDEBAR_MIN_WIDTH: f32 = 200.0;
/// Maximum sidebar width in pixels.
pub const SIDEBAR_MAX_WIDTH: f32 = 440.0;
/// Width of the accent bar shown next to the active sidebar section. A stroke
/// thickness rather than a spacing value, so it stays fixed across densities.
pub const SIDEBAR_ACCENT_BAR_WIDTH: f32 = 6.0;

/// The active theme's spacing scale - COSMIC's Compact/Standard/Spacious
/// density choice. Sizes below are built from these tokens instead of fixed
/// pixels, so the whole layout follows the system setting.
fn spacing() -> cosmic::cosmic_theme::Spacing {
    cosmic::theme::spacing()
}

/// Height of the sidebar header (app icon + user name); 80px at Standard.
pub fn sidebar_header_height() -> f32 {
    let s = spacing();
    f32::from(s.space_xl + s.space_l)
}

/// Height of each fixed navigation item in the sidebar; 56px at Standard.
pub fn sidebar_item_height() -> f32 {
    let s = spacing();
    f32::from(s.space_l + s.space_m)
}

/// Height of the accent bar shown next to the active sidebar section; 40px at
/// Standard.
pub fn sidebar_accent_bar_height() -> f32 {
    let s = spacing();
    f32::from(s.space_l + s.space_xxs)
}

/// Horizontal padding inside the main content panel, taken from the active
/// theme's spacing scale so it grows and shrinks with COSMIC's density setting.
pub fn content_horizontal_padding() -> u16 {
    spacing().space_m
}

/// Number of installed titles shown on the dashboard.
pub const DASHBOARD_VISIBLE_TILES: usize = 4;

/// Number of columns in the application grid.
pub const GRID_COLUMNS: usize = 4;
/// Gap between grid tiles as a fraction of tile width. Tiles are recomputed
/// from the remaining content width, so a larger ratio shrinks each cover a
/// little and buys noticeably more air between cards than the Xbox-style 6.5%.
pub const GRID_GAP_RATIO: f32 = 0.14;

/// Top padding of the scrollable grid; keeps the focus ring on the first row
/// from being clipped by the viewport and gives the first cover row breathing
/// room below the tab strip. Taken from the theme's spacing scale so it follows
/// the density setting.
pub fn grid_top_padding() -> u16 {
    spacing().space_xs
}

/// Width of the 1px vertical dividers.
pub const DIVIDER_WIDTH: f32 = 1.0;

// ---------------------------------------------------------------------------
// Grid tiles (computed — these are functions, not constants)
// ---------------------------------------------------------------------------

/// Compute the sidebar width from the window width.
pub fn sidebar_width(window_width: f32) -> f32 {
    (window_width * SIDEBAR_RATIO).clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH)
}

/// Compute the content area width from the window width.
pub fn content_width(window_width: f32) -> f32 {
    window_width
        - sidebar_width(window_width)
        - DIVIDER_WIDTH
        - 2.0 * f32::from(content_horizontal_padding())
}

/// Compute the gap between grid tiles from the tile width.
/// Uses self-referencing formula: tile = (cw - (cols-1)*gap) / cols,
/// gap = ratio * tile.
pub fn grid_gap(window_width: f32) -> f32 {
    let cw = content_width(window_width);
    let cols = GRID_COLUMNS as f32;
    // cw = cols * tile + (cols - 1) * ratio * tile = tile * (cols + (cols-1)*ratio)
    let est_tile = cw / (cols + (cols - 1.0) * GRID_GAP_RATIO);
    let s = spacing();
    (est_tile * GRID_GAP_RATIO).clamp(f32::from(s.space_xxs), f32::from(s.space_xl))
}

/// Compute the tile width from the padded content width.
pub fn tile_width(window_width: f32) -> f32 {
    let cw = content_width(window_width);
    let gap = grid_gap(window_width);
    ((cw - (GRID_COLUMNS as f32 - 1.0) * gap) / GRID_COLUMNS as f32).max(60.0)
}

/// Games use common 2:3 portrait cover art; applications remain square.
pub fn tile_height(window_width: f32, is_game: bool) -> f32 {
    tile_width(window_width) * if is_game { 1.5 } else { 1.0 }
}

/// Square dashboard tiles fill one horizontal row without shifting on focus.
pub fn dashboard_tile_size(window_width: f32, horizontal_padding: u16, tile_gap: u16) -> f32 {
    let available = window_width
        - 2.0 * f32::from(horizontal_padding)
        - (DASHBOARD_VISIBLE_TILES - 1) as f32 * f32::from(tile_gap);
    (available / DASHBOARD_VISIBLE_TILES as f32).clamp(140.0, 360.0)
}

/// Size of the drag-preview icon shown while dragging a tile.
pub const TILE_DRAG_ICON: f32 = 88.0;
/// Size of the source badge overlaid on the tile artwork corner.
pub const SOURCE_BADGE: f32 = 28.0;

// ---------------------------------------------------------------------------
// Top bar & controls
// ---------------------------------------------------------------------------

/// Width of the search field.
pub const SEARCH_WIDTH: f32 = 400.0;
/// Size of the search field's leading icon.
pub const ICON_SEARCH: u16 = 32;
/// Padding around the search field's leading icon; 4px at Standard.
pub fn search_icon_padding() -> u16 {
    spacing().space_xxxs
}
/// Width of the inline "rename" input next to the page title.
pub const EDIT_NAME_INPUT_WIDTH: f32 = 280.0;
/// Height of the icon buttons next to the page title (rename/delete); 48px at
/// Standard.
pub fn title_action_height() -> f32 {
    f32::from(spacing().space_xl)
}
/// Height of the filter button on the tab row; 40px at Standard.
pub fn filter_button_height() -> f32 {
    let s = spacing();
    f32::from(s.space_l + s.space_xxs)
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

/// Height of a tab (button + underline); 48px at Standard.
pub fn tab_height() -> f32 {
    f32::from(spacing().space_xl)
}
/// Height of the accent underline of the active tab. A fixed hairline rather
/// than a density token: it is a 1px-scale accent, so scaling it with the
/// spacing scale made it visibly too thick at higher densities.
pub fn tab_underline_height() -> f32 {
    4.0
}
/// Per-character text advance used to estimate a tab's intrinsic width.
const TAB_CHAR_ADVANCE: f32 = 0.6;
/// Fixed horizontal padding included in the estimated tab width.
const TAB_BASE_WIDTH: f32 = 64.0;

/// Estimated intrinsic width of a tab, so the accent underline can span just
/// the label without the `Fill` widths expanding (and wrapping) the row.
pub fn tab_width(label: &str) -> f32 {
    label.chars().count() as f32 * TEXT_BODY * TAB_CHAR_ADVANCE + TAB_BASE_WIDTH
}

// ---------------------------------------------------------------------------
// Dialogs & menus
// ---------------------------------------------------------------------------

/// Width of the new-group / delete-group dialogs and their text inputs.
pub const DIALOG_WIDTH: f32 = 432.0;
/// Width of a dialog action button.
pub const DIALOG_ACTION_WIDTH: u16 = 142;
/// Maximum size of the context menu surface.
pub const MENU_MAX_WIDTH: f32 = 300.0;
pub const MENU_MAX_HEIGHT: f32 = 800.0;

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

/// Largest icons: sidebar header and dialog artwork.
pub const ICON_LARGE: u16 = 64;
/// Icons inside body-sized controls (storage, filter, add group).
pub const ICON_BODY: u16 = 24;
/// Icons inside title-row action buttons (rename/delete); also the width/height
/// of those icon buttons.
pub const ICON_TILE_ACTION: f32 = 32.0;
/// Small icons and inline spacers (menu checkboxes, source badges).
pub const ICON_SMALL: u16 = 20;

// ---------------------------------------------------------------------------
// Focus rings
// ---------------------------------------------------------------------------

/// Thickness of the focus ring shared by every focusable element (sidebar
/// items, tabs and grid tiles).
pub const FOCUS_RING_WIDTH: f32 = 4.0;

// ---------------------------------------------------------------------------
// Surfaces
// ---------------------------------------------------------------------------

/// The corner radius shared by every card-like surface: grid and dashboard
/// tiles, the artwork they contain, and tile label scrims. Single source of
/// truth, derived from COSMIC's corner radii so the Round / Slightly round /
/// Square setting applies everywhere at once.
pub fn surface_radius(theme: &Theme) -> [f32; 4] {
    theme.cosmic().corner_radii.radius_m
}

fn active_surface_radius() -> [f32; 4] {
    surface_radius(&cosmic::theme::active())
}

/// Renders an icon as artwork clipped to the shared surface radius.
///
/// Every raster image the app displays goes through here, so rounded corners
/// are a design-system primitive rather than a per-widget decision. Entry
/// icons are always raster by this point (the icon cache rasterizes SVGs); an
/// SVG handle falls back to the plain icon, which has no clip.
pub fn artwork<'a, M: 'a>(handle: &icon::Handle, width: Length, height: Length) -> Element<'a, M> {
    match &handle.data {
        icon::Data::Image(image) => cosmic::widget::image::Image::new(image.clone())
            .content_fit(ContentFit::Fill)
            .border_radius(active_surface_radius())
            .width(width)
            .height(height)
            .into(),
        icon::Data::Svg(_) => handle.clone().icon().width(width).height(height).into(),
    }
}

// ---------------------------------------------------------------------------
// Container styles
// ---------------------------------------------------------------------------

/// Dark semi-transparent overlay for tile labels at the bottom of game cards.
///
/// The scrim is always dark because it sits on top of cover artwork, not on
/// the theme background, so the label uses a fixed light color instead of the
/// theme's `on_bg_color` - that is *dark* under a light theme and would be
/// unreadable here. Only the bottom corners follow the tile radius; the top
/// edge meets the artwork and stays square.
pub fn tile_label_overlay(theme: &Theme) -> container::Style {
    let radius = surface_radius(theme);
    container::Style {
        text_color: Some(Color::WHITE),
        icon_color: Some(Color::WHITE),
        background: Some(Background::Color(Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.65,
        })),
        border: Border {
            radius: [0.0, 0.0, radius[2], radius[3]].into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// Background for the source badge overlaid on tile artwork. A card-like
/// surface, so it shares the same radius and colors as the tile instead of
/// borrowing a different preset.
pub fn source_badge(theme: &Theme) -> container::Style {
    let t = theme.cosmic();
    let on = t.background(theme.transparent).component.on;
    container::Style {
        text_color: Some(on.into()),
        icon_color: Some(on.into()),
        background: Some(Background::Color(
            t.background(theme.transparent).component.base.into(),
        )),
        border: Border {
            radius: surface_radius(theme).into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// Opaque launch layer shown while a selected game is starting.
///
/// Uses the theme's own background at near-full opacity rather than a fixed
/// dark color, so the default text colors stay readable under a light theme.
pub fn launch_overlay(theme: &Theme) -> container::Style {
    let t = theme.cosmic();
    let mut background: Color = t.bg_color().into();
    background.a = 0.97;
    container::Style {
        text_color: Some(t.on_bg_color().into()),
        icon_color: Some(t.accent_color().into()),
        background: Some(Background::Color(background)),
        ..container::Style::default()
    }
}

/// Accent bar: the indicator next to the active sidebar section and the
/// underline of the active tab. Its corner radius comes from the theme so it
/// follows COSMIC's Round/Slightly-round/Square setting.
pub fn accent_bar(theme: &Theme) -> container::Style {
    container::Style {
        text_color: None,
        icon_color: None,
        background: Some(Background::Color(Color::from(theme.cosmic().accent.base))),
        border: Border {
            radius: theme.cosmic().corner_radii.radius_xs.into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// Vertical divider between the sidebar and the content column.
pub fn sidebar_divider(theme: &Theme) -> container::Style {
    container::Style {
        text_color: None,
        icon_color: None,
        background: Some(theme.cosmic().bg_divider().into()),
        border: Border {
            radius: [0.0; 4].into(),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// Root window background.
pub fn root_background(theme: &Theme) -> container::Style {
    let t = theme.cosmic();
    container::Style {
        text_color: Some(t.on_bg_color().into()),
        icon_color: Some(t.on_bg_color().into()),
        background: Some(Color::from(t.background(theme.transparent).base).into()),
        border: Border {
            radius: [0.0; 4].into(),
            width: 0.0,
            color: t.bg_divider().into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

// ---------------------------------------------------------------------------
// Button styles
// ---------------------------------------------------------------------------

/// A subtle neutral fill derived from the theme's on-color so it stays visible
/// on both light and dark themes.
fn chip_background(alpha: f32, theme: &Theme) -> Background {
    let mut color: Color = theme.cosmic().on_bg_color().into();
    color.a = alpha;
    Background::Color(color)
}

/// Applies the shared focus ring to a button style: a single solid accent
/// border.
fn focus_ring(mut style: button::Style, focused: bool, theme: &Theme) -> button::Style {
    if focused {
        style.border_width = FOCUS_RING_WIDTH;
        style.border_color = theme.cosmic().accent.base.into();
        style.outline_width = 0.0;
        style.outline_color = Color::TRANSPARENT;
    }
    style
}

/// Compact icon buttons used by the dashboard's centered top navigation.
pub fn dashboard_nav_button_class(selected: bool) -> Button {
    Button::Custom {
        active: Box::new(move |focused, theme| {
            focus_ring(
                theme.active(focused, selected, &Button::Icon),
                focused,
                theme,
            )
        }),
        disabled: Box::new(|theme| theme.disabled(&Button::Icon)),
        hovered: Box::new(move |focused, theme| {
            focus_ring(
                theme.hovered(focused, selected, &Button::Icon),
                focused,
                theme,
            )
        }),
        pressed: Box::new(move |focused, theme| {
            focus_ring(
                theme.pressed(focused, selected, &Button::Icon),
                focused,
                theme,
            )
        }),
    }
}

/// Sidebar section buttons: transparent when idle, a light-grey full-width bar
/// when selected (with an inset look that avoids a visible seam at the content
/// edge), matching the Xbox reference sidebar.
pub fn section_button_class(selected: bool) -> Button {
    Button::Custom {
        active: Box::new(move |focused, theme| {
            let mut style = theme.active(focused, false, &Button::IconVertical);
            style.border_radius = theme.cosmic().corner_radii.radius_m.into();
            if selected {
                style.background = Some(chip_background(0.18, theme));
                style.text_color = Some(theme.cosmic().on_bg_color().into());
                style.icon_color = Some(theme.cosmic().on_bg_color().into());
            }
            focus_ring(style, focused, theme)
        }),
        disabled: Box::new(|theme| theme.disabled(&Button::IconVertical)),
        hovered: Box::new(move |focused, theme| {
            let mut style = theme.hovered(focused, false, &Button::IconVertical);
            style.border_radius = theme.cosmic().corner_radii.radius_m.into();
            style.background = Some(chip_background(if selected { 0.18 } else { 0.06 }, theme));
            focus_ring(style, focused, theme)
        }),
        pressed: Box::new(move |focused, theme| {
            let mut style = theme.pressed(focused, false, &Button::IconVertical);
            style.border_radius = theme.cosmic().corner_radii.radius_m.into();
            if selected {
                style.background = Some(chip_background(0.18, theme));
            }
            focus_ring(style, focused, theme)
        }),
    }
}

/// Sub-tab row buttons: text-only with a transparent background matching the
/// reference; the active tab's selection is drawn by its text color, and a
/// subtle fill appears only on hover.
pub fn tab_button_class(selected: bool) -> Button {
    Button::Custom {
        active: Box::new(move |focused, theme| {
            let mut style = theme.active(focused, false, &Button::IconVertical);
            style.background = None;
            if selected {
                style.text_color = Some(theme.cosmic().on_bg_color().into());
            }
            focus_ring(style, focused, theme)
        }),
        disabled: Box::new(|theme| theme.disabled(&Button::IconVertical)),
        hovered: Box::new(move |focused, theme| {
            let mut style = theme.hovered(focused, false, &Button::IconVertical);
            style.background = Some(chip_background(0.08, theme));
            focus_ring(style, focused, theme)
        }),
        pressed: Box::new(move |focused, theme| {
            let mut style = theme.pressed(focused, false, &Button::IconVertical);
            style.background = None;
            focus_ring(style, focused, theme)
        }),
    }
}

/// Grid tile appearance: the shared surface radius, plus the focus ring when
/// the tile is focused. Exactly one border is ever drawn.
fn tile_style(mut style: button::Style, focused: bool, theme: &Theme) -> button::Style {
    style.border_radius = surface_radius(theme).into();
    focus_ring(style, focused, theme)
}

/// Grid tile buttons: focused tiles get the shared accent ring so selection is
/// clearly visible. Tiles have the shared surface corner radius.
pub fn tile_button_class(selected: bool) -> Button {
    Button::Custom {
        active: Box::new(move |focused, theme| {
            tile_style(
                theme.active(focused, selected, &Button::IconVertical),
                focused,
                theme,
            )
        }),
        disabled: Box::new(move |theme| {
            tile_style(theme.disabled(&Button::IconVertical), false, theme)
        }),
        hovered: Box::new(move |focused, theme| {
            tile_style(
                theme.hovered(focused, selected, &Button::IconVertical),
                focused,
                theme,
            )
        }),
        pressed: Box::new(move |focused, theme| {
            tile_style(
                theme.pressed(focused, selected, &Button::IconVertical),
                focused,
                theme,
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DASHBOARD_VISIBLE_TILES, GRID_COLUMNS, content_horizontal_padding, content_width,
        dashboard_tile_size, filter_button_height, grid_gap, grid_top_padding, search_icon_padding,
        sidebar_accent_bar_height, sidebar_header_height, sidebar_item_height, sidebar_width,
        tab_height, tab_underline_height, tile_height, tile_width, title_action_height,
    };

    #[test]
    fn theme_metrics_match_standard_density() {
        // The test process uses libcosmic's default (Standard) density, so the
        // theme-derived sizes must reproduce the pixel values they replaced.
        assert_eq!(sidebar_header_height(), 80.0);
        assert_eq!(sidebar_item_height(), 56.0);
        assert_eq!(sidebar_accent_bar_height(), 40.0);
        assert_eq!(title_action_height(), 48.0);
        assert_eq!(filter_button_height(), 40.0);
        assert_eq!(tab_height(), 48.0);
        assert_eq!(tab_underline_height(), 4.0);
        assert_eq!(content_horizontal_padding(), 24);
        assert_eq!(grid_top_padding(), 12);
        assert_eq!(search_icon_padding(), 4);
    }

    #[test]
    fn grid_fits_inside_padded_content() {
        let window_width = 1200.0;
        let occupied = GRID_COLUMNS as f32 * tile_width(window_width)
            + (GRID_COLUMNS - 1) as f32 * grid_gap(window_width);

        assert!((occupied - content_width(window_width)).abs() < 0.01);
        assert!(
            occupied + sidebar_width(window_width) + 2.0 * f32::from(content_horizontal_padding())
                <= window_width
        );
    }

    #[test]
    fn dashboard_tiles_fit_themed_spacing() {
        let window_width = 1200.0;
        // `space_l` horizontal padding and `space_l` gap between tiles, at
        // libcosmic's default (Standard) density.
        let padding = 32;
        let gap = 32;
        let occupied = DASHBOARD_VISIBLE_TILES as f32
            * dashboard_tile_size(window_width, padding, gap)
            + (DASHBOARD_VISIBLE_TILES - 1) as f32 * f32::from(gap)
            + 2.0 * f32::from(padding);

        assert!((occupied - window_width).abs() < 0.01);
    }

    #[test]
    fn game_tiles_are_portrait_and_application_tiles_are_square() {
        let width = tile_width(1200.0);
        assert_eq!(tile_height(1200.0, false), width);
        assert_eq!(tile_height(1200.0, true), width * 1.5);
    }
}
