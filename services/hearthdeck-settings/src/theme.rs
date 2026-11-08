//! The Hearthdeck palette for this window.
//!
//! The kit ships a stock dark theme in which a card's `group_box` is the same
//! `#0a0a0a` as the page, so cards have no surface at all and a page reads as one
//! long list divided by hairlines. `docs/appearance-system.md` says surfaces
//! lighten as they stack; this module makes that true for the one window that
//! uses the kit's palette.
//!
//! The values are Noir. They are applied as colours, never as raw hex in the
//! view: every row still asks the theme for its tokens, so a future palette
//! change lands here and nowhere else.

use gpui::{App, Hsla, rgb};
use gpui_component::{Theme, ThemeMode};

/// The page itself.
const CANVAS: u32 = 0x0a0a0a;
/// A card: one step above the page, or cards are invisible.
const SURFACE: u32 = 0x171717;
/// The page navigation rail: darker than the page, so the content leads.
const SIDEBAR: u32 = 0x111111;
const BORDER: u32 = 0x2e2e2e;
const TEXT: u32 = 0xfafafa;
const MUTED: u32 = 0xa3a3a3;
/// Focus and the active item: blue at ≈3.4:1 against the page, which is the
/// minimum a focus indicator may have.
const FOCUS: u32 = 0x3b82f6;

pub fn install(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);

    theme.colors.background = color(CANVAS);
    theme.colors.foreground = color(TEXT);
    theme.colors.muted_foreground = color(MUTED);

    // The fix the design review asked for: a card that is a surface.
    theme.colors.group_box = color(SURFACE);
    theme.colors.group_box_foreground = color(TEXT);
    theme.colors.border = color(BORDER);

    theme.colors.sidebar = color(SIDEBAR);
    theme.colors.sidebar_foreground = color(MUTED);
    theme.colors.sidebar_accent = alpha(FOCUS, 0x24);
    theme.colors.sidebar_accent_foreground = color(TEXT);

    // Focus: fill, border and ring all agree, so a focused row is one blue thing
    // and not three different greys.
    theme.colors.ring = color(FOCUS);
    theme.colors.list_active = alpha(FOCUS, 0x1f);
    theme.colors.list_active_border = color(FOCUS);
    theme.colors.list_hover = alpha(FOCUS, 0x14);
    theme.colors.accent = color(FOCUS);
}

fn color(value: u32) -> Hsla {
    rgb(value).into()
}

fn alpha(value: u32, alpha: u8) -> Hsla {
    let mut color: Hsla = rgb(value).into();
    color.a = f32::from(alpha) / 255.0;
    color
}
