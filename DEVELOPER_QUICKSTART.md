# Hearthdeck Developer Quick Reference

## Before Your First Push

```bash
git config core.hooksPath .githooks
just setup
```

## Before Committing

```bash
just format          # Auto-format code
just check-services  # Build + test Rust services
just check-frontend  # Lint the Rust frontend
just test-frontend   # Test the Rust frontend
```

Or all at once:

```bash
just check           # Runs all above
```

## Before Pushing

Automatic via git hook (if configured):

```bash
git push
# Pre-push hook runs automatically, blocks if validation fails
```

Manual validation:

```bash
just pre-push-check
```

## Common Fixes

| Error | Command |
|-------|---------|
| Formatting issues | `cargo fmt --manifest-path services/Cargo.toml --all` |
| Build fails | `cargo build --manifest-path services/Cargo.toml --workspace --release` |
| Clippy warnings | `cargo clippy --manifest-path services/Cargo.toml --workspace --all-targets --release` |
| Frontend clippy warnings | `just check-frontend` |

## Documentation

- **Full guide**: `CONTRIBUTING.md`
- **Workflow details**: `PRE_PUSH_WORKFLOW.md`
- **Git hooks setup**: `.githooks/README.md`

## Key Commands

```bash
just setup              # Install toolchains
just check              # Pre-commit validation
just pre-push-check     # Pre-push validation
just build-services     # Release build (services)
just build-frontend     # Release build (frontend)
just check-services     # Tests + Clippy (services)
just check-frontend     # Clippy (frontend)
just test-frontend      # Tests (frontend)
just format             # Format Rust
```

## Environment

- **Rust version**: 1.97.1 (via `mise`)
- **Tool manager**: `mise` (installs Rust and `just` in `just setup`)

## Getting Help

1. Check `CONTRIBUTING.md` → "Common Errors and Fixes"
2. Read `.githooks/README.md` for git hooks troubleshooting
3. Run `just --list` to see all available recipes
4. Open a GitHub issue if stuck
