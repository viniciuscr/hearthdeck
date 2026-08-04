#!/usr/bin/env bash
# Boots a virtual display + VNC bridge, then builds and runs the real
# hearthdeck-daemon/hearthdeck-bridge services and the Flutter UI against
# them, mirroring what scripts/dev does for local development on other
# platforms (adapted here since that script assumes `mise`/`just`, which this
# throwaway image intentionally skips in favor of directly installed tools).
set -euo pipefail

display_num="${DISPLAY#:}"
screen_geometry="${HEARTHDECK_DOCKER_SCREEN:-1920x1080x24}"

log() { echo "[docker] $*"; }

# GTK/GLib expect both of these to exist even in a minimal container; without
# them the app aborts on startup trying to read D-Bus connections.
export XDG_RUNTIME_DIR=/tmp/xdg-runtime
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

# The gamepads_linux plugin's connection listener does a plain opendir() on
# this path at startup (no udev needed) and throws an uncaught C++ exception
# -- aborting the whole process -- if it doesn't exist, which it won't in a
# container with no real input devices.
mkdir -p /dev/input

log "starting Xvfb on ${DISPLAY} (${screen_geometry})"
Xvfb ":${display_num}" -screen 0 "$screen_geometry" -nolisten tcp &
xvfb_pid=$!
for _ in $(seq 1 50); do
  [[ -S "/tmp/.X11-unix/X${display_num}" ]] && break
  sleep 0.1
done

log "starting openbox"
openbox &
openbox_pid=$!

log "starting x11vnc on :5900"
x11vnc -display "$DISPLAY" -forever -shared -nopw -rfbport 5900 -quiet &
x11vnc_pid=$!

log "starting a D-Bus session bus"
eval "$(dbus-launch --sh-syntax)"

log "starting noVNC on http://localhost:6080/vnc.html"
websockify --web=/opt/noVNC 6080 localhost:5900 &
novnc_pid=$!

flutter_pid=""
daemon_pid=""
bridge_pid=""

cleanup() {
  log "shutting down"
  kill "$flutter_pid" "$daemon_pid" "$bridge_pid" \
    "$novnc_pid" "$x11vnc_pid" "$openbox_pid" "$xvfb_pid" \
    "${DBUS_SESSION_BUS_PID:-}" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

cd /workspace

case "${HEARTHDECK_DOCKER_MODE:-dev}" in
  shell)
    log "HEARTHDECK_DOCKER_MODE=shell: dropping into an interactive shell instead of launching the app."
    log "The display/VNC stack above is still running; try 'flutter run -d linux' or 'just check-services' yourself."
    exec bash
    ;;
  ui-only)
    log "HEARTHDECK_DOCKER_MODE=ui-only: launching the Flutter UI against mock data, no daemon/bridge build."
    log "flutter pub get"
    flutter pub get
    log "launching the Flutter app -- open http://localhost:6080/vnc.html to view it"
    flutter run -d linux &
    flutter_pid=$!
    wait "$flutter_pid"
    exit $?
    ;;
esac

log "flutter pub get"
flutter pub get

log "building hearthdeck-daemon and hearthdeck-bridge (debug)"
cargo build --manifest-path services/Cargo.toml --workspace

runtime_dir=$(mktemp -d /tmp/hearthdeck-docker.XXXXXX)
mkdir -p "$runtime_dir/runtime" "$runtime_dir/data"

env \
  XDG_RUNTIME_DIR="$runtime_dir/runtime" \
  XDG_DATA_HOME="$runtime_dir/data" \
  HEARTHDECK_BRIDGE_SOCKET="$runtime_dir/runtime/bridge.sock" \
  ./services/target/debug/hearthdeck-bridge > "$runtime_dir/bridge.log" 2>&1 &
bridge_pid=$!

env \
  XDG_RUNTIME_DIR="$runtime_dir/runtime" \
  XDG_DATA_HOME="$runtime_dir/data" \
  HEARTHDECK_BRIDGE_SOCKET="$runtime_dir/runtime/bridge.sock" \
  HEARTHDECK_DATABASE_PATH="$runtime_dir/data/hearthdeck.db" \
  HEARTHDECK_BIND_ADDRESS="127.0.0.1:38400" \
  HEARTHDECK_LOCAL_ADMIN_ADDRESS="127.0.0.1:38401" \
  ./services/target/debug/hearthdeck-daemon > "$runtime_dir/daemon.log" 2>&1 &
daemon_pid=$!

log "waiting for the daemon to become healthy"
healthy=false
for _ in $(seq 1 100); do
  if curl --fail --silent http://127.0.0.1:38400/v1/health > /dev/null; then
    healthy=true
    break
  fi
  if ! kill -0 "$bridge_pid" 2>/dev/null || ! kill -0 "$daemon_pid" 2>/dev/null; then
    break
  fi
  sleep 0.1
done
if [[ "$healthy" != "true" ]]; then
  echo "[docker] daemon/bridge did not become healthy; logs follow:" >&2
  cat "$runtime_dir/bridge.log" "$runtime_dir/daemon.log" >&2
  exit 1
fi

log "pairing a local client"
pairing_code=$(curl --fail --silent -X POST http://127.0.0.1:38401/v1/pairing | jq -r '.code')
pairing_token=$(curl --fail --silent -X POST http://127.0.0.1:38400/v1/pairing/complete \
  -H 'content-type: application/json' \
  --data "{\"code\":\"$pairing_code\",\"client_name\":\"hearthdeck-docker\"}" | jq -r '.token')

log "launching the Flutter app -- open http://localhost:6080/vnc.html to view it"
flutter run -d linux \
  --dart-define=HEARTHDECK_BACKEND_URL=http://127.0.0.1:38400 \
  --dart-define=HEARTHDECK_PAIRING_TOKEN="$pairing_token" &
flutter_pid=$!

wait "$flutter_pid"
