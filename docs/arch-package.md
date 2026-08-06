# Arch Linux Package

The `hearthdeck` package targets `x86_64` Arch Linux derivatives, including
CachyOS. It installs:

- `/opt/hearthdeck/`: the Flutter Linux client bundle.
- `/usr/bin/hearthdeck`: the desktop launcher command.
- `/usr/lib/hearthdeck/`: the local bridge and daemon binaries, and the
  Kiosk session script.
- `/usr/lib/systemd/user/`: the Hearthdeck target, bridge socket, bridge, and
  API daemon user units.
- `/usr/share/applications/`: the Hearthdeck desktop entry and icon.
- `/usr/share/wayland-sessions/hearthdeck.desktop`: the minimal Hearthdeck
  Kiosk session shown by compatible display managers.
- `/usr/lib/hearthdeck/hearthdeck-overlay` and
  `/usr/lib/systemd/user/hearthdeck-overlay.service`: Guide-button-toggled
  quick-menu overlay, started only by the separate **COSMIC (Test)** session;
  it is never started in the Hearthdeck Kiosk (Gamescope) session. The session
  installs a COSMIC custom shortcut, `Super+Shift+H`, which runs
  `hearthdeck-overlay --toggle`. It does not replace an existing user binding.
- `/usr/lib/systemd/user/cosmic-panel.service`: real `cosmic-panel` top bar
  (time, network, Bluetooth, etc.), started only by the separate
  **COSMIC (Test)** session alongside Hearthdeck (or the spike app below) - see
  `packaging/arch/cosmic-test-session` for the config it force-applies on
  every login (`Panel` only, no `Dock`; `autohide=OnOverlap` so the bar
  hides itself behind a fullscreen game and stays visible behind
  Hearthdeck, which maximizes rather than fullscreens in this session -
  `linux/runner/my_application.cc`; dark theme mode, not time-of-day
  auto-switching). This overwrites those specific COSMIC Settings keys on
  every login, deliberately, rather than seeding them once - see the
  script's own comments for why. Requires the optional `cosmic-panel`
  package; the Hearthdeck Kiosk (Gamescope) session never starts it.
- `/usr/bin/hearthdeck-cosmic-spike`: disposable libcosmic UI spike. The
  **COSMIC (Test)** session launcher prefers this binary when present, and
  falls back to `/opt/hearthdeck/hearthdeck` when it is not.
- `/usr/bin/io.github.viniciuscr.hearthdeck.AppletUser` and
  `/usr/share/applications/io.github.viniciuscr.hearthdeck.AppletUser.desktop`:
  a small `cosmic-panel` applet showing the logged-in username, added to the
  panel's left wing by `cosmic-test-session`. Both files are required -
  `cosmic-panel` finds applets by `.desktop` filename, not by binary name
  alone; see `docs/cosmic-panel-customization.md`. COSMIC (Test) session
  only, same as the panel itself.
- `/usr/bin/hearthdeck-overlay-spike`: **temporary**, not a real feature.
  Disposable hardware-verification tool for an in-progress investigation
  into a system-wide overlay menu; see `services/hearthdeck-overlay-spike`
  and `docs/kiosk-session.md`. Remove this bullet once that crate is
  deleted.

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

See `docs/kiosk-session.md` for the full startup sequence, an incident
writeup of exactly how this session broke once already (and how not to
repeat it), and what not to change.

**Hearthdeck Kiosk** is a plain Gamescope session with no desktop shell: no
panel, launcher, wallpaper, notifications, or settings daemon. Select it in the
display manager, or configure it as the autologin session, to boot straight
into Hearthdeck fullscreen with the lowest possible memory and CPU footprint.

The session script (`/usr/lib/hearthdeck/hearthdeck-session`) starts
`hearthdeck.target` for the current user and then execs Gamescope directly on
the DRM/KMS seat with Hearthdeck as its only child (`gamescope --backend drm
--fullscreen -- /opt/hearthdeck/hearthdeck`). There is no intermediate desktop
compositor to initialize first, and no other process for Gamescope to share
the seat with. Exiting Hearthdeck ends Gamescope and returns to the display
manager's login screen; there is no underlying desktop to fall back to.

Hearthdeck launches registered desktop applications and RetroArch games as
direct clients of that same outer Kiosk session compositor - its embedded
Xwayland `DISPLAY` for X11 apps, its Wayland socket for native-Wayland apps -
rather than a nested Gamescope instance of their own. That used to be the
default and was confirmed, on real hardware, to never actually get shown: a
second Gamescope process joining this session as a Wayland peer is
composited but never focused/displayed, while a plain client is shown
automatically the same way Hearthdeck itself is. See
`docs/kiosk-session.md` for the full account.

Heroic game launches work in Kiosk mode too, but Heroic itself - not each
individual game - is the resource being managed: the bridge tracks it under
one stable, reused systemd unit (`hearthdeck-heroic.service`) instead of a
fresh one per launch, because Electron's single-instance lock means any
launch after the first is handled by the same already-running Heroic process
rather than a new one. Heroic is the one launch that keeps a nested Gamescope
instance of its own - using the SDL backend, connecting through this
session's `DISPLAY` and presenting as an ordinary X11 client rather than a
Wayland peer, specifically so its games can still get their own internal
resolution/upscaling. Heroic is left running between games on purpose
(faster subsequent launches, at the cost of some idle memory); closing it -
and whatever game it's currently running - is a single `systemctl --user
stop hearthdeck-heroic.service`, which reliably tears down the whole process
tree via its cgroup even though Heroic never exits on its own. See
`services/README.md` for the full reasoning.

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
