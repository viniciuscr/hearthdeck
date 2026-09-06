# Kiosk Session: How It Works, and How Not to Break It

This document exists because this session broke, got "fixed" three different
ways by three different guesses at which commit actually worked, and each
guess was wrong in a way that took real time to untangle. Read this fully
before touching `packaging/arch/hearthdeck-session`,
`linux/runner/my_application.cc`, or anything claiming to be an in-game
overlay.

## What actually runs, in order

```
Display manager (SDDM/GDM/greetd/...)
  -> reads /usr/share/wayland-sessions/hearthdeck.desktop
  -> Exec = /usr/lib/hearthdeck/hearthdeck-session
       -> export XDG_CURRENT_DESKTOP/XDG_SESSION_DESKTOP/XDG_SESSION_TYPE
       -> systemctl --user import-environment (same three variables)
       -> systemctl --user daemon-reload
       -> systemctl --user start hearthdeck-kiosk.target
            -> Requires=/After= hearthdeck.target
                -> Wants= hearthdeck-bridge.socket, hearthdeck-daemon.service,
                    hearthdeck-input.service
       -> systemctl --user try-restart hearthdeck-bridge.service
       -> exec gamescope --backend drm --fullscreen --force-grab-cursor -- \
          /usr/bin/hearthdeck
            -> Hearthdeck is Gamescope's ONLY child process
          -> The Hearthdeck launcher imports
               DISPLAY/WAYLAND_DISPLAY into systemd --user too, and
               try-restarts hearthdeck-bridge.service again - this can only
               happen here, not in the script above, because Gamescope only
               assigns these once it starts, after the script's `exec` has
               already handed off control
```

`hearthdeck-kiosk.target` exists as this session's single, dedicated,
documented hook for everything it needs started before Gamescope launches
Hearthdeck — see that unit file's own description. `hearthdeck.target`
stays the general-purpose target started explicitly by each Hearthdeck
launcher or session; the Kiosk session's own additional startup dependencies
(network/Bluetooth adapters, etc.) get added to
`hearthdeck-kiosk.target`, never to the session script itself. The session
script's `systemctl --user start` call intentionally still ends in `|| true`
— Hearthdeck launches either way so its own system-health screen can report
a backend problem, rather than the whole session refusing to start over a
service hiccup — but its output must never be redirected to `/dev/null`; see
"Do not" below.

The `import-environment`/`try-restart` pair exists because a plain shell
`export` only changes this script's own process environment — it never
reaches the systemd `--user` manager's own activation environment, which is
what every unit it starts actually inherits. `hearthdeck-bridge` reads
`XDG_CURRENT_DESKTOP` from its own process environment to decide whether
it's running inside the Kiosk session at all (`is_kiosk_session()` in
`services/hearthdeck-bridge/src/platform/linux.rs`), which controls whether
a launched app/game is assigned to `hearthdeck-kiosk.slice`. Without the
import, the bridge would silently believe it's running outside the Kiosk
session — even while genuinely inside it — and skip that assignment.

The same import happens a second time, for different variables, from a
different place: Hearthdeck's own native startup (`my_application.cc`)
imports `DISPLAY`/`WAYLAND_DISPLAY` into systemd `--user` too. This has to
happen there and not in the session script, because Gamescope only assigns
those two once it starts — which is after the script has already `exec`'d
into it and lost the ability to run anything else. Without this second
import, every launched app/game would have no display to connect to at all
(confirmed on real hardware: `hearthdeck-bridge` had no `WAYLAND_DISPLAY` in
its own environment during the Kiosk session, at all, until this was added).

That's the whole session: one script, one `exec`, three gamescope flags, one
app. Nothing else runs *as the outer compositor* — no desktop compositor, no
panel/dock/launcher. Launched apps, RetroArch, and every other current
launcher connect directly to this same session as ordinary clients (see
"Launching apps and games" below for why that's different from, and safer
than, adding a second compositor process); nothing new joins this outer
Gamescope instance as *another Gamescope process*, which is the specific
thing that caused the incident below and remains the hard rule.

## Launching apps and games: direct connection, not a second Gamescope

