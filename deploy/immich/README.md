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
| `rehearse-restore.sh` | run as the desktop user | Restores a dump into a disposable Postgres and prints the row counts. |

Shared host tooling used by the reinstall:

| Path | Purpose |
| --- | --- |
| `deploy/mount-external.sh` | Mount the exFAT backup disk and add its fstab entry; never formats. |
| `deploy/pre-wipe-check.sh` | Read-only proof that mounts, dumps, timers, and services are ready before a wipe. |
| `deploy/cloudflared/` | Versioned tunnel Quadlet and installer; the token stays outside git. |

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

## Before wiping the machine

The two external disks survive; the OS, rootless Podman storage, the Immich
database volume, the RomM MariaDB volume, the user units, and `~/.config` do
not. Run this list in order.

### 1. Run the read-only readiness check

From the repository, as the desktop user:

```sh
deploy/pre-wipe-check.sh
```

It checks the mounts and their fstab entries, the newest Immich and RomM dumps
(age, gzip, completion marker), that the mirror copy is byte-identical, the
timer state and last result, the `immich` group membership, the upload count,
the live Immich version and health, the tunnel token and unit, RomM, and free
space. It exits 1 on a hard failure and never mounts, writes, or starts a unit.
This first run is an early warning; the proof is the second run after step 2.

### 2. Take a final Immich dump and mirror it

The automatic dump runs at 02:00, so a wipe later in the day would restore a
database missing the most recent uploads. In Immich, open
**Administration → Job Queues → Create job**, choose **Create Database Dump**,
and confirm. Wait for a new `immich-db-backup-*.sql.gz` in
`/mnt/immich/data/backups`, then mirror it:

```sh
newest=$(ls -t /mnt/immich/data/backups/*.sql.gz | head -1)
gzip -t "$newest"
systemctl --user start immich-backup.service
journalctl --user -u immich-backup -n 20 --no-pager
sha256sum "$newest" "/mnt/external/immich/backups/$(basename "$newest")"
deploy/immich/rehearse-restore.sh "$newest"
deploy/pre-wipe-check.sh
```

Both hashes must match. The rehearsal restores the final dump into a disposable
container and prints the row counts; it is the proof that the dump is usable,
not just recent. A manual dump counts toward the 14-dump retention, so the
mirror's next `--delete` run keeps the two directories aligned.

### 3. Take a final RomM dump

```sh
systemctl --user start romm-backup.service
journalctl --user -u romm-backup -n 20 --no-pager
dump=$(ls -t /mnt/external/romM/backups/*.sql.gz | head -1)
gzip -t "$dump"
```

RomM's library, config, assets, and resources are already on the external disk;
this only refreshes the database dump.

### 4. Copy the off-disk credentials somewhere safe

A wipe deletes these, and none is reconstructible from the external disk:

| Secret | Path | If lost |
| --- | --- | --- |
| Cloudflare tunnel token | `~/.config/cloudflared/tunnel.env` | rotate it in the dashboard |
| rclone remote config | `~/.config/rclone/rclone.conf` | re-authenticate pCloud |
| Backup overrides | `~/.config/hearthdeck-backup/backup.env`, `credentials.env` | re-enter in Settings |
| SSH key for the git remote | `~/.ssh/` | add a new key to GitHub |

Store them in a password manager or an encrypted archive, not as plaintext on
the exFAT disk. If you copy them to the external disk for the transition, delete
that copy after the restore.

### 5. Identify the disks, then unplug them if you can

```sh
lsblk -o NAME,SIZE,SERIAL,MODEL,FSTYPE,LABEL,UUID,MOUNTPOINTS
```

On this machine the two disks to preserve are:

| Role | Model | Serial | Filesystem | Label / UUID |
| --- | --- | --- | --- | --- |
| Immich library | `CT1000X9SSD9` | `2542EA90E4C1` | ext4 | `immich` / `934db15b-…` |
| Backup + RomM | `CT1000X10SSD9` | `2517EA801282` | exFAT | `Crucial X10` / `86B6-D516` |

The installer sees device letters that may differ from the running system.
Select the install target by size, model, or serial, never by `sdX` alone. If
the disks are removable, unplug both before booting the installer and reconnect
them after the first boot; that is the cheapest way to make the mistake
impossible.

### 6. Optional: push the final dumps off-site

The external disk holds the final dump, which is enough for a machine wipe. To
also put it in pCloud, run the off-site job and check it:

```sh
systemctl --user start hearthdeck-backup.service
journalctl --user -u hearthdeck-backup -n 30 --no-pager
rclone lsf pcloud:hearthdeck-backups/immich | sort | tail -3
```

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

The second SSD is the exFAT disk labeled `Crucial X10`. Use the versioned
script; it finds the partition by label (or by `--device`/`EXTERNAL_UUID`),
never formats, and appends the `/etc/fstab` entry only when it is missing:

```sh
sudo deploy/mount-external.sh
findmnt /mnt/external
df -h /mnt/external
```

If the device letter moved, the label still finds it. To be explicit:

