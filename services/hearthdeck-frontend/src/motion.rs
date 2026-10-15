//! Timing for screen motion: one clock, one easing, one bounded lifetime.
//!
//! iced has no retained animation system, so every transition in Hearthdeck is
//! plain application state advanced by a frame clock (see
//! `docs/frontend-animations.md`). This module owns *when* an animation runs and
//! how far along it is; the widgets in [`crate::widgets::transition`] own how that
//! progress is drawn, and the app owns the frame subscription. Nothing here
//! touches iced, so the rules below are tested without a compositor.
//!
//! [`Timing`] is the shared clock. The three patterns on top of it differ in what
//! they do with their payload: [`Fade`] keeps the value being left, [`Slide`]
//! holds back the value being arrived at until the cover is opaque, and [`Eased`]
//! interpolates a number - a scroll offset, in this app.

use std::time::{Duration, Instant};

/// When an animation started, and how long it runs.
///
/// The lifetime is bounded by construction: [`Timing::progress`] clamps at `1.0`
/// and [`Timing::is_done`] answers from the clock alone, so an animation cannot
/// outlive its duration even if a frame tick is missed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    started_at: Instant,
    duration: Duration,
}

impl Timing {
    /// Times an animation that starts at `started_at` and runs for `duration`.
    pub fn new(started_at: Instant, duration: Duration) -> Self {
        Self {
            started_at,
            duration,
        }
    }

    /// Eased `0.0..=1.0` progress at `now`.
    ///
    /// Clamped at both ends, so an animation asked about before it starts reads
    /// `0.0` and one asked about after it ends reads `1.0` instead of
    /// overshooting. The curve is `smootherstep`, which starts and ends with zero
    /// slope - what keeps a transition from jolting into motion.
    pub fn progress(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            // Nothing to interpolate: a zero-length animation is already over.
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.started_at);
        let raw = (elapsed.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0);
        cosmic::anim::smootherstep(raw)
    }

    /// Whether `now` is at or past the end of the animation.
    pub fn is_done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.started_at) >= self.duration
    }
}

/// A fade-through in flight: the value it started from, kept so the transition can
/// draw both sides.
///
/// The incoming value is applied immediately - the app already points at it when
/// the fade starts - so what has to be remembered is the outgoing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fade<T> {
    origin: T,
    timing: Timing,
}

impl<T> Fade<T> {
    /// Starts a fade out of `origin` at `started_at`, running for `duration`.
    pub fn new(origin: T, started_at: Instant, duration: Duration) -> Self {
        Self {
            origin,
            timing: Timing::new(started_at, duration),
        }
    }

    /// The value the fade started from, drawn until the transition retires.
    pub fn origin(&self) -> &T {
        &self.origin
    }

    /// Eased `0.0..=1.0` progress at `now`.
    pub fn progress(&self, now: Instant) -> f32 {
        self.timing.progress(now)
    }

    /// Whether the fade has run its course at `now`.
    pub fn is_done(&self, now: Instant) -> bool {
        self.timing.is_done(now)
    }
}

/// What one frame says to do with a [`Slide`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Still travelling: keep drawing the current value and take another tick.
    Waiting,
    /// The cover is opaque, so the target can be applied now.
    Swap,
    /// The travel is over: apply nothing more, and drop the slide.
    Done,
}

/// A shared-axis slide in flight: the value being arrived at, held back until the
/// cover is opaque.
///
/// The outgoing value stays the app's current state, still what gets drawn, and the
/// swap happens once progress passes the midpoint, where the transition widget's
/// cover is fully opaque. `direction` is `+1.0` when the target sits after the
/// current value in the order the surfaces are stacked in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slide<T> {
    target: T,
    direction: f32,
    swapped: bool,
    timing: Timing,
}

impl<T> Slide<T> {
    /// Slides towards `target` in `direction`, starting at `started_at` and running
    /// for `duration`.
    pub fn new(target: T, direction: f32, started_at: Instant, duration: Duration) -> Self {
        Self {
            target,
            direction,
            swapped: false,
            timing: Timing::new(started_at, duration),
        }
    }

    /// The value to apply once the cover is opaque.
    pub fn target(&self) -> &T {
        &self.target
    }

    /// Which way the content travels: `+1.0` towards later values, `-1.0` back.
    pub fn direction(&self) -> f32 {
        self.direction
    }

    /// Eased `0.0..=1.0` progress at `now`.
    pub fn progress(&self, now: Instant) -> f32 {
        self.timing.progress(now)
    }

    /// Advances the slide by one frame.
    ///
    /// The swap is checked before the end, so a frame that lands past the duration
    /// still applies the target rather than retiring a slide whose value never
    /// arrived - the content would otherwise have moved on screen and not in
    /// state.
    pub fn step(&mut self, now: Instant) -> Step {
        if !self.swapped && self.timing.progress(now) >= 0.5 {
            self.swapped = true;
            return Step::Swap;
        }
        if self.timing.is_done(now) {
            return Step::Done;
        }
        Step::Waiting
    }
}

/// An eased number in flight, carrying whatever the app needs to know what it is
/// moving.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Eased<T> {
    target: T,
    from: f32,
    to: f32,
    timing: Timing,
}

impl<T> Eased<T> {
    /// Eases `target` from `from` to `to`, starting at `started_at` and running for
    /// `duration`.
    pub fn new(target: T, from: f32, to: f32, started_at: Instant, duration: Duration) -> Self {
        Self {
            target,
            from,
            to,
            timing: Timing::new(started_at, duration),
        }
    }

