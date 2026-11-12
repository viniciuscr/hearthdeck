#!/usr/bin/env bash
# pre-wipe-check - read-only proof that the machine-wipe backups are ready.
#
# Run as the desktop user (the user whose systemd manager runs the dump
# timers). Nothing here mounts, unmounts, writes, or starts a unit. It checks
# the two mounts, their fstab entries, the newest Immich and RomM dumps, the
# mirror copies, the timers, group access, the live services, and free space.
#
# Exit status: 0 if every hard check passes, 1 otherwise. Warnings do not fail.
#
# Environment:
#   PRE_WIPE_MAX_DUMP_AGE_HOURS  hard limit for the newest dump (default 24)
#   PRE_WIPE_WARN_FRESH_HOURS    warn when older than this (default 12)
#   PRE_WIPE_SKIP_PUBLIC         1 to skip the public hostname check
set -uo pipefail

MAX_AGE=${PRE_WIPE_MAX_DUMP_AGE_HOURS:-24}
WARN_AGE=${PRE_WIPE_WARN_FRESH_HOURS:-12}
IMMICH_URL=${IMMICH_URL:-http://127.0.0.1:2283}
ROMM_URL=${ROMM_URL:-http://127.0.0.1:8080}
PUBLIC_URL=${PUBLIC_URL:-https://photos.viniciuscr.com.br}
IMMICH_DATA=${IMMICH_DATA:-/mnt/immich/data}
EXTERNAL=${EXTERNAL:-/mnt/external}
ROMM_BACKUPS=${ROMM_BACKUPS:-$EXTERNAL/romM/backups}

FAIL=0
WARN=0

ok()   { printf 'ok    %s\n' "$*"; }
warn() { printf 'warn  %s\n' "$*"; WARN=$((WARN + 1)); }
bad()  { printf 'FAIL  %s\n' "$*"; FAIL=$((FAIL + 1)); }
note() { printf 'note  %s\n' "$*"; }
section() { printf '\n== %s ==\n' "$*"; }

newest_dump() {
  find "$1" -maxdepth 1 -type f -name '*.sql.gz' -printf '%T@ %p\n' 2>/dev/null \
    | sort -n | tail -1 | cut -d' ' -f2-
}

age_hours() {
  local age=$(( ($(date +%s) - $(stat -c %Y "$1")) / 3600 ))
  (( age < 0 )) && age=0
  echo "$age"
}

check_mount() {
  local mount=$1 uuid line
  if [[ $(findmnt -rn -o TARGET -- "$mount" 2>/dev/null || true) != "$mount" ]]; then
    bad "$mount is not mounted"
    return
  fi

  uuid=$(findmnt -no UUID "$mount")
  ok "$mount mounted from $(findmnt -no SOURCE "$mount") ($(findmnt -no FSTYPE "$mount"), UUID ${uuid:-none})"

  if [[ -z $uuid ]]; then
    warn "$mount has no filesystem UUID; /etc/fstab cannot match it by UUID"
    return
  fi
  if awk -v m="$mount" -v u="UUID=$uuid" \
      '$1 == u && $2 == m {ok=1} END{exit !ok}' /etc/fstab; then
    ok "/etc/fstab has UUID=$uuid at $mount"
    line=$(awk -v m="$mount" '$1 !~ /^#/ && $2 == m {print; exit}' /etc/fstab)
    if [[ $line == *nofail* ]]; then
      ok "$mount fstab entry has nofail"
    else
      warn "$mount fstab entry has no nofail; a missing disk could block boot"
    fi
  else
    bad "/etc/fstab has no UUID=$uuid entry for $mount"
  fi
}

check_timer() {
  local timer=$1
  if systemctl --user is-enabled "$timer" >/dev/null 2>&1; then
    ok "$timer is enabled"
  else
    bad "$timer is not enabled"
  fi
  if systemctl --user is-active "$timer" >/dev/null 2>&1; then
    ok "$timer is active"
  else
    bad "$timer is not active"
  fi
}

check_last_result() {
  local unit=$1 result
  result=$(systemctl --user show "$unit" -p Result --value 2>/dev/null || true)
  case $result in
    success) ok "$unit last result: success" ;;
    '') warn "$unit has no recorded run yet" ;;
    *) bad "$unit last result: $result" ;;
  esac
}

