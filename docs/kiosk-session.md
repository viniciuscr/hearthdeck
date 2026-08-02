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
       -> systemctl --user daemon-reload
       -> systemctl --user start hearthdeck-kiosk.target
            -> Requires=/After= hearthdeck.target
                 -> Wants= hearthdeck-bridge.socket, hearthdeck-daemon.service
       -> exec gamescope --backend drm --fullscreen --force-grab-cursor -- \
            /opt/hearthdeck/hearthdeck
            -> Hearthdeck is Gamescope's ONLY child process
```

`hearthdeck-kiosk.target` exists as this session's single, dedicated,
documented hook for everything it needs started before Gamescope launches
Hearthdeck — see that unit file's own description. `hearthdeck.target`
itself stays the general-purpose target enabled for any session (COSMIC
included); the Kiosk session's own additional startup dependencies (a future
gamepad input daemon, network/Bluetooth adapters, etc.) get added to
`hearthdeck-kiosk.target`, never to the session script itself. The session
script's `systemctl --user start` call intentionally still ends in `|| true`
— Hearthdeck launches either way so its own system-health screen can report
a backend problem, rather than the whole session refusing to start over a
service hiccup — but its output must never be redirected to `/dev/null`; see
"Do not" below.

That's the whole session: one script, one `exec`, three gamescope flags,
one app. Nothing else. No desktop compositor, no panel/dock/launcher, no
overlay, no `--expose-wayland`, no second Wayland client of any kind.

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

- **Do not add anything that connects to Hearthdeck's own outer Gamescope
  instance as a second Wayland client**, for any reason, including an
  in-game overlay, a notification helper, or a settings daemon. This
  session must have exactly one client: Hearthdeck. This is the entire
  cause of the screen-shrink incident above. If a feature needs to render
  something above a running game, it belongs to the *nested* Gamescope
  instance the bridge starts for that specific game/app launch (see
  `services/README.md`), never to this outer one.
- **Do not add `--expose-wayland`, or any other flag, to the outer
  Gamescope invocation without first proving on real hardware — not just
  in CI — that Hearthdeck still fills the entire screen afterward.** A
  passing CI build only proves the code compiles; it says nothing about
  what the session actually looks like on a monitor.
- **Do not put another compositor, window manager, or session manager in
  front of Gamescope.** No COSMIC, no labwc, no sway, no Xorg, no Xwayland
  forcing (`GDK_BACKEND=x11`). Gamescope must be the only thing that opens
  DRM for this session, and Hearthdeck must talk to it over Wayland
  directly, not through a nested Xwayland client.
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
  (`gamescope ... -- /opt/hearthdeck/hearthdeck`), not a separately
  started/tracked process.
- **Do not remove `--backend drm`.** Without it, Gamescope tries to run
  nested inside another Wayland/X11 session, which does not exist here.
- **Do not merge this outer Gamescope with the bridge's nested Gamescope.**
  The bridge spawns a separate, on-demand Gamescope instance only when the
  user launches a game/app from inside Hearthdeck. That instance is
  unrelated to this session's compositor.
- **Do not trust a chat confirmation as proof a specific commit works.**
  See "The real lesson" above. Check CI for the exact SHA.

## How to safely change something here

1. Read this file fully.
2. Make the smallest possible change. The session script should stay a
   short, linear sequence: environment exports, one blocking service-start
   call, one final `exec`.
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
| Hearthdeck renders into a small, centered, blurry fraction of the screen | Something else is a second Wayland client of this outer Gamescope instance (see the incident above), or the installed `gamescope` package itself changed behavior (check `gamescope --version` and whether the same box's non-Hearthdeck compositor, e.g. a normal desktop session, still fills the screen correctly — if it does, the regression is specific to this session, not the display/driver) | `journalctl --user -b`; confirm no other process connects to `$WAYLAND_DISPLAY` during the session; re-read "Do not" above |
| Session starts then immediately returns to login | Hearthdeck (the Flutter binary) crashed on launch | `journalctl --user -b` around the session's start time; run `/opt/hearthdeck/hearthdeck` directly from a TTY inside a manually started `gamescope --backend drm --fullscreen -- bash` shell to isolate Gamescope vs. the app |
| App loads but library/pairing calls fail | `hearthdeck.target` didn't reach ready in time, or bridge/daemon crashed | `systemctl --user status hearthdeck.target hearthdeck-bridge.socket hearthdeck-daemon.service`; `journalctl --user -u hearthdeck-daemon.service -u hearthdeck-bridge.service --since '10 minutes ago'` |
| CI passes but the session doesn't work on hardware, or vice versa | These are different claims (see "The real lesson"). CI only proves the code compiles and packages; it never runs the graphical session | Always confirm on real hardware separately, and never conflate the two when reporting something as "working" |