```sh
sudo deploy/mount-external.sh --device /dev/sdX2
```

The script refuses anything that is not exFAT and refuses a disk without the
`.hearthdeck-backup` sentinel, so a different stick cannot be mounted over the
backup path. The sentinel is an empty file at the disk root; create it once with
`sudo touch /mnt/external/.hearthdeck-backup` only when setting up a genuinely
new backup disk. It assigns the mount to the desktop user (`uid`/`gid` of the
sudo user) so the nightly timers can write to exFAT, and adds `nofail` so the
machine still boots without the disk. `deploy/mount-external.sh --dry-run`
prints the plan without writing.

The live entry is:

```
UUID=86B6-D516 /mnt/external exfat defaults,nofail,uid=1000,gid=1000,umask=022,x-systemd.device-timeout=20,x-gvfs-show,x-gvfs-name=External 0 0
```

`nofail` keeps the host bootable without the disk. The backup scripts refuse to
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

`install.sh` also creates the `immich` group if needed and adds the invoking
`sudo` user to it, so the dump mirror can read `/mnt/immich/data/backups`.
Group membership reaches a session only at the next login: log out and back in
before installing or running the mirror. Do **not** reboot while a dump is
waiting to be restored: `immich.pod` is enabled at boot and would start the
empty database first. It compares the data directory's `uid:gid` before a
recursive `chown`, so a reinstall where the IDs already match does not rewrite
the whole 110 GB library.

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

This mirrors the dumps. It does not copy the library. The installer refuses to
arm the timer when the current user is not yet in the `immich` group, because
the 03:30 run would fail without read access.

### 8. Reinstall the Cloudflare tunnel

The Quadlet is versioned at
[`deploy/cloudflared/cloudflared-immich.container`](../cloudflared/cloudflared-immich.container).
The token is not: restore `~/.config/cloudflared/tunnel.env` from the copy you
saved, or rotate the token in the Cloudflare dashboard.

```sh
install -d -m 700 ~/.config/cloudflared
install -m 600 /path/to/tunnel.env ~/.config/cloudflared/tunnel.env
deploy/cloudflared/install-cloudflared.sh
systemctl --user status cloudflared-immich
```

The installer refuses to run as root, requires the env file to be mode `600`,
and only restarts the unit when the checked-in Quadlet changed. `is-enabled`
reporting `generated` is normal for a Quadlet. Linger must be on for the tunnel
to start before login:

```sh
sudo loginctl enable-linger "$USER"
```

### 9. Restore the off-site backup

The `hearthdeck` package installs `hearthdeck-backup.timer` **disabled** on
purpose: a nightly job with no cloud remote would only fail. Restore
`~/.config/rclone/rclone.conf` (or run `rclone config` again) and the optional
`~/.config/hearthdeck-backup/backup.env`, then enable it once by hand:

```sh
systemctl --user enable --now hearthdeck-backup.timer
systemctl --user start hearthdeck-backup.service
journalctl --user -u hearthdeck-backup -n 30 --no-pager
rclone lsf pcloud:hearthdeck-backups/immich | sort | tail -3
```

There is no Settings toggle in the current frontend; the manual enable is the
documented escape hatch in [`docs/backup-settings.md`](../../docs/backup-settings.md).
Do not enable it before the rclone remote exists.

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

The newest dump should have the same hash on both disks. It must also contain
the `PostgreSQL database dump complete` marker, not stop mid-table. pg_dump 14
writes a `\unrestrict` line after the marker, so search for it instead of
reading only the last line:

```sh
gzip -dc "$(ls -t /mnt/external/immich/backups/*.sql.gz | head -1)" \
  | grep '^-- PostgreSQL database dump complete$'
```

A real restore is the only proof that the dump is usable. Run the versioned
rehearsal; it creates a disposable container with the same image as `immich-db`,
restores the dump, prints the row counts, and removes the container and volume:

```sh
deploy/immich/rehearse-restore.sh
deploy/immich/rehearse-restore.sh /mnt/external/immich/backups/<dump>.sql.gz
```

It never touches the live database or the library. A successful run exits 0 and
prints `users`, `assets`, `asset_files`, `live_assets`, and the extensions; zero
users or zero assets means the dump is not a usable catalogue.

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
- **The 02:00 dump is not the pre-wipe dump.** Create one by hand immediately
  before the wipe and mirror it, or the restore loses the day's uploads.
- **Group membership is not retroactive.** Adding the desktop user to `immich`
  affects the next login, not the running systemd user manager. Log out and back
  in before arming the mirror. Do not reboot while a dump is waiting to be
  restored: `immich.pod` starts at boot.
- **The tunnel token is not in git and cannot be read back from Cloudflare.**
  Preserve `~/.config/cloudflared/tunnel.env` before the wipe, or rotate the
  token afterwards.
- **Do not trust `sdX`.** Device letters move between boots and inside the
  installer. Match the library disk by the `immich` label or its UUID, the
  backup disk by `Crucial X10` or `86B6-D516`, and use the serials when the
  installer asks for a target.
