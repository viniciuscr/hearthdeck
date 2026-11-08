# The settings window

Settings is a **separate process**: `/usr/lib/hearthdeck/hearthdeck-settings`,
drawn with GPUI, launched by the library app when the Settings destination is
opened. This doc owns that decision, the rollback, and the contract the window
keeps.

## Why two processes

GPUI renders through its own window and event loop. There is no host/renderer
seam between it and libcosmic/iced — no iced `Element` can contain a gpui tree —
so a screen cannot be half-migrated inside one window. The split is therefore
drawn where it is cheap: **the window only reads.** RomM sign-in, Stremio
linking, categorization and BIOS sync stay in the iced page, which already owns
those flows and their daemon calls.

`services/hearthdeck-settings/` is the window. `app/settings_window.rs` is the
shell's half: which binary, whether to launch it, whether one is already up.

### Wayland only, on purpose

The crate depends on GPUI directly instead of the `gpui-kit` facade. The facade
hard-enables `gpui-pre-platform`'s `x11` feature beside `wayland`, and Cargo
features are additive, so every kit user links the X11 backend — that is where
the `libxkbcommon-x11` link failure came from, not from anything the window
does. Depending on `gpui-pre`, `gpui-component` and `gpui-pre-platform` with
`wayland` alone removes the backend: the packaged binary links `libxkbcommon`
and nothing else from the X11 stack.

The one exception is a **dev-dependency on GPUI's `test-support`**, which pulls
the `x11` feature for `cargo test` only: the headless window harness is what
draws the page and reports element bounds. Dev-dependencies are not part of a
normal build, so `cargo build --release` — what the package runs — still links
Wayland alone (`ldd` shows `libxkbcommon.so.0`, no `libX11`/`libxcb`).

## The surface

The window is a **full surface, not a dialog**: it opens fullscreen and draws no
top bar of its own. It is opened from a fullscreen kiosk app, so a floating
panel reads as a utility window over a TV picture; a full surface reads as a
destination, which is what it is.

One layout, two levels:

- a page column on the left (`Status`, `Connections`, `Library`, `Emulation`),
  sized for a sofa;
- the reading column on the right, capped at `CONTENT_MAX_WIDTH`, holding the
  page title, its groups, and their rows.

Each group is a **card**. A card is not one element: the page is a single
scrolling `list`, one item per card heading, card row, or gap between cards, so
the cursor can scroll an individual row into view. The card surface is therefore
drawn by each item with the same colour, width and corner rules — the heading
rounds the top, the card's last row rounds the bottom, and a rule separates the
rows — so the card still reads as one surface. The surface uses the same
`group_box` token and radius `GroupBox`'s `Fill` variant uses; `GroupBox` itself
cannot wrap the rows because a card split across list items has no single parent.

The page heading — title, one sentence, and a rule — does not scroll. Only the
cards do. The page's one-line notice is an `Alert` under the list, above the
footer, where it cannot scroll away from the action that produced it.

A card's heading carries the section name and a status chip (`Tag`) when the
section has a state worth reading at a glance: `Daemon` says Connected or
Unavailable, `RomM` Signed in / Needs sign-in / Not set up, `Stremio` Linked /
Waiting / Not linked, `TMDB` Preview. The rows still carry the detail; the chip
is what lets a glance down the page tell the three connections apart.

A row is a label, an optional sentence under it, and one control on the right.
There are five shapes, and the shape decides what Enter does:

| Shape | Control | Enter |
| --- | --- | --- |
| Fact | a state or measurement | nothing |
| Text | a real `Input` | focuses the field |
| Toggle | a `Switch` | flips it |
| Choice | the current option in a chip | cycles to the next option |
| Action | a `Button` | runs it |

Adding a setting is adding a row, not adding a screen.

## Editing: what is wired

**Every row is wired. The connections share one approval flow.**

