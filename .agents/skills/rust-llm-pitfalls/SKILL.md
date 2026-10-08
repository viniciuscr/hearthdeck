---
name: rust-llm-pitfalls
description: "Habits characteristic of AI-written Rust, and how to avoid them. Covers clone-to-satisfy-the-borrow-checker, silent `as` casts, unwrap_or_default masking errors, combinator and idiom drift, API-shape mistakes, and god functions. Load this when writing new Rust from scratch, when reviewing agent-generated code, or as a self-check before reporting done. Activates on borrow checker errors, .clone(), `as` casts, map_or, unwrap_or, Default::default, Self, must_use, too_many_lines, Rc<RefCell>, invented or hallucinated APIs, and 'is this idiomatic Rust'."
---

# LLM pitfalls in Rust

This catalogue is about a specific failure mode: code that **compiles, looks idiomatic, passes the
default lint set, and is still wrong or unidiomatic.** The default clippy set is clean in this
workspace; these habits live in the gap above it.

Read this when you are generating new Rust rather than editing existing code — that is when these
appear. Most are not bugs. Two groups are: the `as` casts and the masking of errors.

## CRITICAL RULE: `as` casts are silent, and that is the whole problem

`as` never fails. It truncates, wraps, or flips the sign and says nothing.

```rust
// WRONG - a config value above 65535 wraps around silently. So does a negative one.
let port = config.port as u16;

// WRONG - a large u64 becomes a negative i64.
let delta = timestamp_diff as i64;

// CORRECT - fails loudly, and you have to decide what failure means.
let port = u16::try_from(config.port)
    .map_err(|_| ConfigError::PortOutOfRange(config.port))?;

// CORRECT - lossless widening still reads better as a conversion.
let wide = u32::from(byte);
```

Use `try_into()`/`try_from` when the conversion can fail, `From`/`Into` when it cannot, and
`as` only when you have genuinely proven the range — and then say so in a comment.

## When to use this skill

- Writing a new function, module, or crate from scratch.
- A borrow checker error tempts you to add `.clone()`.
- Reviewing code an agent wrote.
- The self-check before reporting done.

## How these were found

Not from a blog list. In this workspace, the default clippy set (which CI enforces with
`-D warnings`) is **entirely clean**, but raising it to `clippy::pedantic` + `clippy::nursery`
surfaces **1,642 findings across 62 distinct lints**. The distribution below is what those lints
report. Reproduce it at any time with:

```
just lint-strict              # whole workspace, ranked summary
just lint-strict <crate>      # one crate
```

This is an opt-in diagnostic, not a gate — pedantic lints are opinionated, and many of the 1,642
are style rather than defects. See the dated note at the end.

## Group 1: ownership — the single biggest tell

Three of the canonical Rust anti-patterns are in this family. "Clone to satisfy the borrow
checker" is an **official anti-pattern**, documented in the Rust Design Patterns book.

```rust
// WRONG - clones so the borrow checker stops complaining.
// The copy is a separate value: mutations to one are invisible to the other.
let y = &mut x.clone();
println!("{x}");

// WRONG - takes ownership the caller still needs, forcing a clone at every call site.
fn render(items: Vec<Item>) -> String { items.len().to_string() }

// CORRECT - borrow.
fn render(items: &[Item]) -> String { items.len().to_string() }
```

The fix is almost always in the signature, not the body. If you need to read it, take `&T`. If the
error is two mutable borrows of one struct, split the borrow (`let (a, b) = (&mut s.a, &s.b);`).
Only clone deliberately, at a boundary, knowing why.

Also in this family:

- **`needless_pass_by_value`** — an owned parameter that is only read. 40 findings.
- **`redundant_clone` / `assigning_clones`** — a clone whose result replaces the original. Provably
  pointless; 14 findings.
- **`Rc<RefCell<T>>` as a borrow-checker escape hatch.** It compiles and moves the check to runtime,
  where a misuse is a panic instead of an error. In a single-threaded, non-async context this is
  usually a design smell, not a solution.
- **`Arc` reflexively.** `Arc` is for genuine sharing across threads or tasks. Reaching for it to
  quiet a borrow error is the `Rc<RefCell>` mistake with a lock on it.

## Group 2: error handling — masking a failure as a value

