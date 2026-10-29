//! The shell's motion: which transition is in flight, and what it applies.
//!
//! [`crate::motion`] owns the timing and the three patterns; this module owns the
//! shell's use of them - the page a fade came from, the section or tab a slide is
//! travelling to, and the list a focus step is dragging. It is an `impl App` block
//! beside the shell rather than inside `app/mod.rs`, so that file keeps the state
//! declaration and the view composition. A child module of `app` sees `App`'s
//! private fields, so the split is a move, not a widening of visibility.

use std::time::Instant;

use cosmic::app::Task;
use cosmic::iced::widget::scrollable::{AbsoluteOffset, scroll_to};

use crate::app_group::Section;
use crate::motion::{Eased, Fade, Step};
use crate::screens::dashboard::{self, backdrop};
use crate::screens::library::{grid, header};
use crate::style::{
    BACKGROUND_TRANSITION_DURATION, PAGE_TRANSITION_DURATION, SCROLL_TRANSITION_DURATION,
};

use super::{App, Message, Page};

impl App {
    /// Switches the visible page, starting a fade-through when the page actually
    /// changes. Re-selecting the current page snaps instead of fading to the same
    /// place.
    ///
    /// The flip itself is immediate - routing, input and tests read `self.page` as
    /// soon as the message lands - and the fade only records where the change came
    /// from, so `view` can draw both sides for [`PAGE_TRANSITION_DURATION`]. A
    /// switch arriving mid-fade starts from the page the shell already points at,
    /// which is the pending target applied immediately.
    pub(super) fn switch_page(&mut self, to: Page) {
        if self.page == to {
            self.page_fade = None;
            return;
        }
        let from = self.page;
        self.page = to;
        // A page that is left takes its fields with it. A field that disappears
        // with the caret in it never reports a blur, so the state has to go with
        // the page - otherwise the pad stays handed to a keyboard nothing is
        // typing into, and every button but Back is dropped.
        self.text_entry.end();
        // The switch drops the grid from the tree, and the library's return
        // rebuilds it at its top: the follow is owed when the fade lands.
        if to == Page::Library {
            self.grid_restore_pending = true;
        }
        // Take the frame clock once, so the first drawn frame is at progress 0.
        self.now = Instant::now();
        self.page_fade = Some(Fade::new(from, self.now, PAGE_TRANSITION_DURATION));
    }

    /// Drops the page fade once it has run its course.
    ///
    /// Runs for *every* message - not only for the frame tick - so a fade cannot
    /// outlive [`PAGE_TRANSITION_DURATION`] even if a tick is missed.
    pub(super) fn expire_page_fade(&mut self) {
        if self
            .page_fade
            .is_some_and(|fade| fade.is_done(Instant::now()))
        {
            self.page_fade = None;
            // The details screen outlives its own fade - the fade draws the
            // page it is leaving - so its state goes with that fade.
            if self.page != Page::Details {
                self.details = None;
            }
        }
    }

    /// Whether any motion is in flight. The frame clock is subscribed on this, so
    /// an idle window schedules no redraws at all.
    pub(super) fn is_animating(&self) -> bool {
        self.page_fade.is_some()
            || self.section_slide.is_some()
            || self.tab_slide.is_some()
            || self.scrolls.is_animating()
            || self
                .backdrop_fade
                .is_some_and(|fade| !fade.is_done(self.now))
            // An owed follow is not motion, but it needs its frame to run on:
            // without one it would wait forever for a redraw nothing asked for.
            || self.grid_restore_pending
    }

    /// Drops the background's fade once it has run its course, for the same
    /// reason the page fade is dropped: an animation left in flight would keep
    /// the frame clock subscribed for a redraw nothing needs.
    pub(super) fn expire_backdrop_fade(&mut self) {
        if self
            .backdrop_fade
            .is_some_and(|fade| fade.is_done(Instant::now()))
        {
            self.backdrop_fade = None;
        }
    }

    /// Records which card the window's background is showing, fading out of the
    /// card it was showing: one title's art cross-fading into the next as the
    /// pad walks, and into the wallpaper as it steps onto a card whose feed
    /// sent no wide art.
    ///
    /// Called from everywhere the dashboard's cursor can move, so the motion is
    /// one decision in one place no matter what moved the pad.
    pub(super) fn sync_backdrop(&mut self) {
        let at = backdrop::At {
            rail: self.dashboard_cursor.rail,
            card: self.dashboard_cursor.card,
        };
        if at == self.backdrop_at {
            return;
        }
        // A change arriving mid-fade starts from where the background was
        // heading - the same rule the page fade follows.
        let from = self.backdrop_at;
        self.backdrop_at = at;
        self.now = Instant::now();
        self.backdrop_fade = Some(Fade::new(from, self.now, BACKGROUND_TRANSITION_DURATION));
    }

