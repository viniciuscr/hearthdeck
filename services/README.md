# Hearthdeck Services

The backend is split into two Rust processes:

- `hearthdeck-daemon`: HTTP/WebSocket API, pairing tokens, SQLite state, and events.
- `hearthdeck-bridge`: Linux desktop-entry scanning and allowlisted, supervised
  app launching.

Discovery uses independently registered provider modules. The current
`desktop-apps` provider scans Freedesktop entries. Future Steam, GOG, Epic,
emulator, movie, and stream providers each get their own source ID, schedule,
and scanner while sharing the catalog repository and event stream. Each source
has a separate coalescing worker, so one slow provider cannot block another.

The daemon listens on `127.0.0.1:38400` over HTTP by default. LAN binding
requires `HEARTHDECK_LAN_ENABLED=true`, a certificate path, and a private key path;
the daemon then serves HTTPS through Rustls. Pairing code creation is exposed
only on the loopback admin listener at `127.0.0.1:38401`.

Set `HEARTHDECK_ROMM_URL` and `HEARTHDECK_ROMM_TOKEN` together to connect a
local RomM instance. The token requires the RomM `platforms.read` permission.
Hearthdeck exposes the resulting console list at `GET /v1/retro/consoles`; the
RomM credential remains on the host and is never sent to paired clients.

The processes communicate through `$XDG_RUNTIME_DIR/hearthdeck/bridge.sock` using
newline-delimited JSON defined in `hearthdeck-protocol`. The protocol has typed scan
launch, active-session, and stop requests only. Arbitrary command execution is
deliberately absent. The daemon supplies a discovered desktop ID, while the
bridge resolves and validates the local launch specification before placing it
in a transient systemd user service. Session records live under the user's
runtime directory, so the bridge can resume tracking a managed service after a
restart.

`GET /v1/health` exposes host capabilities. Remote clients, including Android,
may browse the library and send install requests when supported, but must not
assume they can launch host applications. An install request is a typed request
for host-side approval; it never invokes pacman, Flatpak, Steam, or another
package manager directly.

On Linux, `hearthdeck.target` is the systemd user-session root. It starts the
API daemon and owns `hearthdeck-bridge.socket`; the bridge process starts only
when a typed daemon request arrives. Future NetworkManager and BlueZ adapters
will receive their own socket/service pair under the same target rather than
being folded into the daemon.

`GET /v1/health` reports every discovery and metadata provider as `starting`,
`ready`, or `degraded`, with the last successful record count and safe error
summary. A provider failure cannot erase another source's catalog records.

## Development

```sh
mise exec -- cargo run -p hearthdeck-bridge
mise exec -- cargo run -p hearthdeck-daemon
```

In a separate shell, create a pairing code:

```sh
curl -X POST http://127.0.0.1:38401/v1/pairing
```

The API contract is at `../contracts/openapi.yaml`. Android clients must pin or
otherwise verify the configured server certificate before storing a pairing
token.
