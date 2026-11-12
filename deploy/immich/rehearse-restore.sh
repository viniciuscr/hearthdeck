#!/usr/bin/env bash
# rehearse-restore - restore an Immich dump into a disposable Postgres.
#
# Run as the desktop user. This never touches the live immich-db volume or the
# library: it creates a container and volume named immich-restore-rehearsal,
# restores the dump into them, prints row counts, and removes both on exit.
#
# This is the proof that a dump is usable. The checksum checks in
# pre-wipe-check.sh only prove it is the same file on both disks.
#
# Usage:
#   ./rehearse-restore.sh                       # newest dump in the library
#   ./rehearse-restore.sh /path/to/dump.sql.gz
#
# Environment:
#   IMMICH_POSTGRES_IMAGE  image to use (default: the one in immich-db.container)
set -euo pipefail

IMAGE=${IMMICH_POSTGRES_IMAGE:-ghcr.io/immich-app/postgres:14-vectorchord0.4.3-pgvectors0.2.0}
NAME=immich-restore-rehearsal
VOLUME=$NAME
DUMP=${1:-}
LOG=$(mktemp)

die() { echo "rehearse-restore: $*" >&2; exit 1; }

usage() {
  cat <<EOF
usage: $0 [dump.sql.gz]

Restores a dump into a disposable Postgres container and prints the row counts.
With no argument, uses the newest dump in /mnt/immich/data/backups.
EOF
}

case ${1:-} in
  -h|--help) usage; exit 0 ;;
  '') ;;
  *) DUMP=$1; shift ;;
esac
(($# == 0)) || die "too many arguments (try --help)"

[[ $EUID -ne 0 ]] || die "run as the desktop user, not root"
command -v podman >/dev/null || die "podman is not installed"
command -v gzip >/dev/null || die "gzip is not installed"

if [[ -z $DUMP ]]; then
  DUMP=$(find /mnt/immich/data/backups -maxdepth 1 -type f -name '*.sql.gz' \
    -printf '%T@ %p\n' 2>/dev/null | sort -n | tail -1 | cut -d' ' -f2-)
fi
[[ -n $DUMP && -f $DUMP ]] || die "no dump found; pass one explicitly"
gzip -t "$DUMP" || die "not a valid gzip file: $DUMP"

cleanup() {
  podman rm -f "$NAME" >/dev/null 2>&1 || true
  podman volume rm "$VOLUME" >/dev/null 2>&1 || true
  rm -f "$LOG"
}
trap cleanup EXIT

# Only the rehearsal names are removed. A real immich-db volume is untouched.
podman rm -f "$NAME" >/dev/null 2>&1 || true
podman volume rm "$VOLUME" >/dev/null 2>&1 || true

echo "rehearse-restore: starting $IMAGE"
podman run -d --name "$NAME" \
  -e POSTGRES_PASSWORD=rehearsal \
  -e POSTGRES_DB=immich \
  -e POSTGRES_INITDB_ARGS=--data-checksums \
  -v "$VOLUME:/var/lib/postgresql/data" \
  "$IMAGE" >/dev/null

# The entrypoint starts a temporary server for init scripts, stops it, then
# starts the real one. A single pg_isready can answer the temporary server and
# fail once init finishes, so wait for the second "ready" line as well.
ready=0
for _ in $(seq 1 120); do
  starts=$(podman logs "$NAME" 2>&1 | grep -c 'database system is ready to accept connections' || true)
  if (( starts >= 2 )) && podman exec "$NAME" pg_isready -U postgres -d immich >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 1
done
if (( ready == 0 )); then
  podman logs --tail 40 "$NAME" >&2 || true
  die "database did not become ready"
fi

echo "rehearse-restore: restoring $(basename "$DUMP")"
if ! gzip -dc "$DUMP" \
    | podman exec -i "$NAME" psql -U postgres -d immich -v ON_ERROR_STOP=1 --quiet \
    >"$LOG" 2>&1; then
  tail -30 "$LOG" >&2
  die "restore failed"
fi

users=$(podman exec "$NAME" psql -U postgres -d immich -At -c 'SELECT count(*) FROM "user";')
assets=$(podman exec "$NAME" psql -U postgres -d immich -At -c 'SELECT count(*) FROM asset;')
files=$(podman exec "$NAME" psql -U postgres -d immich -At -c 'SELECT count(*) FROM asset_file;')
live=$(podman exec "$NAME" psql -U postgres -d immich -At -c 'SELECT count(*) FROM asset WHERE "deletedAt" IS NULL;')
extensions=$(podman exec "$NAME" psql -U postgres -d immich -At -c \
  'SELECT extname FROM pg_extension ORDER BY 1;' | tr '\n' ' ')

(( users >= 1 )) || die "restored database has no users"
(( assets >= 1 )) || die "restored database has no assets"
(( files >= 1 )) || die "restored database has no asset files"
for ext in cube earthdistance pg_trgm unaccent uuid-ossp vchord vector; do
  case " $extensions " in
    *" $ext "*) ;;
    *) die "restored database is missing extension $ext" ;;
  esac
done

printf 'rehearse-restore: OK from %s\n' "$(basename "$DUMP")"
printf '  users=%s assets=%s asset_files=%s live_assets=%s\n' \
  "$users" "$assets" "$files" "$live"
printf '  extensions=%s\n' "$extensions"
echo "  container and volume removed"
