# Frontend animations

How the COSMIC (Rust/iced) frontend animates, why it is built this way, and how
to add another animation without hurting idle or scroll performance.

## The model

iced has no retained animation system. A widget is a pure function of application
state, and the framework only redraws when something changes. So *every*
animation in iced is application-driven: the app keeps a progress value in its
state, an explicit clock advances it, and the widget renders whatever progress it
is handed.

iced ships two building blocks for this:

| Primitive | Module | What it is for |
| --- | --- | --- |
| `iced::window::frames()` | `iced::window` | A `Subscription` that yields once per rendered frame. iced documents it as "useful to smoothly draw application-driven animations without missing any frames." |
| `iced::Animation<T>` | `iced::animation` | Interruptible transition helper (easing, duration, delay, repeat). Wraps the `lilt` crate. |

`Animation<T>` is convenient but optional. Hearthdeck uses the clock plus plain
arithmetic because the page transition only needs one eased scalar and the
transition's lifetime must be bounded by the application, not by a widget.

### What the libs do, for reference

- iced's own `gallery` example keeps `Animation<bool>` fields in its app state,
  adds `window::frames().map(|_| Message::Animate)` to `subscription()` only
  while something is animating, and interpolates in `view()`.
- libcosmic's built-in animating widgets (`toggler`, `cards`,
  `reorderable_flex_row`) instead animate inside `Widget::update`, reacting to
  `Event::Window(window::Event::RedrawRequested(_))` and calling
  `shell.request_redraw()` to schedule the next frame. That keeps the animation
  out of the app's `view()` so no re-layout happens per frame, but it also means
  the widget is responsible for its own lifetime.
- libcosmic has **no** theme-level animation timing. Every widget carries its own
  `duration: Duration` field with a local default (toggler 200 ms, cards 200 ms,
  `reorderable_flex_row` 180 ms) exposed through a builder method.

## Three transitions, two patterns

Hearthdeck has three transitions because the navigation has three kinds of
relationship, but only two patterns:

| Change | Relationship | Pattern | Duration |
| --- | --- | --- | --- |
| Dashboard <-> Library <-> Settings | Sibling top-level destinations | Fade through | `PAGE_TRANSITION_DURATION` (320 ms) |
| Tab (group) change in the Library | Peers on an ordered strip | Shared axis X | `TAB_TRANSITION_DURATION` (240 ms) |
| Section change (PC Games / Console Games / Applications) | Stacked peers in the sidebar | Shared axis Y | `SECTION_TRANSITION_DURATION` (300 ms) |

## Why a fade through, not a lateral push

A change between two **sibling top-level destinations** - the Dashboard, the
Library and Settings are all such pages - has no spatial relationship to draw on.
Material's motion system separates two cases:

| Pattern | Use for | Says |
| --- | --- | --- |
| Shared axis (X/Y/Z) | Elements with a spatial or navigational relationship: tabs, steps, peer pages in a hierarchy | "This is next to that" |
| Fade through | Destinations with no direct relationship: top-level destinations | "This replaced that" |

A lateral push is shared-axis-X. Applied to siblings it asserts a spatial model
that does not exist, and on a 10-foot display a full-width slide is visually
heavy and draws the eye to the movement instead of the content. TV/console UIs
lean on **focus** for feedback and keep page motion secondary. So the correct
pattern here is **fade through**: the outgoing page fades out, and the incoming
one fades in, passing through the surface colour at the midpoint.

Microsoft's Fluent guidance agrees on the shape and the timing: page transitions
should "respect the flow of an app," exits should "always combine with fade
out," and page transitions run in the 167-333 ms band.

### Renderer constraint

iced has no opacity for arbitrary content. Only `image` and `svg` expose
`.opacity()`; `Renderer::with_transformation` gives translate and scale, but
there is no layer alpha, so two live pages cannot be composited at partial
alpha at once. Material's fade through passes through the surface colour anyway,
so the implementation is a true fade through: draw the outgoing page, cover it
with the surface colour at full opacity at the midpoint, swap, and fade the
cover back out. Because the cover is the page's own `root_background`, full
cover is indistinguishable from the background.

### Renderer gotcha: layer order

iced_wgpu renders its layers in **allocation order** — `for layer in
self.layers.iter()` in `iced_wgpu::Renderer::render`, over the layers allocated
by `graphics::layer::Stack`. A plain quad added to the current layer is painted
*under* any clipped child that pushed a later layer, such as a `scrollable`.

The surface cover must therefore be drawn inside its own `renderer.with_layer(...)`
call, allocated after the page's layers, or it is painted underneath the page
and the fade silently disappears (the transition reads as a hard cut). This is
exactly the regression the first fade-through implementation shipped with. Any
future full-screen overlay drawn from a widget's `draw` hits the same rule.

