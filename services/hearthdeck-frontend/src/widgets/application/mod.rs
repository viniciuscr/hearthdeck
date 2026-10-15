//! An application tile: cover art, a label, and source/version badges.
//!
//! The label is not permanent: it shows while the tile is picked out or under
//! the pointer, the same moments its border lights up.

mod reveal;
mod style;

use crate::style::{ICON_SMALL, TEXT_CAPTION, artwork_fit, tile_button_class};
use cosmic::Element;
use cosmic::iced::Alignment;
use cosmic::iced::ContentFit;
use cosmic::iced::Length;
use cosmic::iced::alignment::Vertical;
use cosmic::iced::core::alignment::Horizontal;
use cosmic::iced::widget::{row, stack, text};
use cosmic::widget::{self, button, container, icon, mouse_area};
use std::rc::Rc;

use reveal::Reveal;
use style::{SOURCE_BADGE, TEXT_TILE_LABEL, source_badge, tile_label_overlay};

/// Whether a tile is picked out: the context menu's target, or the card the
/// cursor ring is on. An enum rather than a `bool` so the call site reads what
/// it means without needing a comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// A picked-out tile: drawn in the active tile style, and its label shown
    /// the way the border is - always, not only under the pointer.
    Selected,
    /// Every other tile: its label appears only while the pointer is on it,
    /// the same span the border covers when a tile is hovered.
    Unselected,
}

impl Selection {
    /// Whether the tile is drawn in its active style.
    #[must_use]
    pub fn is_selected(self) -> bool {
        matches!(self, Self::Selected)
    }
}

/// A duplicate's source, drawn as a corner badge and appended to the tile label.
///
/// A label and an icon rather than a source *type*, because the tile is shared
/// between the two apps and they do not answer "where did this come from" the
/// same way: the legacy app guesses it from the launcher's path, while the
/// library grid is handed the origin the bridge resolved, carried as a
/// `hearthdeck-source:` category. Taking the rendered pieces keeps the shared
/// widget from having to know which of those it is talking to.
pub struct SourceBadge<'a> {
    /// How the source reads in the tile label, e.g. "Flatpak".
    pub label: &'a str,
    /// The icon drawn in the tile's corner, when the source has one.
    pub icon: Option<icon::Handle>,
}

