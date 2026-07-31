# Arch Linux Package

The `hearthdeck` package targets `x86_64` Arch Linux derivatives, including
CachyOS. It installs:

- `/opt/hearthdeck/`: the Flutter Linux client bundle.
- `/usr/bin/hearthdeck`: the desktop launcher command.
- `/usr/lib/hearthdeck/`: the local bridge, daemon, and native overlay binaries,
  plus the Kiosk session script.
- `/usr/lib/systemd/user/`: the Hearthdeck target, bridge socket, bridge, and
  API daemon user units.
- `/usr/share/applications/`: the Hearthdeck desktop entry and icon.
- `/usr/share/wayland-sessions/hearthdeck.desktop`: the minimal Hearthdeck
  Kiosk session shown by compatible display managers.

## Install

Install the initial `hearthdeck-*.pkg.tar.zst` from the GitHub Actions artifact:

```sh
sudo pacman -U hearthdeck-*.pkg.tar.zst
```

The package enables `hearthdeck.target` globally for the local daemon and
bridge. Launching Hearthdeck starts the target for the current user
immediately. To run the daemon before opening the client, use:

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
typed request. Future network and Bluetooth bridges will follow the same
target-plus-socket lifecycle.

If you previously used `just install-services`, its copies in
`~/.config/systemd/user/` override the package units. Move
`hearthdeck-bridge.service` and `hearthdeck-daemon.service` aside before
enabling the packaged target, then preserve any local customization in a
systemd drop-in.

## Kiosk session

See `docs/kiosk-session.md` for the full startup sequence, how backend
readiness is guaranteed before Hearthdeck launches, and what not to change.

**Hearthdeck Kiosk** is a plain Gamescope session with no desktop shell: no
panel, launcher, wallpaper, notifications, or settings daemon. Select it in the
display manager, or configure it as the autologin session, to boot straight
into Hearthdeck fullscreen with the lowest possible memory and CPU footprint.

The session script (`/usr/lib/hearthdeck/hearthdeck-session`) starts
`hearthdeck.target` for the current user and then execs Gamescope directly on
the DRM/KMS seat with Hearthdeck as its only child (`gamescope --backend drm
--fullscreen --expose-wayland -- /opt/hearthdeck/hearthdeck`). There is no intermediate desktop
compositor to initialize first, and no other process for Gamescope to share
the seat with. Exiting Hearthdeck ends Gamescope and returns to the display
manager's login screen; there is no underlying desktop to fall back to.

Hearthdeck's own runner does not start any other graphical process. A
previous version had it launch the native in-game overlay (see below) as its
own child; that made the overlay a second Wayland client of this same outer
Gamescope instance for the entire time Hearthdeck was running, which
prevented Gamescope from sizing its output to Hearthdeck alone (Hearthdeck
rendered into a fraction of the real screen). See `docs/kiosk-session.md` for
the full incident note — the overlay must only ever run as part of a nested
game/app launch, never here.

Hearthdeck launches registered desktop applications in a separate, on-demand
nested Gamescope instance. That nested instance is unrelated to the outer
Kiosk session compositor above: it is started by the bridge only when a game
or app launch is requested, uses no DRM or memory until then, and is torn down
when the launch ends. X11-only apps use the nested Gamescope's Xwayland
server; Wayland apps use its exposed inner Wayland socket. The native overlay
binary (`/usr/lib/hearthdeck/hearthdeck-overlay`) shows an in-game
resume/close menu above a running game; it has no automatic startup wired up
yet and can be run manually via `just overlay` for development.


Heroic game URI launches are unavailable in Kiosk mode because an existing
Heroic process can accept a URI and detach the game from Hearthdeck's managed
Gamescope lifecycle.

Controller input is Hearthdeck's direct Linux joystick reader; there is no
desktop shell input stack to coordinate with. PipeWire/WirePlumber provide
audio, while NetworkManager and BlueZ remain system services. Their existing
connections and paired devices continue to work, but Hearthdeck does not yet
provide Wi-Fi, Bluetooth, or audio-routing configuration interfaces, and the
Kiosk session does not start a polkit agent, so NetworkManager changes needing
authentication are unavailable from it.

`gamepad-osk` remains optional and external to this package. It must be running
as its upstream daemon service and have its uinput/input permissions configured
before `gamepad-osk --toggle` can provide an OSK. Its evdev grab does not
necessarily suppress Hearthdeck's direct joystick reader, so OSK input isolation
is not guaranteed until controller input is unified behind one process.

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
