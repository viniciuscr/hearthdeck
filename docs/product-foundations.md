# Hearthdeck Product Foundations

This document is the product and engineering baseline for Hearthdeck. It is
the decision reference when a feature, provider, screen, or service is added.
More detailed operational documents must agree with these rules.

## Product Promise

Hearthdeck is a local-first, controller-first library and launcher for a TV or
living-room display. It helps a person find, understand, and launch the
software and games available on their own host without requiring a desktop
shell, a mouse, or a cloud account.

The product is not a general desktop environment, remote shell, package
manager, or store client. It may surface host capabilities and request an
approved action, but it must not turn a paired client into unrestricted host
control.

### Product Priorities

In priority order, a change must preserve:

1. Predictable controller operation.
2. A clear, trustworthy view of the user's library.
3. Low idle memory, CPU, and background I/O use.
4. Local control and explicit trust boundaries.
5. Extensibility through provider and platform boundaries rather than
   platform-specific UI branches.

When priorities conflict, do not trade the first three for visual novelty,
background convenience, or a provider-specific shortcut.

## Interaction Contract

Controller support is not an accessibility add-on. It is the primary way to
operate every Hearthdeck screen.

| Input | Meaning | Required behavior |
| --- | --- | --- |
| D-pad or left stick | Move focus | Move to the expected adjacent actionable control. Never require pointer movement to reveal or reach an action. |
| A / confirm | Activate | Activate only the currently focused control. The label and focus treatment must make the consequence clear before confirmation. |
| B / Back | Return or dismiss | Follow the same dismissal order everywhere. It must never perform a destructive operation or unexpectedly exit Hearthdeck. |
| Right stick | Scroll | Scroll the current scrollable surface without changing the selected control. |
| Keyboard arrows, Enter/Space, Escape | Desktop equivalents | Preserve the equivalent directional, confirm, and back behavior. |

### Back Is A Global Contract

`B`, gamepad Back, and Escape express one intent. They use this order:

1. If a writable text field owns focus, dismiss text input and the on-screen
   keyboard while keeping the current screen open.
2. Otherwise dismiss the topmost transient UI, such as a dialog, side sheet,
   filter, or menu.
3. Otherwise return from the current child route to its parent route.
4. At the root, do not exit the application. A temporary message may be
   dismissed; otherwise Back is a no-op.

Every new route, dialog, sheet, and text input must participate in that order.
Do not assign a local meaning to B that differs from it.

`B` (gamepad) and Escape (keyboard) are handled by one global, focus-independent
mechanism instead of per-screen code:

- `lib/main.dart` registers a single `HardwareKeyboard.instance.addHandler`
  for the lifetime of the app. It fires for every `Escape` key event
  regardless of what currently has focus, and routes it through the same
  `_handleBackIntent` logic (dismiss a focused writable text field via
  `unfocusWritableEditableText`, else `maybePop` the app's global
  `navigatorKey`).
- The framework's own default `Escape -> DismissIntent` keyboard shortcut is
  removed from `MaterialApp.shortcuts` so it cannot also fire through the
  focus tree and cause a double pop alongside the handler above.
- Gamepad Back reaches the same `_handleBackIntent` logic through the
  `TvBackIntent` `Actions` handler in `MaterialApp.builder`.

Screens must **not** reintroduce a local `Focus`/`Shortcuts`-based Escape
handler. Flutter dispatches key events starting from `primaryFocus` and
bubbles *up* the tree, so a per-screen `Focus(onKeyEvent: ...)` widget only
receives Escape once something inside that screen actually holds focus. When
a screen has no `autofocus: true` anywhere, `primaryFocus` sits on the
route's own scope (an ancestor of that widget), and the handler is
unreachable until the user happens to hover or otherwise focus something on
the page. This was a real bug (Service status screen: Escape only worked
after hovering the refresh button) and is exactly why Back is a single
app-wide listener now rather than a per-screen concern. See the regression
test `Escape leaves the Service status screen even when nothing on it has
ever been focused or hovered` in `test/widget_test.dart`.

