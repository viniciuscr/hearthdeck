# Arch Linux Package

The `hearthdeck` package targets `x86_64` Arch Linux derivatives, including
CachyOS. It installs:

- `/opt/hearthdeck/`: the Flutter Linux client bundle.
- `/usr/bin/hearthdeck`: the desktop launcher command.
- `/usr/lib/hearthdeck/`: the local bridge and daemon binaries.
- `/usr/lib/systemd/user/`: the Hearthdeck target, Kiosk launcher, bridge
  socket, bridge, and API daemon user units.
- `/usr/share/applications/`: the Hearthdeck desktop entry and icon.
- `/usr/share/wayland-sessions/hearthdeck-cosmic.desktop`: a COSMIC Kiosk
  session shown by compatible display managers.

## Install

Install the initial `hearthdeck-*.pkg.tar.zst` from the GitHub Actions artifact:

```sh
sudo pacman -U hearthdeck-*.pkg.tar.zst
```

The package enables `hearthdeck.target` globally for the local daemon and
bridge, and enables `hearthdeck-kiosk.service` for `cosmic-session.target`.
The Kiosk service has a desktop-name condition, so it starts only from the
**Hearthdeck Kiosk** session. Launching Hearthdeck starts the target for the
current user immediately. To run the daemon before opening the client, use:

```sh
systemctl --user daemon-reload
systemctl --user start hearthdeck.target
```

Installation adds the managed `/etc/pacman.d/hearthdeck.conf` and an include in
`/etc/pacman.conf`. Future Hearthdeck packages therefore arrive with the normal
system update:

```sh
sudo pacman -Syu
```

The repository is published to GitHub Pages after each successful `main` build.
It is currently unsigned and uses `SigLevel = Optional TrustAll`. Installing
the initial package accepts GitHub Pages over HTTPS as the package trust
boundary; package signing can replace this later.

Pacman enables the target globally for future user sessions but cannot safely
start a currently logged-in user's manager during a root package transaction.
The launcher handles that activation on demand. The units retain
`NoNewPrivileges`, but do not use mount-namespace sandboxing:
Arch systemd user units cannot reliably support directives such as
`ProtectSystem`, `ReadWritePaths`, or `PrivateTmp`.

`hearthdeck.target` starts the API daemon and owns the
`hearthdeck-bridge.socket`; the bridge process is socket-activated on its first
typed request. `hearthdeck-kiosk.service` is the graphical client unit and is
started by COSMIC only after its compositor has initialized the graphical
session. Future network and Bluetooth bridges will follow the same
target-plus-socket lifecycle.

If you previously used `just install-services`, its copies in
`~/.config/systemd/user/` override the package units. Move
`hearthdeck-bridge.service` and `hearthdeck-daemon.service` aside before
enabling the packaged target, then preserve any local customization in a
systemd drop-in.

The Kiosk session runs the distribution's `start-cosmic` entrypoint. This is
intentional: COSMIC performs the DRM/logind handoff, initializes its compositor,
and imports the Wayland socket into the systemd user manager. Once that work is
complete, `cosmic-session.target` starts `hearthdeck-kiosk.service`, which runs
the Flutter GTK client with `GDK_BACKEND=wayland`.

COSMIC's panel, launcher, applets, wallpaper, notifications, and settings
services remain active. The package does not attempt to replace or terminate
them because `cosmic-session` supervises and restarts its components. Hearthdeck
runs fullscreen above the desktop. Select **Hearthdeck Kiosk** in the display
manager, or configure it as the autologin session, for this direct-to-Hearthdeck
experience. Closing Hearthdeck reveals the COSMIC desktop instead of ending the
working Wayland session.

Hearthdeck launches registered desktop applications in nested Gamescope using
cosmic-comp's Wayland socket. Gamescope therefore uses no DRM or memory until
an app is launched, and it never competes with cosmic-comp for the seat. X11-only apps use
the nested Gamescope Xwayland server; Wayland apps use its exposed inner Wayland
socket.

Heroic game URI launches are unavailable in Kiosk mode because an existing
Heroic process can accept a URI and detach the game from Hearthdeck's managed
Gamescope lifecycle.

Controller input remains Hearthdeck's direct Linux joystick reader, independent
of COSMIC's desktop shell. PipeWire/WirePlumber provide audio, while NetworkManager and BlueZ
remain system services. Their existing connections and paired devices continue
to work, but Hearthdeck does not yet provide Wi-Fi, Bluetooth, or audio-routing
configuration interfaces. NetworkManager changes needing authentication require
a polkit agent; the Kiosk session intentionally does not start one.

`gamepad-osk` remains optional and external to this package. It must be running
as its upstream daemon service and have its uinput/input permissions configured
before `gamepad-osk --toggle` can provide an OSK. Its evdev grab does not
necessarily suppress Hearthdeck's direct joystick reader, so OSK input isolation
is not guaranteed until controller input is unified behind one process.

Use **Settings > General > Exit to desktop** inside Hearthdeck Kiosk. Confirm
the prompt to close Hearthdeck and reveal the COSMIC desktop.

The bridge scans the target machine's Freedesktop entries and launches only a
re-discovered desktop entry. It honors desktop visibility constraints, `Path`,
and `TryExec`, and rejects terminal and D-Bus-activated entries because those
cannot be managed safely in the kiosk. Linux launches are placed in transient
systemd user services, allowing Hearthdeck to query and stop the active managed
session even if the bridge restarts.
The daemon maintains the SQLite catalog at `~/.local/share/hearthdeck/hearthdeck.db`
and exposes a loopback API at `127.0.0.1:38400`.

Discovery follows the XDG desktop-entry locations, including
`/usr/share/applications`, the user's XDG data directory, and Flatpak's user
and system exports. Executables without an exported `.desktop` entry are not
treated as launchable applications.

The packaged client obtains a fresh loopback pairing token on each app launch.
It does not expose the pairing-code endpoint to the LAN. Full Library and the
Library rescan setting use this local catalog; the dashboard's current shelves
remain sample content.

## Verify

```sh
systemctl --user status hearthdeck.target hearthdeck-bridge.socket hearthdeck-daemon.service
curl http://127.0.0.1:38400/v1/health
hearthdeck
```

Use `/usr/share/doc/hearthdeck/ACCEPTANCE.md` to verify discovery and launching
in the active graphical session. To enable LAN access, create
`~/.config/hearthdeck/daemon.env` from
`/usr/share/doc/hearthdeck/daemon.env.example`, configure TLS paths, and
restart the daemon.
