#!/usr/bin/env bash
# mount-external - mount the backup SSD at /mnt/external and record it in fstab.
#
# This is the reinstall path for the second disk: it identifies the filesystem by
# label or UUID, never formats, and only appends an /etc/fstab entry when one is
# missing. It is safe to run more than once.
#
# The disk is exFAT, so it has no Unix ownership of its own. The fstab line
# assigns it to the desktop user, which is who runs the dump timers. `nofail`
# keeps the machine bootable when the disk is unplugged; the backup scripts
# refuse to write unless their mount is present.
#
# Usage:
#   sudo ./mount-external.sh
#   sudo ./mount-external.sh --device /dev/sdc2
#   ./mount-external.sh --dry-run
#
# Environment:
#   EXTERNAL_DEVICE  partition to mount (default: filesystem label below)
#   EXTERNAL_LABEL   label to look for when no device is given
#   EXTERNAL_UUID    UUID to look for when no device or label is given
#   EXTERNAL_MOUNT   mount point (default: /mnt/external)
#   EXTERNAL_USER    owner for the exFAT uid/gid (default: the sudo user, or
#                    the current user when run unprivileged)
set -euo pipefail

MOUNT=${EXTERNAL_MOUNT:-/mnt/external}
LABEL=${EXTERNAL_LABEL:-Crucial X10}
UUID_ARG=${EXTERNAL_UUID:-}
DEVICE=${EXTERNAL_DEVICE:-}
USER_NAME=${EXTERNAL_USER:-${SUDO_USER:-$(id -un)}}
DRY_RUN=0

die() { echo "mount-external: $*" >&2; exit 1; }

usage() {
  cat <<EOF
usage: $0 [--device /dev/sdX2] [--dry-run]

Mounts the exFAT backup disk at $MOUNT and adds the matching /etc/fstab entry
when it is missing. It never formats anything.

Environment: EXTERNAL_DEVICE, EXTERNAL_LABEL, EXTERNAL_UUID, EXTERNAL_MOUNT,
EXTERNAL_USER.
EOF
}

while (($#)); do
  case $1 in
    --device)
      [[ -n ${2:-} ]] || die "--device needs a path"
      DEVICE=$2
      shift 2
      ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
done

if [[ $DRY_RUN == 0 ]]; then
  [[ $EUID -eq 0 ]] || die "run this with sudo (or use --dry-run)"
fi

for cmd in blkid findmnt lsblk; do
  command -v "$cmd" >/dev/null || die "$cmd is not installed"
done

fs_type() { lsblk -no FSTYPE -- "$1" | head -1; }
fs_uuid() { lsblk -no UUID -- "$1" | head -1; }

# Identify the filesystem, never the device letter: sdX moves between boots.
if [[ -z $DEVICE && -n $UUID_ARG ]]; then
  DEVICE=$(blkid -U "$UUID_ARG" -o device) || true
fi
if [[ -z $DEVICE ]]; then
  DEVICE=$(blkid -L "$LABEL" -o device) || true
fi
[[ -n $DEVICE && -b $DEVICE ]] \
  || die "no block device found; set EXTERNAL_DEVICE=/dev/sdX2 or EXTERNAL_UUID=..."

FSTYPE=$(fs_type "$DEVICE")
[[ $FSTYPE == exfat ]] \
  || die "$DEVICE is ${FSTYPE:-unknown}, expected exfat; refusing to continue"
UUID=$(fs_uuid "$DEVICE")
[[ -n $UUID ]] || die "$DEVICE has no filesystem UUID"

if [[ -z $USER_NAME || $USER_NAME == root ]]; then
  die "cannot determine the desktop user; set EXTERNAL_USER=<name>"
fi
id "$USER_NAME" >/dev/null 2>&1 || die "user $USER_NAME does not exist"
OWNER_UID=$(id -u "$USER_NAME")
OWNER_GID=$(id -g "$USER_NAME")
OPTS="defaults,nofail,uid=$OWNER_UID,gid=$OWNER_GID,umask=022,x-systemd.device-timeout=20,x-gvfs-show,x-gvfs-name=External"

if [[ $(findmnt -rn -o TARGET -- "$MOUNT" 2>/dev/null || true) == "$MOUNT" ]]; then
  MOUNTED_UUID=$(findmnt -no UUID "$MOUNT")
  [[ $MOUNTED_UUID == "$UUID" ]] \
    || die "$MOUNT is mounted from UUID $MOUNTED_UUID, not $UUID"
  echo "mount-external: $MOUNT already mounted from $DEVICE (UUID $UUID)"
else
  OTHER_MOUNT=$(findmnt -rn -o TARGET --source "$DEVICE" 2>/dev/null | head -1 || true)
  [[ -z $OTHER_MOUNT ]] \
    || die "$DEVICE is already mounted at $OTHER_MOUNT; unmount it first"

  if awk -v m="$MOUNT" '$1 !~ /^#/ && $2 == m {found=1} END{exit !found}' /etc/fstab; then
    awk -v m="$MOUNT" -v u="UUID=$UUID" \
      '$1 == u && $2 == m {ok=1} END{exit !ok}' /etc/fstab \
      || die "/etc/fstab already mounts $MOUNT from a different device; fix it by hand"
    echo "mount-external: /etc/fstab already has UUID=$UUID at $MOUNT"
  elif [[ $DRY_RUN == 1 ]]; then
    echo "mount-external: would append to /etc/fstab:"
    echo "  UUID=$UUID $MOUNT exfat $OPTS 0 0"
  else
    cp /etc/fstab "/etc/fstab.bak.$(date +%Y%m%d%H%M%S)"
    printf '%s\n' "UUID=$UUID $MOUNT exfat $OPTS 0 0" >>/etc/fstab
    systemctl daemon-reload
    echo "mount-external: added UUID=$UUID to /etc/fstab"
  fi

  if [[ $DRY_RUN == 1 ]]; then
    echo "mount-external: would mount $DEVICE at $MOUNT"
  else
    mkdir -p "$MOUNT"
    mount "$MOUNT"
  fi
fi

if [[ $DRY_RUN == 1 ]]; then
  echo "mount-external: dry run; nothing was mounted or written"
  exit 0
fi

findmnt -rn -o TARGET,SOURCE,FSTYPE "$MOUNT"

# The sentinel is created by the off-site backup setup. Requiring it here stops a
# different exFAT stick from being mounted over the backup path by mistake.
[[ -e "$MOUNT/.hearthdeck-backup" ]] \
  || die "mounted, but $MOUNT/.hearthdeck-backup is missing; is this the right disk? (touch it only for a new backup disk)"

echo "mount-external: $MOUNT is ready (owner $USER_NAME, UUID $UUID)"
