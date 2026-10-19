//! The bar across the top of a top-level destination.
//!
//! The dashboard and the settings screen are siblings, and they wear the same
//! chrome: who is signed in on the left, the destinations centered, and the
//! system status on the right. It lives here rather than in either screen so the
//! two cannot drift apart - and so the next thing added to it is added once.
//!
//! The bar is also the one row of controls both pages share with the pad and the
//! keyboard, so this module owns the shape of that walk - which button a step
//! lands on, and where the ring enters and leaves. The shell owns *when* the bar
//! holds the cursor (see `App::bar_cursor`); what a step does once it does is
//! decided here, in functions a test can drive without a compositor.

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::core::alignment::Horizontal;
use cosmic::iced::{Alignment, Length};
use cosmic::theme;
use cosmic::widget::{Id, button, column, container, icon, row, text, tooltip};

use crate::app::Message;
use crate::fl;
use crate::style::{ICON_BODY, ICON_LARGE, TEXT_BODY, TEXT_CAPTION, TEXT_HEADER, nav_button_class};
use crate::system_status::SystemStatus;

/// A destination the top bar opens.
///
/// Every screen the app has, plus the search box's own way in. The screen
/// drawing the bar is the one that reads as current; search is a way *into* the
/// library rather than a screen, so it never does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Dashboard,
    Library,
    Search,
    Settings,
}

impl Destination {
    /// All of them, left to right, which is also the order they are drawn.
    const ALL: [Self; 4] = [Self::Dashboard, Self::Library, Self::Search, Self::Settings];

    /// The icon a button for this destination wears.
    fn icon_name(self) -> &'static str {
        match self {
            Self::Dashboard => "go-home-symbolic",
            Self::Library => "view-grid-symbolic",
            Self::Search => "system-search-symbolic",
            Self::Settings => "preferences-system-symbolic",
        }
    }

    /// What the button's tooltip calls it, which is also what the button says
    /// about itself to anyone who cannot see the icon.
    fn label(self) -> String {
        match self {
            Self::Dashboard => fl!("top-bar-home"),
            Self::Library => fl!("top-bar-library"),
            Self::Search => fl!("top-bar-search"),
            Self::Settings => fl!("top-bar-settings"),
        }
    }

    /// The name this destination's button carries in a widget id.
    fn id_name(self) -> &'static str {
        match self {
            Self::Dashboard => "home",
            Self::Library => "library",
            Self::Search => "search",
            Self::Settings => "settings",
        }
    }

    /// Where this destination's button sits in the bar, which is also the index
    /// the cursor holds while it is on that button.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Dashboard => 0,
            Self::Library => 1,
            Self::Search => 2,
            Self::Settings => 3,
        }
    }

    /// The destination whose button sits at `index`, or `None` past the bar's
    /// end - which is what a cursor read has to tolerate, the same way every
    /// other cursor here does.
    #[must_use]
    pub fn at(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
}

/// The widget id of `destination`'s button on the bar drawn for `current`,
/// which is what pointing the renderer's focus at it addresses - and so what
/// paints the focus ring on it.
///
/// Scoped by the bar's own destination, because a page change fades two pages
/// at once and both wear a bar. A widget id is a global name, and one tree
/// carries one node per name, so buttons that answered to the same name would
/// leave every one but the last without a state to draw.
#[must_use]
pub fn button_id(current: Destination, destination: Destination) -> Id {
    Id::new(format!(
        "top-bar-{}-{}",
        current.id_name(),
        destination.id_name()
    ))
}

/// Where a sideways step from the button at `index` lands: one along the bar,
/// bounded by its ends, or `None` when the step would not move - the ends of
/// the bar are ends, the way a rail's are.
#[must_use]
pub fn stepped(index: usize, delta: i32) -> Option<usize> {
    let last = Destination::ALL.len() as i32 - 1;
    let from = (index as i32).clamp(0, last);
    let to = (from + delta).clamp(0, last);
    (to != from).then_some(to as usize)
}

/// The keyboard's ring over the bar's buttons: it enters at either end, steps
/// one stop, and leaves past either end - `None` being the content holding the
/// cursor again, which is where it started.
#[must_use]
pub fn ring_step(ring: Option<usize>, backwards: bool) -> Option<usize> {
    let last = Destination::ALL.len() - 1;
    let Some(index) = ring else {
        return Some(if backwards { last } else { 0 });
    };
    let index = index.min(last);
    if backwards {
        return index.checked_sub(1);
    }
    (index < last).then_some(index + 1)
}

/// The bar's contents.
pub struct TopBar<'a> {
    /// The destination showing, drawn as the current button.
    pub current: Destination,
    pub user_name: String,
    /// The app logo, drawn beside the name.
    pub app_icon: icon::Handle,
    /// Wi-Fi and Bluetooth, as the last poll read them. Borrowed because the
    /// bar is redrawn far more often than the status changes.
    pub status: &'a SystemStatus,
}

