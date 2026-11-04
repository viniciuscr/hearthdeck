# Input ownership proposal

Status: **proposed, not approved or implemented**. This document is the decision
and acceptance gate for consolidating Hearthdeck's controller readers. Merging
this proposal alone does not authorize changing controller behavior or protocols.

## Why change it

Hearthdeck currently has three independent readers:

- [Frontend gilrs subscription](../services/hearthdeck-frontend/src/subscriptions/gamepad.rs)
  normalizes navigation, repeat and Start+Select.
- [Overlay evdev subscription](../services/hearthdeck-overlay/src/input.rs)
  opens on Guide press and attempts exclusive grabs while visible. A failed
  grab currently permits navigation to leak to the game.
- [Input compatibility service](../services/hearthdeck-input/src/linux.rs)
  maps controllers to keyboard/mouse and invokes `gamepad-osk` on Guide+X.

Guide currently opens the overlay before a Guide+X chord can be resolved.
Separate readers cannot reliably agree which surface owns that press.

## Proposed ownership

Extend `hearthdeck-input`, rather than add another service, into Hearthdeck's
controller broker. It owns device discovery, normalization, held state, repeat,
chords, capture and compatibility output. Frontend and overlay consume semantic
events; neither opens physical controllers after migration.

"One reader" means one **Hearthdeck** reader, not exclusive ownership at all
times. Native games keep their physical controller access during gameplay.
Permanently grabbing controllers would require virtual gamepad forwarding,
mapping and rumble support; that is not part of this proposal.

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

Focus alone is not an authenticated capture grant. The implementation needs a
local, user-only connection with explicit client identity, acknowledged capture
and disconnect cleanup. Do not select or publish a wire schema in this proposal.
Protocol and startup changes require separate review before implementation.

## Proposed buttons and keyboards

- **Guide release:** toggle overlay, unless that hold already triggered a chord.
  This intentionally changes today's press-triggered behavior. A long hold
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

gamepad-osk may itself read controllers. Audit its actual capture and injection
behavior before integrating keyboard ownership. Keep an explicit legacy
adapter during rollout if needed; do not claim all readers are consolidated
until that dependency is verified. Keyboard delivery remains a release gate.

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

## Staged rollout

1. **Approve this contract and assign hardware validation.** Confirm Guide
   release timing, native-game chord leakage, keyboard scope and recovery
   behavior. Record approval and acceptance evidence here.
2. **Implement pure ownership/chord state.** Test transitions, per-device held
   state, failed acquisition rollback and disconnect cleanup without devices.
   Reuse existing mapper and navigation behavior, including neutral arming.
3. **Review and implement local transport and lifecycle.** Define authenticated
   client roles, acknowledgments, reconnect and bounded delivery. Review systemd
   changes explicitly. No production capture or dual-reader switch yet.
4. **Migrate consumers together behind an explicit opt-in.** Disable their old
   device readers in broker mode; preserve one coherent legacy mode for rollback.
   Do not add a fourth production reader as a permanent bridge.
5. **Run Linux acceptance below.** Only then consider broker mode the default.
   Remove legacy readers after the rollback requirement is explicitly retired.

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

Approval: pending. Hardware validation owner: unassigned. Acceptance evidence:
none. Runtime changes are blocked until the applicable gates are satisfied.
