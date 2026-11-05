# RomM backup on the external SSD (Podman Compose)

RomM, its ROM library, and its downloaded metadata live on the external disk;
the database does not. This doc owns the backup and restore decisions. The
tooling is in [`deploy/romm/`](../deploy/romm/).

Checked against RomM **5.1.0**, MariaDB **12.3.2**, podman-compose **1.6.0**,
and Podman **6.1.4** on 2026-11-05.

## Decision

- **Runtime:** an external Compose stack at `/mnt/external/romM/`, started by
  `romm.service` and `romm.path`. It is not versioned in this repository.
- **Library, resources, assets, config:** bind mounts on `/mnt/external`. They
  survive a machine wipe, so the nightly job does not copy them.
- **Database:** MariaDB on the internal `romm_mysql_data` volume (97.5 MB
  logical). It is the only RomM state a machine wipe destroys.
- **Off-disk backup:** a nightly `mariadb-dump` to
  `/mnt/external/romM/backups`, keeping 14. The dump lands on the disk that
  already holds the files, which is enough for a machine wipe and no help at
  all if that disk dies.
- **Not copied:** `resources/` (57 GB, re-downloadable), `library/` (318 GB,
  the source data), `assets/` (saves and states, currently empty), and the
  Redis cache. A dead external disk loses all of them; that is the later
  cloud/off-site job.

## What survives

| Piece | Path | Machine wipe | External disk loss |
| --- | --- | --- | --- |
| ROMs | `/mnt/external/romM/library/` | survives | gone |
| MAME set (not managed by RomM) | `/mnt/external/roms/` | survives | gone |
| Metadata images | `/mnt/external/romM/resources/` | survives | gone; re-fetch on rescan |
| Saves and states | `/mnt/external/romM/assets/` | survives | gone |
| `config.yml` | `/mnt/external/romM/config/` | survives | gone |
| Compose and secrets | `/mnt/external/romM/podman-compose.yaml` | survives | gone |
| MariaDB | internal `romm_mysql_data` volume | gone | gone |
| Database dumps | `/mnt/external/romM/backups/` | survives | gone |

The dump is the catalogue, not the ROMs. Restoring it onto a disk without the
library gives 4,007 ROM rows whose files are missing.

## Nightly dump

Install as the desktop user:

```sh
deploy/romm/install-backup.sh
systemctl --user start romm-backup.service
journalctl --user -u romm-backup -n 50 --no-pager
ls -lh /mnt/external/romM/backups | tail
```

The timer runs at 03:45, after the Immich mirror. The script:

- refuses to run when `/mnt/external` is not mounted;
- requires the `romm-db` container to be running, and tests its `MARIADB_*`
  credentials before writing;
- dumps with `--single-transaction --quick --databases`;
- writes a `.part` file, checks gzip and at least 20 tables, then renames it;
- keeps the newest 14 dumps and leaves every other file in the directory alone.

The password is read from the container's own environment, so the script never
parses the compose file. `ROMM_BACKUP_DRY_RUN=1` checks the guards without
writing. Overrides go in `~/.config/romm-backup.env`.

## Restore after a machine wipe

The external disk survived; only the internal database volume is gone.

### 1. Prepare the host

```sh
sudo pacman -S podman podman-compose
git clone git@github.com:viniciuscr/hearthdeck.git
```

### 2. Mount the external disk

```sh
sudo mount /mnt/external          # or the matching /etc/fstab entry
findmnt /mnt/external
ls /mnt/external/romM/podman-compose.yaml /mnt/external/romM/backups
```

### 3. Start only the database

```sh
podman-compose -f /mnt/external/romM/podman-compose.yaml up -d romm-db
podman ps --filter name=romm-db
timeout 120 sh -c 'until podman exec romm-db healthcheck.sh --connect --innodb_initialized >/dev/null 2>&1; do sleep 2; done'
```

`podman-compose` lowercases the directory name, so this resolves to the
`romm` project and reuses the `romm_mysql_data` volume name.

### 4. Restore the newest dump

This wipes the tables in the target database and replaces them. Do it only
against the fresh database volume, never against a live RomM you care about.

```sh
dump=$(ls -t /mnt/external/romM/backups/romm-db-*.sql.gz | head -1)
echo "$dump"
gzip -t "$dump"
gzip -dc "$dump" | podman exec -i romm-db sh -c \
  'MYSQL_PWD="$MARIADB_ROOT_PASSWORD" mariadb --user=root'
```

The dump includes `CREATE DATABASE` and `USE romm`, so the fresh container
created by `MARIADB_DATABASE=romm` needs no empty-schema preparation.

### 5. Start the rest

```sh
podman-compose -f /mnt/external/romM/podman-compose.yaml up -d
```

Open `http://<host>:8080`, log in with the accounts from the dump, and check
the platform and ROM counts. Run a metadata scan only if `resources/` is
missing; the library and config are already in place.

## Verify

Cheap checks, safe to run any time:

```sh
gzip -t /mnt/external/romM/backups/romm-db-*.sql.gz
gzip -dc "$(ls -t /mnt/external/romM/backups/romm-db-*.sql.gz | head -1)" \
  | grep -c '^CREATE TABLE'
systemctl --user list-timers romm-backup.timer
```

A dump is about 9 MB and has 29 tables. A real restore is the only proof that
it is usable: restore it into a disposable MariaDB container with the same
image, count the users, ROMs, and platforms, then remove the container and
volume. Do not rehearse against the live database.

## Traps

- **Do not copy the raw `romm_mysql_data` volume.** The logical dump is the
  supported path; a raw volume is tied to the MariaDB version and must be
  stopped cleanly.
- **`mariadb:latest` and `romm:latest` move.** Note the versions before an
  upgrade, and restore with the same or a newer RomM. A MariaDB major upgrade
  is another reason to keep logical dumps rather than the raw volume.
- **The compose file holds secrets in plaintext** (DB passwords, the auth
  secret, ScreenScraper and RetroAchievements credentials) and is readable on
  the exFAT mount. Move them to a protected env file when touching the stack;
  never copy the file into git.
- **The dump contains user rows, including password hashes.** It sits on the
  same unencrypted exFAT mount as the compose secrets. Treat
  `/mnt/external/romM/backups` as sensitive and do not sync it to a public
  share.
- **The external disk is a single point of failure.** The dump shares it with
  the library and metadata. Until the cloud copy exists, a disk failure loses
  RomM completely, and a second copy on the same disk would not help.
- **`resources/` is not source data, but it is not free either.** It can be
  re-downloaded, yet a rescan of 4,007 ROMs takes time; budget for it after a
  disk loss.
