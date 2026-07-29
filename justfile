set shell := ["zsh", "-cu"]

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
check-app:
  mise exec -- flutter analyze
  mise exec -- flutter test
  mise exec -- flutter build macos --debug

# Format Flutter and Rust source files.
format:
  mise exec -- dart format lib test
  mise exec -- cargo fmt --manifest-path services/Cargo.toml --all

# Analyze and test the Flutter client.
test-app:
  mise exec -- flutter analyze
  mise exec -- flutter test

# Build, test, and lint the Rust backend workspace.
check-services:
  mise exec -- cargo check --manifest-path services/Cargo.toml --workspace
  mise exec -- cargo test --manifest-path services/Cargo.toml --workspace
  mise exec -- cargo clippy --manifest-path services/Cargo.toml --workspace --all-targets -- -D warnings

# Build optimized Linux-host service binaries.
build-services:
  mise exec -- cargo build --manifest-path services/Cargo.toml --workspace --release

# Build debug service binaries for the combined development target.
build-services-debug:
  mise exec -- cargo build --manifest-path services/Cargo.toml --workspace

# Run the Linux integration bridge in the foreground.
bridge:
  mise exec -- cargo run --manifest-path services/Cargo.toml -p hearthdeck-bridge

# Run the local API daemon in the foreground.
daemon:
  mise exec -- cargo run --manifest-path services/Cargo.toml -p hearthdeck-daemon

# Scan installed macOS application bundles through the real provider.
macos-discovery-check: build-services-debug
  ./scripts/macos-discovery-check

# Request a one-time pairing code from the loopback admin listener.
pairing-code:
  curl --fail --silent --show-error -X POST http://127.0.0.1:38401/v1/pairing

# Request one metadata provider refresh from a paired daemon.
refresh-metadata url token provider="appstream-local":
  curl --fail --silent --show-error -X POST {{url}}/v1/metadata/{{provider}}/refresh -H 'Authorization: Bearer {{token}}'

# Run all portable project checks.
check: format check-services test-app

# Run source validation in CI after Flutter and Rust have been installed.
ci-check:
  flutter pub get
  dart format --output=none --set-exit-if-changed lib test
  cargo fmt --manifest-path services/Cargo.toml --all -- --check
  cargo check --manifest-path services/Cargo.toml --workspace
  cargo test --manifest-path services/Cargo.toml --workspace
  cargo clippy --manifest-path services/Cargo.toml --workspace --all-targets -- -D warnings
  flutter analyze
  flutter test

# Build the Arch Linux package from the current source checkout.
ci-package-arch:
  cd packaging/arch && makepkg --cleanbuild --noconfirm

# Install Linux systemd user units and enable the local services.
install-services:
  mkdir -p "$HOME/.config/systemd/user" "$HOME/.local/bin"
  systemctl --user disable --now ltv-bridge.service ltv-daemon.service 2>/dev/null || true
  if [[ -d "$HOME/.config/ltv" && ! -e "$HOME/.config/hearthdeck" ]]; then mv "$HOME/.config/ltv" "$HOME/.config/hearthdeck"; fi
  if [[ -d "$HOME/.local/share/ltv" && ! -e "$HOME/.local/share/hearthdeck" ]]; then mv "$HOME/.local/share/ltv" "$HOME/.local/share/hearthdeck"; fi
  if [[ -f "$HOME/.config/hearthdeck/daemon.env" ]]; then perl -pi -e 's/LTV_/HEARTHDECK_/g; s#/.config/ltv/#/.config/hearthdeck/#g' "$HOME/.config/hearthdeck/daemon.env"; fi
  rm -f "$HOME/.config/systemd/user/ltv-bridge.service" "$HOME/.config/systemd/user/ltv-daemon.service"
  cp deploy/systemd/hearthdeck-bridge.service "$HOME/.config/systemd/user/"
  cp deploy/systemd/hearthdeck-daemon.service "$HOME/.config/systemd/user/"
  cp services/target/release/hearthdeck-bridge "$HOME/.local/bin/"
  cp services/target/release/hearthdeck-daemon "$HOME/.local/bin/"
  systemctl --user daemon-reload
  systemctl --user enable --now hearthdeck-bridge.service hearthdeck-daemon.service

# Show Linux service status.
services-status:
  systemctl --user status hearthdeck-bridge.service hearthdeck-daemon.service

# Follow Linux service logs.
services-logs:
  journalctl --user -fu hearthdeck-bridge.service -u hearthdeck-daemon.service

# Follow structured daemon logs only.
logs-daemon:
  journalctl --user -fu hearthdeck-daemon.service -o cat

# Follow structured Linux bridge logs only.
logs-bridge:
  journalctl --user -fu hearthdeck-bridge.service -o cat

# Show recent warning and error logs from both services.
logs-errors:
  journalctl --user -u hearthdeck-bridge.service -u hearthdeck-daemon.service -p warning --since '1 hour ago' -o cat
