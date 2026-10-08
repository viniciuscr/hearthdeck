---
name: rust-hygiene
description: "Rust language rules for the Hearthdeck workspace: error handling, unwrap/expect policy, ownership and cloning, type design, async, tracing, module visibility, and clippy discipline. Load this for ANY change to Rust code under services/, including writing a new function, refactoring, fixing a bug, or reviewing a diff. Activates on error enums, thiserror, anyhow, the ? operator, Result/Option, clone/borrow complaints, tokio and async fn, Mutex/RwLock, tokio::spawn, tracing, println, unsafe, pub/pub(crate), edition 2024, and clippy warnings."
---

# Rust hygiene

## CRITICAL RULE: a clippy warning is a build failure

CI runs:

```
cargo clippy --manifest-path services/Cargo.toml --workspace --all-targets --release -- -D warnings
```

Clippy runs with `-D warnings` — every warning is an **error**. There is no "warning budget" and no
"it still compiles". Warnings that only appear in `--all-targets` (unused test imports) or only in
`--release` will still fail CI, so run clippy, not just `cargo check`.

Never silence a lint with a blanket `#[allow(...)]`. An `#[allow]` is acceptable only with an
adjacent comment explaining why the lint is wrong here:

```rust
// WRONG - hides the finding and tells the next reader nothing.
#[allow(clippy::too_many_arguments)]
fn launch(a: A, b: B, c: C, d: D, e: E, f: F, g: G) { }

// CORRECT - either fix it, or justify it.
// These seven fields mirror the systemd unit schema one-for-one; splitting them
// would only re-group the same data at every call site.
#[allow(clippy::too_many_arguments)]
fn launch(a: A, b: B, c: C, d: D, e: E, f: F, g: G) { }
```

The workspace is **edition 2024**, pinned to Rust 1.97.1 via `mise.toml`.

## When to use this skill

- Any edit under `services/`.
- Designing types, error paths, or async flows.
- You are about to write `.unwrap()`, `.clone()`, or `#[allow]`.
- Clippy reports something and you are unsure whether to fix or suppress.

## Errors

**`thiserror` for anything with callers. `anyhow` only at a binary's outer boundary.** The
workspace has both. A library or service module that returns `anyhow::Error` erases the
distinction between "the user asked for a title that does not exist" and "SQLite is corrupt", and
its caller can no longer match on it.

```rust
// WRONG - the caller cannot tell a missing row from a dead database.
pub async fn title(&self, id: TitleId) -> anyhow::Result<Title> {
    let row = sqlx::query_as!(...).fetch_one(&self.pool).await?;
    Ok(row)
}

// CORRECT - a typed error, so callers can react to the cases that matter.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("no title with id {0}")]
    NotFound(TitleId),
    #[error("catalog database unavailable")]
    Database(#[from] sqlx::Error),
}

pub async fn title(&self, id: TitleId) -> Result<Title, CatalogError> {
    let row = sqlx::query_as!(...)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => CatalogError::NotFound(id),
            other => CatalogError::Database(other),
        })?;
    Ok(row)
}
```

Note `#[from]` on the `Database` variant: that is what makes plain `?` work for `sqlx::Error`
without a `map_err` at every call site. Map explicitly only where two variants share a source type,
as above.

Propagate with `?`. Do not `match` just to re-wrap, and do not `let _ =` a fallible call you
actually care about:

```rust
// WRONG - the error is discarded and the failure looks like success.
let _ = self.notify_systemd_ready().await;

// CORRECT - if it matters, handle it; if it truly does not, say so.
if let Err(err) = self.notify_systemd_ready().await {
    tracing::warn!(error = %err, "systemd readiness notification failed; continuing");
}
```

## `unwrap()` / `expect()` policy

Existing code has many of these. That is not a licence to add more. The rule for **new** code:

- **Never** on a path reachable from network input, a WebSocket message, filesystem state, user
  config, an env var, or a database result.
- **Fine** on a value the type system has already proven — a `Mutex` lock you cannot poison
  meaningfully, or a literal you just constructed and can see.
- In tests, `unwrap()` is fine. A panic in a test is a test failure, which is the point.
- Prefer `expect("why this cannot fail")` over `unwrap()` when it genuinely cannot fail, so the
  panic message explains the invariant.

```rust
// WRONG - an attacker or a corrupt config file turns this into a panic.
let port: u16 = env::var("HEARTHDECK_API_PORT").unwrap().parse().unwrap();

// CORRECT - a real error, with the value in the message.
let port: u16 = env::var("HEARTHDECK_API_PORT")
    .map_err(|_| ConfigError::Missing("HEARTHDECK_API_PORT"))?
    .parse()
    .map_err(|_| ConfigError::Invalid("HEARTHDECK_API_PORT"))?;
```

## Ownership: fix the borrow, do not clone past it

A `clone()` added to quiet the borrow checker is a bug you have not found yet. Ask which of these
it is first:

- **You need to read it.** Take `&T`. Most "cannot borrow" errors are a signature asking for
  ownership that only needs a borrow.
- **You need it after handing it away.** Clone then — at the boundary, deliberately.
- **Two mutable borrows of one struct.** Split the borrow with a helper, or restructure so the
  fields are borrowed separately. `let (a, b) = (&mut x.a, &x.b);` is fine when they are fields.

```rust
// WRONG - takes ownership of the caller's catalog, then clones it back out.
fn visible_titles(catalog: Vec<Title>) -> Vec<Title> { catalog.clone() }

// WRONG - clone per iteration to satisfy the borrow checker.
for title in &catalog { items.push(title.clone()); }

// CORRECT - borrow, and borrow at the call site too.
fn visible_titles(catalog: &[Title]) -> Vec<&Title> {
    catalog.iter().filter(|t| t.is_visible()).collect()
}
```