### Focus Is The Navigation State

The focused control is the user's cursor. Every interactive element must:

- be reachable using directional focus from a sensible initial focus;
- expose a high-contrast focus state that does not rely only on color;
- remain visible when focused, scrolling into view when necessary;
- have a useful semantic label and a visible label or icon meaning;
- work with controller, keyboard, and pointer without changing its result.

Use `TvFocusable`, `TvDirectionalFocusNavigation`, and the shared TV controls
before creating a custom focus implementation. A custom control needs an
explicit focus, activation, semantics, scroll-visibility, and return-focus
plan. A screen must not strand focus after a dialog closes or a list reloads.

For a destructive action, A opens a clearly focused confirmation surface; it
does not immediately destroy data. The safe or cancel action is the initial
focus unless the user is confirming an action that is already reversible.

### Navigation Shape

Navigation should be shallow and stable:

- Library surfaces answer *what is available*.
- Details answer *what is this and what can I do with it*.
- Settings answer *how Hearthdeck behaves*.
- A transient surface changes a local choice without becoming a destination.

The same content detail layout and action order must be used regardless of
whether an item came from a desktop entry, Heroic, a future store adapter, or
another catalog source. Source-specific concepts belong in metadata facts, not
in a different navigation model.

## Library And Metadata Model

The catalog is a local materialized view. Discovery establishes that an item is
available and how it may be launched. Enrichment establishes how to describe
and classify that item. They are separate on purpose:

```text
host or installed-client data  -> discovery provider -> library_items
local authoritative metadata   -> metadata provider  -> catalog_enrichments
                                                   \-> catalog read model -> API -> Flutter
```

The daemon never performs enrichment during a Flutter request. The client
receives one already-joined read model and does not infer metadata from a
provider name or application title.

### Canonical Item Fields

Every catalog item must have the following baseline. Missing rich data is
normal and must produce an honest, useful fallback rather than a guessed fact.

| Area | Fields | Purpose |
| --- | --- | --- |
| Identity and ownership | `id`, `source_id`, `title`, `kind`, `updated_at` | Stable identity, source snapshot ownership, display, and freshness. |
| Launch | `launch_id` | Typed identifier used to match enrichment and request an approved launch. It is never a command line. |
| Presentation | `icon`, `summary`, `description`, `screenshots` | A scannable card and detail view. Missing artwork must have a local visual fallback. |
| Classification | `kind`, `categories` | Separates games from applications and produces predictable library groups and detail hints. |
| About | `developer`, `project_license`, typed `urls`, `provenance` | Explains who made an item, applicable licensing, useful links, and where metadata came from. |
| Installed state | `store`, `runner`, `version`, `platform`, `install_size_bytes`, `cloud_saves` | Explains the local installation without making an unsupported launch claim. |
| Compatibility | `requirements`, `memory_compatibility` | Shows publisher-provided requirements and a narrowly scoped memory comparison when available. It is not a CPU, GPU, or general compatibility verdict. |

`id` is globally namespaced and immutable once shipped. Providers currently
use prefixes such as `desktop:` and `heroic:epic:`. `source_id` owns a complete
discovery snapshot; an item ID must not collide with any other source. A
provider may replace only its own snapshot.

`launch_id` is a stable source-specific match and launch key, not a display
label. Metadata providers match desktop IDs, bundle IDs, or platform IDs only.
Matching by a human-readable title is forbidden because it can attach another
application's information to the user's item.

### Classification And Presentation

Classification has three layers:

1. **Kind** is the primary divide: `game` or `application` today. New kinds
   require a shared API contract and a deliberate UI treatment.
2. **Browse category** is the primary recognized category used to group
   applications. Games remain in the Games view unless their game provider
   introduces a stable, useful sub-browse model.
3. **Tags and facts** include all remaining categories, store, runner,
   platform, developer, and requirements in the detail view.

