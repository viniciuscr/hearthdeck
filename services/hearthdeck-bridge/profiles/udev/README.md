# Bundled RetroArch joypad autoconfig profiles (udev driver)

These `.cfg` files are the Linux `udev` input-driver profiles from the
upstream libretro project:

- Source: https://github.com/libretro/retroarch-joypad-autoconfig (`udev/`)
- Revision vendored: `033151045d378b64e712a92592467800d7924227` (2026-08-19)
- License: MIT, Copyright (c) 2019 The RetroArch team (same license as Hearthdeck)

RetroArch matches a connected pad to one of these files by vendor ID,
product ID and device name, then applies the RetroPad (physical button ->
virtual RetroPad) mapping. Distro-packaged RetroArch on Arch ships *no*
profiles (`/usr/share/libretro/autoconfig` is empty unless a separate
`retroarch-joypad-autoconfig` package is installed), so a Hearthdeck-managed
RetroArch install sees every pad as "detected but not configured".

`build.rs` embeds every `.cfg` here into the `hearthdeck-bridge` binary;
`launch_retro_game` (platform/linux.rs) seeds them into
`~/.config/hearthdeck/retroarch/autoconfig/udev/` on each launch.

The bridge rewrites one key on the way to disk: `input_menu_toggle_btn` is
written unbound (`"nul"`). These profiles bind each pad's Guide/Home button as
RetroArch's single-press menu toggle, and that button is Hearthdeck's own — it
opens the quick-menu overlay — so RetroArch's menu is reached with
**Start + Select** instead (the combo the managed config pins; see decision 9 in
`docs/retroarch-integration.md`). The rewrite has to happen in the profile
rather than in Hearthdeck's config because RetroArch applies autoconfig when a
pad is detected, after it has read its config files.

The files in *this* directory stay byte-identical to upstream, so the refresh
commands below keep working and `build.rs` does not need to know about the
rewrite. A copy already seeded on disk is rewritten only when it is one
Hearthdeck wrote — the vendored profile verbatim (what releases before the
rewrite left behind), or that profile rewritten. Anything else is the user's:
the autoconfig directory doubles as the target of RetroArch's own controller
wizard ("Save Controller Profile"), so a mapping the user re-saved or fixed
survives the next launch.

To refresh from upstream:

```
git clone --depth 1 https://github.com/libretro/retroarch-joypad-autoconfig /tmp/joypad-autoconfig
rm services/hearthdeck-bridge/profiles/udev/*.cfg
cp /tmp/joypad-autoconfig/udev/*.cfg services/hearthdeck-bridge/profiles/udev/
# update the revision line above from: git -C /tmp/joypad-autoconfig rev-parse HEAD
```