`Connections` → `RomM` and `Stremio` run the **same** flow, because the daemon
answers both with the same shapes: `Link …` posts the service's `/link` endpoint
(RomM also sends the server address), the code and its link become rows of their
own, and the window polls the matching `GET …/link` every two seconds.
`waiting` updates the countdown, `linked` stores what the daemon reports and
drops the rows, and `expired`, `denied` or `idle` say so and offer the button
again. Each poll belongs to a generation counter, so replacing or unlinking a
pending code stops the older loop instead of writing over the newer one. Typing a
password is gone: both services are approved on another device, and neither the
session nor a password ever crosses this process.

`Unlink` / `Forget connection` post the matching `DELETE` and appear only while a
connection exists. A saved RomM address prefills the field at launch, so linking
again starts from where the daemon already is.

`Connections` → `TMDB` is the one **pasted credential**: a masked field and
`Save key`, written to `PUT /v1/tmdb/settings` on the loopback **admin** listener
— the port the contract serves it on, and the one this window already uses to
pair. It is the only write here that does not carry the pairing token. The card's
chip reads `Set` / `Not set` / `Unavailable` from the same endpoint.

`Library` → `Restore rails` deletes the collections the last scan published
(`DELETE /v1/categorization/collections`), so the dashboard falls back to what it
derives itself.

`Emulation` → `Install missing cores` posts `/v1/retro/cores/install` and
`Sync BIOS` posts `/v1/retro/readiness/sync`. Both answer with a per-package or
per-file report, which the window turns into one line under the page — naming the
first package or file that failed — and then re-reads `/v1/retro/readiness`, so
the card's `Systems` row shows what can play now. The window never chooses a
package: the daemon names them from its own platform-to-core table, and the
bridge re-validates them before the privileged helper runs.

`Library` → `Group the library` is a switch the daemon owns: the value comes from
`GET /v1/categorization`, the press becomes `POST /v1/categorization/enable` (or
`/disable`), and the control settles on the daemon's answer. While the write is in
flight the switch shows the value that was asked for; if the daemon refuses, the
error is printed under the page and the switch falls back to the daemon's answer,
which never changed. `Scan now` posts `/v1/categorization/scan`, is disabled while
the opt-in is off, and reports the refusal (409) when it comes.

`Library` → `Restore rails` deletes the collections the last scan published
(`DELETE /v1/categorization/collections`), so the dashboard falls back to what it
derives itself.

`Emulation` → `Install missing cores` posts `/v1/retro/cores/install` and
`Sync BIOS` posts `/v1/retro/readiness/sync`. Both answer with a per-package or
per-file report, which the window turns into one line under the page — naming the
first package or file that failed — and then re-reads `/v1/retro/readiness`, so
the card's `Systems` row shows what can play now. The window never chooses a
package: the daemon names them from its own platform-to-core table, and the
bridge re-validates them before the privileged helper runs.

`Appearance` is not in this window on purpose: palette, backdrop and motion stay
in the library app, which is where they are chosen and stored.

### Backup

`Backup` is the fifth page, and the only one whose switch does not write on the
press. The whole decision — destination, schedule, and whether the timer runs —
is one request, so the switch marks a draft and `Save and apply` writes it with
everything else. The daemon validates them together, which is why a refused write
leaves the old configuration whole instead of half-applied.

`Remote` and `Destination root` are text rows whose values come from
`GET /v1/backup/settings`. When `remote_available` is false the daemon knows
rclone does not list that remote: a `Remote not found` row explains what to do,
and the switch is drawn disabled — arming a timer that cannot run would only fail
nightly. Every row on the page is dead in the same way when the units are not
installed: the 503 becomes a `Not installed` chip and a single explanatory row,
never an empty switch.

`Schedule` is the kit's `TimeField` (`gpui-base`'s `TimeFieldState`), not a
stepper written here: up and down step the selected part, left and right select
the part, and digits type it. The pad does need one small piece for this —
`pad_key`, which turns a pad press into the *keystroke* the field already binds
while a field owns the caret. That is the whole of it: the d-pad steps the
schedule and moves a caret through the path the keyboard uses, rather than a
second editing scheme.