if [[ $EUID -eq 0 ]]; then
  echo "pre-wipe-check: run as the desktop user, not root; the timers are user units" >&2
  exit 2
fi

if [[ -z ${XDG_RUNTIME_DIR:-} || ! -d ${XDG_RUNTIME_DIR:-} ]]; then
  XDG_RUNTIME_DIR="/run/user/$(id -u)"
  export XDG_RUNTIME_DIR
fi
[[ -d $XDG_RUNTIME_DIR ]] || warn "no user runtime directory at $XDG_RUNTIME_DIR; systemctl --user may fail"

printf 'Pre-wipe check for %s (%s)\n' "$(id -un)" "$(date -Is)"

section "Mounts"
check_mount /mnt/immich
check_mount "$EXTERNAL"

IMMICH_SRC=$(findmnt -no SOURCE /mnt/immich 2>/dev/null || true)
EXTERNAL_SRC=$(findmnt -no SOURCE "$EXTERNAL" 2>/dev/null || true)
if [[ -n $IMMICH_SRC && -n $EXTERNAL_SRC && $IMMICH_SRC != "$EXTERNAL_SRC" ]]; then
  ok "library and backup are separate devices ($IMMICH_SRC, $EXTERNAL_SRC)"
else
  bad "library and backup resolve to the same or a missing device"
fi

section "Disk identity"
lsblk -o NAME,SIZE,SERIAL,MODEL,FSTYPE,LABEL,UUID,MOUNTPOINTS | sed 's/^/      /'
note "Do not format the ext4 'immich' disk or the exFAT 'Crucial X10' disk."
note "Letters can change after a reboot; match by label, UUID, or serial."

section "Immich database dumps"
LIB_DUMPS=$IMMICH_DATA/backups
MIRROR_DUMPS=$EXTERNAL/immich/backups
LIB_NEWEST=$(newest_dump "$LIB_DUMPS")
MIRROR_NEWEST=$(newest_dump "$MIRROR_DUMPS")

if [[ -z $LIB_NEWEST ]]; then
  bad "no dump in $LIB_DUMPS"
else
  age=$(age_hours "$LIB_NEWEST")
  if (( age <= MAX_AGE )); then
    ok "newest dump is ${age}h old: $(basename "$LIB_NEWEST")"
  else
    bad "newest dump is ${age}h old (limit ${MAX_AGE}h): $(basename "$LIB_NEWEST")"
  fi
  if (( age > WARN_AGE )); then
    warn "dump is older than ${WARN_AGE}h; create a fresh one immediately before wiping"
  fi
  note "Create a dump by hand immediately before the wipe; this check only proves the newest one is recent."
  if gzip -t "$LIB_NEWEST" 2>/dev/null; then
    ok "newest dump is valid gzip"
  else
    bad "newest dump is not valid gzip"
  fi
  if gzip -dc "$LIB_NEWEST" 2>/dev/null \
      | grep -q '^-- PostgreSQL database dump complete$'; then
    ok "newest dump has the completion marker"
  else
    bad "newest dump has no completion marker"
  fi
  count=$(find "$LIB_DUMPS" -maxdepth 1 -type f -name '*.sql.gz' | wc -l)
  if (( count >= 7 )); then
    ok "$count dumps retained in $LIB_DUMPS"
  else
    warn "only $count dumps in $LIB_DUMPS"
  fi
fi

if [[ -z $MIRROR_NEWEST ]]; then
  bad "no dump in $MIRROR_DUMPS"
