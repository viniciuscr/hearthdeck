# Settings window — UI/UX roadmap

A design review of the GPUI settings window (`services/hearthdeck-settings`),
written for a 10-foot, gamepad-first TV. Verdict: **the model is right, the
presentation is not.** Rows, cards, painted cursor, token-only colours and
`Option`-typed state all hold up; what is wrong is type scale, elevation, focus
visibility, and three places where the screen says something untrue. Everything
here fits the existing structure and the kit — no new navigation model.

Severity: **blocker** ships a lie or a hang; **high** a TV user cannot read or
act; **medium** they can, badly; **nit** polish.

Work top to bottom. Each phase is a shippable commit; tick a box only when the
change is verified (tests, clippy, and where the item needs eyes, a run on the
box).

## Phase 1 — Truthfulness (blockers) — done

Shipped: the sidebar no longer claims edits are a preview (it reports the daemon
state), the window opens before the read and draws "Reading…" until it lands, an
unreachable daemon renders one card with a retry instead of "Not set up"/"OFF",
and the reading notice is cleared when the read completes. The first read still
prefills the RomM address, the backup destination and the schedule, applied at
the next render because a field cannot be set without a window.

- [x] **CS-01 · the screen lies about writing.** Sidebar says "Preview: edits stay
      in this window for now." (`main.rs:1812`) and the module doc says the same
      (`main.rs:22-27`), while `Save and apply`, the switches, unlink and core
      install all write to the daemon. Fix: delete the sentence; show daemon
      state instead — `Connected to the daemon` / `Daemon unavailable`.
- [x] **FB-01 · the window can take ~45s to appear.** `read_snapshot()` runs
      synchronously in `main` before `application().run` (`main.rs:2569`): eight
      sequential GETs at a 5s timeout each plus two pairing POSTs. Fix: open with
      `Snapshot::default()` and `refreshing = true`, then start the existing
      `refresh()` once the view exists; draw "Reading…" per card.
- [x] **CS-02 · a dead daemon renders as the user's fault.** RomM "Not set up",
      Library switch off, Emulation "Nothing reported yet", Stremio "Not linked".
      Fix: when `snapshot.daemon == false`, every page shows one card: value
      `Unavailable`, chip `danger`, description "Could not reach the daemon.
      Enter to retry." — and the normal rows only once a read succeeded.
- [x] **FB-03 · "Reading the daemon…" is never cleared** (`main.rs:884`): the
      completion path sets no notice, so a progressive-tense string stands for
      good. Fix: clear it on completion.

## Phase 2 — Readable at three metres — done

Shipped: a `TYPE_*` scale (48/26/21/21/18/16) applied to every string on the
screen, and `theme.rs` — the Noir palette applied over the kit's dark theme so a
card is a surface (`group_box #171717`) rather than the page colour, borders are
visible (`#2e2e2e`), and focus is one blue (`#3b82f6`) across fill, border and
ring.

- [x] **TY-01 · page title 30px → 48px**, SEMIBOLD (iced uses 52).
- [x] **TY-02 · four roles share 18px.** Card title 26 SEMIBOLD, row label 21
      MEDIUM, value 21 SEMIBOLD, sidebar item 21. Three sizes is enough to parse
      the page.
- [x] **TY-03 · row descriptions 14px → 18px**, `muted_foreground`,
      `line_height(1.4)`.
- [x] **TY-04 · footer keycaps and the storage percentage 12px → 16px.** The only
      place the control scheme is written down is currently the smallest text.
- [x] **TY-05 · status chips ≈12px inside a 25px tag** → 16px text, `py_1p5`,
      `px_3`.
- [x] **CS-03 · cards have no surface.** In the loaded theme `group_box.background`
      equals `background` (`#0a0a0a`); only a `#262626` hairline divides them.
      Fix: ship a Hearthdeck theme file — `background #0a0a0a`,
      `group_box #171717`, `border #2e2e2e`, `sidebar #111111`,
      `list.active.background #1d4ed81f`, `list.active.border #3b82f6`,
      `ring #3b82f6` — and load it instead of `Theme::change(ThemeMode::Dark)`.
- [x] **CS-06 · stock palette, not Hearthdeck's.** Same fix as CS-03; the accent,
      focus and elevation ladder must match the library behind it.

## Phase 3 — Layout and focus — done

Shipped: a centred 1280 column (`.mx_auto()` on the capped block and on every
card), a card inset so the ring floats rather than touching the card edge, the cursor skipping fact rows entirely, three focus cues (surface,
2px border, 4px accent bar, semibold label), the footer switching to
caret/value/done while a field owns the keys, a dead switch explaining itself,
the 360px rail with 56px items, a fixed 56px notice strip, and values capped at
560px with an ellipsis.

Two items the kit blocks, recorded rather than forced: `Switch` has no public
size and `Icon`'s `Sizable` is unreachable (`Size` lives in a private module), so
the switch track stays the kit's 36x20 and the sidebar icon stays its default.
The 19px rem already lifts their *text*; the track and the glyph need an upstream
export.

