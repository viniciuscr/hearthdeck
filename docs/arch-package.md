# Arch Linux Package

The `hearthdeck` package targets `x86_64` Arch Linux derivatives, including
CachyOS. It installs:

- `/opt/hearthdeck/`: the Flutter Linux client bundle.
- `/usr/bin/hearthdeck`: the desktop launcher command.
- `/usr/lib/hearthdeck/`: the local bridge and daemon binaries.
- `/usr/lib/systemd/user/`: the Hearthdeck target, bridge socket, bridge, API
  daemon, Gamescope compositor, and console client user units.
- `/usr/share/applications/`: the Hearthdeck desktop entry and icon.
- `/usr/share/wayland-sessions/hearthdeck-gamescope.desktop`: the supervised DRM
  console session shown by compatible display managers.
- `/usr/share/wayland-sessions/hearthdeck-gamescope-xterm.desktop`: the isolated
  DRM Gamescope and Xterm recovery session.

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
its first typed request. Future network and Bluetooth bridges will follow this
same target-plus-socket lifecycle.

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
recovery option.

The current Flutter GTK shell uses Gamescope's XWayland display rather than
Gamescope's Wayland client socket. This is intentional until the shell's native
runner is migrated and tested as a Wayland client. The display-manager
entrypoint starts `hearthdeck-gamescope.target` through the user manager.
`hearthdeck-gamescope.service` owns Gamescope and waits for its readiness socket
to publish the Xwayland display. It writes that environment to
`$XDG_RUNTIME_DIR/hearthdeck/gamescope-environment` and signals systemd ready.
Only then can `hearthdeck-console-client.service` start Hearthdeck with the
private Xwayland display. The bridge reads the same environment when launching
managed applications.

If Console returns to the display manager, inspect:

```sh
journalctl --user -u hearthdeck-gamescope.service -u hearthdeck-console-client.service -b
cat ~/.local/state/hearthdeck/console-session.log
cat ~/.local/state/hearthdeck/gamescope-service.log
cat ~/.local/state/hearthdeck/console-client.log
```

Use **Settings > General > Exit to desktop** inside Hearthdeck Console. Confirm
the prompt to close the console session and return to the display manager.

**Hearthdeck Gamescope Xterm Test** starts only direct DRM Gamescope and Xterm.
It does not launch Hearthdeck, systemd services, or a session supervisor. Run
`exit` in Xterm to return to the display manager. Its output is retained at:

```sh
cat /tmp/hearthdeck-gamescope-xterm-$(id -u).log
```

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
