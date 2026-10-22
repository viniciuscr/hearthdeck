# Immich photo/video backup on the external SSD (Podman)

Immich is the phone photo/video backup target. The library lives on the new
external SSD; the database does not.

Checked against Immich **v3.3.4** (released 2026-10-20) and Podman **6.1.4** on
2026-10-21.

## Decision

- **Runtime:** rootless Podman under a dedicated `immich` user, declared as a
  Quadlet pod in `/home/immich/.config/containers/systemd/`, started at boot with
  `loginctl enable-linger immich`.
- **Media:** `/mnt/immich/data` on the external SSD, mounted at `/data` in the
  server container.
- **Database:** a Podman named volume on the internal disk. It is 1–3 GB,
  latency-sensitive, and must not vanish when the SSD is unplugged.
- **Version:** `ghcr.io/immich-app/*:v3`, upgraded by hand after reading the
  release notes. No `podman auto-update`.

Why not Compose: Immich has no official Podman deployment. Its official
single-host deployment is Docker Compose, and its official
`docker-compose.rootless.yml` is written for rootless container runtimes, not
specifically Podman. Quadlet gives the same four services without a compose
provider in the runtime path, plus a systemd unit that can retry when a USB disk
is late. The files below are the official Compose stack translated one-to-one.

## What is official, what is community

