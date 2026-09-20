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
// Dialogs & menus
// ---------------------------------------------------------------------------

/// Width of the new-group / delete-group dialogs and their text inputs.
pub const DIALOG_WIDTH: f32 = 432.0;
/// Width of a dialog action button.
pub const DIALOG_ACTION_WIDTH: u16 = 142;
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

/// Renders an icon as artwork clipped to the shared surface radius, with an
/// explicit `ContentFit`. Game covers want [`ContentFit::Cover`] while logos
/// and other non-cover art want [`ContentFit::Contain`].
///
/// Every raster image the app displays goes through here, so rounded corners
/// are a design-system primitive rather than a per-widget decision. Entry
/// icons are always raster by this point (the icon cache rasterizes SVGs); an
/// SVG handle falls back to the plain icon, which has no clip.
pub fn artwork_fit<'a, M: 'a>(
    handle: &icon::Handle,
    fit: ContentFit,
    width: Length,
    height: Length,
) -> Element<'a, M> {
    match &handle.data {
        icon::Data::Image(image) => cosmic::widget::image::Image::new(image.clone())
            .content_fit(fit)
            .border_radius(active_surface_radius())
            .width(width)
            .height(height)
            .into(),
        icon::Data::Svg(_) => handle.clone().icon().width(width).height(height).into(),
    }
}

/// Artwork scaled to fill its bounds without distortion, for cover art that is
/// expected to be cropped to the surface.
pub fn artwork<'a, M: 'a>(handle: &icon::Handle, width: Length, height: Length) -> Element<'a, M> {
    artwork_fit(handle, ContentFit::Cover, width, height)
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

/// Background for the source badge overlaid on tile artwork. A card-like
/// surface, so it shares the same radius and colors as the tile instead of
/// borrowing a different preset.
pub fn source_badge(theme: &Theme) -> container::Style {
    card_surface(theme)
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

// ---------------------------------------------------------------------------
// Details screen actions
// ---------------------------------------------------------------------------

/// The primary action of a details screen: filled with the theme's accent, so
/// the button that starts the game is unmistakable at TV distance. The focus
/// ring uses the theme's on-accent color, because an accent ring on an accent
/// fill would be invisible.
pub fn primary_action_button_class() -> Button {
    let styled = |focused: bool, theme: &Theme| {
        let t = theme.cosmic();
        let mut style = theme.active(focused, false, &Button::Suggested);
        style.background = Some(Background::Color(t.accent.base.into()));
        style.text_color = Some(t.accent.on.into());
        style.icon_color = Some(t.accent.on.into());
        style.border_radius = surface_radius(theme).into();
        style.outline_width = FOCUS_RING_WIDTH;
        style.outline_color = if focused {
            t.accent.on.into()
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