- [x] **LY-01 · the reading column is not centred.** `max_w(1100)` with no
      `mx_auto` leaves 436px of dead black at 1920. Fix: `CONTENT_MAX_WIDTH =
      1280`, `.mx_auto()` on the title block and every card.
- [x] **FX-01 · the cursor lands on rows that do nothing.** `Fact` rows take focus
      and the full ring; Enter is silent. Fix: count only Text/Toggle/Time/Action
      in `Nav`; draw facts without fill or ring.
- [x] **FX-02 · focus is a 1px grey hairline** (`theme.ring`, `#737373`). Fix:
      `border_2` in `list_active_border`, keep the `list_active` fill, add a 4px
      accent left bar, label SEMIBOLD — four cues, and it survives grayscale.
- [x] **FX-03 · "a field owns the keys" looks like "a row is focused".** Fix:
      while editing, swap the footer for `←→ caret · ↑↓ value · Esc Done` and put
      a 2px `ring` on the row.
- [x] **FX-04 · a disabled switch gives no reason** and is only a dimmed 36×20
      track. Fix: `Switch::with_size(Size::Large)` (44×24) plus a 16px `Tag`
      "Unavailable" / "Needs a remote"; move the `Remote not found` row directly
      above the switch.
- [x] **LY-02 · the focus ring is drawn flush to the card edge** and collides with
      the separator. Fix: card `p_2` inset; rows keep `px_5 py_4`.
- [x] **LY-03 · the title block does not read as a header.** Fix: `pt_8 pb_6` and
      a 32px first gap, the largest space on the page.
- [x] **LY-04 · sidebar items are ~46px with 4px between them.** Fix:
      `min_h(56)`, gap 8, `px_4`, icon 24.
- [x] **LY-05 · sidebar is 320px (16.7% of 1920)** — the narrowest element on
      screen is the one Left/Right drives. Fix: 360.
- [x] **LY-06 · the notice resizes the viewport** and can hide the row just
      pressed, with no re-scroll. Fix: a fixed 56px strip, always present, one
      line, ellipsis.
- [x] **LY-07 · values have no max width.** Shipped `max_w(560)`, no wrap,
      ellipsis. The "full value in a tooltip" half is **not** shipped: this kit has
      no attach-a-tooltip helper on a plain element (`Tooltip::new` builds one, but
      nothing mounts it here). Either mount a `Popover`/`Tooltip` by hand or accept
      the ellipsis — decide in Phase 4.
- [x] **PL-04 · the focused row keeps `border_t_1`**, doubling the line where it
      meets the separator. Fix: skip the separator on the focused row.

## Phase 4 — Say what things mean — partly done

Shipped: one tone vocabulary with `Busy` (a run or a link in progress is info, not
warning) and a failed run as danger; a `Nightly copy` row whose chip reports the
timer (`Scheduled`/`Not scheduled`) and an `Unsaved` chip while the draft is
pending; the schedule's copy fixed to "Enter to edit; then..."; an "Enter to
change" caption on every row that can be edited; the unreachable Close button
gone and the hints in pad language; Refresh that says "Refreshing..." and is
disabled while it is; timestamps in local time ("7 Nov, 14:43"); the card order
fixed on Status (Daemon first) and Backup (Last run first); the storage ring at
64px with "X free of Y".

Shipped since: AF-04 (the first Back with a draft or an edited field asks, the
second closes), AF-07 (the device code is 48px monospaced), FB-02 (`Alert::error`
for a refusal, `Alert::info` for a result; the per-row failure tag is still open),
and PL-01's fullness colour is the only part of it left.

**Still open in this phase:** the per-row failure tag from FB-02, CT-05 (the
missing-systems list, which needs data the snapshot does not carry), CT-06
(localisation), CS-07 (motion), PL-01's fullness colour, and the CS-05 / FB-04 /
CT-07 / PL-02 / PL-03 nits.

- [x] **FX-05 · one tone covers six states.** `Tone::Off` is "Off", "Idle",
      "Timer off", "Unknown", "Not installed" and "Unavailable"; "Running" wears
      `warning`; a failed run gets "Idle". Fix: add `Tone::Busy` → `Tag::info()`;
      a last run whose result is not `success` → `Tag::danger` "Last run failed";
      neutral means "off, and that is fine".
- [x] **AF-01 · no row says it can be edited.** Fix: a 16px caption "Enter to
      change" (or `›`) on every non-Fact row.
- [x] **AF-02 · the schedule's copy is wrong.** "Up and down change the hour, left
      and right pick the part" — but left/right change page until Enter has
      focused the field. Fix: "Enter to edit; then up/down set the hour,
      left/right the part."
- [x] **AF-03 · two switches, two behaviours.** Categorization writes on flip;
      backup is a draft. Fix: `Tag::warning` "Unsaved" on the card while
      `backup_pending.is_some()`.
