# Hearthdeck off-site backup

Optional nightly `rclone` upload of game saves, database dumps and Hearthdeck
metadata. It complements the local mirrors in `deploy/immich` and `deploy/romm`:
those put a second copy on the external disk, this puts a third copy in the
cloud.

The runner, filters and systemd units are **owned by the `hearthdeck` package**,
so `pacman -Syu` updates them. Only the destination, the rclone credentials and
the decision to enable the timer belong to the user. The design and the future
Settings contract are in [`docs/backup-settings.md`](../../docs/backup-settings.md).

`rclone` is an **optional** dependency of `hearthdeck`. The launcher works
without it.

## Install

Run as the desktop user, not root:

```bash
sudo pacman -S --needed rclone
deploy/backup/install-backup.sh
```

The installer:

- writes `~/.config/hearthdeck-backup/backup.env` with the remote and
  destination root (only if absent);
- enables `hearthdeck-backup.timer` **only** when that rclone remote already
  exists.

Without the remote it stops and prints the `rclone config` steps. Configure the
remote, then run the installer again, or enable the timer by hand:

```bash
rclone config            # new remote, name it pcloud, type pcloud
rclone lsd pcloud:

systemctl --user enable --now hearthdeck-backup.timer
/usr/lib/hearthdeck/hearthdeck-backup --dry-run
```

For pCloud EU accounts, set the advanced option `hostname = eapi.pcloud.com`
before authenticating.

## What runs

| Path | Owner | Purpose |
| --- | --- | --- |
| `/usr/lib/hearthdeck/hearthdeck-backup` | package | runner |
| `/usr/lib/hearthdeck/hearthdeck-backup.d/jobs.conf` | package | default jobs |
| `/usr/lib/hearthdeck/hearthdeck-backup.d/filters/` | package | default filters |
| `/usr/lib/systemd/user/hearthdeck-backup.{service,timer}` | package | schedule, disabled |
| `~/.config/hearthdeck-backup/backup.env` | user | remote + destination root |
| `~/.config/hearthdeck-backup/jobs.conf` | user | optional override |
| `~/.config/hearthdeck-backup/filters/` | user | optional override |
| `~/.config/rclone/rclone.conf` | user | cloud credentials |
| `~/.local/state/hearthdeck-backup/` | user | DB snapshots and lock |

A user `jobs.conf` or `filters/` is used only when it exists; otherwise the
packaged copy is read. That keeps upgrades effective without touching user
configuration.

## Safety rules

- `copy` jobs (saves, config, metadata) never delete anything on the remote.
- `sync` jobs are used only for rotating dump folders and DB snapshots.
- A `sync` job is skipped when its source is empty, when the newest dump is
  corrupt or older than 72h, or when a DB snapshot could not be created.
- If `/mnt/external` is not mounted or its `.hearthdeck-backup` sentinel is
  missing, every job sourced from that disk is skipped. An unmounted disk can
  never be mirrored as remote deletions.
- `sync` is capped at 20 deletions per run (`--max-delete 20`).
- The database is copied with `sqlite3 .backup` and checked with
  `PRAGMA quick_check`; the newest 14 snapshots are kept.

## Adding an AWS Glacier job (photos, later)

Create an S3 remote named `aws-glacier`, then add one line to a user
`~/.config/hearthdeck-backup/jobs.conf` copied from the packaged default:

```bash
"immich-photos|/mnt/immich/data/upload|aws-glacier:$HEARTHDECK_BACKUP_ROOT/photos|copy|photos.rclone|--s3-storage-class=DEEP_ARCHIVE"
```

`copy` keeps photos append-only, and `--s3-storage-class=DEEP_ARCHIVE` puts them
in the cheap archive tier. Photos are intentionally not part of the nightly
pCloud run.

## Restore

```bash
# List what is stored
rclone lsf -R pcloud:hearthdeck-backups/retroarch/saves

# Restore the game saves into a scratch directory
rclone copy pcloud:hearthdeck-backups/retroarch/saves ~/restore/saves

# Restore the newest DB snapshot
rclone copy pcloud:hearthdeck-backups/hearthdeck/db ~/restore/db
ls -t ~/restore/db/*.db | head -1
```

## Manual run

```bash
/usr/lib/hearthdeck/hearthdeck-backup --dry-run   # validate, show transfers
/usr/lib/hearthdeck/hearthdeck-backup             # real run
systemctl --user start hearthdeck-backup          # run through systemd
journalctl --user -u hearthdeck-backup -n 50
```