`Last run` reads `GET /v1/backup/status`: the next occurrence, the service's own
`Result` (with its exit status when it failed), when it finished, and the last
journal line when it failed. `Run now` posts and returns accepted; the window
then follows the run itself — one status read every three seconds until the
service stops, bounded at ten minutes — so the card fills in without the window
ever blocking on a backup. A save re-reads the destination and the status
together, because the schedule it just wrote decides the next run. Nothing on either card holds a cloud
credential: rclone owns those, and the window names a remote.

### Why TMDB is a paste, and the others are not

TMDB has no credential exchange to call: a key or v4 read token is generated in
the user's TMDB account page, so `POST …/login` has nothing to mint it from.
RomM *does* have both a password exchange and a device approval — the window uses
the approval, so a password is never typed on the television — and Stremio has
only the approval. That leaves two acquisition modes, and every service picks one:

| Mode | Endpoint shape | Services |
| --- | --- | --- |
| Device approval | `POST …/link` + poll `GET …/link` | RomM, Stremio |
| Pasted credential | one write, then read the stored state | TMDB |

Both modes share the same view vocabulary — a `Control::Action` row plus, for the
approval, the three code rows `link_rows` builds for either service.

## Focus and input

Keyboard today, controller next. Both drive the **same semantic actions**, which
is the whole point of the split:

| Input | Action | Effect |
| --- | --- | --- |
| Up / Down | `MoveUp` / `MoveDown` | Move the cursor between rows, stopping at both ends |
| Left / Right | `PreviousPage` / `NextPage` | Switch page; the cursor lands on that page's first row |
| Enter | `Confirm` | Run or edit the focused row |
| Escape | `CloseWindow` | Dismiss a focused text field first; otherwise close the window |
| Pointer | click | Sets the cursor on the clicked row, then does what Enter would |

While a text field has focus, the kit's input key context owns the arrow keys
and Enter, so typing never moves the cursor. Escape is the one key the input
does not claim, which is why the handler is the thing that decides between
"dismiss the field" and "close the window".

Enter and Escape are the two actions that need the window itself — one focuses a
field, the other blurs it or quits — and an action is dispatched *while the
window is already leased*. Re-entering it from the listener fails without a
word, so both handlers defer their window work by one effect flush (`cx.defer`)
before asking for the handle they captured at window creation. That failure is
what made Enter do nothing at all on the television; the render tests now press
Enter for real and would catch its return.

Pasting into a field: the kit's input binds `Ctrl+V` (and `Cmd+V`) itself, and
that path works — a test pastes into the TMDB field headlessly. `Ctrl+Shift+V`
and `Shift+Insert`, the other two things a Linux desktop does when someone says
"paste", are bound here to `PasteIntoField`, which puts the clipboard into the
focused field and reports either the result or that the clipboard could not be
read. That fallback exists because an unreadable clipboard is otherwise silent:
the field stays empty and nothing says why. The window's default log filter
includes `gpui` and `gpui_linux` warnings for the same reason — a clipboard read
or seat capability that fails is logged by the platform, and the journal is the
only place it would ever show up.

**The clipboard is not the only way in.** A desktop whose X11↔Wayland clipboard
bridge is missing (an XWayland Firefox feeding a native Wayland client) will hand
this window nothing, however many shortcuts are bound — that bridge belongs to
the compositor, not to this code. So the TMDB card also carries a *key file* row
and a **Read key from file** action: the path is `HEARTHDECK_TMDB_KEY_FILE`, else
`~/tmdb.key`, and the token is trimmed and saved through the same admin-port write
the field uses. The path is shown, never typed — putting the file there is the job
of `scp` or a shell, both of which work from a terminal where clipboard paste is
not in question.

The cursor is a painted `selected` position, not platform focus — the same
model the iced screens use for their ring cursor. `nav.rs` owns it as a plain
value with no gpui types, so the rules are tested without a window and the pad
transport lands by calling the methods the keyboard already calls.

