# Linux Acceptance Checklist

Run this on the CachyOS desktop after installing the two user services.
Everything except the final graphical launch is tested on macOS in the Rust
workspace test suite.

## Setup

```sh
systemctl --user daemon-reload
systemctl --user enable --now hearthdeck.target
systemctl --user status hearthdeck.target hearthdeck-bridge.socket hearthdeck-daemon.service
```

For local-only development, verify health:

```sh
curl http://127.0.0.1:38400/v1/health
curl -X POST http://127.0.0.1:38401/v1/pairing
```

The health response reports each discovery and metadata provider as `starting`,
`ready`, or `degraded`, including its last success, record count, and a safe
error summary. A degraded provider leaves other catalog sources available.

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

Select **Hearthdeck Kiosk** in the display manager. Confirm Hearthdeck opens
fullscreen with no desktop shell behind it, then launch a graphical application
and verify it opens inside a separate nested Gamescope instance. Exit
Hearthdeck Kiosk and confirm its managed application services stop and the
session returns to the display manager's login screen.

The overlay has no automatic startup wired up yet. Run `just overlay-toggle`
only while a managed application is open, and confirm Hearthdeck itself never
shrinks or letterboxes while the overlay is present.

Confirm gamepad D-pad, A, B, and sticks navigate Hearthdeck. Confirm existing
audio works through PipeWire/WirePlumber. Confirm preconfigured NetworkManager
and BlueZ connections remain available. Wi-Fi, Bluetooth, and audio-routing
configuration screens are not part of the current Kiosk session.

Heroic game URI launches are intentionally unavailable in Kiosk mode. If
`gamepad-osk` is installed, confirm it is running in daemon mode with access to
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
