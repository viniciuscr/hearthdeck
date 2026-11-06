# Hearthdeck Backend Architecture

## Components

```text
Paired frontend client (Linux)
    |
    | paired HTTP(S) + bearer token, WebSocket events
    v
hearthdeck-daemon
    |
    | user-only Unix socket, typed newline-delimited JSON
    v
hearthdeck-bridge.socket -> hearthdeck-bridge (socket activated)
    |
    v
Freedesktop desktop entries and registered host operations
```

`hearthdeck-daemon` owns the versioned client API, pairing records, SQLite state, and
background work scheduling. `hearthdeck-bridge` is Linux-only and owns desktop-entry
discovery plus allowlisted launches inside the active graphical session.

`hearthdeck-protocol` owns the categorization report and its assignment, trait,
category and failure payloads. The AI crate re-exports these types and keeps the
scanner, taxonomy and `rail_app_ids` resolution logic. The daemon persists and
serves the same report type that the frontend reads; the JSON shape is unchanged.

`hearthdeck.target` is the systemd user-session root. It owns the API daemon
and bridge socket. The bridge is socket-activated, so daemon startup depends on
the socket existing rather than on a race-prone bridge process startup. Future
host capabilities receive their own typed socket/service pair under the same
target.

The daemon's session service owns launch acceptance, active-session lookup,
explicit stops, activity recording, session-change events and exit reconciliation.
HTTP handlers prepare requests and map errors; the activity store only persists
and queries plays.
Explicit stops and the periodic watcher share the same idempotent completion
path. The service borrows its bridge path, activity store, discovery coordinator
and event sender, rather than depending on the whole application state.

Catalog launch planning is a pure mapping from a catalog record to a typed
bridge request and its activity snapshot. It owns Heroic runner selection,
Stremio title selection (including desktop fallback for legacy records) and
desktop-entry dispatch. The HTTP handler only loads the record, asks for a plan
and submits it to the session service. Retro filesystem/core resolution stays
in the retro adapter.

Collection rule composition lives in `collections/rules.rs`. Its service borrows
the catalog, activity store and optional categorizer, without HTTP or application
state dependencies. It keeps existing owner-qualified rule dispatch, resolves
missing icons for curated and rule items, and leaves curated-item write rules
unchanged. Its composition operation isolates failed rails, preserves collection
order and fills missing icons after resolving each rule. HTTP loads the stored
collections and returns the composed result; it no longer owns rail policy.

Curated edits go through `CollectionStore::add_curated_item` and
`remove_curated_item`: the store checks editability, writes membership and returns
the stored snapshot. HTTP only maps typed missing/read-only/storage errors to
responses. Raw membership writes remain available for development seeding.

`categorizer/control.rs` owns the categorization feature's enable/disable,
snapshot, detection, rail publication and dashboard-reset operations. It borrows
settings, collections and the optional job service, not AppState. HTTP maps typed
availability, job-precondition and storage errors. Disabling still discards
running results and removes model-owned rails and verdicts; reset removes only
rails. Cleanup failures retain the existing warning-and-continue behavior.

`romm_link/control.rs` owns the RomM connection lifecycle: settings, password
sign-in, device approval and disconnect. It borrows settings and pending-link
state; HTTP only maps requests, credential-free responses and typed errors.
Loopback URL validation happens before contacting RomM. Password sign-in cancels
pending approval only after minting a client token; disconnect cancels it before
clearing settings. Direct token saves retain pending approval and their existing
bad-request error mapping. The underlying device flow still owns poll throttling
and expiry; this boundary does not change its concurrent-request semantics.

`stremio/control.rs` owns connection reads, link creation, approved-session
persistence and unlink sequencing. It borrows the session repository, pending link
and catalog; HTTP only maps credential-free responses and typed errors. Unlink
cancels pending approval, clears the session, then removes account-owned
`stremio` rows. Public `stremio-arrivals` rows remain. Link completion retains
its existing gateway-error mapping even for persistence failures; settings and
unlink failures remain internal errors. Link codes are not logged. The underlying
link state still owns expiry and polling, with unchanged concurrency semantics.

## Database baseline

The daemon accepts a fresh database or the current deployed schema, preserving
existing data when adopting the latter. Fresh creation includes every final
schema and seed-data effect; only the historical upgrade steps are retired.
Both paths record `PRAGMA user_version = 1`, replacing the old migration ledger.
Pre-baseline schemas and unknown future versions are rejected rather than
partially upgraded. Back up the database before adopting this baseline.
There is no historical `ALTER TABLE` or table-rebuild chain.

## Platform Adaptation Rules

Hearthdeck targets Linux only. Platform-specific behavior still terminates at an
adapter boundary — the bridge's `platform` module is the single place that
launches a host process — but with no second host target to stay portable for,
shared API, database, catalog, discovery scheduling, and frontend client code has
no platform branches at all.