**The controller is forwarded, not re-read.** The broker gives the pad to one
process, so the window does not open a second controller reader — that would
evict the library app's connection and create a second navigation vocabulary.
Instead the library app keeps `Delivery::Frontend` while the child is up, and for
every semantic action it receives it does one thing: if the settings window is
running, it writes the action's name to
`$XDG_RUNTIME_DIR/hearthdeck-settings-input.sock` instead of acting on it. The
window binds that socket, turns each word into the *same* gpui action its keys are
bound to (`up` → `MoveUp`, `back` → `CloseWindow`), and dispatches it. The pad and
the keyboard therefore meet at one handler, and there is no navigation logic in
the transport at all.

While a text field owns the caret the pad gets only `Back`, which is the same
order the keyboard's Escape follows: dismiss the field, then leave. A window with
no socket still runs — the pad is an addition, never a dependency — and a library
app that drops to another role simply stops forwarding.

## Why not the kit's `setting::Settings` component

`gpui_component::setting` is a complete settings surface - sidebar, search,
group scroll-spy, reset - and it is where the row vocabulary above comes from.
It also keeps its page selection in private keyed window state, so a pad cannot
change page without forking it, and its sidebar has no sizing hook for a TV.
The page is therefore built from the kit's public pieces (`GroupBox`, `Icon`,
`ProgressCircle`, `Button`, theme tokens); the row and the cursor are ours.

## The contract

- **Opening Settings does not change page.** The library app stays where it is
  and the window appears over it; when the window closes, focus returns with the
  shell untouched. One Back press, one result.
- **Escape and the Close button close the window**, and a focused text field is
  dismissed first — the pad's Back joins them when the transport lands. Because
  nothing is saved yet, closing loses nothing but the field contents.
- **The window pairs with the daemon itself** — the same loopback handshake
  `hearthdeck_client` performs — so no token crosses the process boundary.
- **Reads happen on open and on the Refresh row**, which re-reads the daemon and
  the disk on a background thread; Enter cannot stack a second read behind the
  first. A daemon that never answers degrades a row, never the page.
- **Nothing degrades to a dead end.** A daemon that does not answer renders
  "Unavailable" rows; a binary that will not start falls back to the in-app page
  (tested in `settings_window.rs`).

## Rollback

Two independent ways back to the in-app settings page, both no-rebuild:

```sh
HEARTHDECK_SETTINGS_UI=iced hearthdeck-frontend    # keep the page
rm /usr/lib/hearthdeck/hearthdeck-settings         # or remove the binary
```

`HEARTHDECK_GPUI_SETTINGS=/path/to/binary` points the shell at an uninstalled
build instead of the packaged one.

## Packaging

The crate is its own workspace on purpose: GPUI's dependency tree would
otherwise be compiled into every `just check`, every pre-push and every CI run,
for a window that shares no code with the daemon. `packaging/arch/PKGBUILD`
builds it with `--manifest-path services/hearthdeck-settings/Cargo.toml` and
installs the binary beside the other Hearthdeck binaries.

## Known limits

- **The pad reaches the window through the library app.** If the library app is
  not running — the window started by hand — there is no controller input, only a
  keyboard. `Keyboard` and the other library-only actions are not forwarded; the
  window does not open an on-screen keyboard of its own.
- **Nothing is a preview any more.** Every row writes through the daemon. The one
  asymmetry left is transport: TMDB's key goes to the loopback admin listener
  because the contract serves it there, while everything else uses the API port
  and the pairing token.
- **Scrolling follows the cursor, not the contents.** Moving the cursor scrolls
  its row into view; scrolling with the wheel does not move the cursor, so the
  two can be apart until the next Up or Down. There is no scrollbar-driven
  cursor, and no scroll-by-page control in the pad vocabulary yet.
- **Text entry is unproven on the box.** GPUI's Wayland backend implements
  `zwp_text_input_v3`, so cosmic-osk should type into a GPUI field; it has not
  been tried on the TV, and no write flow depends on it yet.
- **The iced page still owns the real flows.** RomM sign-in, Stremio linking,
  categorization and BIOS sync keep working there until each is ported.
- **No visual capture in this change.** `cosmic-screenshot` aborts in this
  environment, so the layout was checked by running the window, not by a
  screenshot.
