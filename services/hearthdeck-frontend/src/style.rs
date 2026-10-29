//! Central design system for HearthDeck.
//!
//! Shared visual constants and custom widget styles for HearthDeck.
//!
//! An edit to a token here propagates through the whole UI. Views (`app.rs`) and
//! widgets must not hardcode their own sizes, colors, radii or style structs.
//! Styles that only one widget draws live next to that widget instead - see
//! `widgets/application/style.rs` - so this module keeps what is shared.
//!
//! Theme-derived values (COSMIC's `space_*` spacing, `corner_radii`, accents
//! and colors) are used as-is so the app still follows the user's system
//! theme — they are already a centralized design-token system.

use cosmic::Element;
use cosmic::Theme;
use cosmic::iced::Radians;
use cosmic::iced::core::{Background, Border, Color, Shadow};
use cosmic::iced::gradient::Linear;
use cosmic::iced::{ContentFit, Length, Vector};
use cosmic::theme::Button;
use cosmic::widget::button::Catalog;
use cosmic::widget::{button, container, icon};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Typography
// ---------------------------------------------------------------------------

/// Global multiplier applied to every named text size below. COSMIC's own text
/// scaling from Settings still applies on top at render time.
pub(crate) const TEXT_SCALE: f32 = 1.3;

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

/// Details screen title — larger for TV viewing distance.
pub const DETAILS_TEXT_TITLE: f32 = 56.0 * TEXT_SCALE;
/// Details screen header text.
pub const DETAILS_TEXT_HEADER: f32 = 28.0 * TEXT_SCALE;
/// Details screen emphasis text (action buttons).
pub const DETAILS_TEXT_LARGE: f32 = 22.0 * TEXT_SCALE;
/// Details screen body text (facts, summary).
pub const DETAILS_TEXT_BODY: f32 = 18.0 * TEXT_SCALE;
/// Details screen caption text (chips, labels).
pub const DETAILS_TEXT_CAPTION: f32 = 16.0 * TEXT_SCALE;

/// The class a line that reports a failure wears: the theme's destructive color,
/// so an error does not read as one more quiet caption.
///
/// Resolved at draw time rather than snapshotting the active theme, so a theme
/// change repaints the line with the rest of the page.
pub fn error_text_class() -> cosmic::theme::Text {
    cosmic::theme::Text::Custom(|theme| cosmic::iced::widget::text::Style {
        color: Some(theme.cosmic().destructive_color().into()),
        ..cosmic::iced::widget::text::Style::default()
    })
}

// ---------------------------------------------------------------------------
// Motion
//
// COSMIC ships no theme-level animation timing, so the frontend owns its own
// motion tokens here. Widgets take a duration as a parameter (the same shape as
// libcosmic's own `toggler`/`cards`); the values themselves live only here.
// ---------------------------------------------------------------------------

/// Duration of the fade-through between the Dashboard and the Library. Longer
/// than the tab slide because a cross-fade reads as slower than a slide; sits in
/// the 167-333 ms band Fluent gives for page transitions.
pub const PAGE_TRANSITION_DURATION: Duration = Duration::from_millis(320);

/// Duration of the directional slide when switching tabs. Shorter than the page
/// fade: tabs are switched often, and a shared-axis slide carries its own sense
/// of direction.
pub const TAB_TRANSITION_DURATION: Duration = Duration::from_millis(240);

/// Duration of the vertical slide when switching section (PC Games / Console
/// Games / Applications). Longer than the tab slide because a section swap
/// replaces the whole content column, and the taller travel reads better with a
/// little more time; still inside the 167-333 ms band Fluent gives for this kind
/// of transition.
pub const SECTION_TRANSITION_DURATION: Duration = Duration::from_millis(300);

/// Duration of an eased scroll when focus drags a list by itself (grid rows,
/// dashboard shelves and rails). Every list routes through the same animation,
/// so they all move at this speed. Short, because it fires on each focus step:
/// long enough to read as motion, short enough not to lag the D-pad.
pub const SCROLL_TRANSITION_DURATION: Duration = Duration::from_millis(220);

