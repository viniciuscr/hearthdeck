# Hearthdeck Backend Architecture

## Components

```text
Paired frontend client (Linux)
    |
    | HTTPS + bearer token, WebSocket events
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

`hearthdeck.target` is the systemd user-session root. It owns the API daemon
and bridge socket. The bridge is socket-activated, so daemon startup depends on
the socket existing rather than on a race-prone bridge process startup. Future
host capabilities receive their own typed socket/service pair under the same
target.

## Platform Adaptation Rules

Platform-specific behavior must terminate at an adapter boundary. Shared API,
database, catalog, discovery scheduling, and frontend client code must not
branch on macOS, Linux, or Android.

| Concern | Linux host | macOS development host | Android client |
| --- | --- | --- | --- |
| Application discovery | `desktop-apps` provider | `macos-apps` provider | None |
| Application launch | Supervised systemd user scope | LaunchServices `open -b` adapter | Never launches host apps directly |
| Backend transport | Local daemon | Local integration daemon | Paired HTTPS daemon |
| Client platform code | No Linux-specific discovery code | macOS entitlements only | Android network/security config only |

The daemon chooses providers with compile-time target guards. A provider emits
the same `CatalogRecord` shape on every host. The frontend groups sources by
the backend `source_id`, not by platform names. This keeps future Steam, GOG, Epic,
emulator, movie, and streaming providers portable at the contract level.

## Discovery Providers

Discovery is a registry of asynchronous provider modules, not one global scan.
Each provider has a unique source ID, an optional refresh interval, and
normalization logic. It emits `CatalogRecord` values with globally namespaced
item IDs; the catalog repository atomically replaces only that provider's
source records. A failed provider cannot delete or overwrite items owned by
another provider.

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

The API can enqueue every provider with `POST /v1/library/rescan`, or one
registered source with `POST /v1/discovery/{source_id}/refresh`. It does not
invoke scanners or write catalog rows directly.

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

## macOS Integration

The `macos-apps` provider discovers bundles from `/Applications` and
`~/Applications`, reads `CFBundleIdentifier` from each bundle's `Info.plist`,
and launches only a re-discovered bundle through `open -b <bundle-id>`. It is
the macOS equivalent of Linux `desktop-apps`; both produce the same bridge
`DiscoveredApplication` protocol record and catalog schema.

Run a real macOS discovery scan without launching applications:

```sh
just macos-discovery-check
```

## Trust Boundaries

- The client API receives typed resource and action IDs, never shell commands.
- The bridge protocol has typed health, discovery, launch, active-session, and
  stop-session requests. It has no command or argument field.
- The bridge re-discovers the requested desktop ID locally and validates its
  launch specification before creating a transient systemd user service; it does
  not accept `Exec` strings from the daemon.
- The bridge socket is created with mode `0600` under the active user's runtime
  directory.
- The API binds to loopback HTTP by default.
- LAN access requires `HEARTHDECK_LAN_ENABLED=true` and a Rustls certificate/key. It
  serves HTTPS only. Pairing-code creation remains on a separate loopback-only
  admin listener so nearby clients cannot self-pair.

## API

The public OpenAPI contract is `contracts/openapi.yaml`.

- `GET /v1/health`: unauthenticated daemon discovery/status.
- `POST /v1/pairing/complete`: consumes a host-created one-time pairing code.
- `GET /v1/library`: authenticated library metadata.
- `POST /v1/library/rescan`: authenticated request to refresh every registered
  discovery and metadata provider.
- `POST /v1/apps/{id}/launch`: authenticated launch of a registered item.
- `GET /v1/events`: authenticated WebSocket for library/action events.

The loopback admin listener exposes `POST /v1/pairing` for host-side pairing
flows. It is not included in the public API contract.

## Deployment

Install and enable `hearthdeck.target` from the systemd **user** units in
`deploy/systemd/`. A user service owns graphical application launches. The
Hearthdeck Kiosk session's script execs Gamescope directly on the DRM/KMS seat
with the frontend as its only child; there is no separate desktop
compositor in front of it. The bridge launches registered desktop
applications and RetroArch games as direct clients of that same session
(confirmed on real hardware that a second Gamescope process joining it as a
peer is never shown - see `docs/kiosk-session.md`); Heroic is the one
exception, kept in its own nested Gamescope instance so its games can still
get their own internal resolution/upscaling.
Configure LAN TLS through
`~/.config/hearthdeck/daemon.env`; see `deploy/systemd/daemon.env.example`.

## Next Slice

Add a host pairing screen that displays the pairing code and TLS fingerprint,
then store the paired endpoint/token in the frontend client using secure
storage. After that, replace static library models with `GET /v1/library`
and use WebSocket events to refresh the UI.
