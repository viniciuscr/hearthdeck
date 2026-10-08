---
name: testing-and-validation
description: "How to prove a change works in Hearthdeck: the regression-test rule, choosing the seam to test, test placement and naming, deterministic async tests, testing axum routes with tower, and the exact command that validates each crate. Load this whenever you write or change tests, fix a bug, or are about to claim work is finished or passing. Activates on cargo test, just check, just check-fast, just check-services, just test-frontend, #[test], #[tokio::test], tempfile, tower oneshot, assertion, regression, flaky test, and 'is this done'."
---

# Testing and validation

## CRITICAL RULE 1: no behavior change without a test that fails without it

For a bug fix, the order is fixed:

1. Write a test that reproduces the bug.
2. **Run it and watch it fail.** If it passes before your fix, it is not testing the bug.
3. Fix the bug.
4. Run it again and watch it pass.

For a feature, the test asserts the new behavior and would fail if the behavior were absent or
wrong. "It exercises the new code path" is not sufficient — coverage is not correctness.

This is a standing project rule, not a preference: every behavior change ships with automated
coverage in the same change.

## CRITICAL RULE 2: an unrun command is not evidence

A verification command must be run **and its output read** in the same session as the change. Do
not reason about what the tests would do. If you did not run it, say so explicitly rather than
implying success.

Watch for these, which look like passing:

- A test that is `#[ignore]`d or behind a `#[cfg(feature = …)]` that is off.
- A filter that matched zero tests — cargo prints `running 0 tests` and exits 0.
- A stale binary. `cargo test` rebuilds, but a hand-run `./target/debug/…` does not.

## The validation matrix

| Change touches | Command |
|---|---|
| One crate | `just check-fast <crate>` |
| More than one crate, or unsure | `just check` |
| Before pushing | `just pre-push-check` |

`just check-fast <crate>` runs `cargo fmt --all`, then `cargo clippy -p <crate> --all-targets -- -D
warnings`, then `cargo test -p <crate>`. It mirrors the three gates CI enforces, for one crate, so
it is the loop to run while iterating. `just check` is the full suite.

Narrow further while iterating, but always finish with the crate-level command:

```
cargo test --manifest-path services/Cargo.toml -p hearthdeck-daemon <test_name_substring>
```

**Clippy runs with `-D warnings` in CI, including `--all-targets`.** That means test code is linted
too — an unused import in a `#[cfg(test)] mod tests` fails the build. Run clippy, not just
`cargo check`.

## Choose the seam before writing the test

The hard part is not the assertion, it is deciding what to call. Ranked best to worst:

1. **A pure function.** Filtering, sorting, parsing, formatting, deriving a rail's contents,
   computing which input source owns the gamepad. Cheapest to test, no setup.
2. **A serializer or wire contract.** These are the project's real API surface and are already
   tested this way — e.g. `response_round_trips`,
   `retro_launch_serialization_carries_only_resolved_paths`,
   `shared_api_payloads_keep_their_wire_names`.
3. **An HTTP route.** Test the router, not the network (see below).
4. **A state machine.** Feed a sequence of events, assert the state and the emitted effects. This
   is the right seam for navigation and menu behavior — e.g.
   `toggle_opens_and_closes_the_menu`, `navigation_wraps_both_menu_items`.
5. **The whole process.** Only when the contract *is* the command line; `hearthdeck-ai/tests/cli.rs`
   is the model.

If the only testable seam is a 1,200-line `update` method, the code is wrong, not the test. Extract
the decision into a function and test that. That extraction is usually the whole fix.

## Placement

- **Unit tests**: `#[cfg(test)] mod tests { … }` at the bottom of the module under test. This is
  the dominant pattern here (about 50 modules) and it gives the test access to private items.
- **Integration tests**: `tests/<name>.rs`, driving the crate's public API or its real binary. There
  is currently one — `hearthdeck-ai/tests/cli.rs` — and it is worth reading before writing another:
  it spawns the binary and asserts on the JSON written to `--output`.
- Do not create a `tests/` file that only re-tests what a unit test already covers.

## Naming

Test names are behavior sentences. The existing suite is a good model:

```rust
#[test] fn navigation_wraps_both_menu_items() { … }
#[test] fn opening_resets_selection_and_failure_state() { … }
#[test] fn romm_data_root_follows_the_compose_file() { … }
```

```rust
// WRONG - names the function under test, not the behavior; tells a failure reader nothing.
#[test] fn test_parse_config() { … }
#[test] fn parse_config_works() { … }

// CORRECT - the name is the specification, and a failure names the broken rule.
#[test] fn config_without_api_port_falls_back_to_the_default() { … }
```

## Table-driven tests — no new dependency