else
  if [[ -n $LIB_NEWEST && $(basename "$MIRROR_NEWEST") == "$(basename "$LIB_NEWEST")" ]]; then
    ok "mirror has the newest library dump"
  else
    bad "mirror newest ($(basename "$MIRROR_NEWEST")) is not the library newest"
  fi
  if [[ -n $LIB_NEWEST ]] && cmp -s "$LIB_NEWEST" "$MIRROR_NEWEST"; then
    ok "library and mirror copies are byte-identical ($(sha256sum "$LIB_NEWEST" | cut -c1-12)…)"
  else
    bad "library and mirror copies differ"
  fi
  if gzip -t "$MIRROR_NEWEST" 2>/dev/null; then
    ok "mirror dump is valid gzip"
  else
    bad "mirror dump is not valid gzip"
  fi
  mcount=$(find "$MIRROR_DUMPS" -maxdepth 1 -type f -name '*.sql.gz' | wc -l)
  ok "$mcount dumps in $MIRROR_DUMPS"
fi

section "Immich access and service"
if id -nG "$(id -un)" | tr ' ' '\n' | grep -qx immich; then
  ok "user is in the immich group"
else
  bad "user is not in the immich group; run deploy/immich/install.sh, then log out and back in"
fi
if [[ -r $LIB_DUMPS && -x $LIB_DUMPS ]]; then
  ok "dump directory is readable"
else
  bad "$LIB_DUMPS is not readable"
fi
if id immich >/dev/null 2>&1; then
  want_owner="$(id -u immich):$(id -g immich)"
  have_owner="$(stat -c '%u:%g' "$IMMICH_DATA")"
  if [[ $have_owner == "$want_owner" ]]; then
    ok "$IMMICH_DATA is owned by immich ($want_owner)"
  else
    bad "$IMMICH_DATA is owned by $have_owner, expected $want_owner; rerun install.sh"
  fi
else
  warn "the immich service user does not exist on this host"
fi
if [[ -n $LIB_NEWEST && -r $LIB_NEWEST ]]; then
  ok "newest dump is readable by this user"
else
  bad "newest dump is not readable by this user"
fi

uploads=$(find "$IMMICH_DATA/upload" -type f 2>/dev/null | wc -l)
if (( uploads > 0 )); then
  ok "$uploads original files in $IMMICH_DATA/upload ($(du -sh "$IMMICH_DATA/upload" 2>/dev/null | cut -f1))"
else
  bad "no original files in $IMMICH_DATA/upload"
fi

code=$(curl -fsS -o /dev/null -w '%{http_code}' --max-time 5 "$IMMICH_URL/" 2>/dev/null || true)
if [[ $code == 200 ]]; then
  ok "Immich answers HTTP 200 at $IMMICH_URL"
else
  bad "Immich did not answer 200 at $IMMICH_URL (got ${code:-nothing})"
fi

version_json=$(curl -fsS --max-time 5 "$IMMICH_URL/api/server/version" 2>/dev/null || true)
version=''
if [[ -n $version_json ]]; then
  major=$(sed -n 's/.*"major":\([0-9]\+\).*/\1/p' <<<"$version_json")
  minor=$(sed -n 's/.*"minor":\([0-9]\+\).*/\1/p' <<<"$version_json")
  patch=$(sed -n 's/.*"patch":\([0-9]\+\).*/\1/p' <<<"$version_json")
  [[ -n $major && -n $minor && -n $patch ]] && version="$major.$minor.$patch"
fi
if [[ -n $version ]]; then
  ok "Immich server version $version"
  if [[ -n $LIB_NEWEST && $(basename "$LIB_NEWEST") == *"-v$version-"* ]]; then
    ok "dump filename matches server version v$version"
  else
    bad "dump filename does not carry server version v$version"
  fi
else
  bad "could not read the Immich server version"
fi

section "Cloudflare tunnel"
if systemctl --user is-active cloudflared-immich.service >/dev/null 2>&1; then
  ok "cloudflared-immich.service is active"
else
  bad "cloudflared-immich.service is not active"
fi
state=$(systemctl --user is-enabled cloudflared-immich.service 2>/dev/null || true)
if [[ -n $state ]]; then
  ok "cloudflared unit state: $state"
else
  warn "cloudflared unit is not installed in this user's manager"
