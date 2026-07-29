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
hidden/non-application entries do not appear.

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
transient systemd user scope; an API client cannot supply an `Exec` command or
arguments. Query `GET /v1/sessions/active` after launch, then use
`POST /v1/sessions/{id}/stop` to confirm the managed process exits.

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
