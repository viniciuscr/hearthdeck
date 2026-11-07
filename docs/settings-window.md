# The settings window

Settings is a **separate process**: `/usr/lib/hearthdeck/hearthdeck-settings`,
drawn with GPUI, launched by the library app when the Settings destination is
opened. This doc owns that decision, the rollback, and the contract the window
keeps.

## Why two processes

GPUI renders through its own window and event loop. There is no host/renderer
seam between it and libcosmic/iced — no iced `Element` can contain a gpui tree —
so a screen cannot be half-migrated inside one window. The split is therefore
drawn where it is cheap: **the window only reads.** RomM sign-in, Stremio
linking, categorization and BIOS sync stay in the iced page, which already owns
those flows and their daemon calls.

`services/hearthdeck-settings/` is the window. `app/settings_window.rs` is the
shell's half: which binary, whether to launch it, whether one is already up.

## The contract

- **Opening Settings does not change page.** The library app stays where it is
  and the window appears over it; when the window closes, focus returns with the
  shell untouched. One Back press, one result.
- **Escape, the pad's Back and the Close button all close the window.** It is
  read-only, so there is nothing to discard.
- **The window pairs with the daemon itself** — the same loopback handshake
  `hearthdeck_client` performs — so no token crosses the process boundary.
- **Nothing degrades to a dead end.** A daemon that does not answer renders
  "Unavailable" rows; a binary that will not start falls back to the in-app page
  (tested in `settings_window.rs`).

## Rollback

Two independent ways back to the in-app settings page, both no-rebuild:

```sh
HEARTHDECK_SETTINGS_UI=iced hearthdeck-frontend    # keep the page
rm /usr/lib/hearthdeck/hearthdeck-settings         # or remove the binary
```

`HEARTHDECK_GPUI_SETTINGS=/path/to/binary` points the shell at an uninstalled
build instead of the packaged one.

## Packaging

The crate is its own workspace on purpose: GPUI's dependency tree would
otherwise be compiled into every `just check`, every pre-push and every CI run,
for a window that shares no code with the daemon. `packaging/arch/PKGBUILD`
builds it with `--manifest-path services/hearthdeck-settings/Cargo.toml` and
installs the binary beside the other Hearthdeck binaries.

## Known limits

- **Untested on the deployment box** as of this change: whether the compositor
  hands gamepad focus to the window when it opens and back when it closes, and
  whether `Escape`/Back closes it from a pad. Both need the TV and a controller.
- **The write paths are still in iced.** Turning this into a full replacement
  means porting four flows (RomM, Stremio link, categorization, readiness) over
  the daemon API, not just the shell.
- The window reads once, at launch. Reopening it is the refresh.
