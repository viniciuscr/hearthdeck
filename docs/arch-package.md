# Arch Linux Package

The `hearthdeck` package targets `x86_64` Arch Linux derivatives, including
CachyOS. It installs:

- `/usr/bin/hearthdeck`: the launcher command that starts the local services and the COSMIC frontend.
- `/usr/bin/hearthdeck-frontend`: the COSMIC (libcosmic) frontend.
- `/usr/lib/hearthdeck/linux-acceptance`: target-host service, RomM, API, and
  aggregate-log acceptance checks.
- `/usr/lib/hearthdeck/hearthdeck-autologin`: enables or disables greetd
  autologin into the Hearthdeck session, so a keyboardless box boots past the
  login screen; see `docs/kiosk-session.md`.
- `/usr/lib/hearthdeck/`: the local bridge, daemon, controller compatibility
  broker, and Hearthdeck session script.
- `/usr/lib/systemd/user/`: the Hearthdeck target, bridge socket, bridge, API
  daemon, input broker, aggregate log collector, and optional Podman Compose
  RomM user units.
- `/usr/lib/udev/rules.d/70-hearthdeck-uinput.rules` and
  `/usr/lib/modules-load.d/hearthdeck-uinput.conf`: active-seat access to the
  virtual input device and boot-time loading of the `uinput` kernel module.
- `/usr/share/doc/hearthdeck/romm.env.example`: optional path override for the
  external RomM Compose deployment. The service defaults to
  `/mnt/external/romM/podman-compose.yaml` and skips when that file is absent.
  `romm.path` watches that default path and starts the stack when the file
  appears, so a compose file on an external mount that is not ready at session
  start is still picked up rather than skipped for the whole session.
- `~/hearthdeck.log`: recreated at each Hearthdeck session start with combined
  session, daemon, bridge, input broker, overlay, and RomM output, plus systemd's
  own messages about those units (skipped conditions, start timeouts, failures).
- `/usr/share/applications/`: the Hearthdeck desktop entry and icon.
- `/usr/share/wayland-sessions/hearthdeck.desktop`: the Hearthdeck session
  (cosmic-comp with the frontend fullscreen) shown by compatible display
  managers.
- `/usr/lib/hearthdeck/hearthdeck-overlay` and
  `/usr/lib/systemd/user/hearthdeck-overlay.service`: Guide-button-toggled
  quick-menu overlay, started only by the Hearthdeck session. The session
  installs a COSMIC custom shortcut, `Super+Shift+H`, which runs
  `hearthdeck-overlay --toggle`. It does not replace an existing user binding.
- The COSMIC frontend was migrated from `../cosmic-app-library` and provides
  the fullscreen app/game grid, search, sidebar, gamepad navigation, and
  daemon-client integration. The test session does not start a panel or write
  COSMIC panel/theme configuration.

## Install

Install the initial `hearthdeck-*.pkg.tar.zst` from the GitHub Actions artifact:

```sh
sudo pacman -U hearthdeck-*.pkg.tar.zst
```

The Hearthdeck launchers and sessions start `hearthdeck.target` for the active
user. The package deliberately does not enable it globally: fixed API ports
cannot be shared by every user manager, including display-manager greeter
accounts.