The current Linux rules are deliberately conservative:

- A Freedesktop entry with the `Game` category becomes a game, except known
  game-launcher applications such as Steam, Lutris, Heroic, and Hearthdeck.
- Other desktop entries are applications and are grouped by recognized
  AppStream or Freedesktop categories such as Media, Network, Office, System,
  or Utility.
- Installed Heroic Epic and GOG records are games.

An unknown category belongs in `Other`; it must not be silently renamed to a
more specific category. A source or store is provenance, not classification.

The interface should make each decision legible: show a concise summary on the
card, a stable kind/category in the library, and source, store, runner,
compatibility, and provenance as detail facts when present. URL metadata is an
action, not an inert text field. Screenshots are optional media, never a
requirement for a usable item.

### Metadata Trust, Priority, And Media

Metadata must come from a source that can be linked to a stable identity:

- `appstream-local` reads installed AppStream metadata and is the default
  Linux application source.
- The Heroic provider reads locally cached installed-game data without reading
  account credentials.
- Future store, Flatpak, emulator, or VCS adapters need their own provider ID,
  identity matching rule, refresh policy, priority, and provenance.

Each metadata provider stores an independent snapshot. At read time, the
highest-priority and newest matching enrichment wins; lower-priority metadata
remains stored so it can become useful again. An enrichment failure must retain
the previous successful snapshot.

Provider URLs are untrusted input. Only validated HTTP(S) URLs become external
link actions. The target media design is a daemon-owned asset cache that
downloads, validates, sizes, evicts, and serves artwork locally. Frontend code
must not treat a provider URL as a trusted local asset.

**Current gap:** Heroic artwork URLs are still passed to Flutter and loaded
directly. This is a transitional implementation, not the asset policy. Do not
extend it to additional providers; replace it with the bounded asset cache.

### What Is Not Yet In The Catalog

RomM remains a paginated live source rather than catalog material. The daemon
stores only its connection settings and proxies consoles, ROM pages, artwork,
and managed RetroArch launch; it does not materialize those games into
`library_items`. Flutter uses its dedicated Retro view, while the COSMIC
frontend merges only the selected console's live pages into its Console Games
surface.

Search (`lib/search.dart`) reads through `CatalogRepository.load()` for PC
games/apps (the same repository `FullLibraryPage` uses - live when
configured, fixture-backed `MockCatalogRepository` otherwise) and issues a
live, debounced query against `GET /v1/retro/roms?q=...` for console games,
so it is no longer a separate fixture list. A `LibraryCategory` chip
(All/PC games/Apps/Console games) scopes which of those sources is queried;
opening search from a specific section (e.g. the Console games screen's own
Search button) only pre-selects that chip as a default - the user can widen
it back to All. `platform_id` on `/v1/retro/roms` is optional specifically so
an unscoped search can span every console at once, forwarded to RomM's own
`search_term`, rather than requiring the client to page through every
console's full library to filter locally.

## Service Architecture

The client, daemon, bridge, and host each have a narrow responsibility:

```text
Flutter client
  controller focus, routes, rendering, repository boundary
       | paired HTTP(S) requests and WebSocket events
hearthdeck-daemon
  API, pairing, SQLite, catalog joins, schedules, provider health, events
       | typed newline-delimited JSON over a user-only Unix socket
hearthdeck-bridge (Linux, socket activated)
  desktop-entry discovery, validation, supervised launch/session control
       |
host desktop entries, transient systemd user services, direct-connect or (Heroic only) nested Gamescope
```

The daemon owns the public contract, persistent state, and background work. It
must not contain host-specific command construction. The bridge owns Linux
integration and re-discovers an item before launch; it receives typed IDs, not
shell commands or `Exec` strings. Flutter depends on `CatalogRepository`, not
on a provider, transport, Linux API, or database schema.