/// Builds the tile for one application: a selectable button carrying the cover
/// art, its name, and the source and version badges, with right-click opened
/// through a [`mouse_area`] wrapper when the caller has a context menu to open.
///
/// One tile for every surface that shows an application or a media item - the
/// library grid, the dashboard's game and movie rails, and the legacy app's
/// shelves - so a card looks and focuses the same wherever it is drawn. The
/// surfaces differ only in geometry ([`ContentFit`], and the fixed
/// `tile_width`/`tile_height` they pass), whether they have a context menu
/// (`on_right_release`), and whether the card is picked out ([`Selection`]),
/// which decides the active style and whether its label waits for the
/// pointer.
// The eleven arguments are one tile's identity, geometry, fit, and the messages
// and markers its interactions use; they are all supplied inline at the call
// sites, so a spec struct would only move the same list somewhere else.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn app_tile<'a, Message: Clone + 'a>(
    widget_id: widget::Id,
    name: &str,
    icon_handle: icon::Handle,
    tile_width: f32,
    tile_height: f32,
    // How the artwork fills the tile. Grid tiles contain box art so it is not
    // cropped; media rails cover a poster whose shape already matches.
    fit: ContentFit,
    // Number of selectable versions of this entry (1 for a single file).
    // More than one overlays a disc badge on the artwork so a collapsed
    // multi-disc game is still visibly distinct from a single-file one.
    version_count: usize,
    // The message a right-click sends, or `None` for a surface with no context
    // menu (the dashboard's rails).
    on_right_release: Option<Message>,
    on_pressed: Option<Message>,
    source: Option<SourceBadge<'_>>,
    selection: Selection,
) -> Element<'a, Message> {
    // A duplicate's source, shown as an icon badge on the artwork corner. The
    // name is also appended to the label, so the badge carries only the icon.
    let source_badge_element: Option<Element<'a, Message>> = source
        .as_ref()
        .and_then(|badge| badge.icon.as_ref())
        .map(|handle| {
            container(app_source_icon(handle.clone()))
                .class(cosmic::theme::Container::Custom(Box::new(source_badge)))
                .width(Length::Fixed(SOURCE_BADGE))
                .height(Length::Fixed(SOURCE_BADGE))
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .into()
        });
    let name = tile_label(name, source.as_ref().map(|badge| badge.label));

    // The cover is fitted by the caller: a grid tile contains box art so an
    // odd-shaped cover is not cropped, and a media rail covers a poster whose
    // shape already matches the card. A `Contain` fit bands the artwork and the
    // page shows through the bands.
    let mut artwork_layer: Element<'a, Message> =
        container(artwork_fit(&icon_handle, fit, Length::Fill, Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into();

    // The badges sit in the top corners as layers over the cover, so adding one
    // never moves the artwork: the disc count on the right, the duplicate's
    // source on the left (the label scrim owns the bottom edge).
    if version_count > 1 {
        let disc_badge: Element<'a, Message> = container(
            container(
                row![
                    icon::icon(
                        icon::from_name("media-optical-symbolic")
                            .size(ICON_SMALL)
                            .into()
                    ),
                    text(version_count.to_string()).size(TEXT_CAPTION),
                ]
                .spacing(2)
                .align_y(Alignment::Center),
            )
            .class(cosmic::theme::Container::Custom(Box::new(source_badge)))
            .padding([2, 6]),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Right)
        .align_y(Vertical::Top)
        .padding([6, 6])
        .into();

        artwork_layer = stack![artwork_layer, disc_badge].into();
    }

    if let Some(badge) = source_badge_element {
        let badge_layer: Element<'a, Message> = container(badge)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Left)
            .align_y(Vertical::Top)
            .padding([6, 6])
            .into();

        artwork_layer = stack![artwork_layer, badge_layer].into();
    }

    // The label rides on the bottom of the artwork as a translucent scrim rather
    // than taking a strip below it, so the cover still fills the whole tile.
    let label_layer: Element<'a, Message> = container(
        container(text(name).size(TEXT_TILE_LABEL).width(Length::Fill))
            .padding([2, 6])
            .width(Length::Fill)
            .height(Length::Shrink)
            .class(cosmic::theme::Container::Custom(Box::new(
                tile_label_overlay,
            ))),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Horizontal::Center)
    .align_y(Vertical::Bottom)
    .into();

    let is_selected = selection.is_selected();

    // The label waits for the tile to be picked out or pointed at - the same
    // moments the accent border lights up - so the name appears with the
    // border and the artwork stands alone otherwise. The layer keeps its
    // place in the layout either way; only the drawing is gated.
    let label_layer: Element<'a, Message> = Reveal::new(label_layer, is_selected).into();

    artwork_layer = stack![artwork_layer, label_layer].into();

    let tile_button = button::custom(artwork_layer)
        .id(widget_id)
        .selected(is_selected)
        // A card with no press handler - an item the library does not hold - is
        // drawn by the button's disabled style, which never paints the focus
        // ring. Forcing it enabled *only while it is the cursor card* keeps the
        // two tiles' shared look: an inert card is dim at rest, but shows the
        // same accent ring under the cursor that an app tile does. It stays
        // inert either way, because `on_press_maybe` still gets `None`.
        .force_enabled(is_selected)
        .width(Length::Fixed(tile_width))
        .height(Length::Fixed(tile_height))
        .class(tile_button_class(is_selected))
        .padding(0)
        .on_press_maybe(on_pressed);

    match on_right_release {
        Some(message) => mouse_area(tile_button).on_right_release(message).into(),
        None => tile_button.into(),
    }
}