There is no `rstest`/`proptest`/`insta` in this workspace; the dev-dependencies are `tempfile`,
`tower`, and `tokio`. Do not add a test framework to avoid writing a loop. Use a table:

```rust
#[test]
fn provider_ids_are_normalized() {
    let cases: &[(&str, &str)] = &[
        ("Steam", "steam"),
        ("  ROM_MANAGER ", "rom_manager"),
        ("retroarch", "retroarch"),
    ];
    for (input, expected) in cases {
        assert_eq!(normalize_provider_id(input), *expected, "input: {input:?}");
    }
}
```

Always include the input in the assertion message. A bare `assert_eq!` failure in a loop over ten
cases does not tell you which one broke.

Adding a dev-dependency is a decision worth raising with the user, not a default.

## Async tests

- `#[tokio::test]` for async. 52 tests use it.
- **Never `sleep` to wait for something to happen.** It is slow and flaky. Await the actual signal —
  a channel receive, a `JoinHandle`, a state that the code under test sets — with a timeout if the
  test could hang: `tokio::time::timeout(Duration::from_secs(5), rx.recv())`.
- **Never open a real network connection.** Bind `127.0.0.1:0` and read the assigned port if a
  socket is genuinely needed.
- Do not use `#[tokio::test(flavor = "multi_thread")]` for correctness; if the test needs real
  concurrency to pass, it is probably racy.

## Testing axum routes

The daemon tests routes without a server, using `tower::ServiceExt::oneshot` — see the pattern in
`services/hearthdeck-daemon/src/api.rs`:

```rust
use tower::ServiceExt;

let response = router(state.clone())
    .oneshot(Request::get("/v1/settings").body(Body::empty()).unwrap())
    .await
    .unwrap();
assert_eq!(response.status(), StatusCode::OK);
```

Cover the authorization boundary explicitly: an unauthenticated request must be asserted to fail,
not left implicit. A route test that only checks the happy path does not protect the invariant.

## Determinism and isolation

- **Use `tempfile::tempdir()`** for anything filesystem-shaped. It is already the pattern in the AI,
  bridge, and daemon tests. Never let a test touch `$HOME`, `~/.config/hearthdeck`, or a real
  systemd user directory.
- **Never assert on the real clock.** Inject the time or assert on relative ordering.
- **Never depend on the developer's machine state** — installed desktop entries, a running daemon,
  a compositor, a gamepad. If a test needs one, it is an acceptance test, not a unit test.
- Tests must pass in any order and in parallel. No shared mutable statics, no fixed port numbers, no
  fixed temp paths.

## Platform and OS-facing code

The project convention is to **split testable state and parsing away from the OS integration**, then
cover that logic plus its package/service wiring where a compositor or device cannot run in CI.

A launch that ultimately shells into a systemd unit should be structured as: parse and validate the
request (pure, tested) → resolve paths (tested) → hand the resolved, typed values to the unit
launcher (thin, not unit-tested). `retro_launch_serialization_carries_only_resolved_paths` is
exactly this idea: the test asserts on the resolved payload, not on the spawn.

## Feature flags

`hearthdeck-ai/tests/cli.rs` deliberately stays on `--engine heuristic` so the suite also runs under
`--no-default-features`, where no `laya` feature (and no model checkpoint) exists. Follow that: if
you add a test for feature-gated code, make sure the suite still builds and passes with the feature
off, and do not write a test that silently becomes a no-op when it is.

## Anti-patterns

```rust
// WRONG - passes for the wrong reason; a failure prints nothing useful.
assert!(result.is_ok());
assert_eq!(result.is_err(), false);

// CORRECT - assert the value, with context on failure.
let report = result.expect("scan should succeed on a well-formed library");
assert_eq!(report.sections.len(), 3, "unexpected section count: {report:?}");
```

```rust
// WRONG - restates the implementation, so it passes even when both are wrong.
assert_eq!(visible_titles(&catalog).len(), catalog.iter().filter(|t| t.is_visible()).count());

// CORRECT - states the expected outcome independently.
let titles = visible_titles(&catalog);
assert_eq!(titles.iter().map(|t| t.id).collect::<Vec<_>>(), vec![TitleId(2), TitleId(3)]);
```

Also avoid: asserting only that a call does not panic; a test with no assertions; snapshotting a
huge log; a test that needs the compositor or a gamepad; `#[ignore]` without a comment saying when
it runs.

## Before you report done

- [ ] A test fails without the change and passes with it (for behavior changes).
- [ ] Every new error path has a test that reaches it.
- [ ] Test names describe behavior.
- [ ] Tests are deterministic, isolated (`tempdir`), and order-independent.
- [ ] No `sleep` used as synchronization; no real network.
- [ ] `just check-fast <crate>` is green — clippy included.
- [ ] You ran it in this session and read the output.
