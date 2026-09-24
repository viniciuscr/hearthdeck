# Architecture & Design Review — HearthDeck

Date: 2026-08-15
Status: Open — fix one item at a time, check off when done

---

## Origin note

The god-struct and monolithic update/view were already present in the original
codebase (commit `fe6f466`). We inherited them and added 3 fields. The
architecture debt is pre-existing, not introduced by our work.

---

## Critical

- [ ] **C-A1. God-struct `HearthDeck`** — 27 fields, `update()` is a ~550-line
  match block, `view_main_content()` is ~440 lines. No separation of concerns.
  File: `src/app.rs:293-321` (struct), `:918-1467` (update), `:1834-2270` (view).

  Fix: Extract `NavigationState`, `SearchState`, `DndState`, `DialogState`
  sub-structs. Split `view_main_content()` into `view_sidebar()`,
  `view_topbar()`, `view_grid()`, `view_dialogs()`.

- [ ] **C-A2. Section enum not extensible** — `Section` is a closed 3-variant
  enum. `Sections` has 3 named `Vec<AppGroup>` fields. Adding a section requires
  changes in 12+ locations. File: `src/app_group.rs:144-226`.

  Fix: Replace `Sections` with `HashMap<Section, Vec<AppGroup>>` or
  `[Vec<AppGroup>; 3]` indexed by `Section::index()`. Use `Section::ALL` as
  the single source of truth for iteration.

- [ ] **C-A3. Config persistence errors silently swallowed** — Same
  `if let Err = write_entry { error!() }` copy-pasted 7 times. User gets no
  feedback on save failure. Files: `src/app.rs:1197,1229,1265,1324,1365,1385,1408`.

  Fix: Extract `fn save_config(&mut self) -> Task<Message>` that writes config
  and on failure spawns a notification toast or reverts in-memory state.

---

## High

- [ ] **H-A1. Filter logic duplicated** — `load_apps()` does filtering +
  sorting + duplicate detection synchronously on the main thread.
  `filter_apps()` does the same async. Both call `config.filtered()`.

  Fix: After loading `all_entries`, delegate to `filter_apps()` instead of
  duplicating the pipeline.

- [x] **H-A2. Desktop files subscription fires too aggressively** — Removed.
  The daemon is the sole catalog owner and refreshes desktop applications
  through the bridge.

- [ ] **H-A3. State scattered across 3+ reset locations** — `scroll_offset`,
  `search_value`, `cur_group` are reset in `SelectSection`, `SelectGroup`,
  and `close()` independently. Adding a new state field means remembering
  every reset site. Files: `src/app.rs:594-599,1112-1115,1132-1134`.

  Fix: Consolidate into `fn navigate_to_section()` and
  `fn navigate_to_group()` methods that reset all related state atomically.

- [ ] **H-A4. `Ord` impl on `FilterType`/`AppGroup` is semantically wrong** —
  Two `AppIds` groups with different IDs compare as `Equal`. Violates Rust's
  `Ord` contract (`a == b` must imply `a.cmp(b) == Equal`). File:
  `src/app_group.rs:40-104`.

  Fix: Either implement a meaningful `Ord` (sort by name) or remove `Ord`
  entirely and use an explicit comparator.

- [ ] **H-A5. `group_keys` parallel data structure** — Invariant
  `len == sections[cur_section].len()` maintained by hand in 5 places.
  Desync = broken tabs. Files: `src/app.rs:1809,1261,1190,1404,2154`.

  Fix: Embed the key into `AppGroup` directly, or wrap in a `TabState`
  struct that enforces invariants internally.

---

## Medium

- [ ] **M-A1. Unbounded icon cache** — `RASTER_ICON_CACHE` grows forever. ~170KB
  per RGBA icon at 208x208. File: `src/icon_cache.rs:134`.

  Fix: Use `LruCache` with a capacity bound, or document the memory cost.

- [ ] **M-A2. Custom `ApplicationButton` widget** — Fragile layout indexing
  `children[0].children()[0].children()[0]`. Breaks on upstream libcosmic
  changes. File: `src/widgets/application.rs:367-404`.

  Fix: Use `iced::widget::mouse_area` for right-clicks and `stack!` for
  badge overlay. Eliminate the custom Widget.

- [ ] **M-A3. O(n^2) duplicate detection on main thread** — Allocates a
  `HashMap` every call, uses `fold` with string comparisons. File:
  `src/app.rs:520-552`.

  Fix: Move to async task. Use `HashSet` for O(n) dedup.

- [x] **M-A4. Unclear input precedence** — `input_ownership.rs` now combines
  managed-session state with compositor focus. The frontend subscribes to
  gamepad input only while it owns foreground input and rejects queued events
  after ownership changes.

- [ ] **M-A5. Config migration fragile** — Only migrates to `pc_games`, never
  writes back. Runs repeatedly. File: `src/app.rs:1796-1804`.

  Fix: Write migrated config immediately. Add `config_version` field.
  Chain migration steps.

- [ ] **M-A6. Hardcoded emulator list** — 27 substring matches on exec string.
  False positives possible. File: `src/app_group.rs:231-263`.

  Fix: Use XDG categories or a configurable list. Match against basename,
  not full exec line.

- [ ] **M-A7. Zero tests** — No unit tests for filtering, duplicate detection,
  emulator ID, config migration, or scroll math.

  Fix: Add unit tests for pure functions.

---

## Low

- [ ] **L-A1. Magic numbers in view code** despite centralized style module.
  File: `src/widgets/application.rs:80-88`, `src/app.rs:667`.

- [ ] **L-A2. `debug_fix.rs`** dead file in project root.

- [ ] **L-A3. `VERSION` constant wrong** — `0.1.0` vs Cargo.toml `1.0.12`.
  File: `src/config.rs:2`.

- [ ] **L-A4. `Sections` struct** should use `Vec` or `EnumMap` instead of
  named fields. File: `src/app_group.rs:200-226`.

- [ ] **L-A5. `rust-analyzer` in dev-dependencies** undocumented. File:
  `Cargo.toml:56`.
