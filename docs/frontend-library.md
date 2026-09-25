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

```rust
enum RommScope { All, Console(i64) }        // "all consoles" is its own scope
struct RommListing {                        // one per scope, in romm_listings
    entries: Vec<Arc<DesktopEntryData>>,    // append-only, RomM's order
    total: Option<u64>,                     // from the first page
    next_offset: Option<u32>,
    loading: bool,
    generation: u64,
}
```

A scope is a value (`RommScope`) keyed into a map, not a tuple of globals. That is
the whole point: opening console B **never clears console A's pages**, so
switching back is instant and no scope depends on another. An earlier design kept
one global `total`/`offset`/`loading`/`generation` and poured ROM entries into
`all_entries`, so every console open had to refetch.

`generation` is per scope: a reload bumps only its own, so a page from a previous
load of *that* scope is dropped while other scopes keep loading in parallel.

## Display order is fixed once drawn

Invariant: **an entry never changes position after it is drawn.**

- `projection_source` yields the catalog (name order) then the active scope's
  entries (arrival order). `AppLibraryConfig::filtered` filters but preserves that
  order.
- Pages are appended, never re-sorted.

Why: pages arrive a page at a time in RomM's order, which need not match a local
name sort. The provider channel re-emits every ~30s, and the old global
`all_entries.sort_by(name)` ran over the ROM entries too — so it reshuffled tiles
the user had already scrolled past, every 30 seconds. Keeping ROM records out of
`all_entries` removes the coupling entirely.

**Trap:** never sort ROM entries together with the catalog, and never re-add a
global sort over `all_entries`. There is one ordering source, and it is append-only.

## The count has one unit

The Console Games header shows the scope's **grouped** total — the number of
collapsed games the grid actually draws — taken once from the scope's first page.

**Trap:** do not seed the header from RomM's per-platform `rom_count`. That counts
files (regions, revisions, discs separately), so a library with collapsed titles
shows a bigger number than the grid has tiles; switching from that file count to
the grouped total a moment later is the "900 → 878" jump. While the scope's total
is unknown the header stays blank rather than showing a number it will replace.

## Pages load on demand, and prefetch in the background

- When platforms arrive, `queue_romm_prefetch` enqueues `All` plus every console,
  and `pump_romm_prefetch` issues them under `ROMM_MAX_INFLIGHT` (3), so opening
  any console paints from cache instead of clearing the grid and waiting.
- `ensure_romm_scope` shows a prefetched scope instantly; it only requests page 0
  if the scope has never loaded.
- Plain browsing loads further pages as the viewport approaches the end
  (`ScrollYOffset`). A search or an active filter pulls the **whole** scope
  (`console_needs_all_pages`), because filtering and facets run locally over the
  loaded records.

**Follow-up:** facets come from loaded records today, so the filter sidebar is
partial until a scope is fully loaded. The real fix is to source facets from
RomM's own filter metadata instead of pulling every page; tracked in
`code-review-roadmap.md`.

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

`romm_asset` in the daemon reads per-ROM artwork **straight off the resources
mount** (`read_resources_asset`) — only bundled console art goes through RomM's
HTTP. But the frontend still pulls those bytes over loopback
(`/v1/retro/assets`) and writes a second copy into `~/.cache/hearthdeck/icons`,
and a page is only handed over once all its covers are cached.

This is the known redundancy: a local read plus a copy and a loopback hop, none
of which is needed. The fix is for the daemon to hand back the resolved host path
so the frontend renders `IconSource::Path(...)` directly, deleting the copy and
the per-page wait.

## Traps (do not)

- Merge ROM records into `all_entries`.
- Sort the grid or the catalog with ROM entries in scope.
- Show RomM's per-platform file count anywhere the grid draws collapsed games.
- Drop the focused row from `grid_row_window`.
- Page ROM data through the catalog/`DiscoveryProvider` path (unpaginated, timed
  replacement — see decision 6 in `retroarch-integration.md`).
