//! The Hearthdeck palette for this window.
//!
//! The kit ships a stock dark theme in which a card's `group_box` is the same
//! `#0a0a0a` as the page, so cards have no surface at all and a page reads as one
//! long list divided by hairlines. `docs/appearance-system.md` says surfaces
//! lighten as they stack; this module makes that true for the one window that
//! uses the kit's palette.
//!
//! **Written through `Theme::update`, not `Theme::global_mut`.** The card is
//! painted from `theme.tokens.group_box`, which `global_mut` leaves stale: the
//! colour change would be invisible. `update` reconciles the tokens against the
//! colours it just edited, then re-syncs and repaints.
//!
//! The values are Noir, and their measured contrast against the page matters
//! more than the hex: surface 1.27:1, border 1.78:1, muted text 6.2:1 on a card,
//! focus 5.4:1, white on the primary button 5.2:1.

use gpui::{App, Hsla, rgb};
use gpui_component::{Theme, ThemeMode};

/// The page itself.
const CANVAS: u32 = 0x0a0a0a;
/// A card: one step above the page. `#171717` measures 1.10:1 against the page,
/// which is not a card at three metres; this is 1.27:1.
const SURFACE: u32 = 0x242424;
/// The page navigation rail: darker than the page, so the content leads.
const SIDEBAR: u32 = 0x111111;
const BORDER: u32 = 0x3d3d3d;
const TEXT: u32 = 0xfafafa;
const MUTED: u32 = 0xa3a3a3;
/// Focus, accents and the active item: blue at 5.4:1 against the page, well past
/// the 3:1 a focus indicator needs.
const FOCUS: u32 = 0x3b82f6;
/// A blue a button can carry white text on: 5.2:1.
const PRIMARY: u32 = 0x2563eb;

pub fn install(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        theme.colors.background = color(CANVAS);
        theme.colors.foreground = color(TEXT);
        theme.colors.muted_foreground = color(MUTED);

        // The fix the design review asked for: a card that is a surface.
        theme.colors.group_box = color(SURFACE);
        theme.colors.group_box_foreground = color(TEXT);
        theme.colors.border = color(BORDER);

        theme.colors.sidebar = color(SIDEBAR);
        theme.colors.sidebar_foreground = color(MUTED);
        theme.colors.sidebar_accent = alpha(FOCUS, 0x40);
        theme.colors.sidebar_accent_foreground = color(TEXT);

        // Focus: fill, border and ring all agree, so a focused row is one blue
        // thing and not three different greys.
        theme.colors.ring = color(FOCUS);
        theme.colors.list_active = alpha(FOCUS, 0x1f);
        theme.colors.list_active_border = color(FOCUS);
        theme.colors.list_hover = alpha(FOCUS, 0x14);

        // Anything that paints blue must also say what colour sits on it, or a
        // primary button and the switch track lose their legibility.
        theme.colors.accent = color(FOCUS);
        theme.colors.accent_foreground = color(TEXT);
        theme.colors.primary = color(PRIMARY);
        theme.colors.button_primary = color(PRIMARY);
        theme.colors.button_primary_foreground = color(TEXT);
        theme.colors.progress_bar = color(FOCUS);
    });
}

fn color(value: u32) -> Hsla {
    rgb(value).into()
}

fn alpha(value: u32, alpha: u8) -> Hsla {
    let mut color: Hsla = rgb(value).into();
    color.a = f32::from(alpha) / 255.0;
    color
}

#[cfg(test)]
mod tests {
    use super::{CANVAS, SURFACE, color};

    #[test]
    fn the_card_is_a_surface_and_not_the_page() {
        assert_ne!(SURFACE, CANVAS, "a card must be a step above the page");
        assert_ne!(color(SURFACE), color(CANVAS));
    }
}
