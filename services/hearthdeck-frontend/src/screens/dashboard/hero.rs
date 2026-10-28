//! The dashboard's hero banner: what has just arrived, one slide at a time.
//!
//! The banner is a row of the dashboard like any rail — the pad walks its slides
//! with left and right, and up and down leave it for the rails below — but it
//! draws one slide at a time across the window's whole width instead of a line
//! of cards. What this module owns is that difference: the slide, its caption,
//! and the rules that keep the slide and the pad telling the same story.
//!
//! Those rules are the three functions below ([`stepped`], [`landed`],
//! [`rotate`]). They are pure, so how the banner moves is decided - and tested -
//! without a compositor.

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::alignment::Vertical;
use cosmic::iced::widget::{row, stack};
use cosmic::iced::{Background, Border, Color, ContentFit, Length};
use cosmic::theme::{self, Theme};
use cosmic::widget::{button, column, container, text};

use crate::app::Message;
use crate::screens::dashboard::Shelf;
use crate::screens::dashboard::focus::Cursor;
use crate::style::{
    TEXT_LARGE, TEXT_TITLE, WINDOW_HEIGHT, backdrop_faded, hero_height, tile_button_class,
};
use crate::widgets::application::Selection;

/// The wordmark's box on the banner: wide enough for a title in its own
/// lettering, short enough to leave the synopsis under it.
const LOGO_WIDTH: f32 = 420.0;
const LOGO_HEIGHT: f32 = 64.0;

/// The dot that marks which slide of how many is showing.
const INDICATOR_SIZE: f32 = 8.0;

/// How much of a line of synopsis a slide shows: one line, cut on a word. A
/// description long enough to wrap would push the title out of the banner.
const SYNOPSIS_CHARS: usize = 180;

/// Where the pad landed after a step, and the slide the banner shows.
///
/// Sideways along the banner moves the slide with the pad: left and right are
/// the slide list. Arriving on it from another row keeps the slide it is already
/// showing and brings the pad to that slide, because a slide that changed under
/// an arriving pad would move what the person was reading. A dashboard with no
/// banner is left exactly as it was.
///
/// `hero_len` is how many slides the banner has; `slide` is the slide showing.
#[must_use]
pub fn stepped(before: Cursor, cursor: Cursor, slide: usize, hero_len: usize) -> (Cursor, usize) {
    if hero_len == 0 || cursor.rail != 0 {
        return (cursor, slide);
    }
    if before.rail == 0 {
        (cursor, cursor.card)
    } else {
        landed(cursor, slide)
    }
}

/// The pad arriving on the banner from the row above it, which is the top bar
/// rather than a rail: it lands on the slide showing rather than on the card it
/// remembers, for the same reason [`stepped`] does.
#[must_use]
pub fn landed_from_above(cursor: Cursor, slide: usize, hero_len: usize) -> (Cursor, usize) {
    if hero_len == 0 {
        (cursor, slide)
    } else {
        landed(cursor, slide)
    }
}

/// The cursor put on the slide showing, remembering it for the way back.
fn landed(cursor: Cursor, slide: usize) -> (Cursor, usize) {
    (
        Cursor {
            card: slide,
            remembered: slide,
            ..cursor
        },
        slide,
    )
}

/// The slide the banner moves on to, wrapping round at the end.
///
/// A banner of one slide does not rotate, and no slides is no banner.
#[must_use]
pub fn rotate(slide: usize, hero_len: usize) -> usize {
    if hero_len == 0 {
        0
    } else {
        (slide + 1) % hero_len
    }
}

/// Whether the banner moves on by itself: it holds more than one slide and the
/// pad is not on it. This is the whole of "focus-aware" - the rotation pauses
/// the moment somebody is looking at it and resumes when they leave.
#[must_use]
pub fn rotates(hero_len: usize, cursor: Cursor) -> bool {
    hero_len > 1 && cursor.rail != 0
}

