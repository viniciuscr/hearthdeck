# Immich deployment files

The versioned Quadlet stack for the Immich instance documented in
[`docs/immich-backup.md`](../../docs/immich-backup.md), plus the timer that
copies the database dumps to the second disk. The doc owns the decisions and
the why; this file is the runbook for installing, restoring, and backing up.

## Files

| File | Installed to | Purpose |
| --- | --- | --- |
| `immich.pod` | `/home/immich/.config/containers/systemd/` | Shared pod, publishes `2283`. |
| `immich-db.container` | same | Postgres on the `immich-db` volume. |
| `immich-redis.container` | same | Valkey. |
| `immich-ml.container` | same | Machine learning, model cache volume. |
| `immich-server.container` | same | Immich server, mounts `/mnt/immich/data`. |
| `install.sh` | run with `sudo` | Non-destructive reinstall of the stack. Never formats. |
| `immich-backup` | `~/.local/bin/` | Mirror dumps (and optionally originals) to `/mnt/external`. |
| `immich-backup.service` / `.timer` | `~/.config/systemd/user/` | Nightly run at 03:30. |
| `immich-backup.env.example` | `~/.config/immich-backup.env` | Optional overrides, including the opt-in originals switch. |
| `install-backup.sh` | run as the desktop user | Installs the mirror script, units, and timer (dumps by default). |

## What has to survive

| Piece | Where it lives | Off-disk copy today |
| --- | --- | --- |
| Original photos and videos | `/mnt/immich/data/upload/` | no; cloud backup later |
| Storage-template originals | `/mnt/immich/data/library/` | no; cloud backup later |
| Avatars | `/mnt/immich/data/profile/` | no |
| Database dumps | `/mnt/immich/data/backups/` | nightly to `/mnt/external` |
| Users, albums, metadata, settings | the `immich-db` volume | no: the dump is their backup |
| Database password | the `immich-db-password` secret | no: a fresh cluster gets a new one |
| Thumbnails, transcoded video | `/mnt/immich/data/thumbs/`, `encoded-video/` | no: Immich regenerates them |
| Quadlet units and this runbook | this repository | git |

Two pieces are irreplaceable: the originals and the database dump. The dump only
restores the catalogue; without the originals it lists photos whose files are
gone. The current mirror holds the dumps only, so the library disk is the single
copy of the photos until the cloud backup exists.

The dump contains the `immich` database, not the Postgres roles. A formatted
machine creates a new `immich-db` volume, initialises it with a new secret, and
the restored dump is used with that same new password. That is why `install.sh`
may create a new secret safely. Restoring the raw Postgres volume is not
supported unless the old secret was preserved with it.

## Accounts and passwords

Three different things are called "the Immich password". Only the web login is
one you chose.

**Linux `immich` user: no password.** Setup runs
`useradd -m -s /bin/bash immich` and never `passwd immich`, so the account is
created locked. Do not try to log in as it; switch to it through your own sudo:

```sh
sudo machinectl shell immich@    # or: sudo -iu immich
```

That prompts for *your* sudo password. Verify the account is locked with
`sudo passwd -S immich` (expect `L`). Setting a password with
`sudo passwd immich` is unnecessary: linger starts the user manager at boot, so
nothing needs an interactive login.

**Immich web accounts: chosen on the onboarding screen.** The database stores
only a hash, never the plaintext, so a forgotten password cannot be read back.
List the accounts as the `immich` user:

```sh
sudo machinectl shell immich@
podman exec -it immich-server immich-admin list-users
```

If the admin password is available, reset another account from
**Administration → Users**. If the admin password is the one that is lost,
reset it from the server container:

```sh
podman exec -it immich-server immich-admin reset-admin-password
```

The command is interactive and only rewrites that account's hash; it does not
touch photos, dumps, or other database rows.

**Postgres password: generated, never typed.** Setup creates the
`immich-db-password` Podman secret with `openssl rand -hex 24`, and the database
and server containers read it. It is not a web login and is only needed for a
direct database connection. To read it:

```sh
sudo machinectl shell immich@
podman secret inspect --showsecret immich-db-password
```

A dump restore does not carry this secret. On a fresh machine, `install.sh`
generates a new one and the new cluster uses it; that is expected.

## Reinstall after a machine wipe

### What survived?

| Event | Library disk | Backup disk | Recovery |
| --- | --- | --- | --- |
| machine only | survives | survives | reinstall the host, run `install.sh`, restore from the dump already on the library disk |
| machine + library disk | gone | survives | prepare a new library disk, restore the dump from the backup disk; the originals are gone until the cloud backup exists |
| both disks | gone | gone | only an off-site copy can help; the current mirror has no originals |
| backup disk only | survives | gone | replace the backup disk and run the mirror again; the library is still primary |

