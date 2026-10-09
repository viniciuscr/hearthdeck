# AGENTS.md — working agreement for Hearthdeck

This file is the entry point for any agent (or human) changing this repo. It is short on
purpose. It tells you which skill to load, what "done" means, and which command proves it.

Read this first, load the skill that matches your change, then work.

## Precedence

1. This file.
2. `.agents/skills/*/SKILL.md` — the binding rules for the kind of change you are making.
3. `docs/` — architecture background. Explains *why*; not a checklist.
4. Code comments.

On conflict, the lower number wins. If a skill and this file disagree, this file is right and
the skill should be fixed.

## The one rule

**No change is done until the command that proves it has run and passed, in this session.**

"Should work", "compiles in my head", and "the logic is obviously correct" are not evidence.
If you did not run it, say you did not run it.

## The loop

Work in this order. Skipping a step is what produces sloppy code.

**1. Recon.** Read the module you are about to change *and* its tests before editing anything.
Find the existing helper that already does part of the job. Do not guess at an API — open the file.

**2. Plan.** Name the files you will touch and the smallest change that satisfies the request.
If the plan needs more than a handful of files, say so before starting.

**3. Change.** Follow the skills routed below. Match the surrounding code's style, error
handling, and naming; this codebase is consistent and a new pattern stands out.

**4. Verify.** Run the narrow loop (`just check-fast <crate>`) while iterating, then the
validation-matrix row for what you touched. Fix what fails; never silence it.

**5. Report.** The final summary is a short lesson, not a changelog. Name the file and its job,
explain the mechanism, teach the Rust constructs the diff actually uses, and give the tradeoff.
See `.github/copilot-instructions.md` for the full shape of that summary.

## Skill router

Load the skill when the trigger column matches. Load more than one when more than one matches —
that is normal, and cheap.

| Trigger | Skill | Why |
|---|---|---|
| Any Rust code: types, errors, ownership, async, logging, modules | `rust-hygiene` | Language-level rules this repo enforces |
| Anything under `hearthdeck-frontend`: state, messages, views, widgets, theme, gamepad | `iced-cosmic-ui` | COSMIC/iced MVU and theming rules |
| Writing or changing tests, deciding if something is finished, picking a validation command | `testing-and-validation` | What to test, and the command that proves it |
| Reviewing a diff, refactoring, "clean this up", self-check before reporting | `rust-anti-patterns` | WRONG/CORRECT catalog + review checklist |
| Writing new Rust from scratch, or reviewing agent-generated code | `rust-llm-pitfalls` | Habits the default lint set does not catch |
| Backend services: daemon, bridge, discovery, launches | `rust-hygiene` + `testing-and-validation` | Plus the invariants below |

Two pre-existing skills are also available and are *not* superseded by the above:

- `cosmic-lib` — general libcosmic/COSMIC background. Useful when you need to look up how a
  COSMIC concept works. `iced-cosmic-ui` states this repo's binding rules and wins on conflict.
- `token-saver` — low-token execution mode, when the user explicitly asks for brevity.
  It does **not** relax the verification rule above.

## Definition of done

Every item, every time. An unchecked box is not done.

- [ ] The change does what was asked — and nothing beyond it.
- [ ] A test exists that fails without the change (for any behavior change or bug fix).
- [ ] `just check-fast <crate>` is green for each crate touched.
- [ ] The validation-matrix row below is green.
- [ ] No new clippy warnings. CI runs `-D warnings`, so a warning is a build failure.
- [ ] No leftover scratch files, `dbg!`, or `println!` debugging.
- [ ] `git diff` re-read end to end: only intended changes, no leftovers.
- [ ] The summary teaches the change rather than restating the diff.

## Validation matrix

Pick the row for what you touched. When unsure, take the wider option — it is only slower.

| Change touches | Command |
|---|---|
| `hearthdeck-frontend` | `just check-fast hearthdeck-frontend` |
| `hearthdeck-daemon` | `just check-fast hearthdeck-daemon` |
| `hearthdeck-bridge` | `just check-fast hearthdeck-bridge` |
| `hearthdeck-ai` | `just check-fast hearthdeck-ai` |
| `hearthdeck-protocol` | `just check-fast hearthdeck-protocol` |
| `hearthdeck-input` / `hearthdeck-overlay` / `hearthdeck-observability` | `just check-fast <crate>` |
| More than one crate, or you are unsure | `just check` |
| Before pushing | `just pre-push-check` |

`just check` is the full portable suite: format, then backend check/test/clippy, then frontend
clippy and tests. `just pre-push-check` runs those same gates in **debug**, on purpose: compiling the
~1000-crate dependency tree in release is what made the gate take ~19 minutes. CI runs the release
build and the release clippy on the pinned toolchain, so release-only lints are caught there.

### Optional: strict linting

`just lint-strict [crate]` raises clippy to `pedantic` + `nursery` and prints a ranked summary of
what it found. It is **not a gate** — deliberately kept out of CI and pre-push, because those lints
are opinionated and many findings are style rather than defects. Reach for it when writing a new
module or reviewing generated code, to see the habits the default set does not catch. See
`.agents/skills/rust-llm-pitfalls`.

## Non-negotiables

These are product contracts, not style preferences. Do not work around one without asking.

- **Navigation is controller-first.** Back is a single global contract. Never add a per-screen
  Escape or Back handler that bypasses the global one.
- **Reuse before you build.** Shared widgets and layout helpers come first; adding new UI
  machinery needs a reason.
- **Platform behavior stays behind adapters.** Shared API/catalog logic must never branch on the
  host OS.
- **Launches go through transient systemd user units.** Never a raw shell command from the daemon.
- **Discovery providers are independent.** One provider failing must not clobber another source.
- **LAN access is opt-in** via `HEARTHDECK_LAN_ENABLED=true` plus TLS cert/key env vars. Pairing
  code creation stays on the loopback admin port `127.0.0.1:38401`.
- **`tracing` only** in services — structured fields, never `println!`/`eprintln!`.
- **systemd startup paths stay explicit and idempotent.**

## What sloppy looks like

This framework exists to stop these specific failure modes. If you catch yourself doing one, stop.

- Reporting success without running the command.
- `unwrap()` / `expect()` on any path reachable from network input, filesystem, or user config.
- Cloning to satisfy the borrow checker instead of fixing the borrow.
- Hardcoded colors or sizes in the UI instead of theme tokens.
- A test that asserts nothing, or that restates the implementation instead of the behavior.
- Writing a new helper when one already exists.
- A blanket `#[allow(...)]` to make clippy quiet.
- Widening scope past the request "while I'm here".
- Leaving probe scripts, `dbg!`, or commented-out code in the tree.

## Hygiene

- **Format:** `just format` (rustfmt defaults). There is no `rustfmt.toml`; do not add one.
- **Commits:** one short imperative subject line saying what changed. No git-log archaeology to
  match a house style.
- **Scratch work:** write probes and generated output under `$TMPDIR`, never in the repo. Clean up
  anything you did leave.
- **`#[allow]`:** allowed only with an adjacent comment saying why the lint is wrong here.

## When to stop and ask

Ask before proceeding — do not pick silently — when:

- The change alters a public API, the OpenAPI contract in `contracts/`, or a data model.
- The change touches any non-negotiable above.
- Two reasonable designs differ in a way the user would care about.
- The request conflicts with a documented invariant.
- You have no way to verify the change and the user has not accepted that risk.
