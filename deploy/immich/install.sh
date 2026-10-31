#!/usr/bin/env bash
# install-immich - install the Immich Quadlet stack for an existing library disk.
#
# This is the reinstall path, not the original provisioning script. It mounts an
# existing ext4 filesystem, creates the service user, installs the units checked
# in beside this script, and starts the pod. It never formats a disk and never
# deletes library data. For a brand-new blank disk, partition and format it
# first (docs/immich-backup.md, step 1), then run this.
#
# Usage:
#   sudo ./install.sh
#   sudo IMMICH_DEVICE=/dev/sdb1 IMMICH_LAN_SUBNET=192.168.178.0/24 ./install.sh
#
# Environment:
#   IMMICH_DEVICE       partition holding the library (default: filesystem label "immich")
#   IMMICH_MOUNT        mount point (default: /mnt/immich)
#   IMMICH_USER         service user (default: immich)
#   IMMICH_LAN_SUBNET   optional ufw source subnet for port 2283
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
MOUNT=${IMMICH_MOUNT:-/mnt/immich}
USER_NAME=${IMMICH_USER:-immich}
DEVICE=${IMMICH_DEVICE:-}

die() { echo "install-immich: $*" >&2; exit 1; }

[[ $EUID -eq 0 ]] || die "run this with sudo"
command -v podman >/dev/null || die "podman is not installed"
command -v openssl >/dev/null || die "openssl is not installed"

if [[ -z $DEVICE ]]; then
  DEVICE=$(blkid -L immich -o device) || true
fi
[[ -n $DEVICE && -b $DEVICE ]] || die "no block device found; set IMMICH_DEVICE=/dev/sdX1"

FSTYPE=$(blkid -s TYPE -o value "$DEVICE" || true)
[[ $FSTYPE == ext4 ]] || die "$DEVICE is $FSTYPE, expected ext4; refusing to continue"
UUID=$(blkid -s UUID -o value "$DEVICE" || true)
[[ -n $UUID ]] || die "$DEVICE has no filesystem UUID"

mkdir -p "$MOUNT"

if mountpoint -q "$MOUNT"; then
  MOUNTED_UUID=$(findmnt -no UUID "$MOUNT")
  [[ $MOUNTED_UUID == "$UUID" ]] || die "$MOUNT is mounted from UUID $MOUNTED_UUID, not $UUID"
else
  OTHER_MOUNT=$(findmnt -no TARGET "$DEVICE" 2>/dev/null || true)
  [[ -z $OTHER_MOUNT ]] || die "$DEVICE is already mounted at $OTHER_MOUNT"

  if grep -qE "[[:space:]]${MOUNT}[[:space:]]" /etc/fstab; then
    grep -qE "^UUID=${UUID}[[:space:]]+${MOUNT}[[:space:]]" /etc/fstab \
      || die "/etc/fstab already mounts $MOUNT from a different device; fix it by hand"
  else
    cp /etc/fstab "/etc/fstab.bak.$(date +%Y%m%d%H%M%S)"
    echo "UUID=$UUID $MOUNT ext4 defaults,noatime,nofail,x-systemd.device-timeout=15s 0 2" \
      >> /etc/fstab
    systemctl daemon-reload
  fi
  mount "$MOUNT"
fi

[[ -d "$MOUNT/data" ]] || die "$MOUNT/data does not exist; is this the Immich disk?"

if ! id "$USER_NAME" >/dev/null 2>&1; then
  useradd -m -s /bin/bash "$USER_NAME"
fi
loginctl enable-linger "$USER_NAME"

# useradd normally allocates these. The fallback keeps the original 200000
# range, which is only safe when no other user already owns it.
grep -q "^$USER_NAME:" /etc/subuid || usermod --add-subuids 200000-265535 "$USER_NAME"
grep -q "^$USER_NAME:" /etc/subgid || usermod --add-subgids 200000-265535 "$USER_NAME"

if [[ $(stat -c %u "$MOUNT/data") != $(id -u "$USER_NAME") ]]; then
  echo "setting ownership of $MOUNT/data to $USER_NAME"
  chown -R "$USER_NAME:$USER_NAME" "$MOUNT/data"
fi
chmod 750 "$MOUNT/data"

QUADLET_DIR="/home/$USER_NAME/.config/containers/systemd"
install -d -o "$USER_NAME" -g "$USER_NAME" "$QUADLET_DIR"
chown "$USER_NAME:$USER_NAME" "$QUADLET_DIR"
for unit in immich.pod immich-db.container immich-redis.container \
            immich-ml.container immich-server.container; do
  install -m 0644 -o "$USER_NAME" -g "$USER_NAME" \
    "$SCRIPT_DIR/$unit" "$QUADLET_DIR/$unit"
done

U=$(id -u "$USER_NAME")
systemctl start "user@$U.service" 2>/dev/null || true
for _ in $(seq 1 40); do
  [[ -S "/run/user/$U/bus" ]] && break
  sleep 0.5
done
[[ -S "/run/user/$U/bus" ]] || die "user manager bus for $USER_NAME is not available"

as_user() {
  runuser -u "$USER_NAME" -- env HOME="/home/$USER_NAME" XDG_RUNTIME_DIR="/run/user/$U" "$@"
}

if ! as_user podman secret inspect immich-db-password >/dev/null 2>&1; then
  if as_user podman volume exists immich-db; then
    die "the immich-db volume exists but the immich-db-password secret is missing; a new password would lock you out of the existing database"
  fi
  openssl rand -hex 24 | tr -d '\n' | as_user podman secret create immich-db-password -
fi

if ufw status 2>/dev/null | grep -q "Status: active"; then
  if [[ -n ${IMMICH_LAN_SUBNET:-} ]]; then
    ufw allow from "$IMMICH_LAN_SUBNET" to any port 2283 proto tcp
  else
    echo "ufw is active; open 2283 for your LAN by hand or rerun with IMMICH_LAN_SUBNET=..."
  fi
fi

echo "starting Immich; the first start pulls about 3.6 GB of images"
as_user systemctl --user daemon-reload
as_user systemctl --user start immich-pod
as_user systemctl --user --no-pager status immich-pod || true

cat <<EOF

Immich is starting at http://<server-lan-ip>:2283

On a fresh machine the database is empty. On the welcome screen choose
"Restore from backup" and pick the newest dump in $MOUNT/data/backups.
If the database already exists, do not restore anything.

Logs: sudo machinectl shell $USER_NAME@   then   podman logs -f immich-server
EOF
