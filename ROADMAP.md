# Hearthdeck frontend — settings completion roadmap

Working tracker for the current task. The durable state of this work lives
here, piece by piece, so the conversation does not have to carry it. This
file is **not** part of the code change and is not committed with it.

Last updated: 2026-10-18 — COMPLETE. All seven steps done; committed locally as
8635460 ("Add smart-categories and Stremio settings to the rewritten
frontend", 4 files). Gate: `just check-fast hearthdeck-frontend` green —
fmt + clippy `-D warnings` (all targets) + 389 tests, 0 failed. Push is blocked
in this sandbox (SSH config), so the commit stays local. See the landing record
and the parity-gap sections below for what to pick up next (toast layer, scan
polling, report→tabs application).

## Task

Reach parity with the inherited settings screen for two areas in the
rewritten frontend (`services/hearthdeck-frontend`):

1. **Smart categories** — the missing actions (Create dashboard collections,
   Restore default categories, Remove the downloaded model), the `capable`
   gate from `health.capabilities.categorization`, and toggle parity.
2. **Stremio** — the whole section: link-code flow + 2 s poll chain, unlink,
   busy / reading / error / expired states.

## Constraints

- **Legacy stays behaviorally untouched**: no edits to `src/app_legacy.rs`,
  `src/settings.rs`, `src/settings/stremio.rs`. The new code mirrors their
  `Action` / `pressable` / `Panel` decision design but emits
  `crate::app::Message`, never `app_legacy::Message`.
- **Red tests first** (`testing-and-validation`): every behavior change has a
  test that fails before the implementation lands. New logic lives in
  testable modules, not new `app/mod.rs` match arms.
- clippy `-D warnings` is a build failure (incl. `--all-targets`); theme
  tokens only; Back is one global handler; `update` returns `Task`, never
  blocks.
- **No toast layer yet** (parity backlog item 2): outcomes are logged
  (`log::warn!` / `log::info!`) **and surfaced in each section's own status
  text** (`stremio-failed{reason}`, `categorization-failed{reason}`,
  `categorization-collections-created{count}`) instead of `push_alert` toasts.
  The settled mechanism for the categorization side is the `Notice` enum
  below.

## Phases (mirrors the host task list)

| Phase | Status | Evidence |
|---|---|---|
| set_step_01 Recon: settings.rs, settings/stremio.rs, daemon client API, new settings screen wiring | done | legacy decision layer, settle semantics and every needed daemon method confirmed |
| set_step_02 Red tests: categorization actions + Stremio section state | done | 22 tests written red across two new modules; red baseline run: 0 passed, 22 failed |
| set_step_03 Wire missing categorization actions + update arms | in progress | recon complete + recipe below; `Notice` enum and 5 `Message` variants landed; **the crate does not compile until the update arms exist**; 3 app tests to be written red-first |
| set_step_04 Wire the Stremio section (link/poll/unlink) | in progress | Flow 14/14 green + shell wiring landed (view, 7 Message arms, Effect→task mapping, 4 new app tests, 387 green in scratch build); gate + sign-off pending |
| set_step_05 `just check-fast hearthdeck-frontend` green | pending | — |
| set_step_06 Review subagent + `just lint-strict` + `git diff` re-read | pending | — |
| set_step_07 Commit the settings work | pending | push is blocked in this sandbox (SSH config) — commits stay local |

## CURRENT DIFF STATE (do not undo)

| File | State |
|---|---|
| `src/screens/settings/categorization.rs` | **done**: `Notice` + `section`/`pressable` implemented to the 8 tests, all green |
| `src/app/mod.rs` | **done (03 + 04)**: 5 categorization variants + 7 Stremio variants (`StartStremioLink`, `StremioLinkStarted(Result<Pending, String>)`, `StremioLinkPolled(Result<StremioLink, String>)`, `StremioConnectionLoaded(Result<Option<StremioConnection>, String>)`, `UnlinkStremio`, `StremioUnlinked(Result<(), String>)`, `StremioSourceRefreshed(Result<(), String>)`) with arms, `stremio: Flow` state (both constructions), `run_stremio_effect`, helpers, 3 + 4 new tests |
| `src/screens/settings/stremio_link.rs` | **done**: Flow transitions + `actions`/`pressable` to its 14 tests; 4 clippy test setups fixed (style only) |
| `src/screens/settings/mod.rs` | **wired**: both sections, shared `action_button` helper, pending block + status lines |

Landed `Message` variants (declared after `ScanCategorization`):

```rust
HealthLoaded(Result<crate::providers::daemon::HealthResponse, String>),
CreateCategorizationCollections,
CategorizationCollectionsCreated(Result<usize, String>),
ResetCategorizationTabs,
DeleteCategorizationModel,
```

## Design (settled — execute exactly)

Two decision modules under `screens/settings/`, mirroring the legacy
data-and-actions design but message-agnostic at the decision layer; the shell
maps their enums to `crate::app::Message`.

- `screens/settings/categorization.rs`
  - `Panel { capable: Option<bool>, snapshot: Option<&CategorizationSnapshot> }`
  - `section(&Panel) -> Section::{Unsupported, Checking, Ready { snapshot, actions }}`
  - `Action::{Toggle, Categorize, CreateCollections, Reset, RemoveModel}`
  - `pressable(Action, &Panel) -> bool`
  - Rules: `capable == Some(false)` → Unsupported, nothing pressable;
    `capable == None` or `snapshot == None` → Checking; `!enabled` → only
    Toggle; report present → + CreateCollections, Reset; `ModelStatus::Downloaded`
    → + RemoveModel; `status.running` freezes everything but Toggle and Reset.
  - **`Notice` (landed)** — the no-toast outcome surface:
    `Created(usize)` | `NoCollections` | `Failed(String)`. App state holds
    `categorization_notice: Option<Notice>`; cleared when a new action starts;
    the section renders it as its own status-text line
    (`categorization-collections-created{count}` /
    `categorization-collections-empty` / `categorization-failed{reason}`).
- `screens/settings/stremio_link.rs`
  - `Flow { connection, reading, pending, busy, error, expired }` — the link
    state machine; `Pending { code, link, seconds_left }`.
  - `Flow` transitions return `Effect::{Nothing, Poll, Publish, ReloadConnection}`
    so the shell applies them as tasks: `Poll` → `poll_stremio_link(2 s)`,
    `Publish` → `refresh_source("stremio")` + `load_collections(5 s)`,
    `ReloadConnection` → `load_stremio_connection()`.
  - Transitions: `begin_link` / `link_started`, `link_polled`
    (`StremioLink::{Waiting, Linked, Expired, Idle}` + `Err`), `begin_unlink` /
    `unlinked`, `begin_read` / `connection_read`.
  - `Action::{Link, Unlink}`, `actions(&Flow)`, `pressable(Action, &Flow)`.
  - The module's own 14 tests are the spec; legacy intent is
    `app_legacy.rs::{settle_stremio_link_started, settle_stremio_link_polled,
    settle_stremio_connection, settle_stremio_unlinked}`.

### Settled additions found during set_step_03 recon

- **`capable` has no source in the new shell** (legacy gets it from its
  dashboard health poll; the new app has none). Settled fix: new
  `Message::HealthLoaded(Result<HealthResponse, String>)` + `fn
  load_health(&self) -> Task<Message>` (mirrors `load_categorization`, calls
  `client.health()`). The arm sets `categorization_capable =
  Some(health.capabilities.categorization)`; `Err` logs
  `log::warn!("health read failed: {error}")` and keeps the last answer.
  Called from **init's task batch** and from **OpenSettings**. `OpenSettings`
  batches `load_health()` + `load_categorization()` but **skips the
  categorization read when `categorization_capable == Some(false)`** (legacy
  rule). Stremio read joins this batch in set_step_04.
- **`CategorizationAction` gains `RemoveModel`** →
  `client.delete_categorization_model()`; `fn categorization_action` becomes
  `&mut self` and starts with `self.categorization_notice = None;` (a new
  action starts from a clean slate — mirrors the Flow's "clear error on new
  transition").
- **`ResetCategorizationTabs` is a local change** (no daemon call, no task):
  `fn restore_derived_categories(&mut self)` mirroring `app_legacy.rs:1955`
  but persisting via `self.persist_config()` and without legacy's
  `group_keys` bookkeeping:

```rust
let selected = self
    .cur_group
    .and_then(|index| self.config.sections.get(self.cur_section).get(index))
    .cloned();
if !self.config.sync_categorization(&[]) {
    return;
}
self.persist_config();
self.cur_group = selected.and_then(|selected| {
    self.config
        .sections
        .get(self.cur_section)
        .iter()
        .position(|group| group == &selected)
});
```

  The arm: `self.restore_derived_categories(); self.refilter();` (no task).
  `sync_categorization(&[])` drops only scan-created tabs (`GroupSource::
  Categorization`), keeps the user's own folders — already tested in
  `app_group.rs`.
- **`CreateCategorizationCollections`** → `fn
  create_categorization_collections(&mut self) -> Task<Message>`: clears the
  notice, `client.create_categorization_collections()` →
  `CategorizationCollectionsCreated`. Arm settles:
  `Ok(0)` → `Notice::NoCollections` + `log::warn!("the last categorization
  produced no dashboard collections")`; `Ok(n)` → `Notice::Created(n)` +
  `log::info!`; then `return self.load_collections();` (the dashboard's rails
  **are** the daemon's collections, so the publish isn't done until they are
  read back — see memory: rails come from collections, never hardcoded).
  `Err(error)` → `Notice::Failed(error)` + `log::warn!("could not create the
  dashboard collections: {error}")`, no re-read (legacy alerted and returned).
- **`CategorizationDone(Err)`** additionally sets
  `self.categorization_notice = Some(Notice::Failed(error))` (the existing
  `log::warn!` stays), then re-reads as today.

## Legacy semantics to replicate (settled, from `app_legacy.rs` / `settings.rs`)

- `link_started`: busy=false; `Err` → store error; `Ok` → clear error, set
  pending, poll at 2 s.
- `link_polled`: `Waiting` → update pending + repoll; `Linked` → clear
  pending/error, set connection, `log::info!("stremio account linked")`,
  publish the source now + re-read collections after 5 s; `Expired` → clear
  pending/error, mark expired (legacy alerted `stremio-link-expired` with
  retry); `Idle` → clear pending, re-read connection; `Err` → clear pending,
  store error.
- `connection_read`: reading=false, set connection or error.
- `unlinked`: busy=false; `Ok` → clear connection/pending/error,
  `log::info!` (legacy toasted `stremio-unlinked-toast`); `Err` → store error.
- `OpenSettings`: switch page, clear cursors, then batch `load_categorization()`
  + `load_stremio_connection()`; if `capable == Some(false)` only the Stremio
  read runs. (Plus `load_health()` in the new shell, see above.)
- Categorization actions all end the same way: ask the daemon, then re-read
  (`CategorizationDone` → `load_categorization()`). Create-collections is the
  exception: it returns `Result<usize>` and re-reads collections instead.
- Legacy `settings.rs::pressable` (reference for intent):
  `Toggle → snapshot.is_some()`, `Categorize | RemoveModel → enabled && !busy`,
  `CreateCollections → enabled && !busy && report.is_some()`, `Reset → enabled`.
  The new module's tests pin the (slightly stricter) rules in the Design
  section; the tests win.

## IMPLEMENTATION RECIPE — set_step_03 (exact; execute in order)

**Red-first ritual**: add stub update arms (empty / `Task::none()`), the 2 new
state fields, and the 3 app tests below; run
`cargo test --manifest-path services/Cargo.toml -p hearthdeck-frontend`;
confirm the 3 new tests FAIL (and the 8 `categorization::tests` are red from
the stubs). Keep that output as red evidence. Then implement → green.

1. **`categorization.rs`**: replace the two `Red baseline stub` bodies with the
   Design-section rules. Tests are the spec — read them, never change them.
   `pressable` = `false` unless `section` is `Ready` and the action is in its
   list; when `status.running`, only `Toggle` and `Reset` stay pressable.
2. **`screens/settings/mod.rs`** view:
   - `Settings` struct becomes
     `{ categorization: Option<&'a CategorizationSnapshot>, capable: Option<bool>, notice: Option<&'a categorization::Notice> }`.
   - `fn categorization(...)` builds `categorization::Panel` and dispatches on
     `categorization::section(&panel)`:
     - `Unsupported` → heading + `categorization-unsupported` +
       `categorization-unsupported-hint`;
     - `Checking` → heading + `categorization-reading-status`;
     - `Ready` → intro (`categorization-intro{size=DOWNLOAD_SIZE}`), toggler
       row (`settings::item(fl!("categorization-toggle"),
       toggler(snapshot.enabled).on_toggle(Message::SetCategorization))`),
       `status_line(snapshot)`, **notice line when set**, `model_line(...)`,
       report line when `snapshot.report` is `Some`
       (`categorization-report{apps=report.app_count,
       categories=report.categories.len(),
       unclassified=report.unclassified.len()}`), then a button row over
       `actions` **skipping `Action::Toggle`** (the toggler is its row).
   - Button labels: `Categorize` → `scan_label(snapshot)`,
     `CreateCollections` → `categorization-create-collections`, `Reset` →
     `categorization-reset`, `RemoveModel` → `categorization-remove-model`.
   - Message map (one exhaustive fn): `Toggle → SetCategorization(!snapshot.enabled)`,
     `Categorize → ScanCategorization`, `CreateCollections →
     CreateCategorizationCollections`, `Reset → ResetCategorizationTabs`,
     `RemoveModel → DeleteCategorizationModel`.
   - Pressability via `categorization::pressable(action, &panel)`; unpressable
     buttons render disabled with `.on_press_maybe(...)` (legacy pattern:
     `settings.rs` `action_button`, ~line 329). One button helper, theme
     tokens only (`standard_button_class()`, `TEXT_BODY`, `theme::spacing()`).
3. **`app/mod.rs`**:
   - Fields next to `categorization`: `categorization_capable: Option<bool>`,
     `categorization_notice:
     Option<crate::screens::settings::categorization::Notice>`. Add both as
     `None` to **both** `App` constructions (init ~line 484 and test helper
     ~line 2475).
   - All the settled additions above (load_health, RemoveModel,
     create_categorization_collections, restore_derived_categories, the arms,
     init batch, `view_settings` passing `capable` + `notice`).
4. **3 new app tests** in `mod tests` (behavior-sentence names, house style):
   - health gate: `categorization_capable` starts `None`;
     `HealthLoaded(Ok(health(false)))` → `Some(false)`;
     `health(true)` → `Some(true)`. Helper
     `fn health(categorization: bool) -> HealthResponse` with
     `version: "test".to_owned(), lan_enabled: false, transport:
     "loopback".to_owned(), providers: Vec::new(), capabilities:
     HostCapabilities { launch: false, application_sessions: false,
     install_requests: false, retro_launch: false, categorization }` (both
     types re-exported from `crate::providers::daemon`).
   - restore defaults: `cur_section = Section::Applications`,
     `config.add(Section::Applications, "Mine".to_owned())`,
     `config.sync_categorization(&[CategorizationGroup { name: "Speedruns",
     entry_ids: vec!["app.one".to_owned()] }])` (from `crate::app_group`) so
     the section is `[scan, user]`; `cur_group = Some(1)`;
     `update(ResetCategorizationTabs)` → groups exactly `["Mine"]` and
     `cur_group == Some(0)` (the selection follows the folder).
   - outcomes: `CategorizationCollectionsCreated(Ok(3))` → `Notice::Created(3)`;
     `Ok(0)` → `NoCollections`; `Err("refused")` → `Failed` carrying "refused";
     `CategorizationDone(Err(...))` → `Failed`; `SetCategorization(true)`
     clears the notice (`None`) — a new action starts clean.
5. `just check-fast hearthdeck-frontend` green.

## IMPLEMENTATION RECIPE — set_step_04

1. **`stremio_link.rs`**: implement the `Flow` methods + `actions` /
   `pressable` so its 14 tests pass (tests = spec; legacy intent listed
   above). No API changes.
2. **View** (`screens/settings/mod.rs`): Stremio section under categorization
   — link-code flow UI with `stremio-pending{seconds}`, `stremio-linked{when}`,
   `stremio-failed{reason}` status text, expired state, unlink button;
   `Action::{Link, Unlink}` mapped to `StartStremioLink` / `UnlinkStremio`,
   pressability from `stremio_link::pressable`.
3. **`app/mod.rs`**: state `stremio: stremio_link::Flow` +
   `Message::{StartStremioLink, StremioLinkStarted, StremioLinkPolled,
   StremioConnectionLoaded, UnlinkStremio, StremioUnlinked,
   StremioSourceRefreshed}`; each settle method's returned `Effect` maps to
   tasks (`Poll` → `poll_stremio_link(2 s)`, `Publish` →
   `refresh_source("stremio")` + `load_collections(5 s)`,
   `ReloadConnection` → `load_stremio_connection()`); `OpenSettings` adds
   `load_stremio_connection()` to its batch (runs even when categorization is
   unsupported).
4. Red tests for the new wiring where behavior is observable; then
   `just check-fast hearthdeck-frontend` green.

## set_step_04 landing record — deviations & guesses (carry into review)

Shipped: `stremio: Flow` state in both `App` constructions; 7 Message variants
(the roadmap's, plus `StremioSourceRefreshed(Result<(), String>)` — no
pre-existing variant covered `refresh_source`'s result); arms delegate every
decision to the Flow and only run its returned `Effect` via
`run_stremio_effect` (Nothing → none, Poll → `poll_stremio_link` every 2 s,
Publish → `refresh_source("stremio")` + `load_collections_after(5 s)`,
ReloadConnection → `load_stremio_connection()`); `OpenSettings` and `init` read
the Stremio connection **outside** the `capable == Some(false)` guard; the view
mirrors legacy rows (the pending block replaces the status line; the error
caption sits beside the state, not over it). 4 thin app tests; suite 383 → 387,
0 failed.

Deviations (decided, sane):
- `load_collections_after(delay)` added; `load_collections()` kept as the
  zero-delay wrapper — its other caller is `app/details.rs:114`, out of scope.
- `stremio-link-expired` rides the section's status line (no toast layer).
- `init` batches the connection read the way it already batched categorization.

Open at review time (set_step_06):
- **Button classes**: the shared `action_button` currently flattens everything
  to `standard_button_class()`. Legacy `settings.rs::class()` maps
  `Categorize | CreateCollections → primary_action_button_class()`,
  `RemoveModel → destructive_button_class()`, `Toggle | Reset → standard`;
  Stremio Link → primary, Unlink → destructive (per legacy `stremio.rs`).
  Being restored (helper takes a class).
- `Poll`/`Publish`/`ReloadConnection` arms are pure Task plumbing — untested
  (the harness never runs a returned `Task`); the Flow's 14 tests carry the
  decisions.
- Flow transition guesses (tests pin none of these): (1) `begin_link` clears
  both `error` and `expired`, likewise `link_started(Ok)` / `unlinked(Ok)`;
  (2) `link_polled(Expired)` clears `error` too (legacy did); (3) `begin_unlink`
  clears `error`, leaves `pending`/`expired`; (4) `pressable` ignores `reading`;
  (5) `actions` ignores `busy`/`pending`/`reading` (a busy row is drawn, just
  unpressed); (6) `link_polled(Waiting)` overwrites the whole `Pending` from
  the daemon; (7) log wording copied verbatim from legacy
  (`could not start the Stremio link: {error}`, …).

## Parity gaps found during recon — DELIBERATELY out of scope (report, don't fix)

- **Scan progress is not polled** in the new shell (legacy
  `settle_categorization` re-polls while `status.running`). A running scan's
  phase line freezes until settings is reopened. Not in the task list.
- **The scan report is never applied to the Applications tabs** in the new
  shell (legacy `apply_categorization` + `config.write_entry` inside
  `settle_categorization`). `ResetCategorizationTabs` still has real effect:
  it drops scan tabs that the legacy app or an earlier session wrote into the
  shared `AppLibraryConfig`.
- `categorization-last-run{when}` / `categorization-engine{engine}` lines are
  not drawn (the notes' key list doesn't need them).

## Recon index (file:line, so nobody re-reads)

- `app_legacy.rs:1859-1965` — delete_categorization_model, toggle,
  create_categorization_collections, settle_created_collections (toast texts),
  restore_derived_categories.
- `app_legacy.rs:2009-2065` — settle_categorization (poll + apply-to-tabs
  behavior, out of scope above).
- `app_legacy.rs:5620-5700` — the update arms incl. OpenSettings batching and
  the capable == Some(false) short-circuit.
- `settings.rs:162-177` — legacy pressable; `settings.rs:300-345` —
  toggle_row / buttons_row / action_button (`on_press_maybe` pattern).
- `app_group.rs:977` — `sync_categorization` (swap semantics, tested);
  `app_group.rs:278` — `CategorizationGroup { name, entry_ids }`.
- `providers/daemon.rs` — `health()` (573), `categorization()` (652),
  `start_categorization_scan` (674), `create_categorization_collections`
  (682, returns `Result<usize>`), `enable/disable` (704/709),
  `delete_categorization_model` (715), `list_collections` (1045),
  stremio trio (1173/1193/1210/1227), `refresh_source` (1248).
  `HealthResponse { version, lan_enabled, transport, providers, capabilities:
  HostCapabilities }`; `HostCapabilities` re-exported from
  `hearthdeck-protocol` (fields: launch, application_sessions,
  install_requests, retro_launch, categorization).
- `app/mod.rs` — Message enum ~107-135; update arms ~835-870;
  `load_categorization`/`categorization_action`/`load_collections` ~2142-2191;
  `refilter` ~1261, `list_replaced` ~1292, `persist_config` ~2218;
  init batch ~505-520; test helpers `showing`/`navigable`/`snapshot` ~2465,
  ~3556; existing categorization tests ~3590-3622.
- i18n (`i18n/en/hearthdeck-frontend.ftl`): all keys exist —
  `categorization-unsupported`, `-unsupported-hint`, `-reading-status`,
  `-intro{size}`, `-toggle`, `-categorize`, `-rescan`, `-create-collections`,
  `-collections-created{count}`, `-collections-empty`, `-reset`,
  `-remove-model`, `-report{apps,categories,unclassified}`, `-failed{reason}`,
  `-model-*`, `-phase-*`, `-off`, `-ready`. Stremio keys likewise exist
  (`stremio-pending{seconds}`, `stremio-linked{when}`, `stremio-failed{reason}`,
  `stremio-link-expired`, `stremio-unlinked-toast`, …).
- `fl!` (localize.rs:24) wraps `i18n_embed_fl::fl!` forwarding raw `tt`
  tokens; args are `key = expr` (`&str`, `String`, integers all seen working;
  a `&String` may need `.as_str()` / `.clone()`).

## File map (current diff)

| File | Role |
|---|---|
| `src/screens/settings/categorization.rs` | decision module: `Notice` landed, `section`/`pressable` pending + 8 red tests |
| `src/screens/settings/stremio_link.rs` | link state machine + 14 red tests (set_step_04) |
| `src/screens/settings/mod.rs` | view wiring (set_step_03/04 recipes) |
| `src/app/mod.rs` | 5 Message variants landed; arms/state/helpers/tests pending (recipe) |
| `src/providers/daemon.rs` | **no change needed** — all calls exist |
| `i18n/en/hearthdeck-frontend.ftl` | **no change needed** — all keys exist |

## Red test inventory (all failing at baseline: 0 passed, 22 failed)

`categorization.rs` (8):
`an_installation_that_cannot_categorize_offers_nothing`,
`the_controls_wait_for_the_reads_to_land`,
`the_switch_alone_is_offered_before_anything_is_turned_on`,
`publishing_and_restoring_wait_for_a_report`,
`remove_model_waits_for_a_downloaded_checkpoint`,
`the_switch_stays_live_during_a_scan_but_the_model_actions_do_not`,
`nothing_but_the_switch_is_pressable_before_the_opt_in`,
`an_unsupported_installation_makes_nothing_pressable`

`stremio_link.rs` (14):
`a_started_link_shows_its_code_and_queues_the_first_poll`,
`a_refused_start_says_why_and_releases_the_buttons`,
`a_waiting_code_counts_down_and_keeps_polling`,
`an_approved_code_becomes_the_linked_account`,
`an_expired_code_says_so_instead_of_sitting_there`,
`an_idle_answer_rereads_the_connection`,
`a_failed_poll_says_why_and_drops_the_code`,
`unlinking_forgets_the_account_and_the_code`,
`a_refused_unlink_keeps_the_account_and_says_why`,
`a_connection_read_ends_the_checking_state`,
`a_failed_connection_read_says_why_without_guessing`,
`linking_is_offered_even_when_an_account_is_already_linked`,
`nothing_is_pressable_while_a_request_is_in_flight`,
`an_unlinked_account_cannot_unlink`

Plus 3 app-level tests planned in set_step_03 (health gate, restore-defaults,
outcome notice — assertions spelled out in the recipe).

## Notes for whoever resumes this

- `crate::settings::Action::message()` returns `app_legacy::Message` — the new
  shell never calls it; it maps the enums itself.
- i18n arg names (exact): `stremio-pending { $seconds }`,
  `stremio-linked { $when }`, `stremio-failed { $reason }`,
  `categorization-collections-created { $count }`, `categorization-intro { $size }`,
  `categorization-report { $apps $categories $unclassified }`,
  `categorization-phase-scanning { $completed $total }`.
- App-side additions: `Message::{CreateCategorizationCollections,
  CategorizationCollectionsCreated(Result<usize, String>), ResetCategorizationTabs,
  DeleteCategorizationModel, HealthLoaded(Result<HealthResponse, String>)}`
  (done) and, in set_step_04, `Message::{StartStremioLink, StremioLinkStarted,
  StremioLinkPolled, StremioConnectionLoaded, UnlinkStremio, StremioUnlinked,
  StremioSourceRefreshed}` + state `stremio: Flow` (planned). State
  `categorization_capable: Option<bool>` + `categorization_notice: Option<Notice>`
  (planned, recipe).
- Tooling: `task:subagent` capability takes `prompt` (plus `description`,
  `write_paths`, `run_in_background`, `effort`, `max_steps`, `model`,
  `profile`, `tools`, `continue_from`). Scope writes with `write_paths` when
  dispatching parallel agents on disjoint files.