    /// What this animation is moving.
    pub fn target(&self) -> &T {
        &self.target
    }

    /// The interpolated value at `now`.
    pub fn value(&self, now: Instant) -> f32 {
        let progress = self.timing.progress(now);
        self.from + (self.to - self.from) * progress
    }

    /// Whether the travel has run its course at `now`.
    pub fn is_done(&self, now: Instant) -> bool {
        self.timing.is_done(now)
    }
}

#[cfg(test)]
mod tests {
    use super::{Eased, Fade, Slide, Step, Timing};
    use std::time::{Duration, Instant};

    const DURATION: Duration = Duration::from_millis(320);

    #[test]
    fn progress_runs_from_zero_to_one_across_the_duration() {
        let start = Instant::now();
        let timing = Timing::new(start, DURATION);

        assert_eq!(timing.progress(start), 0.0);
        assert_eq!(timing.progress(start + DURATION), 1.0);
    }

    #[test]
    fn progress_is_clamped_at_both_ends() {
        let early = Instant::now();
        let start = early + Duration::from_millis(50);
        let timing = Timing::new(start, DURATION);

        assert_eq!(timing.progress(early), 0.0, "before the start reads zero");
        assert_eq!(
            timing.progress(start + DURATION * 10),
            1.0,
            "past the end reads one"
        );
    }

    #[test]
    fn progress_eases_in_and_out_rather_than_ramping_linearly() {
        let start = Instant::now();
        let timing = Timing::new(start, DURATION);

        // The curve is symmetric, so the midpoint is exactly halfway...
        let midpoint = timing.progress(start + DURATION / 2);
        assert!(
            (midpoint - 0.5).abs() < f32::EPSILON,
            "midpoint: {midpoint}"
        );

        // ...while both ends move slower than a straight line, which is what makes
        // the transition start and stop without a jolt.
        let quarter = timing.progress(start + DURATION / 4);
        let three_quarters = timing.progress(start + DURATION * 3 / 4);
        assert!(quarter < 0.25, "eases in: {quarter}");
        assert!(three_quarters > 0.75, "eases out: {three_quarters}");
    }

    #[test]
    fn is_done_only_at_or_past_the_duration() {
        let start = Instant::now();
        let timing = Timing::new(start, DURATION);

        assert!(!timing.is_done(start));
        assert!(!timing.is_done(start + DURATION - Duration::from_millis(1)));
        assert!(timing.is_done(start + DURATION));
        assert!(timing.is_done(start + DURATION * 10));
    }

    #[test]
    fn a_zero_duration_animation_is_over_at_once() {
        let start = Instant::now();
        let timing = Timing::new(start, Duration::ZERO);

        assert_eq!(timing.progress(start), 1.0, "nothing to interpolate");
        assert!(timing.is_done(start));
    }

    #[test]
    fn a_fade_keeps_the_value_it_started_from() {
        let start = Instant::now();
        let fade = Fade::new("dashboard", start, DURATION);

        assert_eq!(*fade.origin(), "dashboard");
        assert_eq!(fade.progress(start), 0.0);
        assert_eq!(fade.progress(start + DURATION), 1.0);
        assert!(!fade.is_done(start));
        assert!(fade.is_done(start + DURATION));
    }

    #[test]
    fn a_slide_swaps_once_the_cover_is_opaque() {
        let start = Instant::now();
        let mut slide = Slide::new("library", 1.0, start, DURATION);

        assert_eq!(slide.step(start), Step::Waiting);
        assert_eq!(slide.step(start + DURATION / 4), Step::Waiting);

        // The midpoint is where the cover is fully opaque, which is the one safe
        // moment to change what is behind it.
        let midpoint = start + DURATION / 2;
        assert_eq!(slide.step(midpoint), Step::Swap);
        assert_eq!(slide.step(midpoint), Step::Waiting, "and only once");
        assert_eq!(slide.step(start + DURATION), Step::Done);
    }

    #[test]
    fn a_slide_whose_frames_were_missed_still_swaps() {
        let start = Instant::now();
        let mut slide = Slide::new("library", 1.0, start, DURATION);

        // A tick that lands past the end still applies the target before retiring
        // the slide: otherwise the content moved on screen and the state did not.
        let late = start + DURATION * 2;
        assert_eq!(slide.step(late), Step::Swap);
        assert_eq!(slide.step(late), Step::Done);
    }

    #[test]
    fn a_slide_keeps_its_target_and_direction() {
        let start = Instant::now();
        let slide = Slide::new(Some(2), -1.0, start, DURATION);

        assert_eq!(*slide.target(), Some(2));
        assert_eq!(slide.direction(), -1.0);
        assert_eq!(slide.progress(start), 0.0);
        assert_eq!(slide.progress(start + DURATION), 1.0);
    }

    #[test]
    fn an_eased_value_travels_from_where_it_was_to_where_it_is_going() {
        let start = Instant::now();
        let eased = Eased::new("grid", 100.0, 300.0, start, DURATION);

        assert_eq!(*eased.target(), "grid");
        assert_eq!(eased.value(start), 100.0, "starts where it was");
        assert_eq!(
            eased.value(start + DURATION / 2),
            200.0,
            "halfway at the midpoint"
        );
        assert_eq!(eased.value(start + DURATION), 300.0, "lands on the target");
        assert!(!eased.is_done(start));
        assert!(eased.is_done(start + DURATION));
    }
}
