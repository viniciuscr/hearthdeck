# COSMIC Panel & Theme Customization

How the COSMIC (Test) session's top bar (`cosmic-panel`) and dark theme get
their look, where that configuration actually lives on disk, and how
Hearthdeck controls it from code instead of a GUI. Read this before changing
`packaging/arch/cosmic-test-session`'s `apply_cosmic_overrides` function.

## There is no panel-layout import/export - only theme import/export

`cosmic-settings` (the GUI settings app) ships one real import/export
feature: `cosmic-settings appearance export <file>.ron` /
`... appearance import <file>.ron`. That covers exactly one thing - the
color/corner-radius palette (`cosmic_theme::ThemeBuilder`, `is_dark`,
`auto_switch`) - see `cosmic-settings/src/pages/desktop/appearance/commands.rs`
in the [cosmic-settings source](https://github.com/pop-os/cosmic-settings).
Panel size, anchor, autohide, and the list of applets (widgets) have **no**
equivalent export command anywhere in COSMIC. The panel's own settings page
(`cosmic-settings/src/pages/desktop/panel/`) reads and writes its
`CosmicPanelConfig` directly through `cosmic_config`, with no serialized
snapshot format of its own - dragging an applet in the GUI just rewrites the
same per-key files described below.

That's why Hearthdeck doesn't "import a file" to get its panel layout: there
is no such file to import. Instead, `cosmic-test-session` writes the same
per-key config files `cosmic-settings`/the GUI would have written, directly.

## How `cosmic-config` actually stores settings

Every COSMIC component's settings (panel, theme, app list, etc.) go through
the `cosmic-config` crate
([pop-os/libcosmic](https://github.com/pop-os/libcosmic/tree/master/cosmic-config)).
Each setting is **one plain file per key** - not one big config file - at:

```
$XDG_CONFIG_HOME/cosmic/<app-id>/v<version>/<key>
```

e.g. `~/.config/cosmic/com.system76.CosmicPanel.Panel/v1/size` containing
just `L`. The file contents are [RON](https://github.com/ron-rs/ron)
(Rusty Object Notation), one value per file - a bare enum variant like `L`
or `OnOverlap`, a RON literal like `Some(["a", "b"])`, `true`, etc. There is
no wrapping struct or JSON - `cosmic_config::Config::get::<T>(key)`
deserializes that one file's contents directly as `T`.

If a key's file doesn't exist under `$XDG_CONFIG_HOME`, `cosmic-config`
falls back to a system default under `$XDG_DATA_HOME` (normally
`/usr/share/cosmic/<app-id>/v<version>/<key>`, shipped by the `cosmic-panel`/
`cosmic-theme` packages themselves - see their `data/default_schema/` in
[pop-os/cosmic-panel](https://github.com/pop-os/cosmic-panel/tree/master/data/default_schema)).
**Writing a file under `$XDG_CONFIG_HOME` always wins over that system
default** - that's the mechanism `apply_cosmic_overrides` (below) relies on.

## The keys Hearthdeck's session controls

All of these live under `~/.config/cosmic/` and are force-written on every
`cosmic-test-session` start (see that script for the authoritative values -
this doc explains *what* each key means and *where it's documented upstream*,
not the literal current values, to avoid this file going stale):

| App ID | Key | Meaning | Format example |
|---|---|---|---|
| `com.system76.CosmicPanel` | `entries` | which named profiles (`Panel`, `Dock`) actually run | `["Panel"]` |
| `com.system76.CosmicPanel.Panel` | `size` | overall bar thickness/icon scale: `XS`/`S`/`M`/`L`/`XL`, or `Custom(u32)` | `L` |
| `com.system76.CosmicPanel.Panel` | `autohide` | `Never` / `OnOverlap` (hide behind a fullscreen window) / `Always` | `OnOverlap` |
| `com.system76.CosmicPanel.Panel` | `plugins_wings` | `Option<(Vec<String>, Vec<String>)>` - left-edge and right-edge applet lists, by ID, ordered **closest-to-center first, closest-to-screen-edge last** | `Some((["io.github.viniciuscr.hearthdeck.AppletUser"], ["com.system76.CosmicAppletBluetooth", "com.system76.CosmicAppletNetwork", "com.system76.CosmicAppletTime"]))` |
| `com.system76.CosmicPanel.Panel` | `plugins_center` | `Option<Vec<String>>` - center-anchored applets | `None` |
| `com.system76.CosmicTheme.Mode` | `is_dark` | dark vs. light palette | `true` |
| `com.system76.CosmicTheme.Mode` | `auto_switch` | follow the system clock's day/night schedule instead of `is_dark` | `false` |

`CosmicPanelConfig`'s full field list (padding, spacing, border radius,
margin, opacity, background, exclusive zone, etc. - anything not in the table
above still uses the upstream system default) is documented in
[`cosmic-panel-config/src/panel_config.rs`](https://github.com/pop-os/cosmic-panel/blob/master/cosmic-panel-config/src/panel_config.rs).
Applet IDs are just their Flatpak-style app ID
(`com.system76.CosmicApplet<Name>`) - the full built-in set ships in
[`cosmic-panel`'s default `plugins_wings`/`plugins_center`](https://github.com/pop-os/cosmic-panel/blob/master/data/default_schema/com.system76.CosmicPanel.Panel/v1/plugins_wings).

## A custom applet also needs a matching `.desktop` file - the ID alone isn't enough

Putting a custom app ID in `plugins_wings`/`plugins_center` is not sufficient
by itself. Both the pieces that need to recognize that ID read from
`.desktop` files, not from the running binary's name alone:

- **`cosmic-panel` itself** (the process that actually renders the bar and
  spawns each applet) scans the standard XDG desktop-entry directories
  (`freedesktop_desktop_entry::default_paths()`, e.g.
  `/usr/share/applications`) for a file whose **filename (without
  `.desktop`) equals the app ID**, then spawns whatever that file's `Exec=`
  line says - see
  [`cosmic-panel-bin/src/space/wrapper_space.rs`](https://github.com/pop-os/cosmic-panel/blob/master/cosmic-panel-bin/src/space/wrapper_space.rs).
  An app ID with no matching `.desktop` file is silently dropped - no error,
  nothing spawned, no log beyond a debug-level trace.
- **`cosmic-settings`' own "add a widget" applet picker** (the panel settings
  page's available-applet list) separately scans the same desktop-entry
  directories and only lists a file if it declares the extra key
  `X-CosmicApplet=true` - see
  [`cosmic-settings/src/pages/desktop/panel/applets_inner.rs`](https://github.com/pop-os/cosmic-settings/blob/master/cosmic-settings/src/pages/desktop/panel/applets_inner.rs).
  Without that key (or the file), the applet can still be made to run via
  `plugins_wings` directly, but never appears as something a user could add
  themselves through the GUI.

So a custom applet needs, at minimum, one `.desktop` file named
`<app-id>.desktop` (e.g. `packaging/arch/hearthdeck-applet-user.desktop`,
installed as `/usr/share/applications/io.github.viniciuscr.hearthdeck.AppletUser.desktop`
- see `PKGBUILD`), with:

```ini
[Desktop Entry]
Type=Application
Name=<shown in the "add widget" picker>
Exec=<the installed binary's name, resolved via $PATH>
NoDisplay=true
X-CosmicApplet=true
```

`NoDisplay=true` keeps it out of a normal application launcher/menu (it's
only meant to be added as a panel widget, not launched directly by a user).

## Extending this

To change panel size, autohide, applets, or theme mode for the COSMIC (Test)
session: edit `apply_cosmic_overrides()` in
`packaging/arch/cosmic-test-session` - add a `printf` line for the new key,
following the table above for its RON format. Do not add a settings-import
step or expect a snapshot file to restore from; there isn't one upstream to
import, by design of `cosmic-config` itself. If the change adds a new custom
applet, it also needs its own `.desktop` file per the section above - the
config key alone will silently do nothing.

This intentionally always overwrites these specific keys on every session
start rather than seeding them once - see that function's own comment for
why (a once-only seed silently no-ops on any account that already has a
real COSMIC desktop login with its own files in place).
