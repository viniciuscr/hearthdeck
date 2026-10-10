# Frontend rewrite — a new app, grown from scratch

`app.rs` grew to ~10,200 lines: one `HearthDeck` struct, one giant `update`, and
every screen's `view_*` in one file. This is a rewrite of the **UI and
navigation**, built as a new app module alongside the old one, kept honest by
dropping the cruft inherited from the project this started from.

## Layout

- `app/` — **the new frontend, and what `main` runs.** Grown one screen at a
  time. It reuses the plumbing below the UI (`providers`, `app_group`,
  `icon_cache`, `input_ownership`, `launch_state`, `style`, `ui`, `widgets`);
  only the screens and navigation are rewritten.
- `app_legacy.rs` — the inherited app. Still compiled and runnable behind
  `HEARTHDECK_LEGACY=1`, as reference and fallback, until `app/` covers every
  screen.
- `ui/` — the design system: shared, styled widgets every screen composes from.
  Grows on second use; never a per-screen copy.
- `screens/<name>/…` — one module per screen, each owning its own state,
  `Message`, `update` and `view`.
- `style.rs` — shared tokens and the base button classes; `ui/` composes on top.

## Build speed

- `.cargo/config.toml` links with clang + LLD (installed on the host), which is
  faster than the default bfd linker. Setting `rustflags` costs one full rebuild
  once; after that only linking is faster. Delete the file to opt out.
- sccache is installed but deliberately off; `export RUSTC_WRAPPER=sccache` is
  the documented opt-in (see `CONTRIBUTING.md`).
- Audit candidate: libcosmic is enabled with `xdg-portal`,
  `desktop-systemd-scope` and `single-instance`; the first two look unused
  (we launch via our own daemon/bridge) and the third is only used by
  `app_legacy`. Measure with `cargo build -p hearthdeck-frontend --timings`
  before trimming.

## Decisions: use COSMIC vs own it

- **Sidebar — own it.** COSMIC's `nav_bar` was tried (segmented list, its own
  type/icon sizes and grey panel) and does not fit the kiosk look, and it is not
  ours to restyle. The sidebar stays our component
  (`screens/library/sidebar.rs`).
- **Buttons / containers — use COSMIC roles, restyle once.** Every custom button
  goes through `style.rs::button_class`; surfaces through the `ui/` kit. Do not
  hand-roll a button per screen.
- **Privileged widgets — use COSMIC.** `context_drawer`, `dialog`, `tooltip`,
  `Toasts`, `text_input`, `scrollable`.
