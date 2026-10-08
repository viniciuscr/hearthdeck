set shell := ["zsh", "-cu"]
services_manifest := "services/Cargo.toml"

# Recipes call `cargo` directly. `cargo` resolves to the toolchain pinned in
# mise.toml through mise's shims / activate hook, so wrapping every command in
# `mise exec --` was redundant. It also forced *every* pinned tool to be
# installed first — including ruby, which cargo does not need — before any Rust
# command could run. `just setup` still installs the toolchains.

default:
  @just --list

# List available recipes.
list:
  @just --list

# Install all project-pinned toolchains.
setup:
  mise install

# Run the bridge, local API daemon, and COSMIC frontend together.
dev:
  ./scripts/dev

# Run the local stack on alternate ports, beside an installed hearthdeck.target.
dev-local:
  HEARTHDECK_DEV_API_PORT=38410 HEARTHDECK_DEV_ADMIN_PORT=38411 ./scripts/dev

# Format Rust source files.
format:
  cargo fmt --manifest-path {{services_manifest}} --all

# Build, test, and lint the Rust backend workspace.
check-services:
  cargo check --manifest-path {{services_manifest}} --workspace
  cargo test --manifest-path {{services_manifest}} --workspace
  cargo clippy --manifest-path {{services_manifest}} --workspace --all-targets -- -D warnings

# Build optimized Linux-host service binaries.
build-services:
  cargo build --manifest-path {{services_manifest}} --workspace --release

# Build debug service binaries for the combined development target.
build-services-debug:
  cargo build --manifest-path {{services_manifest}} --workspace

# Run the Linux integration bridge in the foreground.
bridge:
  cargo run --manifest-path {{services_manifest}} -p hearthdeck-bridge

# Run the local API daemon in the foreground.
daemon:
  cargo run --manifest-path {{services_manifest}} -p hearthdeck-daemon

# Request a one-time pairing code from the loopback admin listener.
pairing-code:
  curl --fail --silent --show-error -X POST http://127.0.0.1:38401/v1/pairing

# Request one metadata provider refresh from a paired daemon.
refresh-metadata url token provider="appstream-local":
  curl --fail --silent --show-error -X POST {{url}}/v1/metadata/{{provider}}/refresh -H 'Authorization: Bearer {{token}}'

# Request one categorization scan from a paired daemon.
categorize url token:
  curl --fail --silent --show-error -X POST {{url}}/v1/categorization/scan -H 'Authorization: Bearer {{token}}'

# Show the categorization status and the latest report from a paired daemon.
categorization url token:
  curl --fail --silent --show-error {{url}}/v1/categorization -H 'Authorization: Bearer {{token}}'

# Build the COSMIC frontend.
build-frontend:
  cargo build --manifest-path {{services_manifest}} -p hearthdeck-frontend --release

# Build debug COSMIC frontend.
build-frontend-debug:
  cargo build --manifest-path {{services_manifest}} -p hearthdeck-frontend

# Run the COSMIC frontend.
run-frontend: build-frontend
  ./services/target/release/hearthdeck-frontend

# Run the COSMIC frontend in debug mode.
run-frontend-debug: build-frontend-debug
  ./services/target/debug/hearthdeck-frontend

# Check and lint the COSMIC frontend.
check-frontend:
  cargo clippy --manifest-path {{services_manifest}} -p hearthdeck-frontend --all-targets -- -D warnings

# Test the COSMIC frontend.
test-frontend:
  cargo test --manifest-path {{services_manifest}} -p hearthdeck-frontend

# Usage: just check-fast [crate]   (default: hearthdeck-frontend)
# Mirrors the CI gates (fmt + clippy -D warnings + tests) for one crate only.
# Run `just check` before calling work done.
# Fast, narrow validation loop for a single crate: format, lint, test.
check-fast crate="hearthdeck-frontend":
  cargo fmt --manifest-path {{services_manifest}} --all
  cargo clippy --manifest-path {{services_manifest}} -p {{crate}} --all-targets -- -D warnings
  cargo test --manifest-path {{services_manifest}} -p {{crate}}

# Run all portable project checks.
check: format check-services check-frontend test-frontend

# Usage: just lint-strict [crate]   (default: whole workspace)
# Opt-in and LOCAL ONLY: clippy::pedantic and clippy::nursery are opinionated,
# so this is not a gate. It is deliberately NOT wired into CI or pre-push.
# See .agents/skills/rust-llm-pitfalls for the habits it is meant to surface.
# Ranked summary of clippy::pedantic and clippy::nursery findings, most common first.
lint-strict crate="":
  @cargo clippy --manifest-path {{services_manifest}} {{ if crate == "" { "--workspace" } else { "-p " + crate } }} --all-targets --message-format=json -- -W clippy::pedantic -W clippy::nursery 2>/dev/null | grep -o '"code":"clippy::[a-z_]*"' | sed 's/.*clippy:://; s/"$//' | sort | uniq -c | sort -rn | head -40

