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

Download `hearthdeck-*.pkg.tar.zst` from the GitHub Actions artifact and run:

```sh
sudo pacman -U hearthdeck-*.pkg.tar.zst
systemctl --user daemon-reload
systemctl --user disable --now hearthdeck-bridge.service hearthdeck-daemon.service
systemctl --user enable --now hearthdeck.target
```

Pacman deliberately does not enable a per-user service during a root package
transaction. Enable it as the desktop user that will launch applications.
The units retain `NoNewPrivileges`, but do not use mount-namespace sandboxing:
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
recovery option; it is not started behind Hearthdeck Console.

Exit Hearthdeck Console from its system menu to return to the display manager.
Select the COSMIC session there to return to the desktop. If the console shell
crashes, Gamescope exits as well and the display manager remains available for
recovery.

Use **Settings > General > Exit to desktop** inside Hearthdeck Console. Confirm
the prompt to close the console session and return to the display manager.

The bridge scans the target machine's Freedesktop entries and launches only a
re-discovered desktop entry. Linux launches are placed in a transient systemd
user scope, allowing Hearthdeck to query and stop the active managed session.
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