fi
TUNNEL_ENV=$HOME/.config/cloudflared/tunnel.env
if [[ -f $TUNNEL_ENV ]]; then
  mode=$(stat -c %a "$TUNNEL_ENV")
  if [[ $mode == 600 ]]; then
    ok "$TUNNEL_ENV is mode 600"
  else
    bad "$TUNNEL_ENV is mode $mode, expected 600"
  fi
  if grep -qE '^TUNNEL_TOKEN=.+' "$TUNNEL_ENV"; then
    ok "$TUNNEL_ENV contains a token"
  else
    bad "$TUNNEL_ENV has no TUNNEL_TOKEN"
  fi
else
  bad "$TUNNEL_ENV is missing; the token is not in git and must be restored or rotated"
fi

if [[ ${PRE_WIPE_SKIP_PUBLIC:-0} != 1 ]]; then
  code=$(curl -fsS -o /dev/null -w '%{http_code}' --max-time 10 "$PUBLIC_URL/" 2>/dev/null || true)
  if [[ $code == 200 ]]; then
    ok "public hostname answers 200 ($PUBLIC_URL)"
  else
    warn "public hostname returned ${code:-nothing}; check the tunnel before relying on it"
  fi
fi

section "RomM"
ROM_NEWEST=$(newest_dump "$ROMM_BACKUPS")
if [[ -z $ROM_NEWEST ]]; then
  bad "no RomM dump in $ROMM_BACKUPS"
else
  age=$(age_hours "$ROM_NEWEST")
  if (( age <= MAX_AGE )); then
    ok "newest RomM dump is ${age}h old: $(basename "$ROM_NEWEST")"
  else
    bad "newest RomM dump is ${age}h old (limit ${MAX_AGE}h)"
  fi
  if gzip -t "$ROM_NEWEST" 2>/dev/null; then
    ok "newest RomM dump is valid gzip"
  else
    bad "newest RomM dump is not valid gzip"
  fi
  tables=$(gzip -dc "$ROM_NEWEST" 2>/dev/null | grep -c '^CREATE TABLE' || true)
  if (( tables >= 20 )); then
    ok "newest RomM dump has $tables tables"
  else
    bad "newest RomM dump has only $tables tables"
  fi
fi
if [[ -f $EXTERNAL/romM/podman-compose.yaml ]]; then
  ok "RomM compose file is present"
else
  bad "RomM compose file is missing"
fi
if [[ -f $EXTERNAL/romM/config/config.yml ]]; then
  ok "RomM config.yml is present"
else
  bad "RomM config.yml is missing"
fi
code=$(curl -fsS -o /dev/null -w '%{http_code}' --max-time 5 "$ROMM_URL/" 2>/dev/null || true)
if [[ $code == 200 ]]; then
  ok "RomM answers HTTP 200 at $ROMM_URL"
else
  bad "RomM did not answer 200 at $ROMM_URL (got ${code:-nothing})"
fi
rom_files=$(find "$EXTERNAL/romM/library" -type f 2>/dev/null | wc -l)
if (( rom_files > 0 )); then
  ok "$rom_files ROM files in $EXTERNAL/romM/library"
else
  bad "no ROM files in $EXTERNAL/romM/library"
fi

section "Timers"
check_timer immich-backup.timer
check_timer romm-backup.timer
check_timer hearthdeck-backup.timer
check_last_result immich-backup.service
check_last_result romm-backup.service
check_last_result hearthdeck-backup.service

section "Free space"
check_space() {
  local path=$1 minimum=$2 avail
  avail=$(df -BG --output=avail "$path" 2>/dev/null | tail -1 | tr -dc '0-9')
  if [[ -z $avail ]]; then
    bad "cannot read free space for $path"
  elif (( avail >= minimum )); then
    ok "$path has ${avail}G free"
  else
    warn "$path has only ${avail}G free (wanted at least ${minimum}G)"
  fi
}
check_space / 5
check_space "$IMMICH_DATA" 10
check_space "$EXTERNAL" 20

printf '\n%d failure(s), %d warning(s)\n' "$FAIL" "$WARN"
if (( FAIL > 0 )); then
  echo "PRE-WIPE CHECK FAILED"
  exit 1
fi
echo "PRE-WIPE CHECK PASSED"