| Concern | Linux host |
| --- | --- |
| Application discovery | `desktop-apps` (desktop entries) and `heroic` providers |
| Application launch | Supervised systemd user scope |
| Backend transport | Local daemon over the user-only bridge socket |
| Client platform code | None — the client speaks only the paired HTTP API |

A provider emits the same `CatalogRecord` shape regardless of source. The frontend
groups sources by the backend `source_id`, not by platform names. This keeps future
Steam, GOG, Epic, emulator, movie, and streaming providers portable at the contract
level.

## Discovery Providers

Discovery is a registry of asynchronous provider modules, not one global scan.
Each provider has a unique source ID, an optional refresh interval, and
normalization logic. It emits `CatalogRecord` values with globally namespaced
item IDs; the catalog repository atomically replaces only that provider's
source records. A failed provider cannot delete or overwrite items owned by
another provider.

Built-in catalog and launch source IDs live in `hearthdeck-protocol::sources`,
shared by the providers, bridge and frontend. IDs remain strings on the wire and
in SQLite: the registry accepts additional sources without changing an enum.

```text
Provider worker (one bounded queue per source)
  -> provider.discover()
  -> CatalogRepository.replace_source(source_id, records)
  -> LibraryChanged event
```

The coordinator has one coalescing worker per provider. A slow GOG scan cannot
block desktop apps, Epic, emulators, movie metadata, or stream providers.
Repeated refresh requests while a provider is queued or running return
`AlreadyScheduled`, avoiding duplicate scans.

### Provider Contract

Implement `DiscoveryProvider` for every source:

- `source_id()`: stable identifier, such as `desktop-apps`, `steam`, `gog`,
  `epic`, `emulators`, `jellyfin`, or `stremio`.
- `refresh_interval()`: `Some(Duration)` for independent periodic refreshes or
  `None` for explicit refresh only.
- `discover()`: fetch/scan only that source and produce normalized
  `CatalogRecord` values. It does not write SQL or emit API events.

Startup queues every provider. The API can enqueue one registered source with
`POST /v1/discovery/{source_id}/refresh`, or one metadata provider with
`POST /v1/metadata/{provider_id}/refresh`. It does not invoke scanners or write
catalog rows directly.

## Catalog Update Flow

```text
daemon starts
  -> queues every registered provider
  -> each provider scans independently
  -> CatalogStore.replace_source(source_id, records) commits one source snapshot
  -> daemon emits library_changed { source_id, record_count }
  -> connected frontend DaemonClient reloads GET /v1/library
```

The first scan runs automatically at daemon startup. Later refreshes are
provider-owned scheduled scans or explicit requests from Full Library / Settings.
The request endpoint returns `202 Accepted`; completion is communicated by the
`library_changed` WebSocket event rather than by holding an HTTP request open.

## Frontend Catalog Boundary

The frontend depends on the daemon library read model, never on a provider
name. The Rust `DaemonClient` (`providers/daemon.rs`) reads `CatalogItem`
records from `GET /v1/library` and classifies them into Games and
category-based App views from canonical `kind` and `metadata.categories`.
Desktop-entry and AppStream categories therefore classify applications
without a frontend provider-specific branch. (The former Flutter
`CatalogRepository`/`MockCatalogRepository` abstraction was removed with the
Flutter client.)

Packaged Linux builds pair with the loopback daemon at runtime. The client
reads `HEARTHDECK_BACKEND_URL` and `HEARTHDECK_PAIRING_TOKEN` from the
environment to use an explicit remote endpoint. Selecting a tile opens shared
details; the primary action delegates launch to the daemon.

`just dev` creates a temporary loopback pairing after daemon startup and starts
the COSMIC frontend, so the library uses real discovered catalog data in
development. The dashboard shelves and library search read the same daemon
catalog.

## Provider Health

`GET /v1/health` includes each discovery and metadata provider's stable ID,
kind, status, record count, last successful refresh, and safe error summary.
`starting` means no source snapshot has completed, `ready` means the last run
committed a snapshot, and `degraded` means the last run failed while prior
catalog rows remain untouched. Clients must distinguish `ready` with zero
records from `degraded`; an empty catalog is not a bridge health signal.

## Trust Boundaries

- Protected API routes are grouped behind one authentication route layer, before
  body, path, query and WebSocket extraction. Unpaired malformed requests return
  401 rather than parser errors. Only health and pairing completion are public;
  pairing-code creation remains exclusive to the loopback admin router.
- The client API receives typed resource and action IDs, never shell commands.
- The bridge protocol has typed health, discovery, launch, active-session, and
  stop-session requests. It has no command or argument field.