### 1. Prepare the host

```sh
sudo pacman -S podman rsync openssl gzip git
git clone git@github.com:viniciuscr/hearthdeck.git
cd hearthdeck
```

### 2. Mount the backup disk

```sh
lsblk -f
sudo mkdir -p /mnt/external
sudo mount /dev/sdX2 /mnt/external     # or: sudo mount /mnt/external
findmnt /mnt/external
df -h /mnt/external
```

Add an `/etc/fstab` entry so the nightly timer does not fail after the next
reboot. Replace `sdX2` and the uid/gid with the values from `blkid` and `id`:

```sh
UUID=$(sudo blkid -s UUID -o value /dev/sdX2)
echo "UUID=$UUID /mnt/external exfat defaults,nofail,uid=$(id -u),gid=$(id -g),umask=022,x-systemd.device-timeout=20 0 0" \
  | sudo tee -a /etc/fstab
sudo systemctl daemon-reload
```

`nofail` keeps the host bootable without the disk. The backup script refuses to
run unless the mount is present, so a missing disk fails loudly instead of
writing onto the root filesystem.

### 3. Install the stack without starting it

If the library disk is the old one, `install.sh` mounts it and reuses
`/mnt/immich/data` untouched:

```sh
sudo IMMICH_NO_START=1 deploy/immich/install.sh
```

If the library disk is new, partition and format it first
([`docs/immich-backup.md`](../../docs/immich-backup.md), step 1). That step is
destructive and is only for a genuinely blank disk. Then run the command above;
`install.sh` never formats anything.

`IMMICH_NO_START=1` is what lets you put a dump in place before Immich opens its
onboarding screen. Without it the server starts immediately and the database is
already migrated by the time you look.

### 4. Put a dump where Immich can see it

Onboarding only lists files in `/mnt/immich/data/backups` (the container's
`/data/backups`). The mirror is at `/mnt/external/immich/backups`; copy the
newest dump across:

```sh
newest=$(ls -t /mnt/external/immich/backups/*.sql.gz | head -1)
echo "$newest"
gzip -t "$newest"
sudo install -o immich -g immich -m 644 "$newest" /mnt/immich/data/backups/
```

If the library disk survived, its own `backups/` already has the same dumps.
Copying the newest from the mirror is still the safer choice: it is the copy
that was checked by the backup timer.

### 5. Restore the originals, only if an originals mirror exists

The current mirror is dumps-only, so on a replacement library disk there is
nothing here to copy: the photos are gone until the cloud backup exists. Skip
this step when the library disk survived; its `upload/` is already in place.

If an originals mirror was enabled, restore it before the first start:

```sh
sudo rsync -rlt /mnt/external/immich/data/upload/  /mnt/immich/data/upload/
sudo rsync -rlt /mnt/external/immich/data/profile/ /mnt/immich/data/profile/
sudo rsync -rlt /mnt/external/immich/data/library/ /mnt/immich/data/library/
sudo chown -R immich:immich /mnt/immich/data
sudo chmod 750 /mnt/immich/data
```

`-rlt` because the source is exFAT, which stores neither Unix ownership nor
permissions. The `chown` is what makes the container, whose root maps to the
`immich` user, able to read the files again. Do not restore `thumbs/` or
`encoded-video/`; they are regenerated.

### 6. Start and restore the database

```sh
sudo machinectl shell immich@
systemctl --user start immich-pod
```

Open `http://<server-lan-ip>:2283`, choose **Restore from backup**, select the
dump, and restore. The integrity check should list `upload`, `profile`,
`library`, `thumbs`, `encoded-video`, and `backups` as readable.

After the restore, run **Generate Thumbnails** and **Transcode Videos** in
**Administration → Jobs** if the integrity check reports them missing. Log in
with the accounts that existed when the dump was taken.

On an installation that already has a database, do not use the onboarding path;
use **Administration → Maintenance → Restore database backup** instead. That path
wipes the current database and creates a rollback point first.