/// How long the dashboard's hero banner holds one slide before moving on.
///
/// Long enough to read a wordmark and a line of synopsis, short enough that a
/// banner nobody is looking at comes round again before anyone misses it. The
/// banner only advances while the pad is elsewhere, so this is the pace of the
/// page's own idle motion rather than of anything the user is on.
/// How long the window's background takes to cross-fade from one title's art to
/// the next as the pad walks a rail. A little longer than the page fade,
/// because it is a bigger surface moving and the same smootherstep easing.
pub const BACKGROUND_TRANSITION_DURATION: Duration = Duration::from_millis(400);

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

/// Game cards on the dashboard are portrait, matching the posters they show.
/// Square cards cropped a poster to its middle, which usually cuts off the part
/// of the art that carries the title.
pub const DASHBOARD_GAME_ASPECT: f32 = 2.0 / 3.0;

/// Number of cards a dashboard rail may hold. The rail scrolls horizontally,
/// so this only bounds how much it fetches and renders up front.
pub const DASHBOARD_RAIL_TILES: usize = 12;

/// Number of columns in the application grid.
pub const GRID_COLUMNS: usize = 4;
/// Number of columns in the Console Games grid. A console's covers are the same
/// 2:3 box as any game's, but the section reads better as a denser wall of
/// smaller boxes; six columns makes each tile ~30% smaller than the default
/// four, which is the size the console grid is designed around.
pub const CONSOLE_GRID_COLUMNS: usize = 6;
/// Gap between grid tiles as a fraction of tile width. Tiles are recomputed
/// from the remaining content width, so a larger ratio shrinks each cover a
/// little and buys noticeably more air between cards than the Xbox-style 6.5%.
pub const GRID_GAP_RATIO: f32 = 0.14;

/// Rows of grid built beyond the viewport, above and below, so a small scroll
/// does not reveal a row that has not been built yet. The grid is virtualized,
/// so this is the whole margin between what is on screen and what exists as
/// widgets.
pub const GRID_OVERSCAN_ROWS: usize = 2;

/// How many columns a section's grid uses. Consoles get the denser layout;
/// every other section keeps the default. This is the single place the two grid
/// densities are decided, so a tile's size, its row height and the page's
/// scroll math all agree.
pub fn grid_columns(is_console: bool) -> usize {
    if is_console {
        CONSOLE_GRID_COLUMNS
    } else {
        GRID_COLUMNS
    }
}

/// Vertical space the library header - the title and search row, the tab strip
/// and the spacings between them - takes above the grid. The header is a sum of
/// theme spacings and text the renderer measures, so it cannot be derived here;
/// this is the estimate only the first frame's virtualization needs.
const GRID_HEADER_HEIGHT: f32 = 160.0;

