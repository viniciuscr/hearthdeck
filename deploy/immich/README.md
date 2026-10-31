# Immich deployment files

The versioned Quadlet stack for the Immich instance documented in
[`docs/immich-backup.md`](../../docs/immich-backup.md), plus the timer that
copies database dumps to the second disk. The doc owns the decisions and the
why; this file is the runbook for installing, restoring, and backing up.

## Files

| File | Installed to | Purpose |
| --- | --- | --- |
| `immich.pod` | `/home/immich/.config/containers/systemd/` | Shared pod, publishes `2283`. |
| `immich-db.container` | same | Postgres on the `immich-db` volume. |
| `immich-redis.container` | same | Valkey. |
| `immich-ml.container` | same | Machine learning, model cache volume. |
| `immich-server.container` | same | Immich server, mounts `/mnt/immich/data`. |
| `install.sh` | run with `sudo` | Non-destructive reinstall of the stack. |
| `immich-backup` | `~/.local/bin/` | Mirror dumps (and optionally originals) to `/mnt/external`. |
| `immich-backup.service` / `.timer` | `~/.config/systemd/user/` | Nightly run at 03:30. |
| `immich-backup.env.example` | `~/.config/immich-backup.env` | Optional overrides. |

## Reinstall after a machine format

The library and the database dumps are on the external SSD; the user, units,
secret, images, and live database are not. A fresh machine needs Podman and the
filesystem mounted, nothing else.

```sh
sudo pacman -S podman rsync openssl gzip
git clone git@github.com:viniciuscr/hearthdeck.git
cd hearthdeck

# Finds the ext4 filesystem labelled "immich". Use IMMICH_DEVICE=/dev/sdX1 if
# the label is missing; check with lsblk -f.
sudo deploy/immich/install.sh
```

`install.sh` is deliberately not the original provisioning script. It:

1. verifies the device is ext4, mounts it at `/mnt/immich`, and adds an
   `/etc/fstab` entry by UUID if one is missing (it never formats anything);
2. creates the `immich` user, enables linger, and checks the subuid/subgid range;
3. sets ownership of `/mnt/immich/data` to `immich` (needed when the new
   machine allocated a different UID);
4. installs the five units and starts `immich-pod`;
5. creates the `immich-db-password` secret only when the `immich-db` volume does
   not exist. A missing secret with an existing volume is a hard error: a new
   password would lock you out of the existing database.

If ufw is active, pass the LAN subnet so the phone can reach the port:

```sh
sudo IMMICH_LAN_SUBNET=192.168.178.0/24 deploy/immich/install.sh
```

### Restore the database

On a formatted machine the Postgres volume is new and empty, so Immich shows the
welcome screen. Use the official onboarding restore:

1. Open `http://<server-lan-ip>:2283`.
2. Choose **Restore from backup**.
3. The integrity check should list `upload`, `profile`, `library`, `thumbs`,
   `encoded-video`, and `backups` as readable. The dumps are in
   `/mnt/immich/data/backups` on the host, which is `/data/backups` in the
   container.
4. Select the newest `immich-db-backup-*.sql.gz` and restore.
5. Log in with the accounts that existed when the dump was taken.

On an installation that already has a database, restore from
**Administration → Maintenance → Restore database backup** instead. That path
wipes the current database and creates a rollback point first.

The command-line restore is a last resort and only works on a fresh database
that the server has never migrated. Follow the
[official instructions](https://immich.app/docs/administration/backup-and-restore#restore-via-command-line);
do not improvise it against a running instance.

After the restore, run the **Generate Thumbnails** and **Transcode Videos** jobs
if the backup did not include `thumbs/` and `encoded-video/`. Phones keep the
same server URL and normally resume without re-authenticating.

## Nightly copy to the second disk

The dumps live on the same SSD as the library, so one dead disk loses both. The
timer mirrors them to `/mnt/external` as the desktop user, which only needs read
access to `/mnt/immich/data`.

Install once:

```sh
install -Dm755 deploy/immich/immich-backup ~/.local/bin/immich-backup
install -Dm644 deploy/immich/immich-backup.service ~/.config/systemd/user/immich-backup.service
install -Dm644 deploy/immich/immich-backup.timer ~/.config/systemd/user/immich-backup.timer
systemctl --user daemon-reload
systemctl --user enable --now immich-backup.timer
```

Run it once by hand and check the result:

```sh
systemctl --user start immich-backup.service
journalctl --user -u immich-backup -n 50 --no-pager
ls -lh /mnt/external/immich/backups | tail
```

The script aborts if `/mnt/external` is not mounted, verifies the newest dump
with `gzip -t`, refuses a dump older than 48 hours (which means Immich's own
dump job stopped), and holds a lock so two runs cannot race. It mirrors the dump
directory with `rsync -rlt --delete` (exFAT cannot store Unix ownership).

To copy the originals as well, create `~/.config/immich-backup.env` from the
example and set:

```sh
IMMICH_BACKUP_ORIGINALS=1
```

That mirrors `upload/`, `profile/`, and `library/`. Thumbnails and encoded video
are not copied because Immich regenerates them; the first run copies the ~44 GB
library, later runs only new files. The second disk must have room for both the
dumps and the originals.

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
- **Same or newer Immich, never older.** Restoring a dump into an older server
  can leave a database the old code cannot read. The restore runs migrations
  forward; downgrades are unsupported.
- **Never change the Postgres major version on an existing volume.** Edit the
  image tag only when a release explicitly requires it, and start the database
  before the server.
- **The secret and the volume are a pair.** `install.sh` refuses to create a new
  secret when `immich-db` exists. Deleting the secret without deleting the
  volume breaks the database login.
- **The mount guard is the real protection.** `nofail` in `/etc/fstab` keeps the
  machine bootable without the disk; `ExecStartPre=/usr/bin/mountpoint -q
  /mnt/immich` keeps the server from writing into an empty directory on the root
  filesystem.
- **The backup timer fails loudly when the second disk is absent.** That is
  intentional: writing to `/mnt/external` on the root filesystem would fill the
  small root partition.
