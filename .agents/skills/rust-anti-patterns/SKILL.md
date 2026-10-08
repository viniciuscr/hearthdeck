---
name: rust-anti-patterns
description: "Catalogue of Rust anti-patterns and the self-review checklist for a Hearthdeck diff. Load this when reviewing code, refactoring, cleaning up, or as the final pass before reporting work done. Activates on code review, refactor, cleanup, 'this is messy', 'review my changes', technical debt, god object, duplicated code, dead code, commented-out code, TODO, clippy allow, scope creep, and before writing any change summary."
---

# Rust anti-patterns and diff self-review

## CRITICAL RULE: review the diff, not your memory of writing it

Before reporting done, run `git diff` and read it top to bottom as if a colleague wrote it. You
will find things your memory skipped — a leftover `dbg!`, a `#[allow]`, a debug print, a
half-finished branch, a file you meant to delete.

This is not optional, and it is not the same as "I know what I changed". The purpose is to catch
what you did not intend.

## When to use this skill

- Any code review, including reviewing your own change.
- Any refactor or "clean this up" request.
- The final pass before writing a summary.
- When something feels messy but you cannot name why.

## The self-review pass

Answer these against the diff. Each one has caught real bugs in this codebase's history.

1. **Scope** — does every hunk serve the request? Name the request each hunk serves.
2. **Errors** — is every new `unwrap`/`expect` on a path that cannot fail? Is every discarded
   `Result` deliberately discarded and said so?
3. **Ownership** — did any `clone()` appear to quiet the borrow checker?
4. **Types** — any new boolean parameter, sentinel value, or `String`-typed identifier?
5. **Effects** — does anything block in async, or hold a lock across `.await`?
6. **Duplication** — does a helper already exist for this?
7. **Tests** — would a test fail without this change? Is any new error path untested?
8. **Leftovers** — `dbg!`, `println!`, commented-out code, scratch files, stray `TODO`s?
9. **Lints** — is every new `#[allow]` justified in a comment?
10. **Summary** — does the write-up teach the change, or just list it?

## Category 1: error handling

```rust
// WRONG - panics on network input.
let title: Title = serde_json::from_str(&body).unwrap();

// WRONG - says "should never happen", which is a claim, not a proof.
let id = find_id(&catalog).expect("id should exist");

// WRONG - the failure is swallowed; the caller sees success.
let _ = tx.send(event).await;

// CORRECT
let title: Title = serde_json::from_str(&body)
    .map_err(|e| ApiError::MalformedBody(e.to_string()))?;
```

Also:

- **Stringly-typed errors.** `anyhow!(\"bad request\")` where a typed variant belongs forces callers
  to match on message text. See `rust-hygiene`.
- **Catching and re-throwing with less information.** `map_err(|_| MyError::Failed)` throws away
  the cause. Keep it: `map_err(|e| MyError::Failed { source: e.into() })`.
- **`?` in a function that then returns `Ok(())` unconditionally elsewhere** with no way to tell the
  failure was handled. Be explicit about which failures are fatal.

## Category 2: ownership and cloning

```rust
// WRONG - clones the caller's data to read one field.
fn is_visible(&self, catalog: &Catalog) -> bool { catalog.clone().titles[0].visible }

// WRONG - `to_string()` in a loop that could borrow.
for t in &titles { names.push(t.title.to_string()); }

// CORRECT - borrow.
fn is_visible(&self, catalog: &Catalog) -> bool { catalog.titles[0].visible }
for t in &titles { names.push(t.title.as_str()); }
```

- **`clone()` in `view`.** Cloning catalog state to satisfy the `Element<'a>` lifetime is the most
  common version of this in this repo. Borrow instead; see `iced-cosmic-ui`.
- **`&String` / `&Vec<T>` / `&PathBuf` parameters.** See the parameter table in `rust-hygiene`.
- **`.iter().cloned().collect()` when a slice or iterator would do.**
- **`Arc` used to silence a borrow error.** `Arc` is for shared ownership across tasks, not for
  making the compiler stop complaining about a borrow you could restructure.

## Category 3: abstraction and type design

```rust
// WRONG - `launch(title, true)`; the call site is unreadable and a swap compiles.
fn launch(&self, title: &Title, foreground: bool) { }

// WRONG - "no value" encoded as an empty string; the compiler cannot help.
struct Device { address: String }   // "" means paired but no address?

// WRONG - a god struct that everything depends on, so everything couples.
struct AppState { pool: Pool, client: Client, config: Config, ui: Ui, cache: Cache, … }
```

- **Speculative generality.** A trait with one implementation, a generic over a type only ever
  instantiated once, a config field nobody sets. Add the abstraction when the second case arrives.
