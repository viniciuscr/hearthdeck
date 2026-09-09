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
`~/.config/hearthdeck/retroarch/autoconfig/udev/` on each launch. Existing
files are never overwritten, so a profile the user re-saves from
RetroArch's own controller wizard ("Save Controller Profile") survives.

To refresh from upstream:

```
git clone --depth 1 https://github.com/libretro/retroarch-joypad-autoconfig /tmp/joypad-autoconfig
rm services/hearthdeck-bridge/profiles/udev/*.cfg
cp /tmp/joypad-autoconfig/udev/*.cfg services/hearthdeck-bridge/profiles/udev/
# update the revision line above from: git -C /tmp/joypad-autoconfig rev-parse HEAD
```
