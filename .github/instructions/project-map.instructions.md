---
description: Project layout, key entry points, dev commands, and architecture invariants for Hearthdeck. Read this before exploring the codebase.
---

# Hearthdeck — Project Map

Flutter (Dart) + Rust monorepo. Linux TV/kiosk game library frontend.

## Layout

| Path | What lives there |
|------|-----------------|
| `lib/` | Flutter app (all Dart source) |
| `services/` | Rust workspace: daemon, bridge, protocol, observability, overlay |
| `docs/` | Architecture docs |
| `deploy/` | systemd units, packaging |
| `scripts/` | Build/dev helpers |
| `test/` | Flutter tests |
| `justfile` | All dev commands — start here |

## Rust crates (`services/`)

| Crate | Role |
|-------|------|
| `hearthdeck-daemon` | HTTP/WebSocket API, SQLite state, discovery, pairing |
| `hearthdeck-bridge` | Linux-only desktop-entry discovery + allowlisted launches (socket-activated) |
| `hearthdeck-protocol` | Shared types |
| `hearthdeck-observability` | Shared tracing/metrics |
| `hearthdeck-overlay` | COSMIC layer-shell overlay (Rust/libcosmic) |

## Flutter entry points (`lib/`)

| File/Dir | Role |
|----------|------|
| `main.dart` | App entry |
| `tv_components.dart` | Shared TV-UI primitives (`TvFocusable`, `TvTwoPaneLayout`, etc.) |
| `full_library.dart` | Main catalog surface |
| `settings/` | Settings screens |
| `backend/` | API client |
| `catalog/` | Catalog models + repository |

## Dev commands

```
just setup            # toolchains + Flutter deps
just app              # Flutter client
just app-live ...     # live backend (needs HEARTHDECK_BACKEND_URL + HEARTHDECK_PAIRING_TOKEN)
just dev              # Flutter + backend together
just check            # format + all tests
just check-services   # Rust check/test/clippy
just test-app         # Flutter analyze + tests
just build-services   # release Rust build
just install-services # install systemd units for local testing
just services-status / just logs-daemon / just logs-bridge / just logs-errors
```

## Architecture invariants

- Controller-first navigation; Back is a global contract — no per-screen Escape handlers.
- Reuse `Tv*` widgets before adding new UI machinery.
- Platform-specific behavior behind adapters; shared API/catalog must not branch on OS.
- Launches always via transient systemd user units — never raw shell from daemon.
- Discovery providers are independent; one failure must not clobber another source.
- LAN access is opt-in: `HEARTHDECK_LAN_ENABLED=true` + TLS env vars.

## Validation

- Frontend only → `just test-app`
- Backend only → `just check-services`
- Cross-cutting → `just check`
