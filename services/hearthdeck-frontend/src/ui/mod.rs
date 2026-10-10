//! HearthDeck's UI kit: the shared, styled widgets every screen composes from.
//!
//! This grows from real usage. A component lands here on its second use, or when
//! a pattern repeats inline across screens — never as a per-screen copy. That is
//! how the UI drifts, and drift is what this module exists to stop.
//!
//! Composition only: colors, radii and sizes come from [`crate::style`], and
//! the base button classes live there too. This module is the layer above them.

use cosmic::widget::Icon;

/// An icon by name at `size` — the single way the app draws a themed icon.
///
/// The underlying call, `icon::icon(icon::from_name(name).into()).size(..)`,
/// was repeated verbatim at every call site; this is that, once.
pub fn icon(name: impl Into<std::sync::Arc<str>>, size: u16) -> Icon {
    cosmic::widget::icon::icon(cosmic::widget::icon::from_name(name).into()).size(size)
}
