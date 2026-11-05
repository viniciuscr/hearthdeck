#!/usr/bin/env bash
# install-romm-backup - install the nightly RomM database dump.
#
# Run as the desktop user, not root: these are user units, and the containers
# and ~/.local/bin belong to the desktop user. The script installs the files
# and enables the timer; it does not start a dump, so the first one can be
# inspected by hand.
#
# Usage:
#   ./install-backup.sh
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

die() { echo "install-romm-backup: $*" >&2; exit 1; }

for arg in "$@"; do
  case $arg in
    -h|--help)
      echo "usage: $0"
      exit 0
      ;;
    *) die "unknown argument: $arg (usage: $0)" ;;
  esac
done

[[ $EUID -ne 0 ]] || die "run as the desktop user, not root"
command -v systemctl >/dev/null || die "systemctl is not installed"

install -Dm755 "$SCRIPT_DIR/romm-backup" "$HOME/.local/bin/romm-backup"
install -Dm644 "$SCRIPT_DIR/romm-backup.service" \
  "$HOME/.config/systemd/user/romm-backup.service"
install -Dm644 "$SCRIPT_DIR/romm-backup.timer" \
  "$HOME/.config/systemd/user/romm-backup.timer"

ENV_FILE=$HOME/.config/romm-backup.env
if [[ ! -e $ENV_FILE ]]; then
  install -Dm644 "$SCRIPT_DIR/romm-backup.env.example" "$ENV_FILE"
fi

systemctl --user daemon-reload
systemctl --user enable --now romm-backup.timer

cat <<EOF

Installed and enabled romm-backup.timer.

Run it once now and read the result:

  systemctl --user start romm-backup.service
  journalctl --user -u romm-backup -n 50 --no-pager

The dump goes to /mnt/external/romM/backups. RomM must be running; the script
fails loudly rather than starting the stack by itself.
EOF
