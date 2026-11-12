#!/usr/bin/env bash
# install-cloudflared - install the Immich Cloudflare Tunnel Quadlet.
#
# Run as the desktop user, not root: the tunnel is a user Quadlet and the token
# belongs to that user. This script installs only the unit. The token is not in
# this repository and is not created here; restore ~/.config/cloudflared/tunnel.env
# (mode 0600) first, or rotate the token in the Cloudflare dashboard.
#
# Usage:
#   ./install-cloudflared.sh
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
UNIT=cloudflared-immich.container
UNIT_DIR=$HOME/.config/containers/systemd
ENV_DIR=$HOME/.config/cloudflared
ENV_FILE=$ENV_DIR/tunnel.env
SERVICE=cloudflared-immich.service

die() { echo "install-cloudflared: $*" >&2; exit 1; }

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

[[ -f $ENV_FILE ]] \
  || die "missing $ENV_FILE; create it with TUNNEL_TOKEN=<token> and mode 0600"
mode=$(stat -c %a "$ENV_FILE")
[[ $mode == 600 ]] || die "$ENV_FILE is mode $mode, expected 600"
grep -qE '^TUNNEL_TOKEN=.+' "$ENV_FILE" \
  || die "$ENV_FILE has no non-empty TUNNEL_TOKEN"

if [[ -d $ENV_DIR && $(stat -c %a "$ENV_DIR") != 700 ]]; then
  echo "install-cloudflared: warning: $ENV_DIR is mode $(stat -c %a "$ENV_DIR"), expected 700" >&2
fi

install -d "$UNIT_DIR"
changed=0
if ! cmp -s "$SCRIPT_DIR/$UNIT" "$UNIT_DIR/$UNIT"; then
  install -m 0644 "$SCRIPT_DIR/$UNIT" "$UNIT_DIR/$UNIT"
  changed=1
fi

systemctl --user daemon-reload

if [[ $changed == 1 ]]; then
  systemctl --user restart "$SERVICE"
else
  systemctl --user start "$SERVICE"
fi

if [[ $(loginctl show-user "$(id -un)" -p Linger --value 2>/dev/null || true) != yes ]]; then
  echo "install-cloudflared: warning: linger is off; run: sudo loginctl enable-linger $(id -un)" >&2
fi

systemctl --user --no-pager status "$SERVICE" || true

cat <<EOF

Installed $UNIT.

The token came from $ENV_FILE and was not touched.
Check the tunnel from the public side after Immich is up:

  curl -sS -o /dev/null -w '%{http_code}\n' https://photos.viniciuscr.com.br/
  journalctl --user -u $SERVICE -n 30 --no-pager

EOF
