# Spike: gpui-kit progress in the sidebar's storage footer

**Verdict: the component is good and cheaper than what we ship. The blocker is
not the component, it is that gpui cannot live inside the iced app.**

The spike is `spikes/gpui-storage-progress/` — its own crate, its own workspace
and lockfile, nothing from it reaches `services/`, `just check` or the package.
`cargo run` there opens a window with the library sidebar's storage footer; the
`preview.png` next to it is what it looks like.

## What it replaces

`screens/library/sidebar.rs` draws a flat `drive-harddisk-solidstate-symbolic`
icon beside a caption ("Storage available") and the free-space string. The spike
puts `ProgressCircle` in the icon's place, filled to the **used** percentage,
with the percentage inside it; the caption and the free-space string stay put.
Figures come from the same `statvfs` call, formatted by a copy of the same
`human_size`, so the two are directly comparable. The one new read is the
volume's total size, which a text label never needed.

The ring also animates: `gpui-kit` transitions the arc to a new value on its own
(`transition` in `progress_circle.rs`), so a periodic refresh would need no
animation code from us.

## Cost

Measured on this machine, `cargo tree -e normal`, unique crates actually built:

| tree | crates |
| --- | --- |
| current `hearthdeck-frontend` (cosmic/iced) | 718 |
| `gpui-kit` 0.9 (facade: gpui + base + components) | 664 |
| `gpui-component` 0.9 alone | 474 |
| bare `gpui` 0.3.11 + x11/wayland | 309 |

Cold build of the spike: **3m29s** debug. Incremental (one file, relink): ~3s.
No system dependency the box did not already have. `gpui-kit` pins
`gpui == 0.3.11` (published as `gpui-pre`), so the version we get is the whole
GPUI family at one exact version — good for reproducibility, and it means the
kit follows GPUI releases rather than us.

## The trap

gpui renders through its own window and event loop. There is no way to mount a
gpui element inside a libcosmic/iced `Element`, so "use this component in the
library screen" cannot be a one-screen change. The realistic paths:

1. **Keep iced, match the look.** A circular progress is an arc plus a track;
   iced can draw that today with `canvas`. Gives the better view the user asked
   for, and keeps one toolkit.
2. **Move a surface to gpui first.** A separate gpui window is only worth it if
   something is *entirely* gpui — an overlay, a settings surface — never a slice
   of an existing screen.

Only the second is a migration, and it is all-or-nothing per surface.

## Environment note

This box is a COSMIC Wayland session with rootless Xwayland. A gpui window
launched with `WAYLAND_DISPLAY` unset lands on Xwayland and screenshots fine
(`magick "x:<window id>" out.png`); screenshotting the X root returns black
because rootless Xwayland composites nothing into it. gpui's first frames log
`MESA-EGL ... failed to create dri2 screen` and a ZINK failure, then render
correctly anyway.

## The two-process split, measured

Settings cannot become a gpui *screen* — but it can become a gpui *app*. So the
second spike, `spikes/gpui-settings/`, is a real second client: its own window,
its own focus, its own GPU surface, connected to nothing. The iced side launches
it from the settings page, and only when `HEARTHDECK_GPUI_SETTINGS` names the
binary (`App::launch_gpui_settings`, covered by
`the_gpui_settings_hook_launches_only_what_the_environment_named`). With the
variable unset, the screen renders exactly as before.

```sh
just dev                                                   # the iced app
HEARTHDECK_GPUI_SETTINGS=$PWD/spikes/gpui-settings/target/debug/gpui-settings \
  ./services/target/debug/hearthdeck-frontend              # row appears on Settings
cd spikes/gpui-settings && cargo run                       # or just run it directly
```

What the run showed (both debug builds, same session):

| | RSS | threads | idle CPU |
| --- | --- | --- | --- |
| `hearthdeck-frontend` alone | 393 MB | 33 | adapter-retry artifact, see below |
| `gpui-settings` beside it | 378 MB | 35 | 19 ticks / 10 s (~2 %) |
| the pair | ~771 MB | | |

A settings window costs as much memory as the entire library app. That alone is
the answer for a TV box.

What breaks, in order of severity:

1. **Back stops being one contract.** The gpui window knows nothing about the
   library's navigation, so Escape and the pad's Back do nothing in it. The
   global handler the product contract rests on cannot reach a second process.
2. **Focus splits.** Whichever client the compositor focused takes the gamepad
   and the keyboard; the other is inert. Two focus domains, one controller.
3. **Fullscreen stacks, it does not switch.** `hearthdeck-frontend` holds a
   fullscreen reserved surface; the spike is a normal floating window over it.
   That is what is drawn in `spikes/gpui-settings/preview-iced-beside.png`.
4. **Nothing is shared.** The settings page reads four in-process flows
   (`categorization`, `stremio_link`, `romm`, `readiness`). A second process has
   to re-fetch what it needs over the daemon API and grow a config protocol; the
   spike stands in local state because that is the honest size of the problem.

Not verified here, because this box cannot inject input or reach the GPU: the
row's click (the spawn is unit-tested, not clicked), controller Back, focus
hand-off between the two clients, and any CPU figure with a real GPU. Those need
the deployment box and a pad in hand. Anything that depends on them should not
be decided from this document.

**Verdict: do not split a Back-reachable screen across two processes.** The
split is only defensible for a surface that is genuinely separate — a first-run
wizard, an overlay, a debug panel — where no Back returns to the library.
