//! The window's background behind the dashboard: the focused title's wide art,
//! cross-faded as the pad walks a card at a time, with the desktop wallpaper
//! showing through wherever the pad is on a card whose feed sent no art.
//!
//! What the background shows is a pure function of where the pad is and how far
//! the fade out of the card it left has come - so it is decided, and tested,
//! without a compositor.

use cosmic::Element;
use cosmic::iced::ContentFit;
use cosmic::iced::Length;
use cosmic::iced::widget::stack;

use crate::app::Message;
use crate::screens::dashboard::Shelf;
use crate::style::backdrop_faded;

/// What the background is showing: the card whose art fills the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct At {
    pub rail: usize,
    pub card: usize,
}

/// The background's motion for one frame: the card it is on now, and - while a
/// fade is in flight - the card it was on and how far the fade has come.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Motion {
    pub at: At,
    /// The card being left, and the eased `0.0..=1.0` progress out of it.
    pub fading: Option<(At, f32)>,
}

impl Motion {
    /// The card being left, and the alpha its art draws at - `None` when no
    /// other card is fading out from under the one showing.
    #[must_use]
    pub fn leaving(self) -> Option<(At, f32)> {
        let (from, progress) = self.fading?;
        (from != self.at).then_some((from, 1.0 - progress))
    }

    /// The alpha the showing card's art draws at: full when no other card is
    /// fading out from under it.
    #[must_use]
    pub fn showing_alpha(self) -> f32 {
        self.fading.map_or(
            1.0,
            |(from, progress)| {
                if from == self.at { 1.0 } else { progress }
            },
        )
    }
}

/// The background art for this frame: the card the pad is on and the one it
/// left, cross-faded over whatever wallpaper the page draws behind them.
///
/// `None` when no art is in play at all - the pad is on cards whose feed sent
/// no wide art - so the wallpaper is drawn alone.
#[must_use]
pub fn view(shelves: &[Shelf], motion: Motion) -> Option<Element<'static, Message>> {
    let art = |at: At, alpha: f32| -> Option<Element<'static, Message>> {
        let card = shelves.get(at.rail)?.cards.get(at.card)?;
        let background = card.background.as_ref()?;
        Some(backdrop_faded(
            background,
            ContentFit::Cover,
            Length::Fill,
            Length::Fill,
            alpha,
        ))
    };
    let showing = art(motion.at, motion.showing_alpha());
    match (
        motion.leaving().and_then(|(from, alpha)| art(from, alpha)),
        showing,
    ) {
        // One card's art fading into the next, or into the wallpaper - which is
        // a card with no art fading out alone.
        (Some(leaving), Some(showing)) => Some(stack![leaving, showing].into()),
        (Some(leaving), None) => Some(leaving),
        (None, Some(showing)) => Some(showing),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{At, Motion, view};

    /// One card's art cross-fades into the next's, and a card whose feed sent
    /// no art fades the last one out into the wallpaper instead.
    #[test]
    fn the_background_cross_fades_between_the_card_it_was_on_and_the_one_it_is() {
        let here = At { rail: 1, card: 3 };
        let there = At { rail: 1, card: 2 };
        let moving = Motion {
            at: here,
            fading: Some((there, 0.25)),
        };
        assert_eq!(moving.leaving(), Some((there, 0.75)));
        assert_eq!(moving.showing_alpha(), 0.25);

        // A step onto the same card is no step at all.
        let still = Motion {
            at: here,
            fading: Some((here, 0.25)),
        };
        assert_eq!(still.leaving(), None);
        assert_eq!(still.showing_alpha(), 1.0);

        // And at rest nothing fades.
        let rest = Motion {
            at: here,
            fading: None,
        };
        assert_eq!(rest.leaving(), None);
        assert_eq!(rest.showing_alpha(), 1.0);
    }

    /// The background draws art for a card that has it, and nothing at all for
    /// one that has not - the wallpaper shows through untouched.
    #[test]
    fn a_card_without_wide_art_leaves_the_wallpaper_alone() {
        use cosmic::widget::Id;
        use cosmic::widget::icon;

        use crate::screens::dashboard::{Card, Shelf};

        let raster = || icon::from_raster_pixels(4, 4, vec![0u8; 64]);
        let art =
            Card::new(Id::new("art"), "Wicker", raster(), None).with_background(Some(raster()));
        let plain = Card::new(Id::new("plain"), "Hades", raster(), None);
        let shelf = Shelf::new(false, "New on Stremio", vec![art, plain]);

        let on_art = Motion {
            at: At { rail: 0, card: 0 },
            fading: None,
        };
        assert!(view(std::slice::from_ref(&shelf), on_art).is_some());

        let on_plain = Motion {
            at: At { rail: 0, card: 1 },
            fading: None,
        };
        assert!(view(std::slice::from_ref(&shelf), on_plain).is_none());

        // A card fading out keeps its art on screen until the fade is done.
        let leaving = Motion {
            at: At { rail: 0, card: 1 },
            fading: Some((At { rail: 0, card: 0 }, 0.5)),
        };
        assert!(view(std::slice::from_ref(&shelf), leaving).is_some());
    }
}
