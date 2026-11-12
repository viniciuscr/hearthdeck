#!/usr/bin/env bash
# install-immich-backup - install the nightly Immich dump/original mirror.
#
# Run as the desktop user, not root: these are user units, and the mirror runs
# as the user who owns ~/.local/bin and ~/.config. The script installs the
# files and enables the timer; it deliberately does not start a copy, because
# the first originals run can be tens of gigabytes.
#
# Usage:
#   ./install-backup.sh
#   ./install-backup.sh --originals
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ORIGINALS=0

die() { echo "install-immich-backup: $*" >&2; exit 1; }

for arg in "$@"; do
  case $arg in
    --originals) ORIGINALS=1 ;;
    -h|--help)
      echo "usage: $0 [--originals]"
      exit 0
      ;;
    *) die "unknown argument: $arg (usage: $0 [--originals])" ;;
  esac
done

[[ $EUID -ne 0 ]] || die "run as the desktop user, not root"
command -v systemctl >/dev/null || die "systemctl is not installed"

if ! id -nG "$(id -un)" | tr ' ' '\n' | grep -qx immich; then
  die "current user is not in the immich group; run sudo deploy/immich/install.sh, then log out and back in before arming the mirror"
fi

install -Dm755 "$SCRIPT_DIR/immich-backup" "$HOME/.local/bin/immich-backup"
install -Dm644 "$SCRIPT_DIR/immich-backup.service" \
  "$HOME/.config/systemd/user/immich-backup.service"
install -Dm644 "$SCRIPT_DIR/immich-backup.timer" \
  "$HOME/.config/systemd/user/immich-backup.timer"

ENV_FILE=$HOME/.config/immich-backup.env
if [[ ! -e $ENV_FILE ]]; then
  install -Dm644 "$SCRIPT_DIR/immich-backup.env.example" "$ENV_FILE"
fi

if [[ $ORIGINALS == 1 ]]; then
  if grep -qE '^[[:space:]]*IMMICH_BACKUP_ORIGINALS=' "$ENV_FILE"; then
    sed -i -E 's|^[[:space:]]*IMMICH_BACKUP_ORIGINALS=.*|IMMICH_BACKUP_ORIGINALS=1|' "$ENV_FILE"
  else
    printf '\nIMMICH_BACKUP_ORIGINALS=1\n' >> "$ENV_FILE"
  fi
fi

systemctl --user daemon-reload
systemctl --user enable --now immich-backup.timer

if [[ $ORIGINALS == 1 ]]; then
  ORIGINALS_STATE="on (the first run copies the whole library)"
else
  ORIGINALS_STATE="off (dumps only)"
fi

cat <<EOF

Installed and enabled immich-backup.timer.

Run it once now and read the result:

  systemctl --user start immich-backup.service
  journalctl --user -u immich-backup -n 50 --no-pager

Originals mirror: $ORIGINALS_STATE
EOF
