# The library grid

How the library screen turns records into a grid, and why the RomM path is
shaped the way it is. Read this before changing how consoles load, how entries
are ordered, or how the grid is drawn.

## Two record sources

The grid draws from two lists that must not be mixed:

- **Provider catalog** (`HearthDeck::all_entries`) — desktop/Heroic/AppStream
  records from the daemon, held in name order.
- **RomM pages** (`HearthDeck::romm_listings`) — live ROM listings, held in their
  own per-scope logs (below).

RomM is deliberately *not* a `DiscoveryProvider`: its libraries run to thousands
of ROMs and the catalog has no pagination. See decision 6 in
`retroarch-integration.md`. That separation is also what keeps the grid stable.

## RomM is scoped, and every scope caches its own pages

The model is `screens/library/console.rs` (`ConsoleScope`, `ConsoleScopes`); `app/mod.rs`
drives it (`console_scope`, `ensure_console_scope`, `load_console_page`,
`fetch_console_page`). There is no global ROM list.

```rust
struct ConsoleScope {                        // one per query, in ConsoleScopes
    platform: Option<i64>,                   // the tab; None is "all consoles"
    search: String,                          // the source narrows by these two
    facets: BTreeMap<String, String>,
}
struct ConsoleListing {                      // one per scope
    entries: Vec<Arc<DesktopEntryData>>,     // append-only, the source's order
    versions: BTreeMap<String, Vec<RetroRomVersion>>,
    next_offset: Option<u32>,                // None once the scope is fully loaded
    pending: Option<u64>,                    // generation of the request in flight
    generation: u64,                         // bumped by every first-page load
    loaded: bool,                            // has answered at least one page
}
```

A scope is a value (`ConsoleScope`) keyed into a map, not a tuple of globals. That is the
whole point: opening console B **never clears console A's pages**, so switching back is
instant and no scope depends on another. An earlier design kept one global
`total`/`offset`/`loading`/`generation` and poured ROM entries into `all_entries`, so every
console open had to refetch.

`generation` is per scope: a reload bumps only its own, so a page from a previous load of
*that* scope is dropped while other scopes keep loading in parallel. That is "the load the
user superseded" — not "the tab the user left". Pages of tabs the user left are exactly
what the cache keeps.

The warmth is bounded, so a long session of searching cannot grow the cache without end:
only `MAX_NARROWED_SCOPES` narrowed scopes (a search or facet selection) stay cached beside
the tabs, dropped least recently used first. A tab is never dropped — the strip bounds
them — and neither is a scope with a page in flight, whose answer still has to land in its
log. A dropped scope is not a failure: opening it again goes through `ensure` and fetches
page 0 like a first visit.

**Contract: a tab switch snaps to the target tab's own records.** The moment the tab
changes, the grid draws that scope's cached pages — nothing at all while its first page is
still out, then that page when it lands. The previous tab's records are never drawn under
the new tab, not even for a frame. `ConsoleScopes::ensure` is the decision: `Cached` (paint
from the cache, fetch nothing), `Loading` (a first page is already out, wait), or
`Requested(generation)` (fetch page 0). `App::refilter` reads only the scope on screen,
through `ConsoleScopes::entries`; an arriving page is logged into the scope its message
names (`ConsolePageLoaded { scope, .. }`) whether or not that scope is on screen, and
repaints only when it *is*.

A settled search or facet change is a new scope and behaves the same way. Typing is not:
while the user types, the drawn list stays where it is (no `refilter` per keystroke) — the
console grid is narrowed by the daemon, and re-projecting mid-typing would show the empty
list of a scope nobody has asked for yet. The snap happens once, when the query settles.

Within one scope, a reload does not blank the grid: what is loaded keeps its order until a
first page replaces it, so a reload swaps atomically.

This exact regression shipped once. The rewrite kept one global `ConsoleListing` for the
console grid, so a tab switch copied the previous tab's pages into the drawn list and they
stayed there until the new tab's page 0 arrived — about a second of the old tab's tiles
under the new tab's title. The per-scope map and the snap rule above are what prevent it;
do not collapse them back into one listing.

## Display order is fixed once drawn

Invariant: **an entry never changes position after it is drawn.**