/// Height the grid's viewport is assumed to have before the scrollable has
/// reported its own.
///
/// Only the first frame reads it, to decide how many rows to build; the real
/// height arrives with the first scroll, and the rows already built are covered
/// by the overscan either way. It exists because a grid built from an assumed
/// zero-height viewport would draw a single row until the user scrolled.
pub fn grid_viewport_height_fallback() -> f32 {
    WINDOW_HEIGHT - GRID_HEADER_HEIGHT
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
pub fn grid_gap(window_width: f32, columns: usize) -> f32 {
    let cw = content_width(window_width);
    let cols = columns as f32;
    // cw = cols * tile + (cols - 1) * ratio * tile = tile * (cols + (cols-1)*ratio)
    let est_tile = cw / (cols + (cols - 1.0) * GRID_GAP_RATIO);
    let s = spacing();
    (est_tile * GRID_GAP_RATIO).clamp(f32::from(s.space_xxs), f32::from(s.space_xl))
}

/// Compute the tile width from the padded content width.
pub fn tile_width(window_width: f32, columns: usize) -> f32 {
    let cw = content_width(window_width);
    let gap = grid_gap(window_width, columns);
    ((cw - (columns as f32 - 1.0) * gap) / columns as f32).max(60.0)
}

/// The library grid's tile height: the dashboard's own portrait card, so a cover
/// is drawn at the same proportion wherever it appears. The width is the grid's
/// and the height follows from it, which keeps a row's pitch exact.
#[must_use]
pub fn grid_tile_height(tile_width: f32) -> f32 {
    tile_width / DASHBOARD_GAME_ASPECT
}

/// Square dashboard tiles fill one horizontal row without shifting on focus.
///
/// A tile takes only [`DASHBOARD_TILE_SCALE`] of the space a row divides into,
/// so the rail shows more of itself - and the blank above it grows by the
/// difference.
pub fn dashboard_tile_size(window_width: f32, horizontal_padding: u16, tile_gap: u16) -> f32 {
    let available = window_width
        - 2.0 * f32::from(horizontal_padding)
        - (DASHBOARD_VISIBLE_TILES - 1) as f32 * f32::from(tile_gap);
    (available / DASHBOARD_VISIBLE_TILES as f32).clamp(140.0, 360.0) * DASHBOARD_TILE_SCALE
}

/// How much of the row's space one dashboard tile takes: the tiles are 30%
/// smaller than the row divides into.
const DASHBOARD_TILE_SCALE: f32 = 0.7;

/// Console logo tiles are half the size of a full dashboard tile, so the
/// console rail reads as secondary navigation below Recently Played rather
/// than another full-size shelf.
pub fn dashboard_console_tile_size(
    window_width: f32,
    horizontal_padding: u16,
    tile_gap: u16,
) -> f32 {
    dashboard_tile_size(window_width, horizontal_padding, tile_gap) / 2.0
}

// ---------------------------------------------------------------------------
// Details screen
// ---------------------------------------------------------------------------

/// Share of the content width the details screen's hero image column takes.
/// The rest carries the text column, which needs the room more than the art
/// does once the title and fact rows are in.
const DETAILS_HERO_RATIO: f32 = 0.42;

/// Floor for the hero column, so the art stays legible on a narrow window.
const DETAILS_HERO_MIN_WIDTH: f32 = 240.0;

/// Width of the details screen's hero column.
pub fn details_hero_width(window_width: f32) -> f32 {
    (content_width(window_width) * DETAILS_HERO_RATIO).max(DETAILS_HERO_MIN_WIDTH)
}

/// Raster size for the hero artwork. Handles are cached per (image, size), so a
/// fixed size keeps one decoded copy per game instead of one per window width.
pub const DETAILS_HERO_RASTER: u32 = 640;

/// Raster size for the screenshot strip's thumbnails.
pub const DETAILS_SHOT_RASTER: u32 = 320;

/// Height of one card in the screenshot strip. The width follows from
/// [`DETAILS_SHOT_ASPECT`], so a 16:9 screenshot is shown in its own shape
/// rather than cropped to the dashboard's square cards.
pub const DETAILS_SHOT_SIZE: f32 = 132.0;

/// Screenshot aspect ratio (16:9).
pub const DETAILS_SHOT_ASPECT: f32 = 16.0 / 9.0;

/// Width of the label column in the details screen's fact list.
pub const DETAILS_FACT_LABEL_WIDTH: f32 = 132.0;

/// Height shared by every button on a details action line; 64px at Standard.
/// One height for all of them: the line reads as a single strip of controls, and
/// Play is set apart by width and the accent fill rather than by being taller.
pub fn details_action_height() -> f32 {
    f32::from(spacing().space_xxl)
}

/// Width of a secondary button on the details action line. Fixed, so the line
/// stays even however long its labels are.
pub const DETAILS_ACTION_WIDTH: f32 = 216.0;

/// Width of Play, which leads the action line.
pub const DETAILS_PLAY_WIDTH: f32 = 276.0;

/// Padding inside the action line's backplate.
pub fn details_action_bar_padding() -> u16 {
    spacing().space_s
}

/// Widest the details screen's disc picker panel may grow.
pub const DETAILS_PICKER_WIDTH: f32 = 460.0;

/// Widest the settings page's column may grow; past this, lines get too long to read from a sofa.
pub const SETTINGS_MAX_WIDTH: f32 = 960.0;

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
/// Height of the filter button on the tab row; 48px at Standard. Matches the
/// tab strip's own height so the control sits on the tabs' baseline instead of
/// floating under them.
pub fn filter_button_height() -> f32 {
    f32::from(spacing().space_xl)
}

/// Minimum width of the filter button, so its label never shrinks it into an
/// afterthought next to the tabs.
pub const FILTER_BUTTON_MIN_WIDTH: f32 = 168.0;

/// Width of the console filter sidebar (the right-hand context drawer).
/// Clamped so it keeps a readable label column on small windows without
/// swallowing the grid on a TV-sized one.
pub fn filter_drawer_width(window_width: f32) -> f32 {
    (window_width * 0.43).clamp(340.0, FILTER_DRAWER_MAX_WIDTH)
}

/// Upper bound for [`filter_drawer_width`].
pub const FILTER_DRAWER_MAX_WIDTH: f32 = 560.0;

/// Width of the value slot in a filter sidebar row. Fixed so the stepper arrows
/// of every facet line up and long genre names cannot widen the drawer.
pub const FILTER_VALUE_WIDTH: f32 = 150.0;

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
// Menus
// ---------------------------------------------------------------------------

/// Width of the context menu card. Not a theme token: COSMIC's spacing/density
/// scale changes how much air the card has *inside* itself, not how wide the
/// menu wants to be. Kept fixed so the menu reads the same at every density
/// setting, like the same-shaped quick menu in `hearthdeck-overlay`.
pub const MENU_CARD_WIDTH: f32 = 420.0;
/// Tallest the details disc picker grows before its list scrolls instead.
pub const MENU_MAX_HEIGHT: f32 = 800.0;

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

/// Largest icons: sidebar header and dialog artwork.
pub const ICON_LARGE: u16 = 64;
/// Icons inside body-sized controls (storage, filter).
pub const ICON_BODY: u16 = 24;
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

/// Renders an icon as artwork with an explicit `ContentFit`. Game covers want
/// [`ContentFit::Cover`] while logos and other non-cover art want
/// [`ContentFit::Contain`].
///
/// Every raster image the app displays goes through here, so rounded corners
/// are a design-system primitive rather than a per-widget decision - see
/// [`artwork_fit`] for the clipped surface form and [`backdrop_fit`] for the
/// full-bleed one. Entry icons are always raster by this point (the icon cache
/// rasterizes SVGs); an SVG handle falls back to the plain icon, which has no
/// clip.
fn fit_artwork<'a, M: 'a>(
    handle: &icon::Handle,
    fit: ContentFit,
    width: Length,
    height: Length,
    clip: Option<[f32; 4]>,
    opacity: f32,
) -> Element<'a, M> {
    match &handle.data {
        icon::Data::Image(image) => {
            let mut artwork = cosmic::widget::image::Image::new(image.clone())
                .content_fit(fit)
                .width(width)
                .height(height);
            if opacity < 1.0 {
                artwork = artwork.opacity(opacity);
            }
            if let Some(radius) = clip {
                artwork = artwork.border_radius(radius);
            }
            artwork.into()
        }
        icon::Data::Svg(_) => handle.clone().icon().width(width).height(height).into(),
    }
}

