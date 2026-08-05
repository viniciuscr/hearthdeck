# Git Hooks Setup Guide

This document explains how to set up and use git hooks to prevent CI failures by validating code locally before pushing.

## Quick Setup (1 minute)

Enable git hooks for this repository:

```bash
git config core.hooksPath .githooks
```

That's it! The pre-push hook will now run automatically before every push.

## What the Pre-Push Hook Does

When you run `git push`, the `.githooks/pre-push` script automatically:

1. **Checks Rust formatting**: Ensures `cargo fmt` has been applied
2. **Builds all services**: Compiles in release mode (same as CI)
3. **Runs Clippy linting**: Catches warnings before they fail CI

If any check fails, the push is blocked and the hook provides detailed instructions to fix the problem.

## Manual Validation

### Before Push (Without Hook)

If you haven't enabled git hooks, validate manually:

```bash
just pre-push-check
```

This runs the same checks as the hook without actually attempting a push.

### Before Commit

To catch issues even earlier, validate before committing:

```bash
just check
```

This runs:
- Formatting validation (Dart + Rust)
- Rust compilation and tests
- Flutter analysis and tests

## Skipping the Hook (Not Recommended)

If you absolutely must push code that fails these checks (which you shouldn't!):

```bash
git push --no-verify
```

⚠️ **Warning**: This bypasses local validation and will likely fail in CI, requiring a force-push to fix.

## Troubleshooting

### Hook Not Running?

Verify the configuration:

```bash
git config --get core.hooksPath
```

Should output: `.githooks`

If not set, re-run:

```bash
git config core.hooksPath .githooks
```

### Hook Command Not Found

Make sure Rust and Cargo are installed:

```bash
rustc --version
cargo --version
```

If you're using `mise` (recommended), initialize your shell:

```bash
mise install
```

### Build Takes Too Long

First release build can take several minutes. Subsequent pushes are faster due to incremental compilation.

**Tip**: During development, use `just check` (debug builds) locally, then `just pre-push-check` before the final push.

### Specific Error: "State enum not in scope"

See **CONTRIBUTING.md** → "Common Errors and Fixes" → "State enum not in scope"

### Specific Error: "wayland-protocols not found"

See **CONTRIBUTING.md** → "Common Errors and Fixes" → "wayland-protocols not found"

## Files Overview

- **`.githooks/pre-push`**: The actual hook script that validates on push
- **`CONTRIBUTING.md`**: Complete development guide with common errors and setup
- **`justfile`**: `pre-push-check` recipe for manual validation

## Integration with CI

The pre-push hook runs **locally** before push. After pushing, GitHub Actions runs the `code-quality.yml` workflow which performs **additional** validation:

- Full test suites (Rust + Flutter)
- Strict linting (Clippy with warnings-as-errors in release mode)
- Package building (Arch Linux)

Running local checks prevents most CI failures, making the team more efficient.

## For Teams

When onboarding new developers:

1. After cloning the repo, they should run:
   ```bash
   just setup
   git config core.hooksPath .githooks
   ```

2. Before first push, have them test the hook:
   ```bash
   just pre-push-check
   ```

3. Point them to **CONTRIBUTING.md** for detailed guidance

## Questions?

See **CONTRIBUTING.md** for comprehensive development documentation, or open a GitHub issue.