# Validate code before pushing: format check, release build, and clippy lint.
pre-push-check:
  @echo "=== Pre-Push Validation ==="
  @echo "Running local checks before push..."
  @echo ""
  @echo "⏳ [1/3] Checking Rust code formatting..."
  cargo fmt --manifest-path {{services_manifest}} --all -- --check
  @echo "✅ Formatting check passed"
  @echo ""
  @echo "⏳ [2/3] Building services in release mode..."
  cargo build --manifest-path {{services_manifest}} --workspace --release
  @echo "✅ Release build passed"
  @echo ""
  @echo "⏳ [3/3] Running Clippy lint checks (release mode)..."
  cargo clippy --manifest-path {{services_manifest}} --workspace --all-targets --release -- -D warnings
  @echo "✅ Clippy check passed"
  @echo ""
  @echo "✨ All checks passed! Push when ready."

# Run source validation in CI after Rust has been installed.
ci-check:
  cargo fmt --manifest-path {{services_manifest}} --all -- --check
  cargo check --manifest-path {{services_manifest}} --workspace
  cargo test --manifest-path {{services_manifest}} --workspace
  cargo clippy --manifest-path {{services_manifest}} --workspace --all-targets -- -D warnings

# Build the Arch Linux package from the current source checkout.
ci-package-arch:
  cd packaging/arch && makepkg --cleanbuild --log --noconfirm --nodeps

# Run automated acceptance checks on the target Linux host with RomM required.
acceptance-linux:
  ./scripts/linux-acceptance --require-romm

# Install Linux systemd user units and enable the local services.
install-services:
  mkdir -p "$HOME/.config/systemd/user" "$HOME/.local/bin"
  systemctl --user disable --now ltv-bridge.service ltv-daemon.service 2>/dev/null || true
  systemctl --user disable --now hearthdeck.target hearthdeck-bridge.socket hearthdeck-bridge.service hearthdeck-daemon.service hearthdeck-input.service 2>/dev/null || true
  if [[ -d "$HOME/.config/ltv" && ! -e "$HOME/.config/hearthdeck" ]]; then mv "$HOME/.config/ltv" "$HOME/.config/hearthdeck"; fi
  if [[ -d "$HOME/.local/share/ltv" && ! -e "$HOME/.local/share/hearthdeck" ]]; then mv "$HOME/.local/share/ltv" "$HOME/.local/share/hearthdeck"; fi
  if [[ -f "$HOME/.config/hearthdeck/daemon.env" ]]; then perl -pi -e 's/LTV_/HEARTHDECK_/g; s#/.config/ltv/#/.config/hearthdeck/#g' "$HOME/.config/hearthdeck/daemon.env"; fi
  rm -f "$HOME/.config/systemd/user/ltv-bridge.service" "$HOME/.config/systemd/user/ltv-daemon.service"
  cp deploy/systemd/hearthdeck-bridge.service "$HOME/.config/systemd/user/"
  cp deploy/systemd/hearthdeck-daemon.service "$HOME/.config/systemd/user/"
  cp deploy/systemd/hearthdeck-input.service "$HOME/.config/systemd/user/"
  cp deploy/systemd/hearthdeck-bridge.socket "$HOME/.config/systemd/user/"
  cp deploy/systemd/hearthdeck.target "$HOME/.config/systemd/user/"
  cp deploy/systemd/hearthdeck-log.service "$HOME/.config/systemd/user/"
  cp deploy/systemd/romm.service "$HOME/.config/systemd/user/"
  cp deploy/systemd/romm.path "$HOME/.config/systemd/user/"
  cp services/target/release/hearthdeck-bridge "$HOME/.local/bin/"
  cp services/target/release/hearthdeck-daemon "$HOME/.local/bin/"
  cp services/target/release/hearthdeck-input "$HOME/.local/bin/"
  cp services/target/release/hearthdeck-ai "$HOME/.local/bin/"
  cp packaging/arch/hearthdeck-romm "$HOME/.local/bin/hearthdeck-romm"
  systemctl --user daemon-reload
  systemctl --user enable --now hearthdeck.target

# Show Linux service status.
services-status:
  systemctl --user status hearthdeck.target hearthdeck-log.service hearthdeck-bridge.socket hearthdeck-bridge.service hearthdeck-daemon.service hearthdeck-input.service romm.service

# Follow Linux service logs.
services-logs:
  journalctl --user -fu hearthdeck-bridge.service -u hearthdeck-daemon.service -u hearthdeck-input.service

# Follow the combined per-session service log.
logs-file:
  tail -f "$HOME/hearthdeck.log"

# Follow structured daemon logs only.
logs-daemon:
  journalctl --user -fu hearthdeck-daemon.service -o cat

# Follow structured Linux bridge logs only.
logs-bridge:
  journalctl --user -fu hearthdeck-bridge.service -o cat

# Follow controller compatibility broker logs only.
logs-input:
  journalctl --user -fu hearthdeck-input.service -o cat

# Show recent warning and error logs from both services.
logs-errors:
  journalctl --user -u hearthdeck-bridge.service -u hearthdeck-daemon.service -u hearthdeck-input.service -p warning --since '1 hour ago' -o cat