    /// The background's motion for this frame: where the pad is, and how far
    /// the fade out of where it was has come. A fade that has run its course is
    /// not in flight: the background has arrived.
    pub(super) fn backdrop_motion(&self) -> backdrop::Motion {
        backdrop::Motion {
            at: self.backdrop_at,
            fading: self
                .backdrop_fade
                .filter(|fade| !fade.is_done(self.now))
                .map(|fade| (*fade.origin(), fade.progress(self.now))),
        }
    }

    /// Applies a section selection: reset the view state, reload the grid and put
    /// the cursor on it.
    ///
    /// Called either at once - a section picked as the page changes, or the one
    /// already showing - or by [`App::advance_section_slide`] once the cover is
    /// opaque.
    pub(super) fn apply_section(&mut self, section: Section) -> Task<Message> {
        self.apply_section_with_group(section, None)
    }

    /// [`App::apply_section`], landing on `group`'s tab rather than the "all"
    /// tab - which is what a console picked on the dashboard asks for.
    pub(super) fn apply_section_with_group(
        &mut self,
        section: Section,
        group: Option<usize>,
    ) -> Task<Message> {
        self.cur_section = section;
        self.cur_group = group;
        self.search_value.clear();
        self.facet_selection.clear();
        self.facets.clear();
        self.filter_open = false;
        // The strip holds a different section's tabs now, so it goes back to its
        // start rather than keeping the offset of a strip that no longer exists.
        let strip = self.reset_tab_strip();
        let listed = if section == Section::ConsoleGames {
            // The console grid's facets and records both come from the daemon; it
            // is the one source that loads asynchronously.
            self.refilter();
            Task::batch([self.ensure_console_scope(), self.load_facets()])
        } else {
            self.rebuild_local_facets();
            self.refilter();
            self.list_replaced()
        };
        // The follow reads the facets the section just rebuilt, because they are
        // what decides whether the strip shares its row with the filter button.
        let reveal = self.reveal_tab(header::tab_index(group));
        Task::batch([listed, strip, reveal])
    }

    /// Applies a tab (group) selection, the same way and for the same reasons as
    /// [`App::apply_section`].
    pub(super) fn apply_group(&mut self, group: Option<usize>) -> Task<Message> {
        self.cur_group = group;
        self.facet_selection.clear();
        self.facets.clear();
        self.filter_open = false;
        let listed = if self.cur_section == Section::ConsoleGames {
            self.refilter();
            Task::batch([self.ensure_console_scope(), self.load_facets()])
        } else {
            self.rebuild_local_facets();
            self.refilter();
            self.list_replaced()
        };
        Task::batch([listed, self.reveal_tab(header::tab_index(group))])
    }

    /// Drives the section slide: apply the pending section under the opaque cover
    /// at the midpoint, then drop the slide once it has run its course.
    pub(super) fn advance_section_slide(&mut self) -> Task<Message> {
        let Some(slide) = self.section_slide.as_mut() else {
            return Task::none();
        };
        let step = slide.step(self.now);
        let target = *slide.target();
        match step {
            Step::Swap => self.apply_section(target),
            Step::Done => {
                self.section_slide = None;
                Task::none()
            }
            Step::Waiting => Task::none(),
        }
    }

    /// Drives the tab slide, the same way and for the same reasons as
    /// [`App::advance_section_slide`].
    pub(super) fn advance_tab_slide(&mut self) -> Task<Message> {
        let Some(slide) = self.tab_slide.as_mut() else {
            return Task::none();
        };
        let step = slide.step(self.now);
        let target = *slide.target();
        match step {
            Step::Swap => self.apply_group(target),
            Step::Done => {
                self.tab_slide = None;
                Task::none()
            }
            Step::Waiting => Task::none(),
        }
    }

    /// Eases a scroll from `from` to `to`, or jumps straight there when the two are
    /// already the same.
    ///
    /// Every focus-driven scroll on every list goes through here, so the grid, the
    /// dashboard rails and the dashboard page all move at one speed instead of each
    /// surface snapping on its own. Each [`ScrollTarget`] eases independently, so a
    /// step that moves two lists at once - the page down to a rail, and the rail
    /// sideways to the card - keeps both.
    pub(super) fn animate_scroll_to(
        &mut self,
        target: ScrollTarget,
        from: f32,
        to: f32,
    ) -> Task<Message> {
        if (to - from).abs() < 0.5 {
            self.scrolls.stop(target);
            return scroll_task(target, to);
        }
        self.now = Instant::now();
        self.scrolls.start(target, from, to, self.now);
        Task::none()
    }

    /// Advances the in-flight eased scrolls by one frame: emits each `scroll_to`
    /// for the interpolated offset, and drops the ones that have arrived.
    pub(super) fn advance_scroll(&mut self) -> Task<Message> {
        self.scrolls.advance(self.now)
    }
}