- [x] **AF-04 · Back discards typed values and drafts silently.** Fix: first Back
      shows "Unsaved changes — Back again to close"; second closes.
- [x] **AF-05 · the footer's Close button is unreachable by the cursor** and
      duplicates Escape. Fix: delete it; put `Esc Back` / `B Back` in the hints.
- [x] **AF-06 · footer hints are keyboard-only** and never mention Back. Fix:
      `↑↓ Move · ←→ Page · A Select · B Back`, pad glyph leading, key secondary.
- [x] **AF-07 · the device code is 18px and the URL is inert.** iced had 52px plus
      a QR. Fix: `Value::Code` at 48px mono, wide tracking, on its own row; show
      host/path only, never the full code-laden URL.
- [x] **AF-08 · Refresh stays enabled while refreshing** and a second press does
      nothing. Fix: disable, label "Refreshing…", `Button::loading(true)`.
- [x] **FB-02 · every outcome is one cyan `Alert::info`**, and errors are detached
      from the row that caused them. Fix: `Alert::success/error/info` by outcome
      plus a per-row `Tag::danger` "Failed".
- [x] **CT-01 · raw RFC3339 UTC on screen.** `2025-11-08T02:00:00.123456789Z`, in
      UTC beside a local schedule. Fix: format `Local` — "Tomorrow, 03:00",
      "7 Nov, 14:43".
- [x] **CT-02 · Backup puts the form above the health**, so "did it work?" is
      below the fold. Fix: Last run first.
- [x] **CT-03 · Status leads with Refresh**, above Daemon; three one-row cards.
      Fix: Daemon → Storage → This window.
- [x] **CT-04 · "Enable off-site backup" under an "Off-site backup" card**, plus a
      chip repeating the switch. Fix: row label "Nightly copy"; chip reports the
      timer — `Scheduled` / `Not scheduled`.
- [ ] **CT-05 · Emulation hides what is missing** ("3 of 12 systems ready") and
      offers a blind install. Fix: one row per missing system, capped at 5 + "and
      4 more".
- [ ] **CT-06 · no localisation.** Every string is `&'static str`; iced is Fluent.
      Fix: keep the shape, source from keys.
- [x] **PL-01 · storage ring is 48px with a 12px percentage and no total.** Shipped
      64px, 16px percentage, "X free of Y", "Home volume". The fullness colour is
      still open.
- [ ] **CS-04 · wrong token family**: `theme.ring` as a row border, sidebar using
      `foreground` instead of `sidebar_foreground`/`sidebar_accent_foreground`.
- [ ] **CS-07 · no motion on navigation, but the switch animates.** Fix: 180ms
      cross-fade of the list only on page change; focus instant.
- [ ] **CS-05, FB-04, CT-07, PL-02, PL-03** — magic numbers (`px(190)`, `px(24)`,
      `px(1)`) into consts; plurals (`1 core / 2 cores`); sentence-case drift and
      "Not installed" three times in one card; ring colour never changes with
      fullness; sidebar icons 24px and reused or dropped.

## What must survive the rework

- The row model: label + optional sentence + one control, five shapes.
- Per-card status chips — a glance down the cards is right for a 10-foot page.
- The painted cursor over one scrolling `list`, with `scroll_to_reveal_item`, and
  a pure-Rust, unit-tested `nav.rs` with no gpui types.
- Controls disabled, never removed.
- The data model already distinguishes unknown from off (`Option<…>` everywhere);
  the liar is only the view.
- `mirrored(pending, daemon)`: answer immediately, settle on the daemon's answer.
- One card, one decision, one Save for backup.
- Failure messages that name the first thing that broke.
- The link flow: generation counter, explicit `expired`/`denied`/`idle`, live
  countdown.
- **Zero raw colours** in view code — the fix for elevation is a theme file, not
  `bg(hex)`.
- No chrome, no hover-only affordance, every row pad-reachable.

## Cannot be judged from code (verify on the box)

1. Whether the type scale reads at 3 m — photograph the window at 1080p from 3 m.
2. Whether a 1px ring is visible: gpui px are logical; log `window.scale_factor()`
   and treat 2px as mandatory at 1.0.
3. The resolved theme values — log `group_box`, `border`, `list_active`,
   `list_active_border`, `ring`, `muted_foreground` at first render.
4. Whether a disabled switch is distinguishable from an off one, at 3 m.
5. Whether the notice pushes a row out — testable headlessly: focus the last row,
   set a notice, render twice, assert the row stays inside the list bounds.
6. Whether the last row plus the trailing gap is fully revealed.
7. Long-value overflow — 60-character `base_url` in a stub snapshot.
8. Whether the box delivers `zwp_text_input_v3`, i.e. whether `Text` rows are
   typeable at all from the sofa.
9. Whether the pad socket is live in the shipped session — otherwise every "A"
   hint is a lie and the keyboard wording must lead.
10. Whether `TimeField` draws its selected segment distinctly under the caret.