```rust
// WRONG - a failed parse becomes an empty config, and nobody finds out.
let port = config.port.parse().unwrap_or_default();

// WRONG - .ok() throws the error away; the caller cannot tell why.
let value = self.fetch(key).await.ok()?;

// WRONG - the cause is discarded, so the log says "failed" and nothing else.
self.sync().await.map_err(|_| SyncError::Failed)?;

// CORRECT - keep the cause.
self.sync().await.map_err(SyncError::Sync)?;   // with #[from] / #[source]
```

`unwrap_or_default()` and `.ok()` are the two most common ways an LLM turns a real error into a
plausible-looking default. Neither is wrong in every case — `.ok()` on genuinely-optional data is
fine — but reach for them deliberately, and never on a path where the error means something.

Also: `unwrap`/`expect` policy lives in `rust-hygiene`; the rule there is unchanged.

## Group 3: idiom drift — code that runs but reads as unidiomatic

These are not bugs. They are the accumulated tells of generated code, and together they are why a
file "looks machine-written". ~14 lints, ~350 findings.

```rust
// WRONG -> CORRECT
.map(|v| f(v)).unwrap_or(default)     //  .map_or(default, f)
if let Some(x) = o { a } else { b }   //  o.map_or(b, |x| a)
String::default()                     //  Default::default()   (when the type is already known)
Foo { .. }  inside impl Foo           //  Self { .. }
format!("{}", x)                      //  format!("{x}")
.filter(|x| x.is_ready())             //  .filter(Item::is_ready)
opt.unwrap_or(Vec::new())             //  opt.unwrap_or_default()   (eager allocation!)
s.push_str(&format!("{x}"));          //  write!(s, "{x}")?
match e { A => 1, B => 1, _ => 2 }    //  match e { A | B => 1, _ => 2 }
```

The `unwrap_or` one is a real performance bug, not just style: the argument is evaluated eagerly,
so `unwrap_or(Vec::new())` allocates on every call even when the value is present. Use
`unwrap_or_else(|| …)` or `unwrap_or_default()`.

Implementing `Default` incidentally is fine; calling `String::default()` where `Default::default()`
would infer is the tell that you are pattern-matching on syntax rather than intent.

## Group 4: API shape

From the Rust API Guidelines, and worth getting right because these are the public contracts.

```rust
// WRONG - a pure getter the compiler cannot warn anyone about discarding.
pub fn title(&self) -> &str { &self.title }

// CORRECT - `#[must_use]` makes discarding the result a warning.
#[must_use]
pub fn title(&self) -> &str { &self.title }

// WRONG - a Result-returning public function with no documented failure modes.
pub fn load(path: &Path) -> Result<Config, ConfigError> { … }

// CORRECT - document them; the reader can then decide whether to care.
/// # Errors
/// Returns [`ConfigError::Missing`] if the file does not exist, and
/// [`ConfigError::Invalid`] if it is not valid TOML.
pub fn load(path: &Path) -> Result<Config, ConfigError> { … }
```

- **`must_use_candidate`** (54) — pure getters and conversions.
- **`missing_errors_doc`** (12) — `# Errors` on every public `Result`-returning fn.
- **`return_self_not_must_use`** (10) — builder methods.
- **`struct_excessive_bools`** (4) — several `bool` fields wanting to be an enum. The parameter
  version of this is in `rust-hygiene`.
- **`redundant_pub_crate`** (24) — `pub(crate)` inside a module that is not itself public. It is
  noise; plain `pub` or private is correct.
- **`derive_partial_eq_without_eq`** (10) — deriving `PartialEq` and forgetting `Eq` on a type
  with no float fields.

## Group 5: structure — the god function

```rust
// WRONG - 400 lines, 9 levels of nesting, a dozen locals alive at once.
fn update(&mut self, message: Message) -> Task<Message> {
    match message { /* … everything … */ }
}
```

`too_many_lines` fires 21 times here, and the frontend's `update` is over a thousand lines. Long
functions are how a codebase becomes unreviewable, and they are the hardest thing to fix later.

When a function passes ~100 lines, extract. The extracted piece should usually be a **free function
that takes the data it needs** — not a method on the same struct — because that is what makes it
testable (see `testing-and-validation`). Related: `too_many_arguments` (extract a struct),
`items_after_statements` (declare at the top), `unused_self` (it wants to be a free function).

## Group 6: tests

