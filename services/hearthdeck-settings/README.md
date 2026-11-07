# hearthdeck-settings

Hearthdeck's settings window, drawn with GPUI. It ships as its own process: the
library app launches it from `/usr/lib/hearthdeck/hearthdeck-settings` when the
Settings destination is opened.

Its own workspace (though it lives under `services/`) on purpose: GPUI's
dependency tree would otherwise be compiled into every `just check`, every
pre-push and every CI run, for a window that shares no code with the daemon.
`packaging/arch/PKGBUILD` builds it with `--manifest-path`.

It depends on GPUI directly rather than through `gpui-kit`, because the facade
enables both the Wayland and X11 backends and Cargo features cannot be removed
downstream. This window is Wayland-only, so the X11 stack
(`libxkbcommon-x11`, `libxcb`) is not linked at all.

```sh
cargo run                      # the window on its own
cargo clippy --all-targets -- -D warnings
```

What it does: reads the daemon once at launch (`daemon.rs` pairs over loopback
exactly as `hearthdeck_client` does), draws status, connections and storage, and
closes on Escape, the pad's Back, or its Close button. It writes nothing —
RomM sign-in, Stremio linking, categorization and BIOS sync stay in the library
app's own settings page.

Fallbacks: with the binary missing, or with `HEARTHDECK_SETTINGS_UI=iced` set,
the library app draws its in-app settings page instead.
`HEARTHDECK_GPUI_SETTINGS=/path/to/binary` points it at an uninstalled build.

`preview.png` is this window; `preview-iced-beside.png` is the library app on the
same session, two clients side by side. See `docs/settings-window.md` for the
contract, the rollbacks and the known limits.