- **The source defines the facets.** The console grid's filter options come from
  the daemon (`/v1/retro/facets`, backed by RomM's `/api/roms/filters`), not
  from the loaded page, and selecting one re-queries the daemon. The old
  "Decade" facet was dropped: RomM exposes no release-year filter or year list,
  so it could neither be served nor enumerated.
- **One filter system, every section.** The PC Games and Applications sections
  derive their facets locally (`section_facets`: Store / Source) from the
  catalog they already hold, and filter in memory (`filtered_by_facets`); the
  console grid filters through the daemon query. Same `Facet` type, same
  `screens/library/filters.rs` sidebar (opened by the header's Filter button via
  `context_drawer`, one `◀ value ▶` row per facet) same `facet_selection` map —
  the source is the only thing that differs. The filter surface is a button +
  sidebar, matching the old console drawer, not a strip under the tabs.
- **A filter dimension has to be a choice, not a filing cabinet.** Applications
  filter by *where they were installed* — Flatpak / Snap / Nix / System / Local,
  plus Streaming for the service clients — instead of by freedesktop category.
  "Utility" is a bucket an entry lands in, not something anyone browses a
  launcher by, and it was the section's only dimension. The origin is resolved
  by the bridge: it is the process that reads the `.desktop` files, so it is the
  one that knows which directory each launcher came from, and the answer travels
  in the catalog metadata beside `store` and `license`. A streaming client
  carries its origin *and* `streaming`, so one facet answers both questions.
  The tabs are a separate matter and still follow the entries' categories.
- **Paging and search belong to the source, not the loaded page.** The console
  grid asks for one page at a time and sends its search term as the source's own
  query, so a search covers the whole library rather than the pages that happen
  to be loaded. `screens/library/console.rs` holds the pages and the cursor;
  `app/mod.rs` performs the request it decides on. Each scope change opens a new
  generation, so a page that arrives for a console the user has left is dropped
  rather than merged into the list they are looking at.
- **The grid is virtualized, at the pitch the paging measures.** Only the rows
  around the viewport are built, and each is laid out at exactly
  `grid::Metrics::row_height`, so "the end of the loaded rows is on screen" is
  answered in the same rows that are drawn. Rows that are not built are reserved
  with spacers of exactly their height, so the scrollable keeps the list's true
  extent.
- **Navigation is ours, not iced's.** The shell keeps a cursor
  (`screens/library/focus.rs`) rather than reading iced's focus, for two reasons:
  Confirm has to act on the control the user can see is highlighted even before
  the renderer has reported focus, and a filter row is a container, which cannot
  hold focus at all. Where a step goes is a pure function of the cursors and of
  what currently exists (`focus::step` -> `Outcome`), so the movement rules are
  tested without a compositor. iced is told where the cursor is afterwards
  (`App::focus_task`), which is where a button's `focused` style flag - and so
  the focus ring the design system already had - comes from; the drawer's rows
  are drawn from the cursor directly.
- **The D-pad owns the content.** It moves inside the grid, and inside whatever
  surface is open over it, and it never leaves for the chrome: a step off an edge
  does nothing at all. The first cut of this let the cursor wander between the
  sidebar, the tabs and the grid, and that read as the cursor getting lost —
  from the grid's first row, "up" landed on a tab the user was not looking at.
  The chrome is reached by its own buttons instead: the shoulders switch section,
  the triggers switch tab, the search button puts the caret in the search box,
  and Select opens the filter drawer, which is a surface over the grid rather
  than a place in it and so cannot be walked to. A keyboard reaches the same
  controls through the ring `Tab` walks, and every stop on that ring performs the
  action its button does, so the two cannot drift apart.
- **The pad remembers where it was.** A vertical step returns to the column the
  cursor came down, so a row that ends early - or the short last row of a console
  grid - does not drag it back to the first column. The drawer keeps a row of its
  own rather than being a place in the grid, so the tile it was opened over is
  where the pad still is when it closes. A list that is replaced puts the pad on
  its first tile, except when the search box replaced it: that happens on every
  keystroke, and taking the caret away mid-word is not a redraw.
- **Back is one function.** `App::back` unwinds the surfaces in the order they
  stack - a failed launch, then the filter drawer, then the keyboard's ring, then
  whatever took the caret - and nothing else handles Back for itself. The rung
  still missing is the page behind the library: the Dashboard.

## Running

- `just run-frontend-debug` — the **new** app.
- `HEARTHDECK_LEGACY=1 just run-frontend-debug` — the old app.

## Status

| Slice | Where | Status |
| --- | --- | --- |
| Sidebar (section nav) | `screens/library/sidebar.rs` | done |
| New app shell (sidebar + section selection) | `app/mod.rs` | done |
| Library: header (title + search + tabs, no rename/drag) | `screens/library/header.rs` | done |
| Library: grid (daemon-backed catalog, virtualized) | `screens/library/grid.rs` | done |
| Library: filtering — section/group/search, all sections | `app/mod.rs` | done |
| Library: Console data (RomM, paged) | `screens/library/console.rs`, `app/mod.rs` | done |
| Facets: generic provider-driven model + UI | `providers/filter.rs`, `screens/library/filters.rs` | done |
| Daemon: `/v1/retro/facets` + server-side `genre`/`region` filters | `hearthdeck-daemon` | done |
| Launch: tile activation, overlay, input ownership, session poll | `app/mod.rs` | done |
| Filters on every section: header Filter button + sidebar over one `Facet` model | `app_group.rs`, `app/mod.rs`, `screens/library/{header,filters}.rs` | done |
| Applications filter by install source (flatpak/snap/nix/system/local/streaming) | `hearthdeck-bridge`, `hearthdeck-daemon`, `app_group.rs` | done |
| Console tabs derived from the console list | `app/mod.rs` | done |
| Library: virtualization, paging, source-side search | `screens/library/{grid,console}.rs` | done |
| Library: pad navigation — the D-pad owns the content, chrome by chords, one global Back | `screens/library/focus.rs`, `app/mod.rs`, `subscriptions/gamepad.rs` | done |
| Dashboard | | planned |
| Details | | planned |
| Settings | | planned |

## Dropping (inherited cruft)

- Group rename: `edit_name`, `StartEditName` / `EditName` / `SubmitName`, and
  its virtual-keyboard round-trip.
- Reorderable group tabs: `reorderable_flex_row`, `ReorderGroup`, the
  `group_keys` / `next_group_key` machinery and the tab-strip reveal.

## Invariants (not negotiable)

- Controller-first navigation; Back is one global handler (no per-screen Escape).
- Launches go through transient systemd units (`launch_managed`), never raw shell.
- Theme tokens only — no color or size literals.
- Input ownership via `input_ownership.rs`.
- Reuse `ui/`, `widgets/` and `style.rs`; no per-screen styling.