The command-line restore is a last resort and only works on a fresh database
that the server has never migrated. Follow the
[official instructions](https://immich.app/docs/administration/backup-and-restore#restore-via-command-line);
do not improvise it against a running instance.

### 7. Re-arm the nightly dump mirror

As the desktop user, from the repository:

```sh
deploy/immich/install-backup.sh
systemctl --user start immich-backup.service
journalctl --user -u immich-backup -n 50 --no-pager
```

This mirrors the dumps. It does not copy the library.

## Nightly copy to the second disk

The dumps live on the same disk as the library, so one dead disk loses both. The
timer mirrors them to `/mnt/external` as the desktop user, which only needs read
access to `/mnt/immich/data` (membership in the `immich` group).

Install once:

```sh
deploy/immich/install-backup.sh     # dumps only; this is the current setup
```

The script also has an opt-in `--originals` mode for `upload/`, `profile/`, and
`library/`. It is deliberately not enabled: the second disk is not the place for
a second copy of the library, and the full-library backup is a later cloud step.

Run it once by hand and check the result:

```sh
systemctl --user start immich-backup.service
journalctl --user -u immich-backup -n 50 --no-pager
ls -lh /mnt/external/immich/backups | tail
```

The script aborts if `/mnt/external` is not mounted, verifies the newest dump
with `gzip -t`, refuses a dump older than 48 hours (which means Immich's own
dump job stopped), and holds a lock so two runs cannot race. It mirrors with
`rsync -rlt --delete` (exFAT cannot store Unix ownership).

`IMMICH_BACKUP_DRY_RUN=1` prints what rsync would copy without writing. The
service reads `~/.config/immich-backup.env`, so load it too when running the
script by hand; use it after restoring to confirm the mirror is current:

```sh
set -a; . ~/.config/immich-backup.env; set +a
IMMICH_BACKUP_DRY_RUN=1 ~/.local/bin/immich-backup
```

Specifying `--originals` adds `upload/`, `profile/`, and `library/`. That is off
now: the first run would copy the whole library (about 82 GB on this machine),
and the photos are meant for the cloud backup instead. Thumbnails and encoded
video are never copied because Immich regenerates them.

`--delete` makes the mirror match the source, deletions included. That is what
keeps it from growing forever, but it is not an archive: deleting a dump in
Immich deletes the backup copy on the next run. For protection against
accidental deletion, and for the third copy a 3-2-1 setup needs, add restic or
borg to a remote.

## Verify the backup

Cheap checks, safe to run any time:

```sh
gzip -t /mnt/immich/data/backups/*.sql.gz
gzip -t /mnt/external/immich/backups/*.sql.gz
sha256sum "$(ls -t /mnt/immich/data/backups/*.sql.gz | head -1)" \
          "$(ls -t /mnt/external/immich/backups/*.sql.gz | head -1)"
systemctl --user list-timers immich-backup.timer
```

The newest dump should have the same hash on both disks. The dump must also end
with `PostgreSQL database dump complete`, not stop mid-table:

```sh
gzip -dc "$(ls -t /mnt/external/immich/backups/*.sql.gz | head -1)" | tail -3
```

A real restore is the only proof that the dump is usable. The safe rehearsal is
a disposable Postgres container with the same image as `immich-db`: restore the
dump into it, count the users and assets, then remove it. Do not rehearse against
the live database.

## Verification

As the `immich` user:

```sh
sudo machinectl shell immich@
systemctl --user status immich-pod
podman ps
podman logs --tail 50 immich-server
```

From the desktop user:

```sh
curl -fsS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:2283/
podman-overview --usage    # all rootless users; asks for sudo once
systemctl --user list-timers immich-backup.timer
findmnt /mnt/immich /mnt/external
```

## Traps

- **Do not run the old `immich-setup.sh` on an existing library.** It wipes the
  disk before creating a filesystem. `install.sh` is the reinstall path.
- **Do not format the SSD to "start clean".** Everything that survives a
  machine death is on that disk. Format only a genuinely new, empty disk.
- **The dump is not the photo backup.** The mirror holds only the catalogue
  today. A dead library disk loses the files until the cloud backup exists, so
  do not mistake a successful dump mirror for a complete backup.
- **The mirror is not an archive.** `--delete` propagates deletions to the
  second disk.
- **External libraries are not mirrored.** If `/mnt/immich/archive` (or another
  read-only mount) is in use, back it up separately.
- **Same or newer Immich, never older.** Restoring a dump into an older server
  can leave a database the old code cannot read. The restore runs migrations
  forward; downgrades are unsupported.
- **Never change the Postgres major version on an existing volume.** Edit the
  image tag only when a release explicitly requires it, and start the database
  before the server.
- **The secret and the volume are a pair.** `install.sh` refuses to create a new
  secret when `immich-db` exists. Deleting the secret without deleting the
  volume breaks the database login. A dump restore is different: it does not
  carry the roles, so a fresh volume and a fresh secret are consistent.
- **The mount guard is the real protection.** `nofail` in `/etc/fstab` keeps the
  machine bootable without the disk; `ExecStartPre=/usr/bin/mountpoint -q
  /mnt/immich` keeps the server from writing into an empty directory on the root
  filesystem.
- **The backup timer fails loudly when the second disk is absent.** That is
  intentional: writing to `/mnt/external` on the root filesystem would fill the
  small root partition.
