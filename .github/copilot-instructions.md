# Response style defaults

- Keep token spend lean: do the deliberation internally before writing anything, and never spill exploratory thinking, restated tool output, or near-duplicate drafts into the response or the visible reasoning.
- Be short and sharp by default.
- Answer directly. Skip filler, repetition, and optional explanation unless the user asks for it.
- Prefer the shortest correct reply that still includes the necessary action or result.
- Keep working notes and reasoning minimal; do not spill internal deliberation into the response.
- For coding tasks, return code first, then at most a few short lines of essential context.

---

# The final report is a fixed five-line block

Every change ends with the same block, in the same order, with these labels:

```
Done:     what changed, and where. One line.
Watch out: the one thing a reader of the diff would get wrong. Drop it if there is none.
Checked:  the command you ran, and its result.
Next:     what comes next. Drop it if nothing does.
```

Nothing outside those labels. No headings, no tables, no bullet lists, no summary
paragraph before it, no sign-off after it. When the task failed or was left
unfinished, the same block says so — never dress a failure up as progress.

The full rules, with a WRONG/CORRECT pair, are in
`.agents/skills/reporting/SKILL.md`. Load it before writing the report.

---

# Project map — Hearthdeck

Rust monorepo. Linux TV/kiosk game library frontend built on COSMIC (iced/libcosmic).

## Top-level layout

| Path | What lives there |
|------|-----------------|
| `services/` | Rust workspace: frontend, daemon, bridge, protocol, observability, overlay, input |
| `docs/` | Architecture docs (backend-architecture, kiosk-session, observability, etc.) |
| `deploy/` | systemd units |
| `packaging/` | Arch Linux package build |
| `scripts/` | Build/dev helper scripts |
| `contracts/` | OpenAPI contract |
| `justfile` | All dev commands — start here |

## Key services (under `services/`)

| Crate | Role |
|-------|------|
| `hearthdeck-frontend` | COSMIC (iced/libcosmic) TV UI; talks to the daemon over the paired API. |
| `hearthdeck-daemon` | HTTP/WebSocket API, SQLite state, discovery, pairing. `Type=notify` systemd unit. |
| `hearthdeck-bridge` | Linux-only desktop-entry discovery + allowlisted launches. Socket-activated. |
| `hearthdeck-protocol` | Shared types between daemon and client. |
| `hearthdeck-observability` | Shared tracing/metrics setup. |
| `hearthdeck-overlay` | COSMIC layer-shell overlay surface. |
| `hearthdeck-input` | Gamepad/input broker. |

## Key frontend entry points (under `services/hearthdeck-frontend/src/`)

| File | Role |
|------|------|
| `main.rs` | App entry point |
| `app.rs` | Application state, navigation, views |
| `app_group.rs` | Sections, groups, catalog filtering |
| `style.rs` | Design tokens and widget styles |
| `widgets/` | Reusable widgets (application tile, transitions) |
| `providers/` | Daemon client and record models |
| `subscriptions/` | Gamepad and event subscriptions |

## Dev commands (all via `just`)

```
just setup            # install toolchains
just dev              # frontend + backend together
just run-frontend     # run the COSMIC frontend
just check            # format + all checks/tests
just check-services   # Rust backend check/test/clippy
just check-frontend   # frontend clippy
just test-frontend    # frontend tests
just build-services   # release Rust build
just install-services # install systemd units for local testing
just services-status  # check running services
just logs-daemon      # daemon journal
just logs-bridge      # bridge journal
just logs-errors      # error-only journal
```

## Architecture invariants

- Navigation is controller-first; Back is a global contract — no per-screen Escape handlers.
- Reuse the shared widgets and layout helpers before adding new UI machinery.
- Platform-specific behavior stays behind adapters; shared API/catalog must not branch on OS.
- Launches always go through transient systemd user units (never raw shell from daemon).
- Discovery providers are independent; one failure must not clobber another source.
- LAN access is opt-in (`HEARTHDECK_LAN_ENABLED=true` + TLS env vars).

## Sessions / deployment

- **Hearthdeck session**: cosmic-comp + Hearthdeck frontend fullscreen; greetd autologin by default.
- `hearthdeck.target` = systemd user root; owns daemon + bridge socket.

## Validation rule

- Frontend change → `just test-frontend` (lint with `just check-frontend`)
- Backend change → `just check-services`
- Cross-cutting → `just check`
