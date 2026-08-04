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

Connect a local RomM instance through **Settings > System > Retro & RomM**.
The supplied RomM client token requires the `platforms.read` permission.
Hearthdeck exposes its console list at `GET /v1/retro/consoles`; the credential
is stored in the daemon's SQLite data and is never returned through the API.

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

In the Hearthdeck Kiosk session, the session script starts Gamescope directly
on the DRM/KMS seat as the sole compositor, with Hearthdeck as its only child;
there is no separate desktop compositor to publish the socket first. Launched
desktop applications and RetroArch games connect directly to that same
session (its embedded Xwayland `DISPLAY`, or its Wayland socket) as ordinary
clients, rather than getting a nested Gamescope instance of their own -
confirmed on real hardware that a second Gamescope process joining this
session as a Wayland peer is composited but never actually shown, while a
plain client is shown automatically the same way Hearthdeck itself is (see
`docs/kiosk-session.md`). PipeWire/WirePlumber audio, NetworkManager
networking, and BlueZ Bluetooth remain host services outside the Hearthdeck
process tree.

Heroic game launches go through its own URI handler (`heroic://launch?...`)
rather than an exec'd binary, so the bridge tracks *Heroic itself* as one
stable, reused systemd unit (`hearthdeck-heroic.service`) instead of a fresh
unit per launch: Electron's single-instance lock means any launch after the
first is handled by whichever Heroic process is already running, not a new
one, so a fresh-unit-per-launch model would lose track of every game after
the first. The bridge checks whether that unit is already active before
deciding whether to start it (cold start: a plain `xdg-open`, connecting
directly to the Kiosk session the same way every other launch does - not
wrapped in a nested Gamescope instance of its own, which was tried and
reverted: it added a real, measured second GPU-compositing pass on top of
the outer session's own, and its only actual justification - `--keep-alive`
preventing Gamescope from tearing down the display when its wrapped
`xdg-open` child exits almost immediately - doesn't apply to something no
Gamescope instance is wrapping in the first place) or just ask the
already-running instance to launch the next game directly. Heroic is
intentionally left running between games - faster subsequent launches, at
the cost of some idle memory - and closing it (and whatever game it's
running) is `systemctl --user stop hearthdeck-heroic.service`, which
reliably kills the whole process tree via its cgroup even though Heroic
never exits on its own.

### The launch pipeline shape

Desktop apps, Heroic, and RetroArch (`hearthdeck-bridge/src/platform/linux.rs`,
`main.rs`) share two layers and diverge only in the third, on purpose - a
future launcher should fit the same shape rather than reinvent it:

1. **`launch_with_systemd`**: the one place that calls `systemd-run`. Takes a
   unit name, a command, and a working directory - nothing launcher-specific,
   and no gamescope-wrapping logic of its own. It forwards the display/session
   environment (`DISPLAY`, `WAYLAND_DISPLAY`, etc.) the launched process needs
   to connect to whatever session Hearthdeck itself is running in.
2. **`register_launch`**: the one place that builds an `ApplicationSession`,
   persists it, and inserts it into the in-memory session map, rolling the
   launch back (`stop_application`) if persistence fails. Every launcher's
   success path ends by calling this, not by re-deriving its own version of
   it.
3. **Launcher-specific decision logic**, on top of the two shared layers,
   is where launchers are expected to differ: desktop apps re-discover and
   validate the desktop entry before building a command; RetroArch validates
   the resolved core/rom paths and sets up its config directory; Heroic
   checks whether its stable unit is already running to decide between a
   cold start (which it wraps in its own nested Gamescope command before
   handing to `launch_with_systemd`) and an already-running hand-off. None of
   that belongs in layers 1 or 2, and none of layers 1/2's job belongs
   duplicated here.


A launcher that finds itself re-implementing session bookkeeping instead of
calling `register_launch`, or shelling out to `systemd-run` directly instead
of calling `launch_with_systemd`, has drifted from this shape.

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