/// The bar, with `on_navigate` called for whichever destination is pressed.
#[must_use]
pub fn view<'a>(bar: &TopBar, on_navigate: fn(Destination) -> Message) -> Element<'a, Message> {
    let Spacing {
        space_xxs,
        space_xs,
        space_s,
        space_l,
        space_xl,
        space_xxl,
        ..
    } = theme::spacing();
    let button_size = f32::from(space_xl);

    let profile = row![
        icon::icon(bar.app_icon.clone()).size(ICON_LARGE),
        column![
            text::body(bar.user_name.clone()).size(TEXT_HEADER),
            text::caption("Hearthdeck").size(TEXT_CAPTION),
        ]
        .spacing(space_xxs),
    ]
    .spacing(space_s)
    .align_y(Alignment::Center)
    .width(Length::FillPortion(1));

    // One builder for every destination, so the buttons cannot drift apart.
    // Each wears a widget id, so the cursor can be pointed at it, and its name
    // as a tooltip below it, so the button says what it is.
    let nav = |destination: Destination| -> Element<'a, Message> {
        let button = button::custom(
            icon::icon(icon::from_name(destination.icon_name()).into()).size(ICON_BODY),
        )
        .id(button_id(bar.current, destination))
        .width(Length::Fixed(button_size))
        .height(Length::Fixed(button_size))
        .class(nav_button_class(destination == bar.current))
        .on_press(on_navigate(destination));
        tooltip(
            button,
            text::body(destination.label()).size(TEXT_BODY),
            tooltip::Position::Bottom,
        )
        .into()
    };

    let navigation = Destination::ALL.into_iter().fold(
        row![].spacing(space_xs).align_y(Alignment::Center),
        |navigation, destination| navigation.push(nav(destination)),
    );

    // The system status on the right: the time, then Wi-Fi and Bluetooth, each
    // with its state as a tooltip. A service that could not be read at all says
    // so with the same mark the old shell used, rather than disappearing.
    let status_icon = |icon_name: &'static str, label: String| -> Element<'a, Message> {
        tooltip(
            icon::icon(icon::from_name(icon_name).into()).size(ICON_BODY),
            text::body(label).size(TEXT_BODY),
            tooltip::Position::Bottom,
        )
        .into()
    };
    let wifi = bar.status.wifi.as_ref().map_or_else(
        || {
            status_icon(
                "network-wireless-error-symbolic",
                "Wi-Fi unavailable".to_owned(),
            )
        },
        |wifi| status_icon(wifi.icon_name(), wifi.label().to_owned()),
    );
    let bluetooth = bar.status.bluetooth.as_ref().map_or_else(
        || {
            status_icon(
                "bluetooth-disabled-symbolic",
                "Bluetooth unavailable".to_owned(),
            )
        },
        |bluetooth| status_icon(bluetooth.icon_name(), bluetooth.label().to_owned()),
    );
    let status = row![
        text::body(crate::system_status::current_time()).size(TEXT_HEADER),
        wifi,
        bluetooth,
    ]
    .spacing(space_s)
    .align_y(Alignment::Center);

    row![
        profile,
        navigation,
        container(status)
            .width(Length::FillPortion(1))
            .align_x(Horizontal::Right),
    ]
    .spacing(space_l)
    .align_y(Alignment::Center)
    .height(Length::Fixed(f32::from(space_xxl)))
    .into()
}

/// The height the bar takes above a page, which is what that page's viewport is
/// the window's height less.
#[must_use]
pub fn height() -> f32 {
    let Spacing {
        space_l, space_xxl, ..
    } = theme::spacing();
    f32::from(space_l) + f32::from(space_xxl)
}

#[cfg(test)]
mod tests {
    use super::{Destination, button_id, ring_step, stepped};

    /// The bar's buttons in order, which is every index the cursor can hold.
    const LAST: usize = 3;

    /// A sideways step walks the bar and stops at its ends: the ends are ends,
    /// so a step off one does nothing rather than wrapping or leaving.
    #[test]
    fn a_sideways_step_walks_the_bar_and_stops_at_its_ends() {
        assert_eq!(stepped(0, 1), Some(1));
        assert_eq!(stepped(2, -1), Some(1));
        assert_eq!(stepped(0, -1), None);
        assert_eq!(stepped(LAST, 1), None);
        // A step onto itself is no move at all, wherever it starts from.
        assert_eq!(stepped(1, 0), None);
    }

    /// The ring enters at either end, keeps its direction, and leaves past the
    /// other - back to the content, which is where it began.
    #[test]
    fn the_ring_enters_at_either_end_and_leaves_past_the_other() {
        assert_eq!(ring_step(None, false), Some(0));
        assert_eq!(ring_step(None, true), Some(LAST));
        assert_eq!(ring_step(Some(0), false), Some(1));
        assert_eq!(ring_step(Some(1), true), Some(0));
        assert_eq!(ring_step(Some(LAST), false), None);
        assert_eq!(ring_step(Some(0), true), None);
    }

    /// Every destination is at a known place, has a name to say, and wears a
    /// button id no other destination answers to - so the cursor's index, the
    /// tooltip and the focus target cannot disagree or collide.
    #[test]
    fn every_destination_names_one_button_that_exists() {
        for (index, destination) in Destination::ALL.iter().enumerate() {
            assert_eq!(destination.index(), index);
            assert_eq!(Destination::at(index), Some(*destination));
            assert!(!destination.label().is_empty());
        }
        for current in Destination::ALL {
            for (position, first) in Destination::ALL.iter().enumerate() {
                for second in Destination::ALL.iter().skip(position + 1) {
                    assert_ne!(button_id(current, *first), button_id(current, *second));
                }
            }
        }
        assert_eq!(Destination::at(Destination::ALL.len()), None);
    }

    /// The two pages that wear the bar are both in the tree while a change
    /// fades, so neither page's buttons may answer to the other's names.
    #[test]
    fn the_two_pages_that_wear_the_bar_do_not_share_a_button_id() {
        for destination in Destination::ALL {
            assert_ne!(
                button_id(Destination::Dashboard, destination),
                button_id(Destination::Settings, destination),
                "destination: {destination:?}",
            );
        }
    }
}