/// Renders an icon as artwork clipped to the shared surface radius. See
/// [`fit_artwork`] for the fit itself.
pub fn artwork_fit<'a, M: 'a>(
    handle: &icon::Handle,
    fit: ContentFit,
    width: Length,
    height: Length,
) -> Element<'a, M> {
    fit_artwork(
        handle,
        fit,
        width,
        height,
        Some(active_surface_radius()),
        1.0,
    )
}

/// [`artwork_fit`] without the surface clip: a full-bleed backdrop - the
/// dashboard's wallpaper - fills the window, and clipped corners would show
/// what is beneath it at every edge.
pub fn backdrop_fit<'a, M: 'a>(
    handle: &icon::Handle,
    fit: ContentFit,
    width: Length,
    height: Length,
) -> Element<'a, M> {
    fit_artwork(handle, fit, width, height, None, 1.0)
}

/// [`backdrop_fit`] at an opacity: the banner's art as it grows into the
/// window's background, and the slide it is leaving, which draw at the alphas
/// the banner's motion hands them.
pub fn backdrop_faded<'a, M: 'a>(
    handle: &icon::Handle,
    fit: ContentFit,
    width: Length,
    height: Length,
    opacity: f32,
) -> Element<'a, M> {
    fit_artwork(handle, fit, width, height, None, opacity)
}

