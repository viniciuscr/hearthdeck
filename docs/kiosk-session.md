# Kiosk Session: How It Works, and How Not to Break It

This document is the source of truth for the **Hearthdeck Kiosk** Wayland
session shipped by the Arch package. Read it before touching anything under
`packaging/arch/hearthdeck-session*` or `packaging/arch/hearthdeck.target`.

The session existed in a broken/flaky state for a long time (see git history:
multiple rewrites bouncing between a raw Gamescope session and a COSMIC-based
one) before landing on the design below. That history is not an accident —
it is the record of what didn't work. Read the "Do not" section before
"simplifying" or "improving" this again.

## What actually runs, in order

```
Display manager (SDDM/GDM/greetd/...)
  -> reads /usr/share/wayland-sessions/hearthdeck.desktop
  -> Exec = /usr/lib/hearthdeck/hearthdeck-session
       -> systemctl --user daemon-reload
       -> systemctl --user start hearthdeck.target   (BLOCKS until ready, see below)
       -> exec gamescope --backend drm --fullscreen --force-grab-cursor -- \
            /opt/hearthdeck/hearthdeck
            -> Hearthdeck is Gamescope's ONLY child process
```

That's the entire session. There is no desktop compositor, no session
manager, no panel/dock/launcher, no XDG autostart directory scan, and no
second systemd unit standing between the display manager and Hearthdeck's
window. `hearthdeck-session` is a single bash script that ends in `exec`,
so by the time Hearthdeck is running, `gamescope` has *replaced* that shell
process — there's exactly one extra process (`gamescope`) between the
display manager and the app.

### Files involved

| File | Installed to | Purpose |
|---|---|---|
| `packaging/arch/hearthdeck-session` | `/usr/lib/hearthdeck/hearthdeck-session` | The script above. Owns startup order. |
| `packaging/arch/hearthdeck-session.desktop` | `/usr/share/wayland-sessions/hearthdeck.desktop` | Makes "Hearthdeck Kiosk" selectable/autologin-able in the display manager. |
| `packaging/arch/hearthdeck.target` | `/usr/lib/systemd/user/hearthdeck.target` | Backend services root. `WantedBy=default.target`, so it is also enabled globally and normally starts on its own at any login, independent of which graphical session is chosen. |
| `packaging/arch/hearthdeck-bridge.socket` / `.service` | `/usr/lib/systemd/user/` | Linux integration bridge (desktop-entry discovery, app launch). Socket-activated. |
| `packaging/arch/hearthdeck-daemon.service` | `/usr/lib/systemd/user/` | Local API daemon (library, pairing, health). `Type=notify`. |

None of these files reference COSMIC, a window manager, or any other
compositor. That is intentional.

## How backend services are guaranteed to be up before the app starts

This is the part most likely to regress if touched carelessly, so it gets
its own section.

1. `hearthdeck.target` is `WantedBy=default.target`, and the package enables
   it globally (`systemctl --global enable hearthdeck.target`). That means
   on a normal boot, it starts as part of reaching `default.target` for the
   user, before any session script even runs.
2. `hearthdeck-session` *also* runs `systemctl --user start hearthdeck.target`
   itself, as a synchronous safety net. This matters for:
   - the first login right after install/upgrade (global enablement may not
     have been retroactively applied to an already-running user manager —
     see the `pacman` note in `docs/arch-package.md`);
   - autologin straight from cold boot, where session startup can race the
     user manager reaching its default target.
3. **Why `systemctl --user start` here is not just "fire and forget":**
   `hearthdeck-daemon.service` is `Type=notify`. Systemd's `start` command
   for a `Type=notify` unit **blocks the caller** until the service calls
   `sd_notify(READY=1)` (see `notify_ready()` in
   `services/hearthdeck-daemon/src/main.rs`, sent only after its HTTP
   listeners are actually bound) — or until `TimeoutStartSec` elapses and
   the start is considered failed. Because `hearthdeck-session` runs this
   command with `exec`/normal shell semantics (no `&`, no backgrounding), the
   script does not proceed to launch Gamescope/Hearthdeck until the daemon
   has confirmed it is genuinely ready, not merely "queued to start."
   `hearthdeck-bridge.socket` is a socket unit, so it's active immediately;
   `hearthdeck-bridge.service` itself only starts on first request
   (socket activation) and is not part of this readiness wait.