`with_layer(bounds, f)` pushes a new clipped layer; because the page has already
allocated its own layers by the time the cover is drawn, the cover's layer has a
higher index and is composited last. Layer merging does not fold it back under:
`Stack::merge` only joins a candidate when `candidate.end() <= target.start()`
and the bounds are equal, and a page root layer containing text reports
`end() == 5` against the cover's `start() == 1`.

## How Hearthdeck does it

The page fade through is the reference implementation. `widgets/transition.rs`
draws it; the timing around it lives in `src/motion.rs`, which owns when an
animation started, how far along it is, and when it must end, while the shell
owns the frame subscription. The rewrite (`app/mod.rs::switch_page`) drives the
Dashboard / Library / Settings fade through that way; the inherited
`app_legacy.rs` still carries an inline copy of the same timing until it retires.

1. **State, not pixels.** The shell flips `page` immediately (so routing and tests
   are unaffected) and records the page it came from: `App::page_fade` holds a
   `motion::Fade<Page>` (legacy: `HearthDeck::page_animation` /
   `PageAnimation { from, started_at }`).
2. **One clock, only when needed.** `subscription()` adds
   `window::frames().map(|(_, at)| Message::Animate(at))` *only* while a page,
   tab or section transition is in flight - in the rewrite, while `page_fade` is
   set. An idle window has no frames subscription and schedules no redraws at all.
3. **Progress is derived in `view()`.** `motion::Timed::progress` maps
   `now - started_at` through `cosmic::anim::smootherstep` to an eased `0..=1`.
   libcosmic's `Application::update` does not receive a `now` argument the way raw
   iced's `Application` does, so the frame time is carried on `Message::Animate`.
4. **Bounded lifetime.** Expiry runs at the top of `update` for *every* message
   and drops the transition once `Timed::is_done` says it has exceeded
   `PAGE_TRANSITION_DURATION` (rewrite: `App::expire_page_fade`; legacy:
   `expire_page_animation`). A transition therefore cannot outlive its duration
   even if a frame tick is missed.
5. **Presentation only in the widget.** `PageTransition` takes `progress: f32`
   and, per frame, draws only the page on the near side of the swap plus a
   surface-colour cover sized by `smootherstep` of the half-phase. It owns no
   timing, requests no redraws, and removes nothing.

### Tabs: shared-axis slide

A tab strip *is* an ordered set of peers with a spatial relationship, so tabs are
the one place a lateral slide is correct (Material shared axis X). The
implementation deliberately differs from the page fade in one key way: it renders
**one** child, never two.

- The shell holds the slide while it runs (`motion::Slide`: a target, a direction
  and a `swapped` flag on one `Timing`), recorded **without changing `cur_group`**,
  so the outgoing grid is still the current data - rewrite: `App::tab_slide`,
  legacy: `HearthDeck::tab_animation`.
- `TabTransition` (`widgets/transition.rs`) draws that single grid sliding out in
  `-direction` while the surface cover rises, then the swapped grid sliding in
  from `+direction` while the cover falls. `slide(progress, direction, width)`
  returns `(offset_x, cover_alpha)` and is unit-tested.
- The swap happens on a tick, under the opaque cover: the advance step calls
  `apply_group` once progress passes the midpoint (rewrite: `Slide::step` answers
  `Step::Swap`) and drops the slide once it has run its course (`Step::Done`).
  `apply_group` is the old body of the `SelectGroup` handler, now deferred to the
  midpoint.
- Because only one grid is ever in the tree, no two grids are laid out per frame
  and the shared entry `Id`s never collide. Rendering two live grids would
  duplicate those ids and re-lay out every tile every frame.

Direction comes from `tab_position` (the "all apps" tab is 0, groups follow), so
the grid slides left when moving right along the strip and vice versa. The rewrite
reads it through `app::tab_direction`, the legacy app through its own copy.

### Sections: shared-axis slide on Y

The sidebar sections are peers with a spatial relationship too - they are stacked
top to bottom - so a section change is the same shared-axis slide as a tab
change, on the other axis. It reuses the tab machinery almost verbatim:

- The shell holds the target section the same way - rewrite: `App::section_slide`,
  legacy: `HearthDeck::section_animation` - recorded **without changing
  `cur_section`**, so the outgoing section's title, tabs and grid stay on screen
  while they slide off.
- `SectionTransition` is the vertical twin of `TabTransition`, sharing the same
  `slide(progress, direction, extent)` curve, just fed `bounds.height` instead of
  `bounds.width`. It wraps the whole content column, not the grid, so the title
  and tab strip travel with the grid. The left sidebar stays put: it is the fixed
  navigation, and leaving it still is what makes the motion read as "the content
  changed", not "the app moved".
