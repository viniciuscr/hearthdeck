#!/usr/bin/env bash
# install-backup - create the per-user configuration for the packaged backup.
#
# Run as the desktop user, not root. The runner, filters and units belong to the
# hearthdeck package; this script only writes the user-owned destination file
# and enables the timer once the rclone remote exists.
#
# The Settings screen will eventually do the same thing through the daemon. Until
# then this is the supported setup path.
#
# Usage:
#   ./install-backup.sh
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
CONFIG_DIR=${HEARTHDECK_BACKUP_CONFIG:-"$HOME/.config/hearthdeck-backup"}
ENV_FILE=$CONFIG_DIR/backup.env
REMOTE=${HEARTHDECK_BACKUP_REMOTE:-pcloud}
ROOT=${HEARTHDECK_BACKUP_ROOT:-hearthdeck-backups}
RUNNER=/usr/lib/hearthdeck/hearthdeck-backup

die() { echo "install-hearthdeck-backup: $*" >&2; exit 1; }

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
command -v rclone >/dev/null || die "rclone is not installed; run: sudo pacman -S rclone"
[[ -x $RUNNER ]] || die "$RUNNER is missing; install or update the hearthdeck package first"

[[ $REMOTE =~ ^[A-Za-z0-9._-]+$ ]] || die "invalid rclone remote name: $REMOTE"
[[ $ROOT =~ ^[A-Za-z0-9._/-]+$ ]] || die "invalid destination root: $ROOT"

install -d -m 700 "$CONFIG_DIR"
if [[ ! -e $ENV_FILE ]]; then
  umask 077
  cat >"$ENV_FILE" <<EOF
# Hearthdeck off-site backup destination.
# The Settings screen will manage this file; it is safe to edit by hand too.
HEARTHDECK_BACKUP_REMOTE=$REMOTE
HEARTHDECK_BACKUP_ROOT=$ROOT
EOF
  chmod 600 "$ENV_FILE"
fi

systemctl --user daemon-reload

if rclone listremotes 2>/dev/null | grep -qx "$REMOTE:"; then
  systemctl --user enable --now hearthdeck-backup.timer
  cat <<EOF

Installed user configuration at $ENV_FILE and enabled hearthdeck-backup.timer
(nightly at 04:30).

Run it once now and read the result:

  $RUNNER --dry-run
  systemctl --user start hearthdeck-backup.service
  journalctl --user -u hearthdeck-backup -n 50 --no-pager
EOF
else
  cat <<EOF

Wrote $ENV_FILE, but did not enable the timer: the rclone remote
"$REMOTE" does not exist yet. Configure it, then run this installer again:

  rclone config          # new remote, name it "$REMOTE", type pcloud
  rclone lsd "$REMOTE:"

  "$SCRIPT_DIR/install-backup.sh"
  systemctl --user enable --now hearthdeck-backup.timer
  $RUNNER --dry-run

If this is a pCloud EU account, set the advanced option
  hostname = eapi.pcloud.com
before authenticating.
EOF
fi
