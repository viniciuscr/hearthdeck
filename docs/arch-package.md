# Arch Linux Package

The `hearthdeck` package targets `x86_64` Arch Linux derivatives, including
CachyOS. It installs:

- `/opt/hearthdeck/`: the Flutter Linux client bundle.
- `/usr/bin/hearthdeck`: the desktop launcher command.
- `/usr/lib/hearthdeck/`: the local bridge and daemon binaries.
- `/usr/lib/systemd/user/`: the Hearthdeck target, bridge socket, bridge, and
  API daemon user units.
- `/usr/share/applications/`: the Hearthdeck desktop entry and icon.
- `/usr/share/wayland-sessions/hearthdeck-gamescope.desktop`: the direct DRM
  console session shown by compatible display managers.

## Install

Install the initial `hearthdeck-*.pkg.tar.zst` from the GitHub Actions artifact:

```sh
sudo pacman -U hearthdeck-*.pkg.tar.zst
```

The package enables `hearthdeck.target` globally, so it starts at the next user
login. Launching Hearthdeck starts the target for the current user immediately.
To run the daemon before opening the client, use:

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

`hearthdeck.target` is the only unit users enable. It starts the API daemon and
owns the `hearthdeck-bridge.socket`; the bridge process is socket-activated on
its first typed request. In Console, Gamescope directly starts Hearthdeck as
its primary Xwayland client. The client publishes that live display to the user
manager before restarting the bridge, so managed applications use the same
compositor. Future network and Bluetooth bridges will follow this same
target-plus-socket lifecycle.

If you previously used `just install-services`, its copies in
`~/.config/systemd/user/` override the package units. Move
`hearthdeck-bridge.service` and `hearthdeck-daemon.service` aside before
enabling the packaged target, then preserve any local customization in a
systemd drop-in.

`gamescope` is a runtime dependency, installed by pacman with Hearthdeck. It
is intentionally not bundled: Gamescope needs to match the host's Mesa,
Vulkan, DRM, and kernel graphics stack. Select **Hearthdeck Console** in the
display manager, or configure it as the autologin session, for the minimal
direct-to-display experience. A normal desktop session remains a separate
recovery option; it is not started behind Hearthdeck Console.

The current Flutter GTK shell uses Gamescope's XWayland display rather than
Gamescope's Wayland client socket. This is intentional until the shell's native
runner is migrated and tested as a Wayland client. Gamescope directly starts
the packaged `hearthdeck` executable as its primary client, so it provides its
private Xwayland display without an additional session supervisor. The Console
uses the normal Gamescope window policy, allowing Hearthdeck and managed desktop
applications to be admitted as ordinary Xwayland windows. Managed Console
applications run in a dedicated user slice and are stopped before the display
manager returns. If Console returns to the display manager, inspect
`~/.local/state/hearthdeck/gamescope-session.log` after signing into a normal
desktop session. If the user state directory is unavailable, the log falls back
to `$XDG_RUNTIME_DIR/hearthdeck/`. To reproduce the session from a terminal and
stream its output, run:

```sh
/usr/lib/hearthdeck/hearthdeck-gamescope-session
```

Exit Hearthdeck Console from its system menu to return to the display manager.
Select the COSMIC session there to return to the desktop. If the console shell
crashes, Gamescope exits as well and the display manager remains available for
recovery.

Use **Settings > General > Exit to desktop** inside Hearthdeck Console. Confirm
the prompt to close the console session and return to the display manager.

The bridge scans the target machine's Freedesktop entries and launches only a
re-discovered desktop entry. It honors desktop visibility constraints, `Path`,
and `TryExec`, and rejects terminal and D-Bus-activated entries because those
cannot be managed safely in the console. Linux launches are placed in transient
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