4. A failure to start the target is logged to stderr (visible in the
   session's journal / display manager log) but is **not fatal** — the
   script still proceeds to launch Hearthdeck. A blank/failed backend is a
   recoverable, visible-in-UI problem; a kiosk session that refuses to even
   show a window on any backend hiccup is not an acceptable trade-off for a
   TV/couch device.

If you need the app to hard-block until the API is confirmed serving (not
just that the unit is "ready" per systemd), that would mean adding an actual
HTTP health-check loop (`curl 127.0.0.1:38400/v1/health`) into the script
before the `exec gamescope` line. This has intentionally **not** been done
yet, to avoid adding a new runtime dependency (`curl`) and a potential
indefinite hang if the daemon never becomes healthy. If you add this, always
pair it with a timeout and a clear log line on give-up, and never let it
block forever.

## Do not

These are not style preferences. Each one previously caused this session to
fail in ways that took real effort to diagnose (a black screen, a silent
fallback to login, or a systemd unit sitting `inactive (dead)` for no
apparent reason).

- **Do not put another compositor, window manager, or session manager in
  front of Gamescope.** No COSMIC, no labwc, no sway, no Xorg. Gamescope
  must be the only thing that opens DRM for this session. Every extra layer
  is one more thing that has to finish initializing, publish a Wayland
  socket, and hand off environment/state correctly before the next layer can
  even start — and any one of those hand-offs failing silently is exactly
  what made the previous COSMIC-based session unreliable.
- **Do not gate Hearthdeck's startup behind a separate systemd unit with a
  `ConditionEnvironment=` (or similar) check.** The old
  `hearthdeck-kiosk.service` used
  `ConditionEnvironment=XDG_CURRENT_DESKTOP=hearthdeck:COSMIC`. When a
  systemd `Condition*=` fails, the unit is skipped silently — it is not a
  failure, it produces no error, and `systemctl status` just shows it never
  ran. This is very hard to debug and is the kind of gate you get "for free"
  the moment you split app startup into its own unit instead of a direct
  `exec` from the session script. Keep the session script driving the exec
  chain directly.
- **Do not background (`&`) the `systemctl --user start hearthdeck.target`
  call**, and do not add `>/dev/null 2>&1` back onto it. Backgrounding
  destroys the readiness guarantee described above (Hearthdeck could start
  before the daemon's listeners are bound), and silencing output hides real
  failures the same way the previous session's problems went undiagnosed for
  days.
- **Do not run Hearthdeck as a systemd unit that is a sibling of Gamescope**
  (e.g. `ExecStart=` in some `hearthdeck-app.service` that is merely
  "wanted by" the session, started independently of Gamescope). Hearthdeck
  must be Gamescope's literal child (`gamescope ... -- /opt/hearthdeck/hearthdeck`).
  That is what makes "Hearthdeck exits" and "session ends" the same event
  with no extra supervision logic needed, and it's what lets Gamescope own
  the app's Wayland/X11 environment directly instead of hoping a shared
  socket gets imported into the right place.
- **Do not remove `--backend drm`.** Without it, Gamescope will try to run
  nested inside another Wayland/X11 session, which does not exist here (and
  reintroduces exactly the "who owns DRM" problem this design avoids).
- **Do not confuse this outer Gamescope with the bridge's nested Gamescope.**
  The bridge spawns a *separate*, on-demand Gamescope instance when the user
  launches a game/app from inside Hearthdeck (see `services/README.md` and
  `docs/backend-architecture.md`). That inner instance is unrelated to this
  session's compositor and must never be merged with it or assumed to share
  its lifecycle.
- **Do not add XDG autostart scanning, a settings daemon, a notification
  daemon, or any other "just one small desktop service" to this session.**
  The entire point of this design is that Gamescope + Hearthdeck are the
  only two processes in the graphical session. If a feature seems to need a
  desktop-shell-style background service, it belongs in
  `hearthdeck-bridge`/`hearthdeck-daemon` (already-established backend
  services with their own lifecycle under `hearthdeck.target`), not in the
  session itself.

## How to safely change something here

1. Read this file fully — you just did.
2. Make the smallest possible change to `hearthdeck-session`. It should stay
   a short, linear script: environment exports, one blocking service-start
   call, one final `exec`. If your change needs more logic than that, ask
   whether it belongs in the daemon/bridge instead.
3. Test by running the script directly from a TTY (not just re-logging-in
   through the display manager), so you see stdout/stderr immediately:
   ```sh
   sudo systemctl stop display-manager   # or your DM's unit name
   # switch to a free VT, log in as the target user, then:
   /usr/lib/hearthdeck/hearthdeck-session
   ```
4. Confirm backend readiness and the app both came up:
   ```sh
   systemctl --user status hearthdeck.target hearthdeck-bridge.socket hearthdeck-daemon.service
   curl http://127.0.0.1:38400/v1/health
   ```
5. Only after that, re-enable the display manager and test through a real
   login/autologin cycle.

## Troubleshooting

| Symptom | Likely cause | Check |
|---|---|---|
| Session doesn't appear in display manager's session list | `.desktop` file not installed, or `DesktopNames`/session-type mismatch | `ls /usr/share/wayland-sessions/hearthdeck.desktop` |
| Black screen after selecting Hearthdeck Kiosk | Gamescope failed to acquire DRM/KMS (driver quirk, GPU already owned by another seat) | Run `hearthdeck-session` from a TTY per above; read Gamescope's own stderr directly |
| Session starts then immediately returns to login | Hearthdeck (the Flutter binary) crashed on launch | `journalctl --user -b` around the session's start time; run `/opt/hearthdeck/hearthdeck` directly from a TTY inside a manually started `gamescope --backend drm --fullscreen -- bash` shell to isolate Gamescope vs. the app |
| App loads but library/pairing calls fail | `hearthdeck.target` didn't reach ready in time, or bridge/daemon crashed | `systemctl --user status hearthdeck.target hearthdeck-bridge.socket hearthdeck-daemon.service`; `journalctl --user -u hearthdeck-daemon.service -u hearthdeck-bridge.service --since '10 minutes ago'` |
| Everything works but games/apps launched from Hearthdeck don't render | Confusing outer/inner Gamescope, or nested instance failing | This is the *bridge's* nested Gamescope, not the session — see `services/README.md` |