/// Where the banner is: the slide showing, and whether its art fills the
/// window's background.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct At {
    /// The slide showing.
    pub slide: usize,
    /// Its art fills the window behind the page.
    pub expanded: bool,
}

/// The banner's motion for one frame: where it is now, and - while a fade is in
/// flight - where it was and how far the fade out of there has come.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Motion {
    pub at: At,
    /// The state being left, and the eased `0.0..=1.0` progress out of it.
    pub fading: Option<(At, f32)>,
}

/// Whether the banner's art fills the window's background: the pad is on the
/// banner row. The same rule that pauses the rotation - a banner the pad is
/// looking at is the banner the window is looking at.
#[must_use]
pub fn expanded(cursor: Cursor, hero_len: usize) -> bool {
    hero_len > 0 && cursor.rail == 0
}

impl Motion {
    /// The slide being left, and the alpha it draws at - `None` when no other
    /// slide is fading out from under the one showing.
    #[must_use]
    pub fn leaving(self) -> Option<(usize, f32)> {
        let (from, progress) = self.fading?;
        (from.slide != self.at.slide).then_some((from.slide, 1.0 - progress))
    }

    /// The alpha the slide showing draws at: full when no other slide is fading
    /// out from under it.
    #[must_use]
    pub fn slide_alpha(self) -> f32 {
        self.fading.map_or(1.0, |(from, progress)| {
            if from.slide == self.at.slide {
                1.0
            } else {
                progress
            }
        })
    }

    /// The alpha the banner's art fills the background at: grown out of the
    /// banner when the pad lands on it, handed back when it leaves.
    #[must_use]
    pub fn backdrop_alpha(self) -> f32 {
        let to = alpha_of(self.at.expanded);
        self.fading.map_or(to, |(from, progress)| {
            let from = alpha_of(from.expanded);
            from + (to - from) * progress
        })
    }
}

fn alpha_of(expanded: bool) -> f32 {
    if expanded { 1.0 } else { 0.0 }
}

/// The banner: the window's full width and a third of its height, one slide at a
/// time.
///
/// The row arrives as a [`Shelf`] like every other, so the page draws it in its
/// own place in the stack; what changes is that a slide fills the width instead
/// of sitting in a line of cards. [`Motion`] says which slide is showing and how
/// far the banner is into moving: a slide on its way out is still drawn under
/// the one replacing it, and the row's own art gives way to the picture as it
/// grows into the window's background behind the row.
///
/// Everything it draws is owned by the widgets it builds, so the element does not
/// borrow the row: the page hands it a shelf built for this frame.
#[must_use]
pub fn view(shelf: &Shelf, motion: Motion, selection: Selection) -> Element<'static, Message> {
    let height = hero_height(WINDOW_HEIGHT);
    let Some(card) = shelf
        .cards
        .get(motion.at.slide)
        .or_else(|| shelf.cards.first())
    else {
        // No slides is no banner: the shell does not draw the row at all, so
        // this is only the frame between a reload and the next render.
        return container(text(""))
            .width(Length::Fill)
            .height(Length::Fixed(height))
            .into();
    };

    // The art's share of the row: it dissolves as the same picture fills the
    // window's background behind the row, so the two read as one picture
    // growing rather than two.
    let row_art = 1.0 - motion.backdrop_alpha();
    let alpha = motion.slide_alpha();
    let content: Element<'static, Message> = match motion.leaving() {
        Some((from, from_alpha)) => stack![
            slide(shelf, from, from_alpha * row_art, from_alpha),
            slide(shelf, motion.at.slide, alpha * row_art, alpha),
        ]
        .into(),
        None => slide(shelf, motion.at.slide, alpha * row_art, alpha),
    };

    // The shared tile button, so the banner wears the same accent ring under the
    // pad as a card does, and a click on it presses exactly what Confirm does.
    button::custom(content)
        .id(card.id.clone())
        .selected(selection.is_selected())
        .force_enabled(selection.is_selected())
        .width(Length::Fill)
        .height(Length::Fixed(height))
        .class(tile_button_class(selection.is_selected()))
        .padding(0)
        .on_press_maybe(card.message.clone())
        .name(card.label.clone())
        .into()
}

