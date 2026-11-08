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

- a page column on the left (`Status`, `Connections`, `Library`, `Appearance`,
  `Emulation`), sized for a sofa;
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

## Editing: one setting is wired

**Three settings are wired. All use the same three steps: read the daemon's
state, write, then report what the daemon says afterwards.**

`Library` → `Group the library` is a switch the daemon owns: the value comes from
`GET /v1/categorization`, the press becomes `POST /v1/categorization/enable` (or
`/disable`), and the control settles on the daemon's answer. While the write is in
flight the switch shows the value that was asked for; if the daemon refuses, the
error is printed under the page and the switch falls back to the daemon's answer,
which never changed. `Scan now` posts `/v1/categorization/scan`, is disabled while
the opt-in is off, and reports the refusal (409) when it comes.

`Connections` → `RomM` signs in the way the iced page always has: the address,
username and password are posted **once** to `POST /v1/retro/login`, the daemon
exchanges them for a RomM client token and stores that token, and the password is
never stored anywhere. On success the window keeps only the connection the daemon
reports, empties the password field, and shows the address under `Saved
connection`; the `Forget connection` row appears only while a connection exists
and posts `DELETE /v1/retro/settings`. A saved connection prefills the address
field at launch, so signing in again starts from where the daemon already is.

`Connections` → `Stremio` is a flow, not a field. `Link account` posts
`POST /v1/stremio/link`; the code and the link it comes with become rows of their
own (the link already carries the code, so it is shown for a phone), and the
window polls `GET /v1/stremio/link` every two seconds while the offer stands.
`waiting` updates the countdown, `linked` stores the account and drops the rows,
`expired` and `idle` say so and offer the button again. Each poll belongs to a
generation counter, so replacing or unlinking a pending code stops the older
loop instead of writing over the newer one. `Unlink account` appears only while an
account is linked and posts `DELETE /v1/stremio/settings`. Nothing here ever sees
the Stremio session: the daemon holds it.

**Everything else is still a preview.** Text values live in the window's
`InputState`s, palette, backdrop and reduce-motion in its `Draft`, and every
other action answers with one line under the page saying what it would post, so
the shape of the feedback is visible before the write path exists.

`TMDB`'s key is deliberately **not** wired here. `contracts/openapi.yaml` serves
`/v1/tmdb/settings` on the loopback admin listener only, because the key is set
once from the host and is kept "out of the LAN and out of a controller keyboard".
The field is drawn so the form can be judged, and its action says where the key
really comes from.

What is left to wire, row by row:

- `Appearance` — palette, backdrop, reduce motion. The names come from
  `docs/appearance-system.md`; the daemon already owns them, so wiring them is a
  read and a write, not a new model.
- `Emulation` — install the missing cores, sync the BIOS.

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

The cursor is a painted `selected` position, not platform focus — the same
model the iced screens use for their ring cursor. `nav.rs` owns it as a plain
value with no gpui types, so the rules are tested without a window and the pad
transport lands by calling the methods the keyboard already calls.

**The controller is not wired yet, on purpose.** The broker grants delivery per
role, and the library app drops to `Delivery::Observe` when its window loses
focus (`ownership.rs` returns no d-pad for `Observe`), so the pad goes quiet the
moment this window takes focus. Wiring it means either the library app keeping
`Delivery::Frontend` while the child is up and forwarding semantic actions over
a socket, or a new `Role::Settings` in `hearthdeck-protocol`. Both are decisions
with their own contract change; neither is hidden inside the window.

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

- **No controller input yet.** See above: it needs a transport decision before
  it is a code change.
- **Not everything is written.** Three settings are: the categorization opt-in,
  the RomM connection and the Stremio link. Every other field holds its value in
  the window, and its action only says what it would post. The rest is the next
  change, one row at a time.
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
