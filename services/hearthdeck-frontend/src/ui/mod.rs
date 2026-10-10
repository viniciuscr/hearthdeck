//! HearthDeck's UI kit: the shared, styled widgets every screen composes from.
//!
//! This grows from real usage. A component lands here on its second use, or when
//! a pattern repeats inline across screens — never as a per-screen copy. That is
//! how the UI drifts, and drift is what this module exists to stop.
//!
//! Composition only: colors, radii and sizes come from [`crate::style`], and
//! the base button classes live there too. This module is the layer above them.

use std::sync::LazyLock;

use cosmic::widget::Icon;

/// The application's own mark, drawn in the sidebar header and on the launch
/// overlay. Embedded rather than themed: it is the app's identity, not a
/// symbolic icon that should follow the palette.
static APP_ICON: LazyLock<cosmic::widget::icon::Handle> = LazyLock::new(|| {
    cosmic::widget::icon::from_svg_bytes(include_bytes!(
        "../../data/icons/org.hearthdeck.HearthDeck.svg"
    ))
});

/// An icon by name at `size` — the single way the app draws a themed icon.
///
/// The underlying call, `icon::icon(icon::from_name(name).into()).size(..)`,
/// was repeated verbatim at every call site; this is that, once.
pub fn icon(name: impl Into<std::sync::Arc<str>>, size: u16) -> Icon {
    cosmic::widget::icon::icon(cosmic::widget::icon::from_name(name).into()).size(size)
}

/// The application's own icon.
///
/// Handed out as a clone of the shared handle, so every caller draws the one
/// embedded copy rather than re-decoding it.
pub fn app_icon() -> cosmic::widget::icon::Handle {
    APP_ICON.clone()
}