- **God object.** If a struct has 30 fields and 40 methods, it wants splitting by responsibility.
- **`Option<Option<T>>`**, `Option<bool>`, and `Result<Option<T>>` — three states are almost always
  an enum.
- **Deriving everything.** `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]` on a struct
  that needs none of it is noise and, for `Clone`, an invitation to clone sloppily.

## Category 4: async

```rust
// WRONG - blocks the executor thread.
std::thread::sleep(Duration::from_millis(50));

// WRONG - the guard is held across an await.
let guard = self.state.lock().unwrap();
let data = self.client.fetch().await?;
guard.push(data);

// WRONG - the task's error is dropped on the floor.
tokio::spawn(async move { sync_all().await });

// CORRECT - spawn and keep or log the outcome.
tokio::spawn(async move {
    if let Err(err) = sync_all().await {
        tracing::error!(error = %err, "background sync failed");
    }
});
```

- **Unbounded channels** between tasks: a slow consumer turns into unbounded memory. Use a bounded
  channel and decide the backpressure policy.
- **`tokio::spawn` where `Task::perform` belongs** in the frontend — it detaches the result from the
  message loop.
- **Awaiting sequentially what could be concurrent** (`join!`/`try_join!`) when the calls are
  independent — but only when they are genuinely independent; do not parallelize a sequence that
  depends on order.

## Category 5: modules and structure

- **`util.rs` / `helpers.rs` dumping grounds.** Unrelated helpers that share only their vagueness.
  Move each next to its caller.
- **`pub` on everything.** `pub` is a cross-crate promise. Default to `pub(crate)`.
- **A module that imports from a sibling's internals.** That is a missing public API on the sibling.
- **Growing a large file.** `app.rs` is already ~7,500 lines. New screens go in their own module;
  new logic goes in a testable function, not a new arm in `update`.
- **Feature-gated code with no non-default-feature coverage.** See `testing-and-validation`.

## Category 6: comments and dead code

```rust
// WRONG - restates the code.
// Increment the counter.
counter += 1;

// CORRECT - explains the non-obvious constraint.
// systemd's Type=notify requires readiness within 90s or the unit is killed.
counter += 1;

// WRONG - dead code kept "in case".
// let old_path = resolve_legacy(&config);
```

- **Commented-out code.** Delete it. Version control remembers.
- **`TODO` with no owner or trigger.** Either it is a real task with a reference, or it is noise.
- **A comment that contradicts the code.** Worse than no comment; it actively misleads.

## Category 7: development hygiene

These are the ones that survive into a commit most often.

- `dbg!(…)` and `println!` used for debugging — in service code always; in the frontend too, since it
  goes nowhere useful in a kiosk session.
- Scratch scripts, fixtures, or generated files left in the tree. Write probes under `$TMPDIR`.
- A `#[allow(...)]` added to get past a lint instead of fixing it.
- An accidental `.unwrap()` added while exploring and never removed.
- A stray dependency added to a `Cargo.toml` for a one-off experiment.

## Category 8: tests

Cross-check against `testing-and-validation`. The review-level questions:

- Does a test actually fail without the change?
- Does every new error path have a test that reaches it?
- Is the test deterministic and isolated (`tempdir`, no `$HOME`, no real socket, no `sleep` for
  synchronization)?
- Does the test name describe behavior, or just restate the function name?
- Is the test asserting an independently-derived expectation, rather than re-running the
  implementation to compute the expected value?

## Category 9: scope creep

The most common way a well-intentioned change becomes a bad one.

- Renaming or reformatting code you did not need to touch — it buries the real diff.
- Fixing an unrelated bug found along the way. Mention it; do not fix it in this change.
- Adding a feature flag, config option, or abstraction "for later". Later can ask.
- Refactoring a file while making a behavior change. Separate the two changes.

## Triage: fix now vs. mention

| Finding | Action |
|---|---|
| Correctness bug, panic path, security issue | **Fix now.** |
| Broken invariant from `AGENTS.md` | **Fix now.** |
| Clippy warning | **Fix now.** It fails CI. |
| Untested new behavior | **Fix now.** |
| Leftover debug output or scratch file | **Fix now.** |
| Pre-existing ugly code nearby | **Mention.** Do not expand the diff. |
| Unrelated bug you noticed | **Mention.** File it. |
| Style preference with no project rule behind it | **Skip.** |

## Reporting

Findings, not vibes. Cite the real `file:line` from the diff you just read, say what is wrong, and
say what the fix is. The shape:

```
- <path>:<line> — what is wrong, and the concrete consequence.
- <path>:<line> — what is wrong, and the concrete fix.
```

Two rules about that list:

- **Cite lines you actually read, out of the diff in front of you.** Never reconstruct a path or a
  line number from memory — a guessed citation sends the reader to the wrong place and discredits
  the rest of the review.
- If you reviewed and found nothing, say that plainly rather than padding the report.
