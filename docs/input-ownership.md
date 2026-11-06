# Input ownership and host acceptance

Status: **broker only; hardware acceptance pending**. The user authorized
removal of the legacy input path. `HEARTHDECK_INPUT_MODE` is no longer read;
remove it from existing input.env files. There is no runtime reader switch.

## Why change it

The former frontend, overlay and compatibility-service readers could not
reliably agree which surface owned a press. They have been removed. The
[broker](../services/hearthdeck-input/src/broker.rs) now owns Hearthdeck's
physical controller reads and uses the
[Linux adapter](../services/hearthdeck-input/src/linux.rs) for device I/O.

## Broker ownership

`hearthdeck-input` is Hearthdeck's controller broker. It owns device discovery,
normalization, held state, repeat,
chords, capture and compatibility output. Frontend and overlay consume semantic
events; neither opens physical controllers.

"One reader" means one **Hearthdeck** reader, not exclusive ownership at all
times. Native games keep their physical controller access during gameplay.
Permanently grabbing controllers would require virtual gamepad forwarding,
mapping and rumble support; that is not implemented.

The daemon remains the authority for managed game sessions. The broker owns
input delivery, not launch, process supervision or UI navigation. Frontend
events still pass through the existing global Back and focus contracts.

### Delivery states

| State | Delivery |
| --- | --- |
| Frontend focused, no capture | Semantic events to frontend; compatibility output off |
| Launch handoff or unrelated window focused | No frontend navigation; release synthetic keys |
| Native game | Physical gamepad stays available to game; observe system chords only |
| Desktop-profile app | Existing keyboard/mouse mapping; no frontend navigation |
| Overlay capture | Semantic events only to overlay; compatibility output off; physical events excluded from game |
| Keyboard capture | Keyboard owns its controls; underlying frontend/overlay navigation and compatibility mapping suspended |

Keyboard and overlay capture are mutually exclusive. Opening the overlay from
a keyboard must first dismiss that keyboard and confirm release; otherwise
reject the transition visibly. Game exit returns to frontend only once focus
and session state agree. A running but background game must not prevent use of
the focused frontend.

Focus alone is not a capture grant. Versioned, newline-delimited JSON on
`$XDG_RUNTIME_DIR/hearthdeck-input-v1.sock` carries role, requested delivery and
Ready/Action/Failed replies. Frames are capped at 1024 bytes; per-client queues
hold 32 events and writes time out after two seconds. The socket is mode 0600
and peers must have the broker's UID. Roles restrict allowed delivery modes,
not mutually distrustful applications: another process with the same UID can
impersonate a role. There is one live connection per role; replacement revokes
the old connection's ownership. Ready acknowledges successful capture.

The bridge sends session context on the existing input datagram socket.
Desktop mapping and game keyboard availability require a matching active COSMIC
toplevel app ID and context received within five seconds. An unknown app ID or
unavailable focus protocol disables them rather than targeting an unrelated
window. Desktop IDs are matched exactly after removing a `.desktop` suffix;
RetroArch has an explicit app-ID list. Host acceptance must check real IDs.

## Buttons and keyboards

- **Guide release:** toggle overlay, unless that hold already triggered a chord.
  Unlike the removed reader, this waits for release. A long hold
  alone has no extra action.
- **Guide then X on the same controller:** toggle the in-game keyboard when
  enabled; consume the Guide action. X then Guide is not a keyboard chord.
  One trigger per hold; no overlay flash. Guide+X elsewhere does not invoke an
  in-game keyboard.
- **Start+Select in frontend:** keep current behavior in either press order.
  Select alone still opens the filter drawer on release.
- **Frontend text:** retain caret-driven cosmic-osk.
- **Games:** retain gamepad-osk until cosmic-osk is proven usable in native games
  and desktop-profile apps. Do not remove it for architectural symmetry.
- **Multiple controllers:** preserve shared navigation, but held/chord state is
  per device. Buttons from different pads must never combine into one chord.

Observation alone cannot consume events already delivered to a native game.
Without virtual forwarding, Guide/X and the press opening capture may also be
seen by that game. The no-leak guarantee begins at acknowledged exclusive
capture, not at Guide press. Approving this limitation is part of the decision.

The game keyboard uses an explicit foreground adapter. Before spawning it the
broker releases synthetic input and closes all physical controller descriptors;
it stops scanning until that process exits. It then rediscovers devices and
requires neutral. This is a deliberate handoff to an external reader, not semantic
keyboard delivery. Without a configured adapter, the request shows an error.
`gamepad-osk --toggle` is not a valid adapter: it can exit while the keyboard is
still active. The installed keyboard's actual lifetime/capture behavior must
be verified on the host; no adapter is invented or enabled by default.

## Capture and failure rules

1. Release synthetic keys and stop pointer motion before changing ownership.
2. Grant overlay capture only after every participating device is successfully
   grabbed. Roll back partial grabs on failure; show an error, leave the game
   usable, and do not accept interactive overlay navigation.
