# Response style defaults

- Keep token spend lean: do the deliberation internally before writing anything, and never spill exploratory thinking, restated tool output, or near-duplicate drafts into the response or the visible reasoning.
- Be short and sharp by default.
- Answer directly. Skip filler, repetition, and optional explanation unless the user asks for it.
- Prefer the shortest correct reply that still includes the necessary action or result.
- Keep working notes and reasoning minimal; do not spill internal deliberation into the response.
- For coding tasks, return code first, then at most a few short lines of essential context.

---

# Change summaries teach the code and the codebase

Applies to the final summary of every bug fix or feature. Overrides the short-by-default style above for this output only.

The reader is a senior engineer who is learning Rust — and does not know this codebase well, since most of it was agent-written. A summary is not a changelog ("implemented X, fixed Y"). It is a small lesson that helps them understand the change and, gradually, the system around it. Write so someone who never saw the code can follow:

- **Place the change first**: name the file/module touched and explain its job in the system in one or two plain sentences. Do not assume they already know what that module does.
- **Explain the mechanism**: walk through what the code now does and why that fixes the issue or adds the feature, tied concretely to the diff.
- **Teach the Rust it uses**: when a construct does real work in this change, explain it briefly — what it does and why it is used here (e.g. `?` to propagate errors up, an enum + `match` so the compiler checks every case, why a value is borrowed or cloned). Only what this diff actually uses; no Rust 101 and no term-dumping.
- **Say why this design**: one or two lines on the realistic alternative and the accepted tradeoff.
- **Grow their map slowly**: each summary is one step toward understanding the codebase; link back to things explained in earlier changes when it helps. Do not recap the whole architecture.
- **Stay short**: one-pass read, depth where the change is subtle, silence where it is obvious.

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