// ---------------------------------------------------------------------------
// Container styles
// ---------------------------------------------------------------------------

/// A container that only groups and centres its child: no fill, and - the point
/// - no text or icon colour.
///
/// A bare `container()` is COSMIC's `Transparent`, which *sets* the current
/// container's on-background text colour. Inside a button that is fatal to
/// theming: iced's container draws with `style.text_color.unwrap_or(outer)`, so
/// the explicit colour it sets replaces the one the button's own class
/// assigned. A button whose class paints its label - the accent-filled primary
/// action, a selected control - was therefore still drawn in the default
/// foreground. Any button that wraps its labelled content in a container must
/// use this instead.
pub fn passthrough(_theme: &Theme) -> container::Style {
    container::Style::default()
}

/// Neutral card surface for content laid over the page background: the
/// details screen's hero box and its metadata chips, and the source badge on a
/// tile. Uses the theme's card background so it reads as a raised surface under
/// both light and dark themes.
pub fn card_surface(theme: &Theme) -> container::Style {
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

/// The hero card on a details screen: the shared card surface, lifted off the
/// backdrop with a shadow so the artwork reads as a card over the page rather
/// than a hole in it.
pub fn hero_card(theme: &Theme) -> container::Style {
    let mut style = card_surface(theme);
    style.shadow = Shadow {
        color: Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.45,
        },
        offset: Vector::new(0.0, 6.0),
        blur_radius: 24.0,
    };
    style
}

/// Full-bleed wash over a details screen's backdrop art: opaque enough to keep
/// body text readable over any screenshot, translucent enough to tint the page
/// with the game's own palette.
pub fn backdrop_wash(theme: &Theme) -> container::Style {
    let mut color: Color = theme.cosmic().bg_color().into();
    color.a = 0.8;
    container::Style {
        background: Some(Background::Color(color)),
        ..container::Style::default()
    }
}

/// Backplate behind a details screen's action line: one translucent bar, so the
/// buttons read as a single strip of controls rather than floating shapes over
/// the artwork.
pub fn action_bar(theme: &Theme) -> container::Style {
    let mut style = card_surface(theme);
    if let Some(Background::Color(color)) = style.background.as_mut() {
        color.a = 0.72;
    }
    style
}

/// Bottom-up scrim for a details screen: clear over the artwork, solid behind
/// the action line so the buttons sit on a stable surface.
pub fn backdrop_scrim(theme: &Theme) -> container::Style {
    let mut solid: Color = theme.cosmic().bg_color().into();
    solid.a = 0.98;
    let mut clear = solid;
    clear.a = 0.0;
    // Radians(0) runs top-to-bottom, the direction this scrim needs.
    let gradient = Linear::new(Radians(0.0))
        .add_stop(0.0, clear)
        .add_stop(0.55, clear)
        .add_stop(1.0, solid);
    container::Style {
        background: Some(Background::from(gradient)),
        ..container::Style::default()
    }
}

/// Dimming layer behind a modal panel (the disc picker): dark enough that the
/// panel clearly sits above the page, light enough to keep the game visible
/// behind it.
pub fn modal_scrim(theme: &Theme) -> container::Style {
    let mut color: Color = theme.cosmic().bg_color().into();
    color.a = 0.7;
    container::Style {
        background: Some(Background::Color(color)),
        ..container::Style::default()
    }
}

/// Dimming layer behind the action menu.
///
/// Black, so the card keeps the same relative contrast over any page art, at an
/// opacity the theme's own brightness picks: a dark theme can afford a stronger
/// dim than a light one, which would otherwise go black behind the card. Same
/// reasoning as the quick menu's scrim in `hearthdeck-overlay`.
pub fn menu_scrim(theme: &Theme) -> container::Style {
    let alpha = if theme.cosmic().is_dark { 0.6 } else { 0.4 };
    container::Style {
        background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, alpha))),
        ..container::Style::default()
    }
}