- `stop_active_application_session` selects and stops the active session under
  the launch-transition lock. Overlay Close App and logout call the authenticated
  daemon route `POST /v1/sessions/active/stop`; only the daemon sends this bridge
  command and processes play completion before replying. No client-side lookup
  or automatic stop retry can race a new launch.
- `hearthdeck-client` owns pairing and authenticated transport for frontend and
  overlay. Automatic pairing requests a code on loopback admin port 38401 only;
  remote clients need an explicit token. The overlay uses the same
  `HEARTHDECK_BACKEND_URL` and `HEARTHDECK_PAIRING_TOKEN` settings as the frontend.
  Its HTTP request timeout is 130 seconds, allowing the bridge's 120-second
  exchange deadline to return an error first. The overlay no longer opens the
  bridge socket or reads unbounded JSON lines. Update daemon and overlay together.
- The bridge re-discovers the requested desktop ID locally and validates its
  launch specification before creating a transient systemd user service; it does
  not accept `Exec` strings from the daemon.
- The bridge socket is created with mode `0600` under the active user's runtime
  directory.
- Every daemon-to-bridge exchange has a 120-second deadline covering connection,
  request writes and response reads. Responses must be newline-terminated JSON
  and fit within 16 MiB including the newline. A timeout is not proof that a
  launch or stop failed: the bridge may still finish it. Calls are not retried
  automatically.
- The API binds to loopback HTTP by default.
- LAN access requires `HEARTHDECK_LAN_ENABLED=true` and a Rustls certificate/key. It
  serves HTTPS only. Pairing-code creation remains on a separate loopback-only
  admin listener so nearby clients cannot self-pair.

## API

The public OpenAPI contract is `contracts/openapi.yaml`.

- `GET /v1/health`: unauthenticated daemon discovery/status.
- `POST /v1/pairing/complete`: consumes a host-created one-time pairing code.
- `GET /v1/library`: authenticated library metadata.
- `POST /v1/discovery/{source_id}/refresh`: authenticated request to refresh one
  discovery source.
- `POST /v1/metadata/{provider_id}/refresh`: authenticated request to refresh one
  metadata provider.
- `POST /v1/launch/{item_id}`: authenticated launch of whatever an item id
  names — the daemon resolves the prefix and picks the catalog or retro
  launch (operationId `launchItem`).
- `GET /v1/events`: authenticated WebSocket for library/action events.

The loopback admin listener exposes `POST /v1/pairing` for host-side pairing
flows. It is not included in the public API contract.

The unused `/v1/activity/recent`, `/v1/library/rescan`,
`/v1/retro/service/restart` and `/v1/settings` routes are retired. They return
`404` with or without credentials. Recently played games still arrive through
`GET /v1/collections`; discovery still runs at startup and on provider refresh.
Frontend appearance preferences live in COSMIC config, not the daemon's former
theme/backdrop API. RomM and Stremio connection settings keep their separate,
authenticated `/v1/retro/settings` and `/v1/stremio/settings` routes.

## Deployment

The packaged [session script](../packaging/arch/hearthdeck-session) starts
`hearthdeck.target` as a systemd **user** target, waits for the daemon and bridge
socket with bounded waits, then runs `cosmic-comp` with the frontend as its
client. RomM startup is not a compositor-readiness gate. The script starts the
overlay and optional cosmic-osk services; their restart policies handle the
initial display-environment race. This is a minimal COSMIC session, not
`cosmic-session` or the retired Gamescope kiosk.

The bridge launches desktop applications, RetroArch and Heroic through transient
systemd user services in the active display session. It does not impose a nested
Gamescope wrapper; Heroic's own per-game launch settings remain authoritative.
See [arch-package.md](arch-package.md#hearthdeck-session) for the current session
and [kiosk-session.md](kiosk-session.md) for historical incident notes.

All units live in [deploy/systemd](../deploy/systemd/); packaging paths are
symlinks to those canonical files. `just install-services` copies the backend
units and binaries and adds `90-hearthdeck-checkout.conf` drop-ins for
`~/.local/bin` paths. It only reloads the user manager: it never starts, stops,
enables, or disables units, and preserves existing enablement. Start explicitly
with `systemctl --user start --no-block hearthdeck.target`; the packaged session
already does this. A running checkout needs an explicit restart to use new binaries.
To undo a persistent enable left by an older installer, run
`systemctl --user disable hearthdeck.target` manually; this does not stop the
running target. Explicit `start` needs no enablement.
Configure LAN TLS through `~/.config/hearthdeck/daemon.env`; see
[daemon.env.example](../deploy/systemd/daemon.env.example).

## Where work is tracked

This doc owns the current shape only. Track review follow-ups in issues;
[`retroarch-integration.md`](retroarch-integration.md) owns the RomM/RetroArch
roadmap. See [`README.md`](README.md) for the full doc map.
