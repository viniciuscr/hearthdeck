---
name: iced-cosmic-ui
description: "Binding rules for the Hearthdeck COSMIC frontend (`services/hearthdeck-frontend`): the iced/libcosmic MVU contract, message and state design, Task construction, subscriptions, widget reuse, theme tokens, and the controller-first navigation contract. Load this for ANY change to frontend state, messages, views, widgets, styling, animations, or gamepad handling. Activates on cosmic::Application, update, view, Element, Task::perform, Task::batch, Subscription, theme::active, theme::spacing, style.rs, widgets/, app.rs, gamepad, keybinds, and Escape/Back handling."
---

# COSMIC / iced UI rules for Hearthdeck

## CRITICAL RULE 1: theme tokens, never color literals

The UI must render correctly in both light and dark mode. Any hardcoded color breaks one of them.

```rust
// WRONG - looks right in dark mode, unreadable in light.
container(text("Continue Watching")).style(|_| container::Style {
    background: Some(Background::Color(Color::from_rgb(0.12, 0.12, 0.14))),
    ..Default::default()
});

// CORRECT - semantic tokens that follow the active theme.
container(text("Continue Watching")).style(|theme: &Theme| container::Style {
    background: Some(Color::from(theme.cosmic().background.component.base).into()),
    ..Default::default()
});
```

The theming API in this crate:

- `theme::active()` — the current `Theme`.
- `theme.cosmic()` — the COSMIC theme; palette lives here (`theme.cosmic().accent.base`,
  `.background`, …). This is the dominant pattern in `style.rs`.
- `theme::spacing()` — the spacing scale. Destructure what you need:
  `let Spacing { space_l, space_s, .. } = theme::spacing();`
  Available steps: `space_xxs`, `space_xs`, `space_s`, `space_m`, `space_l`, `space_xl`.
- Interaction states: `theme.hovered`, `theme.pressed`, `theme.disabled`, `theme.active`,
  `theme.transparent`.

A translucent scrim (e.g. `Color::from_rgba(0.0, 0.0, 0.0, alpha)` for a dimming layer behind a
dialog) is legitimate — it is an overlay, not a themed surface. Anything that paints *content* must
come from tokens.

**Sizes are tokens too.** `style.rs` owns text sizes (`TEXT_TITLE`, `TEXT_HEADER`, `TEXT_LARGE`,
`TEXT_BODY`, `TEXT_CAPTION`, `TEXT_RAIL_LABEL`), window geometry, sidebar ratios, and transition
durations, plus measured helpers (`sidebar_header_height()`, `content_horizontal_padding()`). Use
them; do not introduce a new magic number in a view. A token names the surface it belongs to: the
grid tile's own label size and scrim live with the tile
(`widgets/application/style.rs::TEXT_TILE_LABEL`, `::tile_label_overlay`) and are not the rail's.

## CRITICAL RULE 2: Back is one global contract

Navigation is controller-first. **Back is handled in exactly one place, globally.** A per-screen
Escape or Back handler that bypasses the global one is a product bug, not a shortcut — it makes
Back behave differently depending on where the user is.

If a new screen needs different Back behavior, change the global handler's state machine. Do not
add a local key check in the view.

## The MVU contract

This crate implements `cosmic::Application` in `app.rs`. The shapes you must preserve:

```rust
impl cosmic::Application for HearthDeck {
    type Message = Message;

    fn init(core: Core, _flags: Args) -> (Self, iced::Task<cosmic::Action<Self::Message>>) { … }
    fn update(&mut self, message: Message) -> Task<Self::Message> { … }
    fn view<'a>(&'a self) -> Element<'a, Message> { … }
    fn view_window<'a>(&'a self, id: SurfaceId) -> Element<'a, Message> { … }
    fn subscription(&self) -> Subscription<Message> { … }
    fn title(self) -> String { … }
}
```

- **`update` is a pure state transition.** It mutates `self` and returns a `Task` describing any
  effect. It must not block, sleep, or do I/O inline.
- **`view` takes `&self`.** It must not mutate, and it must not clone the world.
- **`Message` is an event, not a command.** Name it for what happened (`TitleSelected(TitleId)`,
  `MetadataLoaded { id, result }`), not for what you want done. The `update` arm decides the
  response. A message like `Message::FetchMetadata` hides a decision that belongs in `update`.
- **Effects are returned, not performed.** `update` returns the `Task`; the runtime runs it.

