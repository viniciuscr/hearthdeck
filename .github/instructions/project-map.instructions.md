---
description: Project layout, key entry points, dev commands, and architecture invariants for Hearthdeck. Read this before exploring the codebase.
---

# Hearthdeck — Project Map

Rust monorepo. Linux TV/kiosk game library frontend built on COSMIC (iced/libcosmic).

## Layout

| Path | What lives there |
|------|-----------------|
| `services/` | Rust workspace: frontend, daemon, bridge, protocol, observability, overlay, input |
| `docs/` | Architecture docs |
| `deploy/` | systemd units |
| `packaging/` | Arch Linux package build |
| `scripts/` | Build/dev helpers |
| `contracts/` | OpenAPI contract |
| `justfile` | All dev commands — start here |

## Rust crates (`services/`)

| Crate | Role |
|-------|------|
| `hearthdeck-frontend` | COSMIC (iced/libcosmic) TV UI; talks to the daemon over the paired API |
| `hearthdeck-daemon` | HTTP/WebSocket API, SQLite state, discovery, pairing |
| `hearthdeck-bridge` | Linux-only desktop-entry discovery + allowlisted launches (socket-activated) |
| `hearthdeck-protocol` | Shared types |
| `hearthdeck-observability` | Shared tracing/metrics |
| `hearthdeck-overlay` | COSMIC layer-shell overlay |
| `hearthdeck-input` | Gamepad/input broker |

## Frontend entry points (`services/hearthdeck-frontend/src/`)

| File | Role |
|------|------|
| `main.rs` | App entry point |
| `app.rs` | Application state, navigation, views |
| `app_group.rs` | Sections, groups, catalog filtering |
| `style.rs` | Design tokens and widget styles |
| `widgets/` | Reusable widgets (application tile, transitions) |
| `providers/` | Daemon client and record models |
| `subscriptions/` | Gamepad and event subscriptions |

## Dev commands

```
just setup            # install toolchains
just dev              # frontend + backend together
just run-frontend     # COSMIC frontend
just check            # format + all checks/tests
just check-services   # Rust backend check/test/clippy
just check-frontend   # frontend clippy
just test-frontend    # frontend tests
just build-services   # release Rust build
just install-services # install systemd units for local testing
just services-status / just logs-daemon / just logs-bridge / just logs-errors
```

## Architecture invariants

- Controller-first navigation; Back is a global contract — no per-screen Escape handlers.
- Reuse the shared widgets and layout helpers before adding new UI machinery.
- Platform-specific behavior behind adapters; shared API/catalog must not branch on OS.
- Launches always via transient systemd user units — never raw shell from daemon.
- Discovery providers are independent; one failure must not clobber another source.
- LAN access is opt-in: `HEARTHDECK_LAN_ENABLED=true` + TLS env vars.

## Validation

- Frontend only → `just test-frontend` (lint with `just check-frontend`)
- Backend only → `just check-services`
- Cross-cutting → `just check`