3. Newly attached devices cannot deliver capture events before being grabbed.
   If a new device cannot be captured, revoke the capture and report the failure.
4. On release, require neutral buttons/sticks before arming the next owner;
   never replay buffered presses into a newly active surface.
5. On device disconnect, clear only that device's held/chord/repeat state and
   release its contributed synthetic keys. Other pads remain usable.
6. On client disconnect, revoke its capture and release grabs. On broker crash,
   OS file-descriptor cleanup releases grabs and removes virtual output; verify
   applications do not retain stuck input. Do not silently restore capture
   after restart.
7. Missing broker disables controller navigation with a visible status; it must
   not start competing fallback readers. Physical keyboard/mouse and the
   documented session recovery path remain available.
8. Use bounded delivery. If a consumer cannot keep up, disconnect and revoke its
   ownership rather than drop releases or replay a stale navigation backlog.

## Migration status

The ownership state machine, local transport and consumer migration are
implemented. Legacy readers and their mode selection have been removed together.
Native and desktop compatibility profiles still exist; they select delivery
behavior inside the broker, not a different reader.

Run Linux acceptance below. Removing the rollback path is an explicit product
decision, not evidence that hardware checks passed.

## Linux acceptance gate

Automated tests prove state decisions, not kernel grabs, game input or keyboard
usability. Run these checks in the real COSMIC session with one wired controller,
one Bluetooth controller and both connected together. Record model, connection,
kernel, compositor and tested commit; untested combinations remain unaccepted.

- [ ] Frontend: D-pad/stick repeat, all existing actions, global Back and
  Start+Select in both orders; no double navigation or cross-pad chords.
- [ ] Native game: normal controls and rumble still work; Guide opens once on
  release; Guide+X opens only the keyboard. Record any game-visible chord input.
- [ ] Desktop-profile app: keyboard/mouse mapping works; holding a mapped key
  through capture and return leaves no stuck key or pointer motion.
- [ ] Overlay: after capture acknowledgment, direction/confirm do not reach
  game. Resume, Close App and Guide dismissal work without replaying held input.
- [ ] Keyboard: type and dismiss in frontend, native game and desktop-profile
  app; only the intended surface reacts. Overlay/keyboard transitions release
  the old capture before activating the next.
- [ ] Hot-plug: unplug a held pad, reconnect, attach another during capture;
  neutral arming and per-device cleanup work in each state.
- [ ] Failures: deny a grab, fail one of two grabs, terminate each consumer,
  terminate/restart broker, and stall a consumer. No partial capture, stuck
  keys, silent fallback reader or stale event replay.
- [ ] Recovery: physical keyboard/mouse can recover the session after broker
  loss; game exit, failed launch, focus return and reconnect restore only the
  rightful owner.

Implementation approval: granted 2026-11-04. Hardware validation: delegated by
the user to a host agent; acceptance evidence is still pending. Broker-only
delivery is explicit user direction, not hardware validation.

## Host-agent runbook

Use the implementation PR's exact commit, not a package predating the broker.
Perform these steps from a recoverable physical keyboard/SSH session. Do not
restart an active game just to run the checklist.

1. Build/install all changed binaries together: protocol consumers include the
   frontend, overlay, bridge and input service. Install the matching packaged
   units and session script; do not mix checkout and packaged units.
2. Save any existing `~/.config/hearthdeck/input.env`. Copy
   `/usr/share/doc/hearthdeck/input.env.example` there for keyboard-adapter
   configuration. Remove any `HEARTHDECK_INPUT_MODE` line; it has no effect.
   Use plain `KEY=value` assignments, no shell
   expansion: both systemd and the session script read this file.
3. Leave `HEARTHDECK_GAMEPAD_OSK_ADAPTER` empty for the first pass. For the
   keyboard pass set it to an absolute executable path whose foreground process
   lasts exactly as long as the visible keyboard. Verify dismiss, crash and
   controller release behavior independently before trusting the adapter.
4. Reload user units, then log out and start a fresh Hearthdeck session.
   A whole-session restart is required; changing only one service creates a
   mixed-version setup.
5. Inspect unit environments with `systemctl --user show` for the input,
   bridge and overlay services using `-p Environment`, plus actual running
   process environments. Confirm the adapter setting agrees across processes.
   Check the user journal for `semantic input broker ready`, ownership
   changes and capture errors. A running unit is not proof of capture.
6. Run every acceptance item above. Record commit, hardware, actual app IDs,
   commands, observed results and journal excerpts (without credentials).
   Mark each item PASS/FAIL/BLOCKED, including keyboard-adapter absence as
   BLOCKED, never PASS.

Recovery: after a broker failure, use physical keyboard/mouse, SSH or a TTY to
inspect the journal and restart `hearthdeck-input.service`. Reconnect does not
silently restore capture. If a release must be reverted, reinstall a complete
known-good package and restart the whole session; an environment change cannot
restore removed readers.