Parameter types that quietly cost a caller an allocation:

| Write this | Not this | Why |
|---|---|---|
| `&str` | `&String` | Accepts both, callers keep ownership |
| `&[T]` | `&Vec<T>` | Same, and works on arrays |
| `impl AsRef<Path>` | `&PathBuf` | Accepts `&str`, `String`, `Path` |
| `impl Into<String>` | `String` | Lets callers pass a literal without `.to_string()` |

## Types: make illegal states unrepresentable

**No boolean parameters.** At a call site `launch(title, true)` is unreadable and the compiler will
not catch a swap.

```rust
// WRONG - what does `true` mean here?
fn launch(&self, title: &Title, foreground: bool) { }

// CORRECT - the call site says what it means, and a mix-up is a type error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchMode { Foreground, Background }
fn launch(&self, title: &Title, mode: LaunchMode) { }
```

**Newtype your identifiers.** Two `u64`s or two `String`s can be swapped by accident; distinct
types cannot. This also gives one obvious place to put validation.

```rust
// WRONG - `pairing_complete(user_id, device_id)` with the arguments reversed compiles.
fn pairing_complete(user_id: u64, device_id: u64) { }

// CORRECT
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserId(u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceId(u64);
fn pairing_complete(user_id: UserId, device_id: DeviceId) { }
```

**Use `Option<T>` for "absent", not a sentinel.** `String::new()`, `0`, and `-1` as "no value" are
invisible to the compiler and read as real data.

**Prefer an exhaustive `match` over `if/else` on an enum.** When a variant is added later, the
compiler lists every place that needs updating — that is the whole benefit. Avoid a trailing
`_ =>` arm on a local enum; it silently swallows new variants.

## Async

- **Never hold a lock across `.await`.** Use `std::sync::Mutex` for short, non-async critical
  sections; if the guard must live across an await, use `tokio::sync::Mutex`. Better still,
  do the work and release the guard, then await.
- **`tokio::spawn` needs a `'static + Send` future.** If it will not compile, that is the borrow
  checker correctly refusing to let a task outlive what it borrows — clone the small owned piece
  you actually need rather than reaching for `std::sync::Arc` reflexively.
- **`tokio::time::sleep`, never `std::thread::sleep`,** inside async code. The latter blocks the
  executor thread and stalls every other task on it.
- **Do not block on the runtime from inside it** (`block_on` inside a task deadlocks).
- **Spawning a task discards its error.** Log inside the task or keep the `JoinHandle`.

```rust
// WRONG - blocks the whole executor thread; all other tasks stall.
tokio::time::sleep(...); // <- in an async fn, the std version is the bug
std::thread::sleep(Duration::from_secs(1));

// WRONG - the guard lives across the await, so every other task waits on this one.
let guard = state.lock().unwrap();
let title = client.fetch(id).await?;
guard.entries.push(title);

// CORRECT - fetch first, hold the lock only for the mutation.
let title = client.fetch(id).await?;
state.lock().await.entries.push(title);
```

## `unsafe`

There are a few existing `unsafe` blocks (overlay, bridge, daemon retro). Any **new** one requires:

1. A `// SAFETY:` comment on the line above stating the invariant that makes it sound.
2. A reason the safe alternative does not work.
3. A test or a manual check that the invariant holds.

In edition 2024 `unsafe_op_in_unsafe_fn` warns by default — an `unsafe fn` body needs its own
inner `unsafe {}` block. Unsafe attributes are written `#[unsafe(no_mangle)]`.

## Logging: `tracing`, never `println!`

Services log with `tracing`. `println!` is invisible to the journal's filtering, has no level, and
cannot carry structured fields.

```rust
// WRONG
println!("pairing failed for {device_id}");

// CORRECT - level, message, and a structured field the journal can filter on.
tracing::warn!(device_id = %device_id, "pairing failed");
```

Use `%value` for `Display` and `?value` for `Debug`. Prefer `info!` for state transitions,
`warn!` for recoverable problems, `error!` for "someone should look at this", and `debug!` for
detail. Do not log secrets, pairing codes, or tokens.

## Modules and visibility

- `pub(crate)` is the default. `pub` is a promise to other crates — make it deliberately.
- One module per responsibility. A `util.rs` that accumulates unrelated helpers is a signal the
  helpers belong with their callers.
- Put `use` statements at the top of the module, not inside functions, unless the import is only
  valid under `#[cfg(...)]` or would create a real cycle.
- Order imports the way rustfmt does: `just format` settles it, so do not hand-format.

## Edition 2024 notes

The workspace is on edition 2024. A few things that surprise people coming from 2021:

- **`gen` is a reserved keyword.**
- **`-> impl Trait` now captures all in-scope lifetimes by default.** Code that previously needed
  `+ '_` may now compile without it; code relying on the old non-capture needs `+ use<>`.
- **Tail-expression temporaries drop before local variables.** If a value you return borrows a
  local, the compiler will now say so instead of accepting it.
- **`static mut` references are denied.** Use `AtomicU*` or a lock.
- **Let-chains are available:** `if let Some(x) = opt && x.is_ready() { ... }`.

## Before you report done

- [ ] `just check-fast <crate>` is green for every crate you touched.
- [ ] No new `unwrap`/`expect` on a fallible, input-reachable path.
- [ ] No `clone()` added purely to satisfy the borrow checker.
- [ ] No `println!`/`eprintln!`/`dbg!` left in service code.
- [ ] Any `#[allow]` carries a reason comment.
- [ ] No `_ =>` arm silently swallowing enum variants.
