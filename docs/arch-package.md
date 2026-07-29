# Arch Linux Package

The `hearthdeck` package targets `x86_64` Arch Linux derivatives, including
CachyOS. It installs:

- `/opt/hearthdeck/`: the Flutter Linux client bundle.
- `/usr/bin/hearthdeck`: the desktop launcher command.
- `/usr/lib/hearthdeck/`: the local bridge and daemon binaries.
- `/usr/lib/systemd/user/`: packaged user units for the bridge and daemon.
- `/usr/share/applications/`: the Hearthdeck desktop entry and icon.

## Install

Download `hearthdeck-*.pkg.tar.zst` from the GitHub Actions artifact and run:

```sh
sudo pacman -U hearthdeck-*.pkg.tar.zst
systemctl --user daemon-reload
systemctl --user enable --now hearthdeck-bridge.service hearthdeck-daemon.service
```

Pacman deliberately does not enable a per-user service during a root package
transaction. Enable it as the desktop user that will launch applications.
The units retain `NoNewPrivileges`, but do not use mount-namespace sandboxing:
Arch systemd user units cannot reliably support directives such as
`ProtectSystem`, `ReadWritePaths`, or `PrivateTmp`.

The bridge scans the target machine's Freedesktop entries and invokes
`gtk-launch` only for a desktop ID returned by that scan. The daemon maintains
the SQLite catalog at `~/.local/share/hearthdeck/hearthdeck.db` and exposes a
loopback API at `127.0.0.1:38400`.

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
systemctl --user status hearthdeck-bridge.service hearthdeck-daemon.service
curl http://127.0.0.1:38400/v1/health
hearthdeck
```

Use `/usr/share/doc/hearthdeck/ACCEPTANCE.md` to verify discovery and launching
in the active graphical session. To enable LAN access, create
`~/.config/hearthdeck/daemon.env` from
`/usr/share/doc/hearthdeck/daemon.env.example`, configure TLS paths, and
restart the daemon.