| Thing | Status |
| --- | --- |
| [Docker Compose install](https://immich.app/docs/install/docker-compose) | Official, recommended. [Requirements](https://immich.app/docs/install/requirements) say Docker + Compose plugin; Podman is not mentioned. |
| [`docker-compose.yml`](https://github.com/immich-app/immich/releases/latest/download/docker-compose.yml) | Official release asset. Four services: server, machine learning, Valkey, Postgres. |
| [`docker-compose.rootless.yml`](https://github.com/immich-app/immich/releases/latest/download/docker-compose.rootless.yml) | Official release asset for rootless containers. Pins `user: '1000:1000'`, which needs `userns_mode: keep-id` on Podman (see the trap below). |
| [Kubernetes / Helm](https://immich.app/docs/install/kubernetes) | Official, out of scope here. |
| [immich-podman-quadlets](https://github.com/linux-universe/immich-podman-quadlets) | Community Quadlet translation, active (last push 2026-10-07). Source for the files below. |
| [Melonslab: rootless Podman guide](https://www.melonslab.com/guides/immich-podman) | Community, tested with Immich 3.2.2 on Debian 13 (2026-09-26): pod, dedicated user, Caddy, backup. |

Current upstream values worth copying rather than inventing: Postgres image
`ghcr.io/immich-app/postgres:14-vectorchord0.4.3-pgvectors0.2.0`, Valkey 9, port
`2283`, ML port `3003`. Immich release notes occasionally require a new database
image; never change the Postgres major version on an existing volume.

## This machine

| Resource | Value | Consequence |
| --- | --- | --- |
| Podman / compose provider | 6.1.4 / podman-compose 1.6.0 | `podman compose` delegates to `podman-compose`. |
| CPU / RAM | 4 cores / 7.6 GiB | Immich minimum is 6 GiB, recommendation 8 GiB. It fits, but ML and the desktop compete; swap is already configured. |
| Root filesystem | 18 GiB free | Images (~3.6 GB), model cache (~1 GB) and the database (1–3 GB) all live here. This is the tightest constraint. |
| External SSD | 1 TB Crucial X9 (USB) | Library. Use a blue USB 3 port; the 480 Mbps bus caps copies at roughly 40 MB/s. |
| Second disk | 1 TB Crucial X10 at `/mnt/external` (exFAT) | Backup copy only, never the database. |
| Firewall | ufw active, no SELinux | Open `2283` for the LAN subnet only. Do not add `:z` mount flags here. |
| LAN address | your server's LAN IP (Wi-Fi) | Phones use `http://<server-lan-ip>:2283`. |

## 1. Prepare the SSD

Replace `/dev/sdX` with the new disk; confirm with `lsblk -o
NAME,SIZE,MODEL,SERIAL,TRAN` before writing anything. This erases the disk.

```sh
sudo wipefs -a /dev/sdX
sudo parted -s /dev/sdX mklabel gpt
sudo parted -s /dev/sdX mkpart primary ext4 1MiB 100%
sudo wipefs -a /dev/sdX1
sudo mkfs.ext4 -F -L immich -m 0 /dev/sdX1
```

Mount it by UUID, not by `/dev/sdX`:

```sh
sudo mkdir -p /mnt/immich
UUID=$(sudo blkid -s UUID -o value /dev/sdX1)
echo "UUID=$UUID /mnt/immich ext4 defaults,noatime,nofail,x-systemd.device-timeout=15s 0 2" \
  | sudo tee -a /etc/fstab
sudo systemctl daemon-reload
sudo mount -a
findmnt /mnt/immich
```

`nofail` keeps the machine bootable when the disk is unplugged. It also means
`/mnt/immich` silently becomes an empty directory on the root filesystem, so
Immich must refuse to start without the real mount — the `ExecStartPre` guard
in step 3 does that.

Then create the media directory (ownership is set in step 2, after the service
user exists):

```sh
sudo mkdir -p /mnt/immich/data
```

Decide on LUKS before the first upload. If the disk is encrypted, it must be
unlocked before Immich starts (key file or TPM); otherwise the same guard
protects you, but backups from phones will fail until it is unlocked.

## 2. Create the service user

```sh
sudo useradd -m -s /bin/bash immich
sudo loginctl enable-linger immich
grep immich /etc/subuid /etc/subgid

sudo chown immich:immich /mnt/immich/data
sudo chmod 750 /mnt/immich/data
```

Linger starts the user manager at boot, so the pod comes up without a login.
`useradd` normally allocates a free subuid/subgid range; if the `grep` prints
nothing, add one that does not overlap your desktop user's range, then run
`podman system migrate` as `immich`.

Work as `immich` for the rest of the setup:

```sh
sudo machinectl shell immich@
```

Rootless mapping matters here. Container **root** maps to the host `immich`
user, and the Immich server and ML images run as root, so the server writes
`/mnt/immich/data` as `immich` with no `:U` chown and no `keep-id`. The
official rootless Compose file instead runs everything as `1000:1000`, which
maps into the subuid range and needs `userns_mode: keep-id`; do not copy that
part into Quadlet.

## 3. Describe the pod

Create the database password as a Podman secret (not in a unit file):

```sh
openssl rand -hex 24 | tr -d '\n' | podman secret create immich-db-password -
```

Create `~/.config/containers/systemd/` and these five files:

```sh
mkdir -p ~/.config/containers/systemd
cd ~/.config/containers/systemd
```

`immich.pod` — publishes the web port and gives the containers one shared
network namespace, so they reach each other on `127.0.0.1`:

```ini
[Pod]
PodName=immich
PublishPort=2283:2283
ShmSize=128m

[Install]
WantedBy=default.target
```

`immich-db.container` — database on an internal named volume:

```ini
[Unit]
Description=Immich database

[Container]
ContainerName=immich-db
Image=ghcr.io/immich-app/postgres:14-vectorchord0.4.3-pgvectors0.2.0
Pod=immich.pod
Volume=immich-db:/var/lib/postgresql/data
Environment=POSTGRES_USER=postgres POSTGRES_DB=immich POSTGRES_INITDB_ARGS=--data-checksums
Secret=immich-db-password,type=env,target=POSTGRES_PASSWORD

[Service]
Restart=always
TimeoutStartSec=900
```

`immich-redis.container`:

```ini
[Unit]
Description=Immich Valkey

[Container]
ContainerName=immich-redis
Image=docker.io/valkey/valkey:9
Pod=immich.pod

[Service]
Restart=always
```

`immich-ml.container` — model cache stays on the internal disk:

```ini
[Unit]
Description=Immich machine learning

[Container]
ContainerName=immich-ml
Image=ghcr.io/immich-app/immich-machine-learning:v3
Pod=immich.pod
Volume=immich-model-cache:/cache

[Service]
Restart=always
```

`immich-server.container` — the SSD bind mount, plus the guard and retry that
make a missing USB disk a wait instead of a corruption:

```ini
[Unit]
Description=Immich server
Requires=immich-db.service immich-redis.service
After=immich-db.service immich-redis.service
StartLimitIntervalSec=0

[Container]
ContainerName=immich-server
Image=ghcr.io/immich-app/immich-server:v3
Pod=immich.pod
Volume=/mnt/immich/data:/data
Volume=/etc/localtime:/etc/localtime:ro
Environment=TZ=Europe/Amsterdam
Environment=DB_HOSTNAME=127.0.0.1 DB_USERNAME=postgres DB_DATABASE_NAME=immich
Environment=REDIS_HOSTNAME=127.0.0.1
Environment=IMMICH_MACHINE_LEARNING_URL=http://127.0.0.1:3003
Secret=immich-db-password,type=env,target=DB_PASSWORD

[Service]
Restart=always
RestartSec=15s
TimeoutStartSec=900
ExecStartPre=/usr/bin/mountpoint -q /mnt/immich
```

The containers need no `[Install]` section: Quadlet adds `Wants=` for every
container that references `immich.pod` to the pod unit, so starting the pod
starts them all and the pod is what boots.

Start it and check:

```sh
systemctl --user daemon-reload
systemctl --user start immich-pod
systemctl --user status immich-pod immich-server
podman pod ps
```

The first start pulls roughly 3.6 GB of images and takes a few minutes. The ML
container later downloads about 800 MB of models into its volume.

## 4. First login and phones

Open `http://<server-lan-ip>:2283`, choose **Getting Started**, and create the
admin account. Immich has no open sign-up. Then, in **Administration → Users**,
create accounts for the other people who will back up.

On each phone install the Immich app and use
`http://<server-lan-ip>:2283` as the server URL. That is HTTP on the LAN, which
is what Immich documents for local use. Do not forward `2283` on the router;
for access from outside the LAN, put a TLS reverse proxy or VPN in front later
and publish the port on `127.0.0.1` instead of all interfaces.

Allow the port through ufw for the LAN only (adjust the subnet if it changes):

```sh
sudo ufw allow from <lan-subnet> to any port 2283 proto tcp
```

## 5. Where the files actually go

With the default storage template **off**, the SSD looks like this:

| Path | Contents |
| --- | --- |
| `/mnt/immich/data/upload/<user-id>/` | Original photos and videos, named by Immich. |
| `/mnt/immich/data/thumbs/` | Generated thumbnails. |
| `/mnt/immich/data/encoded-video/` | Transcoded videos; originals are kept. |
| `/mnt/immich/data/profile/` | Avatars. |
| `/mnt/immich/data/backups/` | Immich's database dumps (default: daily 02:00, keep 14). |
| `/mnt/immich/data/library/` | Only used when the storage template is on. |

Do not move, rename or delete files under `data/` by hand; the database stores
the paths and Immich does not rescan its own library. Everything goes through
the web or mobile app.

If a browsable `YYYY/YYYY-MM-DD/filename` tree matters, enable **Administration
→ Settings → Storage Template** *before* the first big upload, then run the
storage-template migration job. Existing files get moved under `library/`. This
is a one-way decision in practice: turning it off later leaves the tree in
place.

If the SSD already contains a curated archive that should be indexed rather
than copied, do not mount it as `/data`. Mount it read-only as an extra volume
(for example `/mnt/immich/archive:/archive:ro`) and add `/archive` under
**Administration → External Libraries**. Use the path as the container sees it
(`/archive`), not the host path. Immich indexes those files in place; they are
not uploaded, deduplicated, or managed by the storage template, and the
read-only mount prevents deleting them from the web UI.

## 6. Backups

The SSD is primary storage, not a backup. A dead USB controller loses
everything on it. Immich's automatic dumps live on the same disk, so they do
not count either.

The minimum worth doing:

1. Keep **Administration → Settings → Database Dump Settings** enabled. The
   dumps land in `/mnt/immich/data/backups/`.
2. Copy the library and the dumps to the second disk regularly, database first:

   ```sh
   sudo rsync -rlt --delete /mnt/immich/data/ /mnt/external/immich/
   ```

   `/mnt/external` is the second 1 TB disk (exFAT, so `-rlt` instead of
   `-a`: exFAT cannot store Unix ownership or permissions). Do **not** use
   `/srv` on the internal disk; it has only ~18 GiB free and cannot hold the
   library. Check `findmnt /mnt/external` and `df -h /mnt/external` first: if
   the disk is not mounted, rsync would write to the internal disk instead.
   Both disks are 1 TB, so the backup only fits if the second disk is mostly
   empty. `--delete` mirrors deletions; make sure `/mnt/external/immich` is the
   right target before the first run.
3. Take an immediate dump before upgrades:

   ```sh
   mkdir -p ~/backup
   podman exec immich-db pg_dump --clean --if-exists --dbname=immich --username=postgres \
     | gzip > ~/backup/immich-db-$(date +%F).sql.gz
   ```

4. Test a restore. A backup that has never been restored is a hypothesis. The
   supported path is **Administration → Maintenance → Restore database backup**;
   the command-line disaster path is in the
   [official backup docs](https://immich.app/docs/administration/backup-and-restore).

For a real 3-2-1 setup, add a third copy off-site (another disk taken elsewhere,
or restic/borg to a remote). `rsync` only gives you the second copy.

## 7. Upgrades

`:v3` is a moving tag, but Podman does not re-pull a tag that already exists
locally. Upgrading is therefore explicit:

```sh
podman exec immich-db pg_dump --clean --if-exists --dbname=immich --username=postgres \
  | gzip > ~/backup/pre-upgrade-$(date +%F).sql.gz
podman pull ghcr.io/immich-app/immich-server:v3
podman pull ghcr.io/immich-app/immich-machine-learning:v3
systemctl --user restart immich-server immich-ml
```

Read the release notes first. Major versions can change the API and require
matching mobile apps. Some releases also name a newer
`ghcr.io/immich-app/postgres` tag: edit `Image=` in `immich-db.container`, run
`systemctl --user daemon-reload`, and restart `immich-db` before the server.
Never change the Postgres major version on an existing volume. Downgrades are
not supported. Do not enable `podman auto-update` for this stack: an unattended
breaking change takes the backup target down while nobody is looking.

## 8. Operations and traps

- **SSD absent at boot:** `immich-server` retries every 15 s and never writes
  to the root filesystem. Plug the disk, run `sudo mount /mnt/immich` if it is
  not mounted automatically, and the next retry starts the server.
- **Logs:** `podman logs -f immich-server`; `journalctl --user -u
  immich-server -f`.
- **Disk use:** `df -h / /mnt/immich`; `podman system df`. The root filesystem
  is the scarce one. Prune old images after upgrades (`podman image prune`),
  never the volumes.
- **SELinux hosts:** add `:z` to the two bind mounts. CachyOS runs without
  SELinux, so adding it here is unnecessary noise.
- **Podman on a rolling distro:** a Podman upgrade can change Quadlet
  behaviour. After a Podman upgrade, `systemctl --user daemon-reload` and check
  `systemctl --user status immich-pod` before assuming all is well.
- **Do not** copy the rootless Compose file's `user: '1000:1000'` into Quadlet;
  that is the rootless-Compose/keep-id trap described in step 2.
- **Do not** put the Postgres volume on the USB SSD. A brief unplug during a
  write is how a database gets corrupted.
- **Do not** mount the SSD root as `/data`. Keep the `data/` subdirectory so a
  future use of the disk does not collide with Immich's layout.

A deployment is not done until a real phone upload and a restore from an
automatic dump have both succeeded.
