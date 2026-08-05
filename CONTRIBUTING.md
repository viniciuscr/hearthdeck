# Contributing to Hearthdeck

Welcome to the Hearthdeck project! This guide covers the development workflow, pre-commit requirements, and how to avoid common CI failures.

## Quick Start

### 1. Install Development Environment

```bash
just setup
```

This installs project-pinned versions of all toolchains (Rust, Flutter, etc.) using `mise`.

### 2. Set Up Git Hooks (Recommended)

Prevent CI failures by catching errors locally before pushing:

```bash
git config core.hooksPath .githooks
```

This enables automatic validation of code formatting, builds, and tests before push.

## Pre-Commit Workflow

Run these checks **before committing** to catch errors early:

### Format Check
```bash
just format
```
Formats all Dart (Flutter) and Rust code. This fixes most formatting issues automatically.

### Build & Test (Services)
```bash
just check-services
```
Runs cargo check, tests, and clippy linting on all backend services. Must pass before pushing.

### Build & Test (Flutter App)
```bash
just check-app
```
Analyzes, tests, and builds the macOS debug app. Catches Dart issues early.

### Full Validation (All Components)
```bash
just check
```
Runs format, check-services, and check-app in sequence. Use this before creating a PR.

## Pre-Push Validation

### Automatic Hook (If Configured)
If you ran `git config core.hooksPath .githooks`, the pre-push hook runs automatically:
- Checks Rust formatting: `cargo fmt --all -- --check`
- Builds all services in release mode: `cargo build --workspace --release`
- Fails the push if either check fails

### Manual Pre-Push Check
To manually run the same checks without pushing:

```bash
just pre-push-check
```

This validates:
1. ✅ No `cargo fmt` violations
2. ✅ Full release build succeeds for all services
3. ✅ No clippy warnings in release mode

## Common Errors and Fixes

### Error: `rustfmt` couldn't find `state.rs`
**Cause**: A Rust source file uses an enum or type from another module but doesn't import it.  
**Example**:
```rust
// In services/hearthdeck-bridge/src/main.rs
fn handle_state(s: State) { }  // ❌ Error: State not in scope
```

**Fix**: Add the import at the top of the file:
```rust
use crate::state::State;  // ✅ Correct
```

Or if importing from sibling module:
```rust
use super::state::State;  // ✅ Correct
```

### Error: `cargo build` fails with "wayland-protocols" not found
**Cause**: Missing GTK/Wayland development headers on Linux, or missing dependency in `Cargo.toml`.  
**Fix**:
```bash
# On Arch Linux / ArchLinux CI:
pacman -S wayland-protocols gtk3

# On Debian/Ubuntu:
sudo apt install libwayland-dev libgtk-3-dev libwayland-protocols-dev

# Then verify the service has the wayland feature enabled:
# In services/hearthdeck-overlay/Cargo.toml:
# [dependencies]
# gtk = { version = "0.x", features = ["v3_22"] }
```

### Error: `cargo fmt --all -- --check` reports diff
**Cause**: Code formatting doesn't match project standards.  
**Fix**: Run format automatically:
```bash
just format
# or for Rust only:
cargo fmt --manifest-path services/Cargo.toml --all
```

Then commit the changes:
```bash
git add .
git commit -m "chore: apply rustfmt formatting"
```

### Error: `cargo clippy` reports warnings in release mode
**Cause**: Code violates Clippy linting rules (enforced as errors in CI).  
**Example**:
```rust
// ❌ Clippy error: this loop could be written as a map
for x in vec {
    vec.push(x * 2);
}
```

**Fix**: Use the suggestion from Clippy output:
```bash
just check-services  # Runs clippy and shows suggestions
# Apply suggested changes, e.g., convert to iterator:
```

### Error: Service compiles locally but fails in CI
**Cause**: Local Rust version differs from CI (1.97.1), or missing release mode build.  
**Fix**: Build in release mode locally (same as CI):
```bash
just build-services
# or: cargo build --manifest-path services/Cargo.toml --workspace --release
```

### Error: Build succeeds but CI fails on "unused import"
**Cause**: Feature flags or conditional compilation differs between local and CI.  
**Fix**: Build with the same target and flags as CI:
```bash
# CI uses Linux target, so test locally:
cargo build --target x86_64-unknown-linux-gnu --manifest-path services/Cargo.toml --workspace --release
```

(Requires: `rustup target add x86_64-unknown-linux-gnu`)

## Workflow: Creating a PR

1. **Make changes** to Flutter or Rust code
2. **Run pre-commit checks**:
   ```bash
   just check
   ```
3. **Fix any issues** (formatting, build, test failures)
4. **Commit changes**:
   ```bash
   git add .
   git commit -m "feat: your feature description"
   ```
5. **Push** (git hook validates automatically if configured):
   ```bash
   git push origin your-branch
   ```
6. **PR opens** and GitHub Actions runs the full CI suite:
   - Flutter analysis and tests
   - Rust formatting check
   - Rust compilation and tests
   - Clippy linting (warnings = errors)
   - Arch Linux package build

## Architecture Overview

- **`services/`**: Rust backend services
  - `hearthdeck-daemon`: Local API server and state manager
  - `hearthdeck-bridge`: Linux desktop environment integration
  - `hearthdeck-observability`: Telemetry and logging
  - `hearthdeck-protocol`: Shared types and API contracts
  - `hearthdeck-overlay`: Gamescope overlay integration (new)
  - `hearthdeck-overlay-spike`: Temporary spike for overlay investigation

- **`lib/`**: Flutter frontend (Dart)
- **`test/`**: Flutter tests
- **`docs/`**: Architecture and design documentation
- **`.github/workflows/`**: CI/CD pipeline (GitHub Actions)

## Testing

### Backend (Rust)
```bash
# Run all Rust tests (debug mode):
just check-services

# Run specific service tests:
cargo test --manifest-path services/Cargo.toml -p hearthdeck-daemon
```

### Frontend (Dart/Flutter)
```bash
# Run Flutter tests:
just test-app
```

## Troubleshooting

### Git hook not running on push
**Solution**: Ensure hooks path is configured:
```bash
git config core.hooksPath .githooks
git config --get core.hooksPath  # Should print .githooks
```

### Pre-push checks take too long
**Solution**: The release build can be slow on first run. Subsequent pushes will be faster due to incremental compilation. For development, use `just check-services` (debug build) locally.

### Environment variables or paths differ from CI
**Solution**: The CI container is Arch Linux with specific packages. To match more closely:
- Use `just ci-check` to run the exact validation sequence used in CI
- Consider running in Docker if you need exact environment parity

## Additional Resources

- [Product Foundations](docs/product-foundations.md) - Design principles and contracts
- [Backend Architecture](docs/backend-architecture.md) - Service design and API structure
- [Metadata Enrichment](docs/metadata-enrichment.md) - Catalog and provider implementation
- [Kiosk Session](docs/kiosk-session.md) - Linux desktop integration notes

## Questions?

Open an issue or discussion on GitHub. Our team is here to help!
