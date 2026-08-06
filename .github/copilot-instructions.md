# Response style defaults

- Be short and sharp by default.
- Answer directly. Skip filler, repetition, and optional explanation unless the user asks for it.
- Prefer the shortest correct reply that still includes the necessary action or result.
- Keep working notes and reasoning minimal; do not spill internal deliberation into the response.
- For coding tasks, return code first, then at most a few short lines of essential context.

---

# Project map — Hearthdeck

Flutter (Dart) + Rust monorepo. Linux TV/kiosk game library frontend.

## Top-level layout

| Path | What lives there |
|------|-----------------|
| `lib/` | Flutter app (all Dart source) |
| `services/` | Rust workspace: daemon, bridge, protocol, observability, overlay |
| `docs/` | Architecture docs (backend-architecture, kiosk-session, observability, etc.) |
| `deploy/` | systemd units, packaging |
| `scripts/` | Build/dev helper scripts |
| `test/` | Flutter tests |
| `justfile` | All dev commands — start here |

## Key services (under `services/`)

| Crate | Role |
|-------|------|
| `hearthdeck-daemon` | HTTP/WebSocket API, SQLite state, discovery, pairing. `Type=notify` systemd unit. |
| `hearthdeck-bridge` | Linux-only desktop-entry discovery + allowlisted launches. Socket-activated. |
| `hearthdeck-protocol` | Shared types between daemon and client. |
| `hearthdeck-observability` | Shared tracing/metrics setup. |
| `hearthdeck-overlay` | COSMIC layer-shell overlay surface (Rust/libcosmic). |

## Key Flutter entry points (under `lib/`)

| File | Role |
|------|------|
| `main.dart` | App entry point |
| `tv_components.dart` | Shared TV-UI primitives (`TvFocusable`, `TvTwoPaneLayout`, etc.) |
| `full_library.dart` | Main catalog surface |
| `settings/` | Settings screens |
| `backend/` | Backend API client |
| `catalog/` | Catalog models and repository |

## Dev commands (all via `just`)

```
just setup            # install toolchains + Flutter deps
just app              # run Flutter client
just app-live ...     # run with live backend (needs BACKEND_URL + PAIRING_TOKEN)
just dev              # Flutter + backend together
just check            # format + all tests
just check-services   # Rust check/test/clippy only
just test-app         # Flutter analyze + tests
just build-services   # release Rust build
just install-services # install systemd units for local testing
just services-status  # check running services
just logs-daemon      # daemon journal
just logs-bridge      # bridge journal
just logs-errors      # error-only journal
```

## Architecture invariants

- Navigation is controller-first; Back is a global contract — no per-screen Escape handlers.
- Reuse `Tv*` widgets before adding new UI machinery.
- Platform-specific behavior stays behind adapters; shared API/catalog must not branch on OS.
- Launches always go through transient systemd user units (never raw shell from daemon).
- Discovery providers are independent; one failure must not clobber another source.
- LAN access is opt-in (`HEARTHDECK_LAN_ENABLED=true` + TLS env vars).

## Sessions / deployment

- **Kiosk**: Gamescope on DRM/KMS, Hearthdeck as sole child.
- **COSMIC (Test)**: cosmic-comp + Hearthdeck overlay.
- `hearthdeck.target` = systemd user root; owns daemon + bridge socket.

## Validation rule

- Frontend change → `just test-app`
- Backend change → `just check-services`
- Cross-cutting → `just check`