/// The label on a tile: the entry's name, truncated to fit the tile, with
/// the duplicate's source appended in parentheses when there is one.
fn tile_label(name: &str, source: Option<&str>) -> String {
    let source_suffix_len = source.map_or(0, |source| source.len() + 3); // 3 for " ()"
    let too_long = name.chars().count() > 34 - source_suffix_len;
    match (source, too_long) {
        (Some(source), true) => {
            let truncated: String = name.chars().take(22).collect();
            format!("{truncated}... ({source})")
        }
        (None, true) => {
            let truncated: String = name.chars().take(30).collect();
            format!("{truncated}...")
        }
        (Some(source), false) => format!("{name} ({source})"),
        (None, false) => name.to_string(),
    }
}

fn app_source_icon(handle: widget::icon::Handle) -> widget::Icon {
    let symbolic = handle.symbolic;
    handle.icon().size(ICON_SMALL).class(if symbolic {
        cosmic::theme::Svg::Custom(Rc::new(|t| {
            let color = t.cosmic().on_primary_component_color().into();
            widget::svg::Style { color: Some(color) }
        }))
    } else {
        cosmic::theme::Svg::Default
    })
}

#[cfg(test)]
mod tests {
    use super::{Selection, SourceBadge, app_tile, tile_label};
    use cosmic::iced::ContentFit;
    use cosmic::iced::{Length, Size};
    use cosmic::widget::{self, icon};

    #[derive(Clone, Debug)]
    enum Msg {
        Open,
        Activate,
    }

    fn icon_handle(size: u32) -> icon::Handle {
        icon::from_raster_pixels(size, size, vec![0u8; (size * size * 4) as usize])
    }

    #[test]
    fn a_short_name_is_shown_in_full() {
        assert_eq!(tile_label("Steam", None), "Steam");
    }

    #[test]
    fn a_name_at_the_limit_is_not_truncated() {
        let name = "A".repeat(34);
        assert_eq!(tile_label(&name, None), name);
    }

    #[test]
    fn a_long_name_is_truncated() {
        let name = "A".repeat(40);
        assert_eq!(tile_label(&name, None), format!("{}...", "A".repeat(30)));
    }

    #[test]
    fn a_name_with_a_source_keeps_the_source() {
        assert_eq!(tile_label("Steam", Some("Flatpak")), "Steam (Flatpak)");
    }

    #[test]
    fn a_long_name_with_a_source_keeps_the_source() {
        let name = "A".repeat(40);
        assert_eq!(
            tile_label(&name, Some("Flatpak")),
            format!("{}... (Flatpak)", "A".repeat(22))
        );
    }

    // A full paint needs a GPU renderer, which a headless test cannot build, so
    // the tile is exercised through the widget tree instead: it must build for
    // every badge combination and keep the fixed size the grid lays out around.
    #[test]
    fn a_tile_lays_out_at_its_fixed_size_for_every_badge_combination() {
        for (version_count, has_source) in [(1usize, false), (3, true)] {
            let source = has_source.then(|| SourceBadge {
                label: "Flatpak",
                icon: Some(icon_handle(16)),
            });
            let tile = app_tile(
                widget::Id::unique(),
                "Steam",
                icon_handle(64),
                240.0,
                300.0,
                ContentFit::Contain,
                version_count,
                Some(Msg::Open),
                Some(Msg::Activate),
                source,
                Selection::Selected,
            );

            assert_eq!(
                tile.as_widget().size(),
                Size::new(Length::Fixed(240.0), Length::Fixed(300.0)),
                "version_count={version_count} has_source={has_source}",
            );
        }
    }

    /// A rail card is the same tile as a grid tile, only at another shelf's
    /// fixed portrait size and with a cover fit. It has no right-click message
    /// (the rails have no context menu) and, for an item the library does not
    /// hold, no press message either - and it must still build at that fixed
    /// size rather than collapse or grow.
    #[test]
    fn a_rail_card_lays_out_at_its_fixed_size_without_a_press_or_right_click() {
        let (width, height) = (240.0, 240.0 * 1.5);
        let tile = app_tile::<Msg>(
            widget::Id::unique(),
            "Blade Runner 2049",
            icon_handle(64),
            width,
            height,
            ContentFit::Cover,
            1,
            None,
            None,
            None,
            Selection::Selected,
        );

        assert_eq!(
            tile.as_widget().size(),
            Size::new(Length::Fixed(width), Length::Fixed(height)),
        );
    }
}
