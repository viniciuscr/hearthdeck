//! An application tile: cover art, a label, and source/version badges.

mod style;

use crate::style::{ICON_SMALL, TEXT_CAPTION, tile_button_class};
use cosmic::Element;
use cosmic::iced::Alignment;
use cosmic::iced::Length;
use cosmic::iced::alignment::Vertical;
use cosmic::iced::core::alignment::Horizontal;
use cosmic::iced::widget::{row, stack, text};
use cosmic::widget::{self, button, container, icon, mouse_area};
use std::rc::Rc;

use style::{SOURCE_BADGE, TEXT_TILE_LABEL, artwork_contained, source_badge, tile_label_overlay};

/// Whether a tile is the context menu's target. An enum rather than a `bool` so
/// the call site reads what it means without needing a comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// The tile the context menu is open on: drawn in the active tile style.
    Selected,
    /// Every other tile.
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
/// through a [`mouse_area`] wrapper.
// The ten arguments are one tile's identity, geometry, and the two messages its
// interactions send; they are all supplied inline at the single call site, so a
// spec struct would only move the same list somewhere else.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn app_tile<'a, Message: Clone + 'a>(
    widget_id: widget::Id,
    name: &str,
    icon_handle: icon::Handle,
    tile_width: f32,
    tile_height: f32,
    // Number of selectable versions of this entry (1 for a single file).
    // More than one overlays a disc badge on the artwork so a collapsed
    // multi-disc game is still visibly distinct from a single-file one.
    version_count: usize,
    on_right_release: Message,
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

    // The cover is fitted, not cropped: some box art is not the tile's 2:3
    // shape, and `Cover` cut its title art off, so a fitted cover still reads as
    // a tile of a consistent size next to its neighbours. The bands a `Contain`
    // fit leaves on the two sides show the page through.
    let mut artwork_layer: Element<'a, Message> =
        container(artwork_contained(&icon_handle, Length::Fill, Length::Fill))
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

    artwork_layer = stack![artwork_layer, label_layer].into();

    let is_selected = selection.is_selected();
    let tile_button = button::custom(artwork_layer)
        .id(widget_id)
        .selected(is_selected)
        .width(Length::Fixed(tile_width))
        .height(Length::Fixed(tile_height))
        .class(tile_button_class(is_selected))
        .padding(0)
        .on_press_maybe(on_pressed);

    mouse_area(tile_button)
        .on_right_release(on_right_release)
        .into()
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
                version_count,
                Msg::Open,
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
}