- **`assert_is_empty`** (36) — `assert_eq!(x.len(), 0)` instead of `assert!(x.is_empty())`.
- Asserting `is_ok()` without the value — see `testing-and-validation`.
- A test with no assertion, or one that recomputes the expected value with the implementation.

## Group 7: docs and comments

- **`doc_markdown`** (459 — by far the most common finding). Identifiers, paths, and types in doc
  comments must be in backticks: `` /// Loads the `Title` from `catalog.rs`. `` Without it,
  rustdoc renders them as prose and clippy objects.
- Comments that restate the code. Explain the *why*; the *what* is already in the code.
- A `///` doc comment on a private helper that already has an obvious name — noise.

## Group 8: the meta-habits

These are about how the code was produced, and they are the ones a reviewer cannot fix with a lint.

- **Invented APIs.** Calling a method, field, or argument that does not exist. The compiler catches
  it, but the tempting "fix" is to add a wrapper that makes the invented API real. **Check that the
  API exists and means what you think before coding against it** — open the file, do not infer from
  the name.
- **Speculative abstraction.** A trait with one implementor, a generic instantiated once, a config
  flag nobody sets, an `enum` with one variant "for later". Add it when the second case arrives.
- **Unnecessary dependencies.** Check the workspace `Cargo.toml` and `std` first. This project's
  `std`+`tokio`+existing-deps baseline covers far more than it looks.
- **Reinventing what exists in the repo.** Read `widgets/`, `style.rs`, and the providers before
  writing a new helper.
- **`#![deny(warnings)]` in a crate root.** This is an official anti-pattern: a dependency's new
  warning breaks your build on a toolchain bump. Enforce warnings with the **flag** in CI
  (`cargo clippy … -- -D warnings`), which is what this repo does — never in source.
- **Deref polymorphism** — implementing `Deref` to emulate inheritance. The third official
  anti-pattern; use composition or a trait method.

## Self-check before reporting done

- [ ] No `.clone()` added to silence a borrow checker error.
- [ ] No new `as` cast on a fallible or narrow conversion; `try_into`/`From` instead.
- [ ] No `unwrap_or_default()` / `.ok()` masking a meaningful error.
- [ ] `map_or` / `unwrap_or_else` / `Default::default()` where the eager form allocates.
- [ ] `#[must_use]` on new pure getters; `# Errors` on new public `Result` fns.
- [ ] No function I added is over ~100 lines.
- [ ] Doc comments backtick their identifiers.
- [ ] Every API I called — I opened the file and confirmed it exists.
- [ ] No abstraction added for a second case that does not exist yet.
- [ ] `just check-fast <crate>` is green.

## Evidence note (measured 2026-10-08)

Recorded so the claims above are checkable rather than asserted. Re-run `just lint-strict` to
refresh; the numbers will drift as the code changes, and that is expected — the habit list is the
durable part, not the counts.

| Lint | Count | Group |
|---|---|---|
| `doc_markdown` | 459 | docs |
| `missing_const_for_fn` | 155 | idiom |
| `use_self` | 128 | idiom |
| `cast_possible_truncation` | 84 | **casts** |
| `cast_possible_wrap` | 57 | **casts** |
| `must_use_candidate` | 54 | API shape |
| `map_unwrap_or` | 50 | idiom |
| `default_trait_access` | 50 | idiom |
| `option_if_let_else` | 48 | idiom |
| `cast_precision_loss` | 46 | **casts** |
| `needless_pass_by_value` | 40 | **ownership** |
| `assert_is_empty` | 36 | tests |
| `uninlined_format_args` | 34 | idiom |
| `cast_sign_loss` | 30 | **casts** |
| `suboptimal_flops` | 30 | idiom |
| `too_many_lines` | 21 | structure |

**Total 1,642 across 62 distinct lints.** Distribution by crate: frontend 876, daemon 452, ai 158,
bridge 89, protocol 24, overlay 22, input 21.

Two caveats, stated plainly:

1. **The default set is clean.** All of this is *above* the bar CI enforces — which is the point.
   The habits are real, visible, and currently ungated.
2. **Not all 1,642 are defects.** `doc_markdown` and `missing_const_for_fn` are stylistic. The
   rows in bold are the ones with correctness or performance consequences: 217 `as` casts,
   40 needless-by-value parameters, 14 redundant clones.