Every desktop app and RetroArch launch (`launch_application`/
`launch_retro_game` in `hearthdeck-bridge`'s `linux.rs`) connects directly to
this same outer session — the same `DISPLAY`/`WAYLAND_DISPLAY` Hearthdeck
itself uses, forwarded via `systemd-run --setenv` — instead of being wrapped
in a nested Gamescope instance the way earlier versions of this bridge did.
That earlier nested-Gamescope-per-launch approach was confirmed, on real
hardware, to never actually get shown: a second Gamescope process joining
this session as an independent Wayland client is composited but never
focused/displayed by the outer instance. A plain client (X11, via this
session's embedded Xwayland `DISPLAY`) *is* shown automatically, the same
way Hearthdeck itself is — that's the mechanism this now relies on.
Making a plain client that doesn't manage its own fullscreen state (e.g. a
bare `xterm`) fill the screen without per-app configuration is still an open
problem: `--force-windows-fullscreen` on the outer Gamescope invocation was
tried and **caused a black screen after login on real hardware** (all
expected processes stayed alive - Hearthdeck, Gamescope, Xwayland - nothing
crashed, but nothing rendered either), so it was reverted. Its own
documentation talks about "the nested display", which may mean it doesn't
behave the same way against `--backend drm` standalone mode as it does in
genuinely nested mode; this needs a real fix, tested on hardware, not
another blind flag addition.

Heroic used to keep a nested Gamescope instance of its own for its cold-start
launch (SDL backend, presenting as an ordinary X11 client the way this whole
approach relies on) - **reverted**: it added a second real GPU-compositing
pass (outer Gamescope compositing the nested one compositing the actual
game) on top of the direct-scanout path the outer instance already provides,
and was confirmed on real hardware to make game performance noticeably
worse than the same game launched from a normal desktop session. Its only
actual justification, `--keep-alive` preventing Gamescope from tearing down
the display when a wrapped `heroic` cold-start process exits almost
immediately, doesn't apply once nothing is wrapping it in a Gamescope
instance at all - so Heroic's cold start is now `heroic --no-gui` exec'd
directly (not via `xdg-open` - see `services/README.md` for why that
specifically broke the overlay's close button), direct-connecting to
this session exactly like every other launch. Custom per-game internal
resolution/upscaling, the one thing the nested instance would have
provided, is not available today as a result; it would need a real answer
to the same performance question before being reintroduced, not just
re-added and hoped for.

**Not yet verified on hardware:** whether a genuinely native-Wayland client
(no Xwayland involved) connecting directly via `WAYLAND_DISPLAY` is shown
the same automatic way a plain X11 client is, or whether it hits the same
"connected but never shown" problem the nested-Gamescope-as-Wayland-peer
case did. Both `DISPLAY` and `WAYLAND_DISPLAY` are forwarded to every launch
today so an app can use whichever it prefers; if a native-Wayland app turns
out invisible the same way, forcing Xwayland (unset `WAYLAND_DISPLAY` for
the launched process, forcing an X11-capable app down that path) is the
likely fix, but this hasn't been tested yet.

## Controller compatibility

`hearthdeck-input.service` creates one virtual keyboard/mouse and observes
physical controllers without an evdev grab. The library context menu enables
the desktop profile per application; native remains the default. The bridge
owns profile activation because it also owns the managed-session lifetime, and
always restores native input when that session stops or launch setup fails.

The broker remaining nonexclusive is intentional. It lets the Guide button
continue to reach `hearthdeck-overlay`; when the overlay is visible, its own
exclusive controller grab prevents mapped events from leaking to the launched
application. Do not enable compatibility for controller-native games: both the
original gamepad events and emulated keyboard/mouse events would reach them.

## The incident: a full account, in order

This is the timeline of what actually happened, reconstructed from commit
history and CI results, not from memory or chat transcripts (see "The real
lesson" below for why that distinction matters).

1. **The session was broken for a long time** under a COSMIC-desktop-based
   design (`hearthdeck-cosmic-session` → `cosmic-session.target` →
   `hearthdeck-kiosk.service`, gated by a `ConditionEnvironment=` check).
   That design had several silent-failure points: a `ConditionEnvironment=`
   that skips a unit with no error if it doesn't match, a Wayland-socket
   hand-off between COSMIC and systemd that could race, and Hearthdeck
   started as a separate systemd unit rather than the compositor's direct
   child. It was replaced with the plain Gamescope design above.

2. **That replacement worked and was confirmed on real hardware.** Its CI
   build passed. This is the commit this document now describes, and the
   one the tree is reset to.

3. **An in-game overlay feature was added afterward**, first using
   `libcosmic`/`iced` as its UI toolkit. This overlay crate **never
   successfully built in CI** — its first CI run failed with eight
   compile errors (`cosmic::Task::perform` signature mismatches, an
   `Application` trait not being in scope, `evdev::KeyCode::BTN_GAMEPAD`
   not existing in the pinned `evdev` version, a borrow-checker conflict).
   Despite that, further work continued on top of it in the same working
   tree.

4. **The overlay was wired into Hearthdeck's own session startup.**
   Hearthdeck's GTK runner (`linux/runner/my_application.cc`) started
   started `hearthdeck-overlay` as its own child immediately after the
   first frame rendered, *unconditionally*, for the entire time Hearthdeck
   was running — not only while a game was active. The outer session's
   `gamescope` invocation also gained `--expose-wayland` to let that
   overlay process connect to the same compositor. This made the overlay
   a second, persistent Wayland client of the same Gamescope instance
   that was supposed to have exactly one client: Hearthdeck. Gamescope
   stopped sizing its output to Hearthdeck alone. **The visible result was
   Hearthdeck rendering into a small, centered, blurry fraction of the
   screen instead of filling it** — this is the bug that triggered this
   whole incident.

5. **The overlay was rewritten** from `libcosmic`/`iced` to
   `smithay-client-toolkit` (raw Wayland/layer-shell), fixing the original
   compile failure. Further bugs in that rewrite (D-pad wired to the wrong
   axis, the overlay never hiding once shown because it relied on
   attaching a null buffer instead of destroying the surface) were fixed
   on top of it.

6. **The screen-shrink bug was diagnosed and "fixed"** by removing the
   overlay's auto-start from Hearthdeck's runner, on the theory that
   `--expose-wayland` itself was harmless plumbing needed for nested game
   launches. That fix's CI build passed. It was never independently
   re-confirmed working on real hardware before more work landed on top
   of it.

7. **More commits landed directly on `main`** attempting further
   "restores" of the session — reintroducing an even older
   `hearthdeck-console-session`/`hearthdeck-gamescope-xterm` design that
   had already been abandoned, then forcing the client through Gamescope's
   nested Xwayland with `GDK_BACKEND=x11`. The screen problem persisted
   through all of this, because none of it addressed the actual
   regression (step 4) and some of it reintroduced designs that were
   already known not to work.

8. **Multiple "revert to the working commit" attempts followed, and the
   first two picked the wrong commit:**
   - The first attempt reverted to the "fix `--expose-wayland` theory"
     commit from step 6 — never independently confirmed working, and
     still carrying an unverified assumption.
   - The second attempt reverted to the commit right before the overlay
     was wired into the session at all — but that commit's overlay crate
     was the **original, never-CI-passing** `libcosmic` version from step
     3. A package built from that exact commit could not have run at all,
     let alone rendered a working session, because it never compiled.
   - The third and correct attempt cross-referenced **CI run results**
     (`gh run list`) against **commit timestamps**, instead of relying on
     when a chat message said something "worked." That is what led back
     to the commit described in this document: the original, plain
     Gamescope session, confirmed both by a passing CI build and by real
     hardware, with **no overlay code in the tree at all**.

## The real lesson

Every wrong turn in this incident came from the same mistake: **treating
"a person said it worked" as equivalent to "this exact commit is known
good."** Those are not the same claim. A chat confirmation tells you
something worked at some point in time, on some build, running on some
machine — it does not tell you which commit produced that build, and it
is dangerously easy to associate it with the wrong one, especially once
several commits have landed close together.

**The only reliable way to know a commit is safe to build from is to check
its actual CI result**, not to reason about when someone said something
worked:

```sh
gh run list --repo <owner>/<repo> --workflow "Arch Linux Package" --limit 20 \
  --json headSha,conclusion,createdAt
```

Cross-reference the exact commit SHA you're about to build from or revert
to against that list before trusting it. If you cannot check CI, do not
claim a commit is "the confirmed-working one" — say plainly that you don't
know, and find out.

## Do not

Each of these previously caused a real, hours-long regression. They are not
style preferences.

- **Do not add a second Gamescope/compositor process that connects to
  Hearthdeck's own outer Gamescope instance as a Wayland peer**, for any
  reason, including an in-game overlay, a notification helper, or a settings
  daemon. This is the entire cause of the screen-shrink incident above, and
  separately confirmed again on real hardware: a second Gamescope process
  joining this session as a Wayland client is composited but never actually
  shown. Ordinary clients (an X11 app via this session's own `DISPLAY`, or
  RetroArch/desktop-app launches — see "Launching apps and games" above) are
  fine and expected; **another Gamescope process** claiming to be a peer is
  the specific thing that's both forbidden and, separately, confirmed not to
  work. If a feature needs to render something above a running Heroic game
  specifically, it belongs to *Heroic's own* nested Gamescope instance (see
  `services/README.md`) — every other launch has no nested instance to
  attach to at all anymore.
- **Do not add `--expose-wayland`, or any other flag, to the outer
  Gamescope invocation without first proving on real hardware — not just
  in CI — that Hearthdeck still fills the entire screen afterward.** A
  passing CI build only proves the code compiles; it says nothing about
  what the session actually looks like on a monitor. This rule was violated
  once already: `--force-windows-fullscreen` was added here, reasoned
  through carefully, shipped with clippy/tests/fmt all green, and produced a
  black screen on the first real-hardware boot (every process stayed alive
  - Hearthdeck, Gamescope, Xwayland - nothing rendered). Green CI and sound
  reasoning are not what this rule asks for.
- **Do not put another compositor, window manager, or session manager in
  front of Gamescope.** No COSMIC, no labwc, no sway, no Xorg, no Xwayland
  forcing (`GDK_BACKEND=x11`) *for Hearthdeck itself*. Gamescope must be the
  only thing that opens DRM for this session, and Hearthdeck must talk to it
  over Wayland directly, not through a nested Xwayland client. (Launched
  apps/games are a different case — see "Launching apps and games" above;
  they're expected to use this session's embedded Xwayland.)
- **Do not gate Hearthdeck's startup behind a separate systemd unit with a
  `ConditionEnvironment=` (or similar) check.** The old
  `hearthdeck-kiosk.service` used
  `ConditionEnvironment=XDG_CURRENT_DESKTOP=hearthdeck:COSMIC`. When a
  systemd `Condition*=` fails, the unit is skipped silently: no error, no
  obvious symptom, `systemctl status` just shows it never ran.
- **Do not background (`&`) the `systemctl --user start hearthdeck-kiosk.target`
  call**, and do not swallow its output with `>/dev/null 2>&1`. Real
  failures need to be visible in the journal, not hidden. (The script did
  exactly this for a time despite this rule already being written down here
  — check the actual file before trusting this document's own description
  of it.)
- **Do not run Hearthdeck as a systemd unit that is a sibling of Gamescope.**
  Hearthdeck must be Gamescope's literal child
  (`gamescope ... -- /usr/bin/hearthdeck`), not a separately
  started/tracked process.
- **Do not remove `--backend drm`.** Without it, Gamescope tries to run
  nested inside another Wayland/X11 session, which does not exist here.
- **Do not add a nested Gamescope instance for any launch, including
  Heroic.** This used to be the default for every launch, then specifically
  for Heroic's cold start, and both were tried and reverted: neither
  actually got composited content shown reliably (desktop apps/RetroArch),
  and the Heroic case additionally measured a real, noticeable performance
  regression from the extra GPU-compositing pass, on top of not being
  needed at all once nothing wraps the cold-start `heroic` process in a
  compositor to begin with.
  Direct connection is not a shortcut taken for convenience - it's the
  thing proven to work, twice now, after the nested version was proven not
  to (or proven worse).
- **Do not launch Heroic via `xdg-open`.** Tried, and confirmed on real
  hardware to silently break the overlay's close button: `xdg-open`
  resolves the custom `heroic://` scheme through `gio open`/`gio launch`,
  which has its own systemd integration
  (https://systemd.io/DESKTOP_ENVIRONMENTS/) and unconditionally starts
  registered `.desktop` apps in a *new* `app-<name>-<pid>.scope` under
  `app.slice` - migrating Heroic, and everything it spawns (wineserver, the
  game itself), out of `hearthdeck-heroic.service`'s cgroup entirely.
  `systemctl --user stop hearthdeck-heroic.service` then only ever reached
  `xdg-open` itself and a couple of early zygote helpers, never the actual
  game. Exec the `heroic` binary directly instead (see
  `services/README.md`); Heroic reads the launch URI from its own argv, so
  it never needed `xdg-open`/`gio` in the first place.
- **Do not remove the `systemctl --user import-environment` call, or "simplify"
  it away as redundant with the `export` lines above it.** It looks
  redundant; it is not. The `export`s only affect this script's own process;
  the import is what actually reaches the systemd `--user` manager's
  activation environment, which is what `hearthdeck-bridge` (lazily started
  by socket activation, possibly well after this script exits) actually
  inherits. Removing it silently breaks `is_kiosk_session()` — the bridge
  would believe every launch is happening outside the Kiosk session, so
  apps/games stop getting assigned to `hearthdeck-kiosk.slice`.
- **Do not trust a chat confirmation as proof a specific commit works.**
  See "The real lesson" above. Check CI for the exact SHA.

## How to safely change something here

1. Read this file fully.
2. Make the smallest possible change. The session script should stay a
   short, linear sequence: environment exports, importing them into
   systemd `--user`, one blocking service-start call, one final `exec`.
3. Test by running the script directly from a TTY, not just by
   re-logging-in through the display manager, so you see stdout/stderr
   immediately:
   ```sh
   sudo systemctl stop display-manager   # or your DM's unit name
   # switch to a free VT, log in as the target user, then:
   /usr/lib/hearthdeck/hearthdeck-session
   ```
4. Confirm on the actual monitor that Hearthdeck fills the entire screen,
   not just that the process started. A running session and a correctly
   sized session are different claims; this incident happened because
   that distinction was skipped.
5. Confirm backend readiness:
   ```sh
   systemctl --user status hearthdeck.target hearthdeck-bridge.socket hearthdeck-daemon.service
   curl http://127.0.0.1:38400/v1/health
   ```
6. Check the exact commit's CI result before calling it "confirmed
   working" in any commit message, PR description, or chat message:
   ```sh
   gh run list --repo <owner>/<repo> --workflow "Arch Linux Package" --limit 20 \
     --json headSha,conclusion,createdAt
   ```
7. Only after all of that, re-enable the display manager and test through
   a real login/autologin cycle.

## Troubleshooting

| Symptom | Likely cause | Check |
|---|---|---|
| Session doesn't appear in display manager's session list | `.desktop` file not installed, or `DesktopNames`/session-type mismatch | `ls /usr/share/wayland-sessions/hearthdeck.desktop` |
| Black screen after selecting Hearthdeck Kiosk | Gamescope failed to acquire DRM/KMS (driver quirk, GPU already owned by another seat) | Run `hearthdeck-session` from a TTY per above; read Gamescope's own stderr directly |
| Hearthdeck renders into a small, centered, blurry fraction of the screen | Something else is a second Gamescope/compositor process attached to this outer instance (see the incident above), or the installed `gamescope` package itself changed behavior (check `gamescope --version` and whether the same box's non-Hearthdeck compositor, e.g. a normal desktop session, still fills the screen correctly — if it does, the regression is specific to this session, not the display/driver) | `journalctl --user -b`; confirm no other *Gamescope* process connects to `$WAYLAND_DISPLAY` during the session (ordinary app/game clients are expected and fine); re-read "Do not" above |
| Session starts then immediately returns to login | The Hearthdeck frontend crashed on launch | `journalctl --user -b` around the session's start time; run `/usr/bin/hearthdeck` directly from a TTY inside a manually started `gamescope --backend drm --fullscreen -- bash` shell to isolate Gamescope vs. the app |
| App loads but library/pairing calls fail | `hearthdeck.target` didn't reach ready in time, or bridge/daemon crashed | `systemctl --user status hearthdeck.target hearthdeck-bridge.socket hearthdeck-daemon.service`; `journalctl --user -u hearthdeck-daemon.service -u hearthdeck-bridge.service --since '10 minutes ago'` |
| Launched apps/games aren't in `hearthdeck-kiosk.slice` | `hearthdeck-bridge` doesn't see `XDG_CURRENT_DESKTOP=hearthdeck` in its own environment, so `is_kiosk_session()` returns false even though this genuinely is the Kiosk session | `systemctl --user show-environment \| grep XDG_CURRENT_DESKTOP`; confirm the `import-environment` line in `hearthdeck-session` still runs and still lists all three variables |
| Trigger/launch succeeds (accepted, no errors in daemon/bridge logs), but nothing appears on screen | Either the launched process has no `DISPLAY`/`WAYLAND_DISPLAY` to connect to (if `hearthdeck-bridge`'s own environment is missing them), or - if using a nested Gamescope for something other than Heroic's own SDL-backend instance - it's connected but never shown, the confirmed-broken pattern described in "Launching apps and games" above | `systemctl --user show-environment \| grep -iE 'display\|wayland'`; confirm `/usr/bin/hearthdeck` imported the display environment; check `journalctl --user -u 'hearthdeck-app-*' -u hearthdeck-heroic.service` for the specific launch's own stderr, which the daemon/bridge logs never show |
| CI passes but the session doesn't work on hardware, or vice versa | These are different claims (see "The real lesson"). CI only proves the code compiles and packages; it never runs the graphical session | Always confirm on real hardware separately, and never conflate the two when reporting something as "working" |