// ---------------------------------------------------------------------------
// Button styles
//
// The design system's button palette, from most to least prominent. Views pick
// one of these by role rather than reaching for a raw `theme::Button` preset,
// so two controls that mean the same thing look the same everywhere:
//
//   * `primary_action_button_class` - the one affirmative action on a screen.
//   * `destructive_button_class`    - the action that removes something.
//   * `standard_button_class`       - an ordinary secondary action.
//   * `control_button_class`        - labelled tools and toggles (the filter
//                                     button, drawer actions).
//   * `text_button_class`           - a bare-text control (a value you click).
//   * `icon_button_class`           - icon-only tools in a toolbar or row.
//   * `tile_button_class`           - the media tiles (grid and rail cards).
//
// They all share `focus_ring` and a radius, so "this is selected" looks
// identical whether the focus is on a tile, a tab or a control.
// ---------------------------------------------------------------------------

/// A labelled control that reads as a control: a chip fill at rest, a stronger
/// one on hover, and the accent tint while `selected`. Unlike
/// [`section_button_class`], which is transparent when idle because it is
/// navigation, this one is always visible — a tool should look pressable
/// before it is focused.
pub fn control_button_class(selected: bool) -> Button {
    let styled = move |focused: bool, theme: &Theme, hovered: bool| {
        let mut style = theme.active(focused, false, &Button::IconVertical);
        style.border_radius = theme.cosmic().corner_radii.radius_m.into();
        style.background = Some(if selected {
            let mut color: Color = theme.cosmic().accent.base.into();
            color.a = 0.22;
            Background::Color(color)
        } else {
            chip_background(if hovered { 0.16 } else { 0.10 }, theme)
        });
        if selected {
            style.text_color = Some(theme.cosmic().accent_text_color().into());
            style.icon_color = Some(theme.cosmic().accent_text_color().into());
        }
        focus_ring(style, focused, theme)
    };

    Button::Custom {
        active: Box::new(move |focused, theme| styled(focused, theme, false)),
        disabled: Box::new(|theme| theme.disabled(&Button::IconVertical)),
        hovered: Box::new(move |focused, theme| styled(focused, theme, true)),
        pressed: Box::new(move |focused, theme| styled(focused, theme, true)),
    }
}

/// Builds a Hearthdeck button class from a COSMIC role: the shared corner
/// radius, the focus ring, and the app's single hover rule.
///
/// `role` is a factory rather than a value because `cosmic::theme::Button` is
/// not `Clone` (it can hold closures), so each interaction arm needs its own
/// value. `selected` carries the role's selected state through to COSMIC.
///
/// Hover draws the accent ring instead of the role's hover fill, so the pointer
/// gets the same affordance as keyboard/controller focus and no button paints a
/// box over its content. This is the one place to change hover for the app.
pub fn button_class(role: fn() -> Button, selected: bool) -> Button {
    Button::Custom {
        active: Box::new(move |focused, theme| {
            let mut style = theme.active(focused, selected, &role());
            style.border_radius = theme.cosmic().corner_radii.radius_m.into();
            focus_ring(style, focused, theme)
        }),
        disabled: Box::new(move |theme| {
            let mut style = theme.disabled(&role());
            style.border_radius = theme.cosmic().corner_radii.radius_m.into();
            style
        }),
        hovered: Box::new(move |focused, theme| {
            let mut style = theme.hovered(focused, selected, &role());
            style.background = None;
            style.border_radius = theme.cosmic().corner_radii.radius_m.into();
            // Ring on hover regardless of focus, so the pointer reads the same
            // as the controller.
            focus_ring(style, true, theme)
        }),
        pressed: Box::new(move |focused, theme| {
            let mut style = theme.pressed(focused, selected, &role());
            style.border_radius = theme.cosmic().corner_radii.radius_m.into();
            focus_ring(style, focused, theme)
        }),
    }
}

/// An icon-only button for a toolbar or list row. Transparent at rest so a row
/// of them stays quiet.
pub fn icon_button_class() -> Button {
    button_class(|| Button::Icon, false)
}

/// The action that removes something: the theme's destructive preset.
pub fn destructive_button_class() -> Button {
    button_class(|| Button::Destructive, false)
}

/// An ordinary secondary action (a settings toggle, a reset).
pub fn standard_button_class() -> Button {
    button_class(|| Button::Standard, false)
}

/// A bare-text control - a value you click to change (the filter drawer's
/// current value). Transparent at rest like [`tab_button_class`].
pub fn text_button_class() -> Button {
    button_class(|| Button::Text, false)
}