/// One slide: its art and its caption, at the alphas the banner's motion draws
/// the pair at. `art` is the art's own share of the row - what is left of it as
/// the picture fills the window's background - and `alpha` the rest of the
/// slide's.
fn slide(shelf: &Shelf, index: usize, art: f32, alpha: f32) -> Element<'static, Message> {
    let Spacing {
        space_s,
        space_m,
        space_l,
        ..
    } = theme::spacing();
    // A slide a reload shortened the list under is on its way out anyway: it is
    // gone rather than redrawn as some other slide's art.
    let Some(card) = shelf.cards.get(index) else {
        return container(text("")).into();
    };

    // The slide's own wide art, filling the window edge to edge.
    let artwork: Element<'static, Message> = backdrop_faded(
        &card.icon,
        ContentFit::Cover,
        Length::Fill,
        Length::Fill,
        art,
    );

    let mut title: Vec<Element<'static, Message>> = Vec::new();
    match &card.logo {
        // The feed's own wordmark, when it has one: the title in its own
        // lettering reads better than the same words in the UI's font.
        Some(logo) => title.push(backdrop_faded(
            logo,
            ContentFit::Contain,
            Length::Fixed(LOGO_WIDTH),
            Length::Fixed(LOGO_HEIGHT),
            alpha,
        )),
        None => title.push(text(card.label.clone()).size(TEXT_TITLE).into()),
    }
    if let Some(synopsis) = card.description.as_deref().and_then(synopsis) {
        title.push(text(synopsis).size(TEXT_LARGE).width(Length::Fill).into());
    }

    let caption: Element<'static, Message> = container(
        container(
            row![
                column(title).spacing(space_s).width(Length::Fill),
                indicators(shelf.cards.len(), index, alpha),
            ]
            .spacing(space_l)
            .align_y(Vertical::Bottom),
        )
        .class(theme::Container::Custom(Box::new(caption_scrim(alpha))))
        .width(Length::Fill)
        .padding([space_m, space_l]),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_y(Vertical::Bottom)
    .into();

    stack![artwork, caption].into()
}

/// The banner's art as the window's background: the slide showing, and the one
/// it is leaving, at the alphas of the fade between them and of the growth out
/// of the banner itself.
///
/// `None` when there is no banner to grow - or when its art has not grown at
/// all, so the wallpaper behind it is drawn alone.
#[must_use]
pub fn backdrop(shelf: Option<&Shelf>, motion: Motion) -> Option<Element<'static, Message>> {
    let shelf = shelf.filter(|shelf| shelf.hero)?;
    let alpha = motion.backdrop_alpha();
    if alpha <= 0.0 {
        return None;
    }
    let art = |index: usize, alpha: f32| {
        shelf.cards.get(index).map(|card| {
            backdrop_faded(
                &card.icon,
                ContentFit::Cover,
                Length::Fill,
                Length::Fill,
                alpha,
            )
        })
    };
    let showing = art(motion.at.slide, alpha * motion.slide_alpha())?;
    Some(
        match motion
            .leaving()
            .and_then(|(from, from_alpha)| art(from, alpha * from_alpha))
        {
            Some(leaving) => stack![leaving, showing].into(),
            None => showing,
        },
    )
}

/// Which slide of how many is showing, as dots in the caption's corner, at the
/// alpha the slide draws at.
fn indicators(count: usize, shown: usize, alpha: f32) -> Element<'static, Message> {
    let dots: Vec<Element<'static, Message>> = (0..count)
        .map(|index| indicator(index == shown, alpha))
        .collect();
    row(dots)
        .spacing(INDICATOR_SIZE / 2.0)
        .align_y(Vertical::Bottom)
        .into()
}