```rust
// WRONG - blocks the UI thread inside update; the whole app freezes on a slow daemon.
Message::Refresh => {
    let titles = reqwest::blocking::get(url).unwrap().json().unwrap();
    self.titles = titles;
    Task::none()
}

// CORRECT - update records intent and returns the effect.
Message::RefreshRequested => {
    self.loading = true;
    Task::perform(fetch_titles(url), |result| Message::TitlesLoaded(result))
}

Message::TitlesLoaded(result) => {
    self.loading = false;
    match result {
        Ok(titles) => self.titles = titles,
        Err(err) => tracing::warn!(error = %err, "title refresh failed"),
    }
    Task::none()
}
```

## Tasks

`Task::none()` is the no-op — 144 uses in this crate, and the right return for a purely local state
change. `Task::perform(future, map)` runs an async effect. `Task::batch(vec![…])` runs several.

- Return `Task::none()` when nothing external happens. Do not invent busywork.
- Do not `tokio::spawn` from `update` — 7 `spawn(` calls exist, and each one is a detached task
  whose error nobody sees. Prefer `Task::perform`, which keeps the result in the message loop.
- Map every fallible effect result back into a `Message` so failures reach `update` and can be
  surfaced and logged.

## Subscriptions

`subscription(&self) -> Subscription<Message>` is called on every state change, so it must be
cheap and must only subscribe to what is currently needed. Combine with `Subscription::batch`.

```rust
// CORRECT - animation frames are subscribed only while something is animating.
let mut subs = vec![gamepad_subscription()];
if self.is_animating() {
    subs.push(window::frames().map(|(_window, at)| Message::Animate(at)));
}
Subscription::batch(subs)
```

- Gate high-frequency streams (`window::frames()`, gamepad polling) on actual need. An
  unconditional frame subscription keeps the compositor busy and burns battery on a TV box.
- Use `Subscription::run_with((), |_| …)` for a stateful stream that should survive across
  rebuilds.
- Never open a socket or spawn a thread directly in `subscription`; return the stream.

## Widgets: reuse before you build

`widgets/` already provides `application`, `controller_key`, `menu`, `rail`, and `transition`. The
crate uses `row!`/`column!` macros, `widget::scrollable`, `widget::icon`, `widget::stack`,
`widget::operation`, and `widget::dialog`. `reorderable_flex_row` is available from
`cosmic::widget`.

Before writing a new widget, check whether one of those or a `cosmic::widget` primitive already
does it. A near-duplicate widget is how a UI drifts.

- Build layout with `row!` / `column!` and `Element<'_, Message>`; do not hand-roll flex maths.
- Compose with `.push()` / `.extend()` over allocating intermediate `Vec<Element>` where a
  builder chain reads at least as well.
- The `Element<'a, Message>` lifetime is tied to `&'a self`. If you find yourself calling
  `.clone()` inside `view` to satisfy it, you are fighting the design — borrow instead.

## Where new code goes

`app.rs` is already ~7,500 lines and `update` alone runs over a thousand. **Do not grow it.**

- A new screen gets its own `view_*` method **and** its state in a dedicated module, not another
  200-line arm inside `update`.
- Pure logic — filtering, sorting, formatting, deriving rail contents — belongs in a free
  function or an `impl` block that takes `&self` data, **outside** `app.rs`, so it can be unit
  tested without a compositor. This is the single highest-leverage habit for making this frontend
  testable; see the `testing-and-validation` skill.
- `app_group.rs`, `launch_state.rs`, `input_ownership.rs`, `icon_cache.rs`, and
  `providers/` are the model for that split.

## Gamepad

- Gamepad state arrives as a `Subscription<GamepadEvent>` from `subscriptions/gamepad.rs` and is
  translated into `Message`s. Views never read input devices directly.
- Do not add a local keybind that shadows a global one. Input routing goes through
  `input_ownership.rs` so overlays and the context menu can capture input correctly.
- New binds need a discoverable hint; the UI surfaces key-caps via `widgets/controller_key.rs`.

## Checklist before reporting done

- [ ] No `Color::from_rgb` / literal color for any themed surface — tokens only.
- [ ] No new magic sizes; `style.rs` tokens or `theme::spacing()`.
- [ ] No per-screen Escape/Back handling.
- [ ] `update` still blocks on nothing; all effects returned as `Task`.
- [ ] Every fallible effect maps its result back into a `Message`.
- [ ] New subscriptions are gated on actual need.
- [ ] `view` does not clone large state.
- [ ] Reused an existing widget rather than adding a near-duplicate.
- [ ] New logic lives in a testable module, not a new arm in `app.rs`.
- [ ] `just check-fast hearthdeck-frontend` is green.