/// A subtle neutral fill derived from the theme's on-color so it stays visible
/// on both light and dark themes.
fn chip_background(alpha: f32, theme: &Theme) -> Background {
    let mut color: Color = theme.cosmic().on_bg_color().into();
    color.a = alpha;
    Background::Color(color)
}

/// Applies the shared focus ring to a button style: a single solid accent
/// border.
pub(crate) fn focus_ring(mut style: button::Style, focused: bool, theme: &Theme) -> button::Style {
    if focused {
        style.border_width = FOCUS_RING_WIDTH;
        style.border_color = theme.cosmic().accent.base.into();
        style.outline_width = 0.0;
        style.outline_color = Color::TRANSPARENT;
    }
    style
}

/// Compact icon buttons used by a top-level destination's centered navigation.
pub fn nav_button_class(selected: bool) -> Button {
    button_class(|| Button::Icon, selected)
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

/// One facet row in the console filter sidebar: a full-width row that fills
/// with the selection color once its facet is narrowed, and rings while the
/// controller cursor is on it.
///
/// The arrows and value inside are ordinary buttons; this only paints the row
/// they sit in, so the whole row reads as one control rather than as three
/// loose widgets - the compact, list-row shape a large option set needs.
pub fn filter_row(selected: bool, cursor: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let alpha = match (selected, cursor) {
            (true, _) => 0.18,
            (false, true) => 0.08,
            (false, false) => 0.0,
        };
        container::Style {
            text_color: Some(theme.cosmic().on_bg_color().into()),
            icon_color: Some(theme.cosmic().on_bg_color().into()),
            background: (alpha > 0.0).then(|| chip_background(alpha, theme)),
            border: Border {
                radius: theme.cosmic().radius_m().into(),
                width: if cursor { FOCUS_RING_WIDTH } else { 0.0 },
                color: if cursor {
                    theme.cosmic().accent.base.into()
                } else {
                    Color::TRANSPARENT
                },
            },
            ..container::Style::default()
        }
    }
}

/// Grid tile buttons: the shared surface radius and focus ring, with the app's
/// hover ring so a hovered tile reads like a focused one.
pub fn tile_button_class(selected: bool) -> Button {
    button_class(|| Button::IconVertical, selected)
}

// ---------------------------------------------------------------------------
// Details screen actions
// ---------------------------------------------------------------------------

/// The primary action of a details screen: filled with the theme's accent, so
/// the button that starts the game is unmistakable at TV distance.
///
/// Built on the theme's `Button::Suggested` component rather than pairing
/// `accent.base` with `accent.on` by hand. That pair is not a fill and its
/// text: `accent.on` is a neutral control step, while the suggested button's
/// own `on` color is derived from the accent fill's contrast (`get_text` in
/// `cosmic-theme`). Using the former painted the label white (or black) with no
/// relationship to the button underneath it, so on a light accent the text
/// became unreadable. The suggested component is the one place the theme
/// guarantees the fill and its label agree, so we take both from it and only
/// add the shared radius and focus ring.
pub fn primary_action_button_class() -> Button {
    let styled = |focused: bool, theme: &Theme| {
        let mut style = theme.active(focused, false, &Button::Suggested);
        style.border_radius = surface_radius(theme).into();
        // Ring in the label's own color: the theme guarantees it contrasts with
        // the fill, and an accent ring on an accent fill would be invisible.
        style.outline_width = FOCUS_RING_WIDTH;
        style.outline_color = if focused {
            style
                .text_color
                .unwrap_or_else(|| theme.cosmic().accent_text_color().into())
        } else {
            Color::TRANSPARENT
        };
        style
    };

    Button::Custom {
        active: Box::new(styled),
        disabled: Box::new(|theme| theme.disabled(&Button::Suggested)),
        hovered: Box::new(styled),
        pressed: Box::new(styled),
    }
}

