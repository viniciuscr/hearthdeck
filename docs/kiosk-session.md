# Hearthdeck Kiosk Session

The Arch package's Kiosk session is deliberately a short direct Gamescope
chain:

```text
Display manager
  -> /usr/lib/hearthdeck/hearthdeck-session
  -> systemctl --user start hearthdeck.target
  -> gamescope --backend drm --fullscreen --force-grab-cursor --expose-wayland
  -> /opt/hearthdeck/hearthdeck
```

Hearthdeck is the outer Gamescope instance's only graphical child. Do not add a
desktop compositor, session manager, panel, launcher, wallpaper, notification
service, or overlay client to that outer session.

The Flutter runner only publishes the outer Gamescope Wayland socket to
`$XDG_RUNTIME_DIR/hearthdeck/gamescope-wayland-display`. The bridge uses it to
place a managed app or game in a separate nested Gamescope instance. This keeps
game launch lifetime separate from Hearthdeck itself.

The native overlay is not started by Hearthdeck's runner. Starting it as a
persistent outer Gamescope client previously caused Hearthdeck to render at a
fraction of the display size. Keep it manual until it is launched with a nested
game session.

For a TTY recovery check, log in as the target user and run:

```sh
/usr/lib/hearthdeck/hearthdeck-session
```

Then inspect the services from another terminal or after returning to a desktop:

```sh
systemctl --user status hearthdeck.target hearthdeck-bridge.socket hearthdeck-daemon.service
journalctl --user -u hearthdeck-daemon.service -u hearthdeck-bridge.service --since '10 minutes ago'
```
