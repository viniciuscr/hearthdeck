Cosmic Desktop Development
This skill provides guidelines and best practices for developing applications and applets for the System76 COSMIC Desktop Environment (COSMIC DE).

1. Overview & Technology Stack
COSMIC applications are built using Rust and libcosmic.

Language: Rust (Safe, fast, concurrent)
GUI Toolkit: libcosmic (Built on top of iced, a cross-platform GUI library inspired by Elm)
Design System: COSMIC Design System (Theming, consistent widgets)
2. Project Setup
Always start strict adherence to COSMIC standards by using official templates.

App Template
For standalone applications:

*.bash
Shell
cargo generate --git https://github.com/pop-os/cosmic-app-template
Applet Template
For panel applets:

*.bash
Shell
cargo generate --git https://github.com/pop-os/cosmic-applet-template
3. Core Development Principles
GUI Architecture (The Elm Architecture)
libcosmic (and iced) follows the Model-View-Update (MVU) pattern:

State (Model): The data structure describing the application's state.
Message: Enum variants representing user actions or events.
Update: A pure function (fn update(&mut state, message)) that modifies the state based on a message.
View: A pure function (fn view(&state) -> Element) that renders the UI based on the state.
Best Practices
Use libcosmic Widgets: Always prefer libcosmic widgets over raw iced widgets when available to ensure consistent styling and integration with the desktop theme.
Modular Design: Separate your update, view, and state logic. For complex apps, break components into sub-modules with their own MVU cycles.
Configuration: Integrate with cosmic-config for handling user settings. This ensures your app's settings persist and respect system-wide overrides.
Theming: Do not hardcode colors! Use the semantic colors provided by the cosmic-theme (e.g., theme.palette.primary, theme.palette.background). This ensures your app looks correct in both Light and Dark modes.
4. Applet Specifics
Composability: Applets often live in the panel. Keep them lightweight.
Popup vs Embedded: Decide if your applet needs a popover menu (like WiFi) or just an icon/text (like a clock).
Responsiveness: Applets must handle panel resizing gracefully.
Wayland Integration: COSMIC is Wayland-first. Ensure your applet interacts correctly with the compositor layers if doing custom windowing.
5. Important Libraries (The "Cosmic Stack")
cosmic-text: Advanced text shaping and rendering.
cosmic-config: Type-safe configuration management.
cosmic-theme: Access to system colors and metrics.
cosmic-comp: The compositor (useful for reference if interacting with window management).
6. Resources
libcosmic Source: https://github.com/pop-os/libcosmic
Official Examples: Check examples/ in the libcosmic repo.
Iced Documentation: https://docs.rs/iced (Foundational knowledge)
System76 Dev Docs: https://github.com/pop-os/cosmic-epoch
7. Troubleshooting
"Component not found": Ensure you have libcosmic features enabled in Cargo.toml.
Theming issues: Verify you are taking Theme as an argument in your view function and passing it correctly.
8. Gamepad/Controller Navigation (Kiosk & TV-style Apps)
libcosmic/iced have no native gamepad API. For an app or overlay meant to be driven by a controller (not just mouse/keyboard), read the gamepad directly via the `evdev` crate against `/dev/input/event*` and translate raw events into your own `Message` variants - there is nothing built in that does this for you.

D-pad reporting is genuinely ambiguous across controllers/drivers: some report discrete `KeyCode::BTN_DPAD_UP`/`DOWN`/`LEFT`/`RIGHT` key events, others (especially older drivers) report the D-pad as the `ABS_HAT0X`/`ABS_HAT0Y` hat axis instead (`AbsoluteAxisCode`, value -1/1 for each direction, 0 released). Read both and map them to the same navigation messages; confirm which one an actual target controller sends with `evtest` before assuming either.

`libcosmic`'s own `cosmic::keyboard_nav` module (`Action::FocusNext`/`FocusPrevious`, driven by iced's `operation::focus_next()`/`focus_previous()`) is a keyboard/Tab-order mechanism - it does not automatically respond to gamepad input just because a button widget is focusable. For a controller-first surface, don't rely on it: track your own `selected: usize` (or similar) index in the app's state, update it from your gamepad `Subscription` on D-pad up/down, and drive both the visual highlight and which action "Activate" (the A/South button) triggers directly from that index - not from focus state.

For a controller-navigable **list of choices** (as opposed to a handful of individually-clickable big buttons), prefer `cosmic::widget::list_column()` with `cosmic::widget::list::button(content).on_press(message).selected(is_selected)` per row. This renders the native COSMIC settings-list look (a bordered list with per-row hover/selected background) and the `.selected(bool)` flag is exactly the hook for showing which row your own D-pad-driven index currently highlights - no need to hand-roll list-row styling.
9. Async Background Work → Messages
To run background async work (a blocking OS call, an HTTP request, anything that shouldn't block `update`) and get a `Message` back once it finishes, return `cosmic::task::future(async { ... })` from `update` - it wraps `iced::Task::future` and lets the future's output convert directly `Into<Message>`. This is strictly better than a bare `std::thread::spawn` for anything the UI needs to react to when done (e.g. hiding a "Working..." status): a raw thread has no way to signal the `Application`'s event loop on completion, so you'd need to bolt on your own polling or shared atomic/channel state to notice it finished - `cosmic::task::future` already reports back through the normal `update(message)` path.

For CPU-bound or blocking (non-async) work specifically, wrap it in `tokio::task::spawn_blocking` inside the future passed to `cosmic::task::future`, so it runs on tokio's dedicated blocking-thread pool instead of stalling the async runtime.
10. Layer-Shell / Overlay Surfaces
For an always-on-top overlay that isn't a normal window (a quick-menu, an OSD, an in-session HUD), use the wlr-layer-shell protocol via `cosmic::surface::surface_task(app_layer_shell(...))`, configuring an `SctkLayerSurfaceSettings` (layer, anchor, `keyboard_interactivity`, `exclusive_zone`, etc.) rather than `cosmic::app::run`'s normal window. Tear it down with `destroy_layer_surface(window_id)`, not by hiding/resizing it to zero.

If the overlay needs `KeyboardInteractivity::Exclusive` (grabs all keyboard input while shown), be aware this clears the previously-focused window/toplevel's "Activated" state in the compositor's toplevel-management protocol the moment the overlay's layer surface maps. Any logic that needs to know "what was active/focused before the overlay opened" must capture that *before* calling whatever shows the overlay - querying it afterward will find nothing activated, because the overlay itself now holds keyboard focus instead.

Do not add a second Wayland-compositor process (a nested Gamescope instance, a second cosmic-comp, etc.) as a way to layer UI on top of another full-screen client - even for something that sounds like it should be "just an overlay." A second compositor joining an existing session as a Wayland peer is composited but not shown; if you need to render something above another app's own content, it has to be a client of that same compositor (layer-shell, as above), not a second compositor process.