- Each grid is drawn in exactly one order: a local section is the catalog in name order
  (`AppLibraryConfig::filtered_by_facets`), and the console grid is one scope's pages in
  the source's arrival order (`ConsoleScopes::entries`). The two never share a grid.
- Pages are appended, never re-sorted.

Why: pages arrive a page at a time in RomM's order, which need not match a local
name sort. The provider channel re-emits every ~30s, and the old global
`all_entries.sort_by(name)` ran over the ROM entries too — so it reshuffled tiles
the user had already scrolled past, every 30 seconds. Keeping ROM records out of
`all_entries` removes the coupling entirely.

**Trap:** never sort ROM entries together with the catalog, and never re-add a
global sort over the loaded pages. There is one ordering source per grid, and the
console one is append-only.

## The count has one unit

The Console Games header shows the scope's **grouped** total — the number of
collapsed games the grid actually draws — taken once from the scope's first page.

**Trap:** do not seed the header from RomM's per-platform `rom_count`. That counts
files (regions, revisions, discs separately), so a library with collapsed titles
shows a bigger number than the grid has tiles; switching from that file count to
the grouped total a moment later is the "900 → 878" jump. While the scope's total
is unknown the header stays blank rather than showing a number it will replace.

## Pages load on demand, and prefetch in the background

- When platforms arrive (`Message::ConsolesLoaded`), `queue_console_prefetch` enqueues the
  "all consoles" tab plus every console tab in tab order — "all" is the landing view — and
  `pump_console_prefetch` issues them under `MAX_PREFETCH_INFLIGHT` (3), so opening any
  console paints from cache instead of clearing the grid and waiting. Scopes that already
  answered or are loading are left out; the prefetch is never cancelled and its limits
  never shrink.
- `ensure_console_scope` shows a warmed scope instantly (`Ensure::Cached`); it requests
  page 0 only if the scope has never loaded (`Ensure::Requested`), and never asks twice
  while a first page is out (`Ensure::Loading`).
- Plain browsing loads further pages as the viewport approaches the end
  (`ConsoleScopes::extend_offset`, one row early). A search or an active facet selection
  narrows at the daemon, so each is its own scope with its own pages — there is no local
  filtering over loaded records to complete.
- Console facets come from the daemon per tab (`retro_facets`), not from loaded records,
  so the filter sidebar does not wait for a scope to page in.

## The grid is virtualized

Only the rows around the viewport become widgets (`grid_row_window`), with
spacer elements reserving the rest so the scrollable's extent is exact. Each row
element is sized to `grid_row_height` (tile + gap), so spacers are exact multiples
and no column spacing is needed. Icons are resolved per visible tile, not for the
whole list.

Why: building and rasterizing every tile is the dominant cost of a big library,
and the prefetch makes big lists common.

**Trap:** `grid_row_window` must always include the focused row, or `focus(id)`
has no widget to resolve and controller focus silently fails.

## Artwork is read off disk, then copied once

`romm::asset` in the daemon reads per-ROM artwork **straight off the resources
mount** (`read_resources_asset`) — only bundled console art goes through RomM's
HTTP. But the frontend still pulls those bytes over loopback
(`/v1/retro/assets`) and writes a second copy into `~/.cache/hearthdeck/icons`,
and a page is only handed over once all its covers are cached.

This is the known redundancy: a local read plus a copy and a loopback hop, none
of which is needed. The fix is for the daemon to hand back the resolved host path
so the frontend renders `IconSource::Path(...)` directly, deleting the copy and
the per-page wait.

## Traps (do not)

- Merge ROM records into the catalog (`App.catalog`).
- Sort the grid or the catalog with ROM entries in scope.
- Show RomM's per-platform file count anywhere the grid draws collapsed games.
- Draw one scope's pages as another scope's list. The console grid reads only
  `ConsoleScopes::entries` of the scope on screen, and a tab switch snaps to the target
  tab's own records (or nothing at all) — never to what the previous tab had on screen.
- Re-project the console grid while the user is typing. The snap to the new scope's own
  records happens once, when the query settles, not per keystroke.
- Drop the focused row from the virtualized window (`grid::row_window`).
- Page ROM data through the catalog/`DiscoveryProvider` path (unpaginated, timed
  replacement — see decision 6 in `retroarch-integration.md`).
