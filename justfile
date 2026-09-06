set shell := ["zsh", "-cu"]
services_manifest := "services/Cargo.toml"
dart_sources := "lib test"

default:
  @just --list

# List available recipes.
list:
  @just --list

# Install all project-pinned toolchains.
setup:
  mise install
  mise exec -- flutter pub get

# Run the Flutter application on the selected device.
app device="macos":
  mise exec -- flutter run -d {{device}}

# Run Flutter against a paired Hearthdeck API catalog.
app-live url token device="macos":
  mise exec -- flutter run -d {{device}} --dart-define=HEARTHDECK_BACKEND_URL={{url}} --dart-define=HEARTHDECK_PAIRING_TOKEN={{token}}

# Run the bridge, local API daemon, and Flutter app together.
dev device="macos":
  ./scripts/dev {{device}}

# Analyze, test, and build the Flutter macOS debug application.
check-app: test-app
  mise exec -- flutter build macos --debug

# Format Flutter and Rust source files.
format:
  mise exec -- dart format {{dart_sources}}
  mise exec -- cargo fmt --manifest-path {{services_manifest}} --all

# Analyze and test the Flutter client.
test-app:
  mise exec -- flutter analyze
  mise exec -- flutter test

# Build, test, and lint the Rust backend workspace.
check-services:
  mise exec -- cargo check --manifest-path {{services_manifest}} --workspace
  mise exec -- cargo test --manifest-path {{services_manifest}} --workspace
  mise exec -- cargo clippy --manifest-path {{services_manifest}} --workspace --all-targets -- -D warnings

# Build optimized Linux-host service binaries.
build-services:
  mise exec -- cargo build --manifest-path {{services_manifest}} --workspace --release

# Build debug service binaries for the combined development target.
build-services-debug:
  mise exec -- cargo build --manifest-path {{services_manifest}} --workspace

# Run the Linux integration bridge in the foreground.
bridge:
  mise exec -- cargo run --manifest-path {{services_manifest}} -p hearthdeck-bridge

# Run the local API daemon in the foreground.
daemon:
  mise exec -- cargo run --manifest-path {{services_manifest}} -p hearthdeck-daemon

# Build the disposable Gamescope external-overlay spike (see
# services/hearthdeck-overlay-spike). This is only for iterating locally
# without waiting on a push+CI+pacman cycle. The normal path is: push, let
# CI publish the updated package, then `sudo pacman -Syu` on the kiosk box —
# the package now installs this as /usr/bin/hearthdeck-overlay-spike, ready
# to run with no separate build/copy step. Delete the crate once the finding
# from docs/kiosk-session.md's overlay investigation is confirmed either way.
build-overlay-spike:
  mise exec -- cargo build --manifest-path services/Cargo.toml -p hearthdeck-overlay-spike --release
  @echo "Local build only: services/target/release/hearthdeck-overlay-spike"
  @echo "Normal path: push, wait for CI, then 'sudo pacman -Syu' on the kiosk box and run 'hearthdeck-overlay-spike'."

# Scan installed macOS application bundles through the real provider.
macos-discovery-check: build-services-debug
  ./scripts/macos-discovery-check

# Request a one-time pairing code from the loopback admin listener.
pairing-code:
  curl --fail --silent --show-error -X POST http://127.0.0.1:38401/v1/pairing

# Request one metadata provider refresh from a paired daemon.
refresh-metadata url token provider="appstream-local":
  curl --fail --silent --show-error -X POST {{url}}/v1/metadata/{{provider}}/refresh -H 'Authorization: Bearer {{token}}'

# Build the COSMIC frontend.
build-frontend:
  mise exec -- cargo build --manifest-path {{services_manifest}} -p hearthdeck-frontend --release

# Build debug COSMIC frontend.
build-frontend-debug:
  mise exec -- cargo build --manifest-path {{services_manifest}} -p hearthdeck-frontend

# Run the COSMIC frontend.
run-frontend: build-frontend
  ./services/target/release/hearthdeck-frontend

# Run the COSMIC frontend in debug mode.
run-frontend-debug: build-frontend-debug
  ./services/target/debug/hearthdeck-frontend

# Check and lint the COSMIC frontend.
check-frontend:
  mise exec -- cargo clippy --manifest-path {{services_manifest}} -p hearthdeck-frontend --all-targets -- -D warnings

# Test the COSMIC frontend.
test-frontend:
  mise exec -- cargo test --manifest-path {{services_manifest}} -p hearthdeck-frontend

# Run all portable project checks.
check: format check-services check-frontend test-frontend test-app

# Validate code before pushing: format check, release build, and clippy lint.
pre-push-check:
  @echo "=== Pre-Push Validation ==="
  @echo "Running local checks before push..."
  @echo ""
  @echo "⏳ [1/3] Checking Rust code formatting..."
  mise exec -- cargo fmt --manifest-path {{services_manifest}} --all -- --check
  @echo "✅ Formatting check passed"
  @echo ""
  @echo "⏳ [2/3] Building services in release mode..."
  mise exec -- cargo build --manifest-path {{services_manifest}} --workspace --release
  @echo "✅ Release build passed"
  @echo ""
  @echo "⏳ [3/3] Running Clippy lint checks (release mode)..."
  mise exec -- cargo clippy --manifest-path {{services_manifest}} --workspace --all-targets --release -- -D warnings
  @echo "✅ Clippy check passed"
  @echo ""
  @echo "✨ All checks passed! Push when ready."

# Run source validation in CI after Flutter and Rust have been installed.
ci-check:
  flutter pub get
  dart format --output=none --set-exit-if-changed {{dart_sources}}
  cargo fmt --manifest-path {{services_manifest}} --all -- --check
  cargo check --manifest-path {{services_manifest}} --workspace
  cargo test --manifest-path {{services_manifest}} --workspace
  cargo clippy --manifest-path {{services_manifest}} --workspace --all-targets -- -D warnings
  flutter analyze
  flutter test

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
  cp services/target/release/hearthdeck-bridge "$HOME/.local/bin/"
  cp services/target/release/hearthdeck-daemon "$HOME/.local/bin/"
  cp services/target/release/hearthdeck-input "$HOME/.local/bin/"
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