- The swap happens on a tick, under the opaque cover, exactly as a tab change does:
  `apply_section` is the old body of the `SelectSection` handler, now deferred to
  the midpoint.
- Direction comes from `Section::index()` (sidebar order), so moving down the
  sidebar makes the new content rise from the bottom edge and moving up makes it
  descend from the top. The rewrite reads it through `app::section_direction`.

One caveat: the slide travels the library's content column, so a `SelectSection`
delivered while some other page is showing applies immediately instead of
animating. The rewrite states that in the handler (`SelectSection` applies at once
when the page is changing); the sidebar is the only thing that emits the message,
and it only exists on the Library screen.

### Focus-follow scroll: eased, on one clock

When the cursor moves off the edge of a list, the list follows - eased, not snapped,
so a step reads as the view moving rather than the selection jumping. Every such
scroll goes through one entry point (rewrite: `App::animate_scroll_to`, legacy:
`HearthDeck::animate_scroll_to`), so the grid, the dashboard rails, the dashboard
page and the legacy tab strip all move at `SCROLL_TRANSITION_DURATION`.

- `motion::Eased` carries `from`, `to` and what is being moved; each frame the
  advance step emits a `scroll_to` at the interpolated offset, and the animation is
  dropped on arrival. A scroll the cursor did not drag stays a plain `scroll_to`
  with no animation at all: a list that is replaced goes back to the top
  immediately, under whatever cover is up.
- The shell names the scrollable (`ScrollTarget::Grid` / `Rail(usize)` / `Page`)
  rather than carrying iced's widget id and axis through the animation, and one
  function maps a target back to its `scroll_to`.
- A virtualized list records the offset it is *going* to, not the one it is at: the
  rows around the target are what get built, and the eased scroll is only what the
  eye sees.

### Where timings live

All motion timing is a design token in `style.rs`, in the `Motion` section:

```rust
pub const PAGE_TRANSITION_DURATION: Duration = Duration::from_millis(320);
pub const TAB_TRANSITION_DURATION: Duration = Duration::from_millis(240);
pub const SECTION_TRANSITION_DURATION: Duration = Duration::from_millis(300);
```

Widgets take a duration as a parameter (the same shape as libcosmic's widgets);
they never define their own. Add a token here rather than a literal in a view.

## Adding an animation

1. Add state to the shell for the payload, and time it with `motion.rs` (`Timed`,
   or `Fade` where the value being left has to stay drawable).
2. Add a `Message` variant that carries the frame time if the animation is
   app-wide.
3. In `subscription()`, add `window::frames()` **gated on the animation being
   active**.
4. In `update`, advance or expire the state; clear it when it is done.
5. Render the interpolated value in `view()`.

## Performance rules

- **Gate the subscription.** Never keep `window::frames()` running when nothing
  is animating; that pins the app at display refresh rate forever.
- **Bound every animation.** Prefer an explicit end time and clear the state in
  `update`. An animation that never terminates keeps forcing full redraws, which
  is exactly the failure that made library scrolling sluggish: a lingering
  transition kept redrawing both pages inside a clipping layer on every frame.
- **Keep transitions short.** The three `Motion` tokens (240 ms tab, 300 ms
  section, 320 ms page) sit inside Fluent's 167-333 ms band. Much longer starts
  to feel sluggish and multiplies full-window redraws.
- **Do not animate backdrops.** `docs/appearance-system.md` forbids procedural
  animation, shaders, or periodic repainting in the backdrop surface; content
  and navigation are animated instead.

## Related decisions

- The Library tab strip sets `.animation_duration(Duration::ZERO)` on its
  `reorderable_flex_row` (`app.rs`). That is a libcosmic slot animation, unrelated
  to the page transition; it is disabled because its mid-layout repositioning
  overlapped chips and forced a per-frame full redraw. Revisit only with the
  performance rules above in mind.

## Sources

- Material motion, transition patterns (shared axis / fade through / container
  transform): <https://m3.material.io/styles/motion/transitions/transition-patterns>
- Microsoft Fluent motion, page transitions and timing:
  <https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/motion>
- iced `window::frames`: <https://docs.rs/iced/latest/iced/window/fn.frames.html>
- iced `animation` module (`Animation`, `Easing`): <https://docs.rs/iced/latest/iced/animation/index.html>
- iced gallery example (Animation + frames in an application):
  <https://github.com/iced-rs/iced/blob/master/examples/gallery/src/main.rs>
- libcosmic animating widgets: `src/widget/toggler.rs`, `src/widget/cards.rs`,
  `src/widget/reorderable_flex_row/widget.rs`, `src/anim.rs`
- Android TV design (TV UI, motion restraint): <https://developer.android.com/design/ui/tv>