/// One dot: the accent for the slide showing, the page's own text colour for the
/// rest. Both are tokens, so the pair follows the palette under either theme.
fn indicator<'a>(active: bool, alpha: f32) -> Element<'a, Message> {
    container("")
        .width(Length::Fixed(INDICATOR_SIZE))
        .height(Length::Fixed(INDICATOR_SIZE))
        .class(theme::Container::Custom(Box::new(move |theme: &Theme| {
            let mut color: Color = if active {
                theme.cosmic().accent.base.into()
            } else {
                theme.cosmic().on_bg_color().into()
            };
            if !active {
                color.a = 0.45;
            }
            container::Style {
                background: Some(Background::Color(faded(color, alpha))),
                border: Border {
                    radius: (INDICATOR_SIZE / 2.0).into(),
                    width: 0.0,
                    color: Color::TRANSPARENT,
                },
                ..container::Style::default()
            }
        })))
        .into()
}

/// The scrim behind a slide's caption, at the alpha the slide draws at.
///
/// The caption sits on artwork, not on the page, so it needs a scrim to stay
/// readable over a bright frame. Like the tile label's, it is the theme's own
/// background at partial opacity with its text colour on top, so it follows the
/// palette under both themes while the art shows through.
fn caption_scrim(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |theme: &Theme| {
        let t = theme.cosmic();
        let mut background: Color = t.bg_color().into();
        background.a = 0.6 * alpha;
        container::Style {
            text_color: Some(faded(t.on_bg_color().into(), alpha)),
            icon_color: Some(faded(t.on_bg_color().into(), alpha)),
            background: Some(Background::Color(background)),
            border: Border {
                width: 0.0,
                color: Color::TRANSPARENT,
                ..Border::default()
            },
            shadow: cosmic::iced::Shadow::default(),
            snap: false,
        }
    }
}

/// A theme colour at a fade's alpha.
fn faded(color: Color, alpha: f32) -> Color {
    Color {
        a: color.a * alpha,
        ..color
    }
}

/// The line under the title: the feed's own words, flattened to one line and cut
/// at a word boundary. `None` when there is nothing to say.
fn synopsis(description: &str) -> Option<String> {
    let flattened = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.is_empty() {
        return None;
    }
    if flattened.chars().count() <= SYNOPSIS_CHARS {
        return Some(flattened);
    }
    let cut: String = flattened.chars().take(SYNOPSIS_CHARS).collect();
    let words = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    Some(format!("{words}…"))
}

#[cfg(test)]
mod tests {
    use super::{At, Motion, expanded, landed_from_above, rotate, rotates, stepped, synopsis};
    use crate::screens::dashboard::focus::Cursor;

    fn on(rail: usize, card: usize) -> Cursor {
        Cursor {
            rail,
            card,
            remembered: card,
        }
    }

    #[test]
    fn the_banner_moves_on_by_itself_and_comes_back_round() {
        assert_eq!(rotate(0, 3), 1);
        assert_eq!(rotate(1, 3), 2);
        assert_eq!(rotate(2, 3), 0);
        // One slide is not a rotation, and no slides is not a banner.
        assert_eq!(rotate(0, 1), 0);
        assert_eq!(rotate(3, 0), 0);
    }

    #[test]
    fn the_banner_rotates_only_while_the_pad_is_elsewhere() {
        assert!(rotates(4, on(1, 0)), "the rails below leave it rotating");
        assert!(!rotates(4, on(0, 2)), "the pad on it pauses it");
        assert!(!rotates(1, on(2, 0)), "one slide has nowhere to go");
        assert!(!rotates(0, on(1, 0)), "no banner, no rotation");
    }

    #[test]
    fn a_sideways_step_on_the_banner_moves_the_slide_with_the_pad() {
        let (cursor, slide) = stepped(on(0, 1), on(0, 2), 1, 5);

        assert_eq!((cursor.card, slide), (2, 2), "cursor: {cursor:?}");
    }