Nothing is left to run after an install or an upgrade. `hearthdeck.install`
reloads the invoking user's manager and restarts what it owns, for the reasons in
[After an upgrade](#after-an-upgrade): a manager keeps the unit definitions it
already loaded, and a running service keeps executing the old binary.

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

A root package transaction cannot use a logged-in user's manager directly, so
`hearthdeck.install` reaches it through that manager's own runtime directory
(`runuser` plus `systemctl --user` with `XDG_RUNTIME_DIR` set). It is skipped,
silently and harmlessly, when no session is logged in - the next session start
reads the new units by itself. The units retain `NoNewPrivileges`, but do not use
mount-namespace sandboxing: Arch systemd user units cannot reliably support
directives such as `ProtectSystem`, `ReadWritePaths`, or `PrivateTmp`.

`hearthdeck.target` starts the API daemon, owns the
`hearthdeck-bridge.socket`, and attempts the optional `romm.service` (with
`romm.path` starting it if its compose file appears only after the session is
up). The RomM
unit is skipped (cleanly, not failed) unless its compose file exists, so
installations without RomM are unaffected. The bridge process is
socket-activated on its first typed request. Future network and Bluetooth
bridges will follow the same target-plus-socket lifecycle.

If you previously used `just install-services` or copied units by hand, those
copies in `~/.config/systemd/user/` override the package units and keep unit
fixes from reaching the session. The package names the files it finds there at
the end of an install; delete them (or move the differences into a systemd
drop-in) to run the packaged units.

### After an upgrade

An upgrade replaces the unit files on disk, but a running manager keeps the
definitions it already loaded - including a `hearthdeck.target` from before
RomM existed, which wants no `romm.service` at all - and a running service keeps
executing the old binary. The package therefore applies its own upgrade, for the
user that ran it, with nothing left to do by hand:

- `systemctl --user daemon-reload`, then `try-restart` of
  `hearthdeck-daemon.service` and `hearthdeck-bridge.service`. `try-restart`
  leaves a unit the user never started alone, and
  `hearthdeck-input.service` is deliberately not restarted: it only lives inside
  a managed launch, where that would drop the controller mapping it is applying.
- `start romm.path` and `start romm.service`. Starting rather than restarting is
  deliberate - a stack an earlier session already has running must not be torn
  down and rebuilt by an upgrade, so `start` is a no-op on a unit that is up, runs
  `podman-compose up -d` on one that is not, and skips cleanly when there is no
  compose file. This is what brings RomM back after an upgrade without a
  sign-out.

A session that has never been started, or a machine with no session logged in,
is the one case the transaction cannot cover; the session script and the launcher
each run `systemctl --user daemon-reload` before starting the target, so a
sign-out and back in (or the next boot) is always enough.

Confirm the stack actually came up with the session rather than assuming
it did:

```sh
systemctl --user status romm.service        # active (exited): RemainAfterExit
podman-compose -f "$ROMM_COMPOSE_FILE" ps   # containers running
grep 'RomM' ~/hearthdeck.log                # "Starting RomM Podman Compose stack"
```

`systemctl --user status romm.service` reporting `could not be found` means the
installed package predates `romm.service`; check `pacman -Q hearthdeck` and
upgrade. RomM is external, so `romm.service` also needs `podman-compose`
installed and - unless the deployment uses host networking - podman's default
networking to be able to open `/dev/net/tun` (see `docs/retroarch-integration.md`
for the `ROMM_COMPOSE_ARGS` override and why the unit has no restart policy).

## Hearthdeck session

See `docs/kiosk-session.md` for the startup history and the "do not" lessons;
note that document now describes the retired Gamescope session.

**Hearthdeck** is a minimal COSMIC session: `cosmic-comp` runs with the
Hearthdeck frontend fullscreen as its only app client, and no panel, launcher,
wallpaper, notifications, or settings daemon. Select it in the display manager,
or leave it as the autologin session - which is what a keyboardless TV box needs,
and what the package sets up automatically (or `hearthdeck-autologin enable`
does by hand) - to boot straight into Hearthdeck fullscreen.

The session script (`/usr/lib/hearthdeck/hearthdeck-session`) imports the
session environment into the systemd user manager, starts `hearthdeck.target`
(and the overlay) for the current user, and then execs `cosmic-comp` with the
frontend as its single client. Exiting Hearthdeck ends `cosmic-comp` and
returns to the display manager's login screen.

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

The frontend reads controllers directly only while it owns foreground input.
It relinquishes its gamepad subscription before every managed launch and keeps
it disabled while the launched session owns the display. Session polling and
COSMIC focus events return ownership after the app exits or Hearthdeck regains
focus. The quick-menu overlay reads the Guide button independently and grabs
the controller exclusively while its menu is visible.

Applications without native controller support can opt into **Controller
compatibility** from their library context menu. The choice is stored per
catalog entry and defaults off. While an opted-in managed session is active,
`hearthdeck-input.service` maps common controls to a virtual keyboard and mouse:

| Controller | Keyboard or mouse |
|---|---|
| D-pad or left stick | Arrow keys |
| A / Start | Enter |
| B | Escape |
| X | Space |
| Y / Select | Tab |
| Left / right bumper | Page Up / Page Down |
| Right stick | Mouse movement |
| Left / right trigger | Right / left mouse button |

The bridge resets the broker to the native profile when a launch fails, exits,
is stopped, or is replaced. The physical controller is not grabbed, so Guide
continues to reach the overlay; the overlay's existing exclusive grab takes
precedence while it is visible. Leave compatibility disabled for games with
native controller support, or they may receive both controller and emulated
keyboard/mouse input. RetroArch's own menu deliberately does not answer to
Guide, so the two uses of that button cannot race: Hearthdeck seeds its joypad
autoconfig profiles with the Guide/Home menu-toggle binding unbound and pins
the emulator's menu combo to **Start + Select** (decision 9 in
`docs/retroarch-integration.md`).

PipeWire/WirePlumber provide audio, while NetworkManager and BlueZ remain
system services. Their existing connections and paired devices continue to
work, but Hearthdeck does not yet provide Wi-Fi, Bluetooth, or audio-routing
configuration interfaces, and the Kiosk session does not start a polkit agent,
so NetworkManager changes needing authentication are unavailable from it.

`gamepad-osk` remains optional and external to this package. It must be running
as its upstream daemon service and have its uinput/input permissions configured
before `gamepad-osk --toggle` can provide an OSK. During a managed RetroArch
session, press **Guide/Home + X** to toggle it, matching the Rust frontend's
use of the same command. The shortcut is disabled outside RetroArch sessions. Its
OSK opens and closes automatically when a text input gains or loses focus in the
COSMIC frontend, including search, group rename, and new-group inputs. Its
evdev grab does not necessarily suppress Hearthdeck's direct joystick reader, so
OSK input isolation is not guaranteed until controller input is unified behind
one process.

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
systemctl --user status hearthdeck.target hearthdeck-bridge.socket hearthdeck-daemon.service hearthdeck-input.service
curl http://127.0.0.1:38400/v1/health
hearthdeck
```

Use `/usr/share/doc/hearthdeck/ACCEPTANCE.md` to verify discovery and launching
in the active graphical session. To enable LAN access, create
`~/.config/hearthdeck/daemon.env` from
`/usr/share/doc/hearthdeck/daemon.env.example`, configure TLS paths, and
restart the daemon.
