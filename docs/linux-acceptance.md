# Linux Acceptance Checklist

Run the automated gate on the CachyOS host after installing or upgrading
Hearthdeck:

```sh
just acceptance-linux
# Installed-package equivalent:
/usr/lib/hearthdeck/linux-acceptance --require-romm
```

The command restarts `hearthdeck.target`, requires the configured RomM stack,
checks required user units, the daemon health endpoint, running Podman
containers, the input broker socket and virtual device, `~/hearthdeck.log`, and
failed-unit state. It exits nonzero on an automated failure and prints the
graphical/controller checks that still need a person. Run
`scripts/linux-acceptance` without `--require-romm` on hosts where RomM is
intentionally absent.

## Setup

```sh
systemctl --user daemon-reload
systemctl --user enable --now hearthdeck.target
systemctl --user status hearthdeck.target hearthdeck-bridge.socket hearthdeck-daemon.service hearthdeck-input.service
```

For local-only development, verify health:

```sh
curl http://127.0.0.1:38400/v1/health
curl -X POST http://127.0.0.1:38401/v1/pairing
```

The health response reports each discovery and metadata provider as `starting`,
`ready`, or `degraded`, including its last success, record count, and a safe
error summary. A degraded provider leaves other catalog sources available.

## RomM

The automated gate verifies the existing Podman Compose RomM deployment. For
manual investigation:

```sh
systemctl --user daemon-reload
systemctl --user restart hearthdeck.target
systemctl --user status romm.service
tail -f ~/hearthdeck.log
```

Confirm the Compose stack at `/mnt/external/romM/podman-compose.yaml` starts,
the COSMIC frontend shows only platforms with installed ROMs under Console
Games, selecting a platform loads its ROMs, and launching one creates a managed
session through RetroArch. Stop `hearthdeck.target` and confirm the Compose
stack is stopped. Temporarily move the compose file away and confirm
`romm.service` is skipped while the rest of `hearthdeck.target` still starts.

## Library Scan

Pair a client using the one-time code, then use its bearer token:

```sh
curl -X POST http://127.0.0.1:38400/v1/library/rescan \
  -H 'Authorization: Bearer <token>'
curl http://127.0.0.1:38400/v1/library \
  -H 'Authorization: Bearer <token>'
```

Confirm entries are discovered from KDE/Freedesktop `.desktop` files and that
hidden, non-application, and desktop-incompatible entries do not appear.

The bridge follows the XDG data-directory defaults even when a systemd user
environment leaves them empty. It also scans Flatpak's user and system export
directories. Check its structured scan-directory log if the catalog is empty:

```sh
journalctl --user -u hearthdeck-bridge.service --since '10 minutes ago' -o cat
```

At the default log level, each scanned directory reports `desktop_entry_count`
and `accepted_entry_count`. A count of zero from every directory means the
desktop session has no exported Freedesktop launchers in those locations;
installed packages without a `.desktop` entry are intentionally not listed as
launchable applications.

## Launch

Call `POST /v1/apps/{id}/launch` with an ID returned by the library endpoint.
Confirm the registered desktop app opens in the active graphical session.

The bridge intentionally re-discovers the desktop ID locally and creates a
transient systemd user service; an API client cannot supply an `Exec` command or
arguments. Query `GET /v1/sessions/active` after launch, then use
`POST /v1/sessions/{id}/stop` to confirm the managed process exits.

## Controller Compatibility

Confirm `/dev/uinput` is writable by the active user, the broker socket exists
at `$XDG_RUNTIME_DIR/hearthdeck-input.sock`, and the virtual device appears as
`Hearthdeck Compatibility Input` in `/proc/bus/input/devices`. If `evtest` is
installed, select that virtual device and verify the mapped events.

In Full Library, open an application's context menu and enable **Controller
compatibility**. Launch an application without native controller support and
verify D-pad/left stick arrow keys, A/Start Enter, B Escape, X Space, Y/Select
Tab, bumpers Page Up/Page Down, right-stick mouse movement, and trigger mouse
buttons. Open Guide and confirm the overlay exclusively owns the controller
while visible. Close the application and confirm no mapped events remain.

Launch a controller-native game with compatibility disabled and confirm it does
not receive duplicate keyboard or mouse input. Compatibility is deliberately
per application and disabled by default for this reason.

Select **Hearthdeck Kiosk** in the display manager. Confirm Hearthdeck opens
fullscreen with no desktop shell behind it (no panel, wallpaper, or other
window ever appears), then launch a graphical application and verify it opens
and fills the screen, connected directly to this same session (not a nested
Gamescope instance - see `docs/kiosk-session.md` for why that changed). Exit
Hearthdeck Kiosk and confirm its managed application services stop and the
session returns to the display manager's login screen.

Confirm gamepad D-pad, A, B, and sticks navigate Hearthdeck. Confirm existing
audio works through PipeWire/WirePlumber. Confirm preconfigured NetworkManager
and BlueZ connections remain available. Wi-Fi, Bluetooth, and audio-routing
configuration screens are not part of the current Kiosk session.

Launch a Heroic game (Epic or GOG). Confirm it opens inside its own nested
Gamescope instance (the one launch path that still uses one - see
`services/README.md`) and fills the screen. Return to Hearthdeck and launch a
*different* Heroic game: confirm it also opens correctly (this specifically
exercises the reused-Heroic-instance path, not just the first, cold-start
launch - the two behave differently, so testing only the first launch does
not confirm the second works). Confirm `systemctl --user status
hearthdeck-heroic.service` remains active after the game exits (Heroic itself
stays running by design). Finally, close/stop the session from Hearthdeck and
confirm `systemctl --user status hearthdeck-heroic.service` shows the unit
fully stopped, with no lingering Heroic or game process still running
(`ps -u $USER -o pid,cmd | grep -i heroic`).

While every managed game or application is in the foreground, exercise all
controller buttons and sticks. Confirm the launched app receives them while
Hearthdeck does not move selection, open another app, close itself, or end the
session. Close a normal managed app and confirm controller navigation returns
to Hearthdeck. For persistent Heroic, return focus to Hearthdeck and confirm
navigation returns without the still-active Heroic unit taking ownership back.

If `gamepad-osk` is installed, confirm it is running in daemon mode with access to
the controller input and `/dev/uinput`. Its evdev grab cannot reliably suppress
Hearthdeck's direct joystick reader, so OSK input isolation is not guaranteed.

## LAN Mode

Configure `~/.config/hearthdeck/daemon.env` from
`deploy/systemd/daemon.env.example`, including a certificate and private key.
Restart the daemon, then confirm:

- the daemon will not start LAN mode without TLS paths;
- Android connects with HTTPS only;
- the Android client verifies the displayed/pinned certificate fingerprint;
- pairing code creation remains unavailable on the LAN listener.

`just install-services` migrates a prior LTV systemd installation, configuration,
and database to Hearthdeck before enabling the renamed services.