    #[test]
    fn arriving_on_the_banner_keeps_the_slide_it_is_showing() {
        let (cursor, slide) = stepped(on(1, 3), on(0, 3), 4, 5);

        assert_eq!(
            (cursor.card, cursor.remembered, slide),
            (4, 4, 4),
            "cursor: {cursor:?}"
        );
    }

    #[test]
    fn the_top_bar_lands_on_the_slide_showing_rather_than_a_remembered_card() {
        let (cursor, slide) = landed_from_above(on(0, 1), 3, 5);

        assert_eq!((cursor.card, slide), (3, 3), "cursor: {cursor:?}");
        // With no banner the first row is an ordinary rail, and the remembered
        // card is the right place to land after all.
        let (cursor, slide) = landed_from_above(on(0, 1), 3, 0);
        assert_eq!((cursor.card, slide), (1, 3), "cursor: {cursor:?}");
    }

    #[test]
    fn a_step_off_the_banner_leaves_its_slide_where_it_was() {
        let (cursor, slide) = stepped(on(0, 2), on(1, 2), 2, 5);

        assert_eq!((cursor.rail, cursor.card, slide), (1, 2, 2));
    }

    #[test]
    fn a_step_below_the_banner_does_not_touch_its_slide() {
        let (_, slide) = stepped(on(1, 1), on(1, 2), 3, 5);

        assert_eq!(slide, 3);
    }

    #[test]
    fn a_synopsis_is_one_flattened_line_that_ends_on_a_word() {
        assert_eq!(
            synopsis("A  film\nabout   a house."),
            Some("A film about a house.".to_owned())
        );
        assert_eq!(synopsis("   "), None);

        let long = "word ".repeat(60);
        let cut = synopsis(&long).expect("a long description still has a line");
        assert!(cut.ends_with('…'), "cut: {cut}");
        assert!(
            cut.chars().count() <= super::SYNOPSIS_CHARS + 1,
            "cut: {cut}"
        );
        assert!(!cut.trim_end_matches('…').ends_with("wor"), "cut: {cut}");
    }

    /// The banner's art fills the window's background exactly while the pad is
    /// on the banner row - the same rule that pauses the rotation - and is
    /// handed back to the wallpaper when the pad goes.
    #[test]
    fn the_banners_art_fills_the_background_while_the_pad_is_on_it() {
        assert!(expanded(on(0, 2), 5));
        assert!(!expanded(on(1, 0), 5), "the rails below hand it back");
        assert!(!expanded(on(0, 0), 0), "no banner fills nothing");
    }

    /// The fade out of where the banner was: one slide cross-fading into the
    /// next while both draw, and the art growing into the background at the
    /// same progress that gives the row's own art up.
    #[test]
    fn the_banner_cross_fades_between_where_it_was_and_where_it_is() {
        let there = At {
            slide: 2,
            expanded: true,
        };
        let moving = Motion {
            at: there,
            fading: Some((
                At {
                    slide: 1,
                    expanded: false,
                },
                0.25,
            )),
        };

        assert_eq!(moving.leaving(), Some((1, 0.75)));
        assert_eq!(moving.slide_alpha(), 0.25);
        assert_eq!(moving.backdrop_alpha(), 0.25);

        // The same slide grown rather than replaced: only the background moves.
        let growing = Motion {
            at: there,
            fading: Some((
                At {
                    slide: 2,
                    expanded: false,
                },
                0.25,
            )),
        };
        assert_eq!(growing.leaving(), None);
        assert_eq!(growing.slide_alpha(), 1.0);
        assert_eq!(growing.backdrop_alpha(), 0.25);

        // And at rest nothing fades at all.
        let rest = Motion {
            at: there,
            fading: None,
        };
        assert_eq!(rest.leaving(), None);
        assert_eq!(rest.slide_alpha(), 1.0);
        assert_eq!(rest.backdrop_alpha(), 1.0);
    }
}