/// The focus-follow scrolls in flight, one per list.
///
/// A single step can move two lists at once: landing on a rail below the fold
/// scrolls the page to it and the rail sideways to the remembered card. A single
/// animation slot made the second overwrite the first, which left the card off
/// screen; keying them by [`ScrollTarget`] lets both travel.
#[derive(Debug, Default)]
pub(super) struct Scrolls {
    in_flight: Vec<Eased<ScrollTarget>>,
}

impl Scrolls {
    /// Whether any list is still easing.
    pub(super) fn is_animating(&self) -> bool {
        !self.in_flight.is_empty()
    }

    /// Starts easing `target` from `from` to `to`, replacing any travel that list
    /// already had in flight.
    pub(super) fn start(&mut self, target: ScrollTarget, from: f32, to: f32, now: Instant) {
        self.stop(target);
        self.in_flight.push(Eased::new(
            target,
            from,
            to,
            now,
            SCROLL_TRANSITION_DURATION,
        ));
    }

    /// Drops `target`'s travel, if it has one.
    pub(super) fn stop(&mut self, target: ScrollTarget) {
        self.in_flight.retain(|scroll| *scroll.target() != target);
    }

    /// Whether `target` is easing right now. A report from a scroll the shell
    /// is driving describes where the travel happens to be, not where the
    /// shell sent it.
    pub(super) fn is_moving(&self, target: ScrollTarget) -> bool {
        self.in_flight
            .iter()
            .any(|scroll| *scroll.target() == target)
    }

    /// Advances every scroll by one frame, emitting one `scroll_to` per list and
    /// dropping the ones that have arrived.
    pub(super) fn advance(&mut self, now: Instant) -> Task<Message> {
        let mut tasks = Vec::with_capacity(self.in_flight.len());
        self.in_flight.retain(|scroll| {
            let target = *scroll.target();
            tasks.push(scroll_task(target, scroll.value(now)));
            !scroll.is_done(now)
        });
        Task::batch(tasks)
    }

    /// The lists currently easing, for tests that assert one step moved two.
    #[cfg(test)]
    pub(super) fn targets(&self) -> Vec<ScrollTarget> {
        self.in_flight
            .iter()
            .map(|scroll| *scroll.target())
            .collect()
    }
}

/// What an eased scroll is moving.
///
/// The shell names the scrollable rather than carrying iced's widget id and axis
/// through the animation: the three lists that follow the cursor scroll on fixed
/// axes, so the mapping back to a `scroll_to` belongs in one place
/// ([`scroll_task`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScrollTarget {
    /// The library grid, which scrolls vertically by row.
    Grid,
    /// The library's tab strip, which scrolls horizontally by tab.
    Tabs,
    /// One dashboard rail, which scrolls horizontally by card.
    Rail(usize),
    /// The dashboard page, which scrolls vertically by rail.
    Page,
}

/// The `scroll_to` for one frame of an eased scroll, wherever it is going.
fn scroll_task(target: ScrollTarget, offset: f32) -> Task<Message> {
    match target {
        ScrollTarget::Grid => grid::scroll_to_offset(offset),
        ScrollTarget::Tabs => scroll_tabs(offset),
        ScrollTarget::Rail(rail) => scroll_rail(rail, offset),
        ScrollTarget::Page => dashboard::scroll_to_page(offset),
    }
}

/// The tab strip's order: the section's "all" tab first, then its groups.
fn tab_position(group: Option<usize>) -> i32 {
    group.map_or(0, |index| index as i32 + 1)
}

/// Which way a tab slide travels: the strip is ordered, so a later tab sits to the
/// right of an earlier one.
pub(super) fn tab_direction(from: Option<usize>, to: Option<usize>) -> f32 {
    if tab_position(to) >= tab_position(from) {
        1.0
    } else {
        -1.0
    }
}

/// Which way a section slide travels: the sidebar lists sections top to bottom, so
/// a later section's content rises from below.
pub(super) fn section_direction(from: Section, to: Section) -> f32 {
    if to.index() >= from.index() {
        1.0
    } else {
        -1.0
    }
}

/// Scrolls one dashboard rail to a horizontal offset, leaving it where it is
/// vertically - a rail only ever moves sideways.
fn scroll_rail(rail: usize, offset: f32) -> Task<Message> {
    scroll_to(
        dashboard::rail_id(rail),
        AbsoluteOffset {
            x: Some(offset),
            y: None,
        },
    )
}

/// Scrolls the library's tab strip sideways, the only axis it has.
fn scroll_tabs(offset: f32) -> Task<Message> {
    scroll_to(
        header::tab_strip_id(),
        AbsoluteOffset {
            x: Some(offset),
            y: None,
        },
    )
}