/// A details action that holds a state (favorite, play later): the chip fill is
/// always present so the secondary buttons read as one strip of controls, and
/// the accent colors appear only while the state is on.
pub fn details_toggle_button_class(active: bool) -> Button {
    let styled = move |focused: bool, theme: &Theme, hovered: bool| {
        let mut style = theme.active(focused, false, &Button::IconVertical);
        style.border_radius = theme.cosmic().corner_radii.radius_m.into();
        let fill = match (active, hovered) {
            (true, _) => 0.26,
            (false, true) => 0.18,
            (false, false) => 0.10,
        };
        style.background = Some(chip_background(fill, theme));
        if active {
            let accent = theme.cosmic().accent_text_color().into();
            style.text_color = Some(accent);
            style.icon_color = Some(accent);
        }
        focus_ring(style, focused, theme)
    };

    Button::Custom {
        active: Box::new(move |focused, theme| styled(focused, theme, false)),
        disabled: Box::new(|theme| theme.disabled(&Button::IconVertical)),
        hovered: Box::new(move |focused, theme| styled(focused, theme, true)),
        pressed: Box::new(move |focused, theme| styled(focused, theme, true)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CONSOLE_GRID_COLUMNS, DASHBOARD_GAME_ASPECT, DASHBOARD_VISIBLE_TILES, GRID_COLUMNS,
        content_horizontal_padding, content_width, dashboard_tile_size, filter_button_height,
        grid_columns, grid_gap, grid_tile_height, search_icon_padding, sidebar_accent_bar_height,
        sidebar_header_height, sidebar_item_height, sidebar_width, tab_height,
        tab_underline_height, tile_width,
    };

    #[test]
    fn theme_metrics_match_standard_density() {
        // The test process uses libcosmic's default (Standard) density, so the
        // theme-derived sizes must reproduce the pixel values they replaced.
        assert_eq!(sidebar_header_height(), 80.0);
        assert_eq!(sidebar_item_height(), 56.0);
        assert_eq!(sidebar_accent_bar_height(), 40.0);
        assert_eq!(filter_button_height(), 48.0);
        assert_eq!(tab_height(), 48.0);
        assert_eq!(tab_underline_height(), 4.0);
        assert_eq!(content_horizontal_padding(), 24);
        assert_eq!(search_icon_padding(), 4);
    }

    #[test]
    fn grid_fits_inside_padded_content() {
        let window_width = 1200.0;
        let columns = GRID_COLUMNS;
        let occupied = columns as f32 * tile_width(window_width, columns)
            + (columns - 1) as f32 * grid_gap(window_width, columns);

        assert!((occupied - content_width(window_width)).abs() < 0.01);
        assert!(
            occupied + sidebar_width(window_width) + 2.0 * f32::from(content_horizontal_padding())
                <= window_width
        );
    }

    #[test]
    fn console_grid_is_denser_than_the_default() {
        let window_width = 1200.0;
        assert_eq!(grid_columns(false), GRID_COLUMNS);
        assert_eq!(grid_columns(true), CONSOLE_GRID_COLUMNS);
        // Six columns of the same content width leave each cover about 30%
        // smaller than four columns, which is the console grid's target size.
        let ratio =
            tile_width(window_width, CONSOLE_GRID_COLUMNS) / tile_width(window_width, GRID_COLUMNS);
        assert!((0.62..0.72).contains(&ratio), "ratio was {ratio}");
    }

    #[test]
    fn dashboard_tiles_are_thirty_percent_smaller_than_the_row_divides() {
        let (window_width, padding, gap) = (1200.0, 32, 32);
        let tile = dashboard_tile_size(window_width, padding, gap);
        let divided = (window_width
            - 2.0 * f32::from(padding)
            - (DASHBOARD_VISIBLE_TILES - 1) as f32 * f32::from(gap))
            / DASHBOARD_VISIBLE_TILES as f32;

        assert!((tile - divided * 0.7).abs() < 0.01, "tile was {tile}");
    }

    #[test]
    fn a_library_tile_is_the_dashboards_portrait_card() {
        let width = tile_width(1200.0, GRID_COLUMNS);

        assert!(
            grid_tile_height(width) > width,
            "a horizontal box only fits a card taller than it is wide"
        );
        assert!(
            (grid_tile_height(width) - width / DASHBOARD_GAME_ASPECT).abs() < f32::EPSILON,
            "the library tile wears the dashboard card's proportion"
        );
    }
}