On Linux, `hearthdeck.target` starts the daemon and owns the bridge socket. The
socket has a user-only `0600` mode and activates the bridge only when the daemon
needs it. PipeWire/WirePlumber, NetworkManager, and BlueZ remain independent
host services. Any future host integration gets its own typed adapter boundary
and socket/service pair rather than becoming a privileged daemon feature.

In Kiosk mode, the outer Gamescope process is the only compositor beneath
Hearthdeck. A managed desktop application runs in a separate, on-demand nested
Gamescope session. The outer kiosk must not gain a desktop shell just to launch
an application.

### Refresh And Failure Model

Each discovery or metadata provider has one coalescing worker and a one-item
queue. A provider may run independently, but repeated requests while it is
queued or running are coalesced. Completion atomically replaces only that
provider's snapshot, then emits `library_changed` or `metadata_changed`.

`202 Accepted` means work was queued or already scheduled, not that the
library is current. Clients refresh after the completion event. Provider health
is `starting`, `refreshing`, `ready`, or `degraded`; `ready` with zero records
is valid, while `degraded` preserves previously successful catalog data.

Current schedules are intentionally modest: Linux desktop entries every 15
minutes, Heroic every 5 minutes, local AppStream every hour, and macOS bundles
every 30 minutes. Startup queues each registered provider. No provider should
poll a filesystem, network endpoint, or client request in a tight loop.

## Resource Discipline

Hearthdeck is expected to remain responsive on hardware where the launcher is
always visible. A small background feature is not automatically cheap: idle
timers, duplicate caches, image decoders, and long-lived subprocesses all
compete with games and media applications.

### Non-Negotiable Rules

- Do not add an always-running process when a daemon module, socket activation,
  or an existing host service is sufficient.
- Do not do discovery, enrichment, image downloads, or blocking host work on
  the Flutter UI path.
- Bound queues, payload sizes, event history, result pages, retries, image
  dimensions, asset cache size, and diagnostic output. Coalesce repeated work.
- Store durable catalog state in SQLite and keep only the working set in
  memory. Do not mirror the full catalog in multiple services.
- Refresh only on startup, a measured interval, a relevant host event, or an
  explicit user action. A visible screen must not cause a recurring rescan.
- Lazy-load and page large collections. Decode and retain only artwork that is
  near the visible viewport.
- Measure idle and active CPU, resident memory, wakeups, database size, and
  launch latency before accepting a new background capability. Record the
  target device and measurement method with the change.
- Make failure cheap: retain the last good snapshot, surface a safe status,
  and retry on the normal schedule instead of spinning.

### Current Enforcement And Gaps

The current design already uses SQLite WAL, source-scoped transactions,
one-item coalescing queues, bounded API bodies, a bounded WebSocket event
buffer, and socket activation for the bridge. The kiosk session avoids a
desktop shell.

There is not yet a device-specific resource budget, systemd CPU/memory quota,
provider execution timeout, or global cross-provider concurrency limit. These
are known gaps. A provider added with network I/O, large media, or expensive
scanning must establish its resource budget and cancellation/timeout behavior
before it is enabled by default. Resource limits belong in the provider and
service design, not only in a post-release profiling exercise.

## Change Checklist

Before merging a product or architecture change, answer these questions in the
design, issue, or pull request:

1. Can every action, including errors and empty states, be completed with the
   controller? What exactly do A and B do at each new surface?
2. Does the screen use the shared focus, route, detail, and action conventions?
3. Which catalog fields are authoritative, optional, derived, and shown to the
   user? How are identity, classification, and provenance preserved?
4. Does metadata match a stable ID, avoid title guessing, and retain prior
   data when a refresh fails?
5. Which process owns the work? Why is it not a new resident service? What is
   bounded and what wakes it up?
6. Does the client receive typed data and actions only, with no command,
   credential, or host-path leakage?
7. What will happen on a slow machine, disconnected network, empty source,
   malformed provider data, or a controller-only session?

If an answer requires a new exception, document it here or in the relevant
architecture document before shipping it.
