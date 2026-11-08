# Off-site backup: tooling and Settings contract

Optional nightly upload of game saves, database dumps and Hearthdeck metadata to
an rclone remote. This doc owns the split between the packaged tooling, the
per-user configuration, and the Settings section that will expose it.

**Status:** the tooling is packaged and usable today via
`deploy/backup/install-backup.sh`. The Settings section is deliberately not
implemented in the current screen because the frontend is being rewritten; the
contract below is what the new screen should implement.

## What the user configures

The section has four controls and one read-only status area:

| Control | Meaning | Default |
| --- | --- | --- |
| Enable off-site backup | Enables the systemd timer | off |
| Remote | Name of an existing rclone remote (`rclone listremotes`) | `pcloud` |
| Destination root | Folder under that remote | `hearthdeck-backups` |
| Schedule | Local hour and minute for the nightly run | 04:30 |
| Last run | Result, finish time, next run | read-only |
| Run now | Starts one run immediately | — |

Do **not** collect cloud credentials in Hearthdeck. rclone owns them in
`~/.config/rclone/rclone.conf`; the UI selects a remote by name. This mirrors the
Stremio rule that the shell never holds the account credential.

The section must show the precondition plainly: if the selected remote is not in
`rclone listremotes`, the Enable toggle is disabled with an explanation and a
short "how to add it" caption (`rclone config`). Enabling a timer with no remote
would only produce nightly failures.

## What is backed up

Fixed defaults, shipped in `/usr/lib/hearthdeck/hearthdeck-backup.d/jobs.conf`:

| Data | Mode |
| --- | --- |
| Immich database dumps | `sync` (rotating dumps) |
| RomM database dumps | `sync` (rotating dumps) |
| RetroArch saves (`.srm`, `.raw`, `.gci`) | `copy` |
| Save states | `copy` |
| PS2 memory cards | `copy` |
| Dreamcast VMUs | `copy` |
| Hearthdeck DB snapshots | `sync` (newest 14) |
| Hearthdeck config and RetroArch core config | `copy` |
| Hearthdeck metadata (no models, no shader cache) | `copy` |

`copy` never deletes remote files. `sync` is allowed only for rotating data and
is skipped when the source is empty, the newest dump fails `gzip -t`, is older
than 72h, or the DB snapshot could not be created. External-disk jobs require
`/mnt/external` to be mounted and its `.hearthdeck-backup` sentinel to exist.

## File ownership

| Path | Owner | Purpose |
| --- | --- | --- |
| `/usr/lib/hearthdeck/hearthdeck-backup` | package | runner, updated by `pacman -Syu` |
| `/usr/lib/hearthdeck/hearthdeck-backup.d/jobs.conf` | package | default job list |
| `/usr/lib/hearthdeck/hearthdeck-backup.d/filters/` | package | default rclone filters |
| `/usr/lib/systemd/user/hearthdeck-backup.service` | package | oneshot runner |
| `/usr/lib/systemd/user/hearthdeck-backup.timer` | package | nightly schedule, disabled |
| `~/.config/hearthdeck-backup/backup.env` | user | remote and destination root |
| `~/.config/hearthdeck-backup/jobs.conf` | user | optional job override |
| `~/.config/hearthdeck-backup/filters/` | user | optional filter override |
| `~/.config/rclone/rclone.conf` | user | cloud credentials |
| `~/.local/state/hearthdeck-backup/` | user | DB snapshots, lock |

The runner uses a user `jobs.conf` or `filters/` only when they exist; otherwise
it reads the packaged copy. That keeps package updates effective by default.

`backup.env` is read by the service through `EnvironmentFile=` and contains only:

```sh
HEARTHDECK_BACKUP_REMOTE=pcloud
HEARTHDECK_BACKUP_ROOT=hearthdeck-backups
```

## Backend contract (for the new Settings screen)

Add a daemon module `backup` with a systemd adapter, following the existing
`categorizer/control.rs`, `romm_link/control.rs` and `stremio/control.rs` split.
The daemon already spawns fixed `systemctl --user` and `journalctl --user`
commands; keep that pattern and never accept a unit name or argv from the API.

### Routes

All under the protected router, so they require the paired bearer token.

```
GET  /v1/backup/settings
PUT  /v1/backup/settings
GET  /v1/backup/status
POST /v1/backup/run
```

`GET /v1/backup/settings`:

```json
{
  "enabled": true,
  "remote": "pcloud",
  "root": "hearthdeck-backups",
  "schedule": { "hour": 4, "minute": 30 },
  "remote_available": true,
  "updated_at": "2026-11-08T15:00:00Z"
}
```

`PUT /v1/backup/settings` accepts the same shape minus `remote_available` and
`updated_at`. Validation:

- `remote`: `^[A-Za-z0-9._-]+$`, at most 64 characters.
- `root`: relative path, no leading slash, no `..`, `^[A-Za-z0-9._/-]+$`.
- `hour`: 0–23; `minute`: 0–59.
- `enabled: true` is rejected when the remote is absent, with a 409 that the UI
  turns into the "add the remote first" caption.

Writes are atomic: a temp file in the same directory, then rename, mode `0600`,
directory `0700`.

`GET /v1/backup/status`:

```json
{
  "timer_active": true,
  "service_active": false,
  "next_run": "2026-11-09T04:30:00Z",
  "last_result": "success",
  "last_finished_at": "2026-11-08T04:36:12Z",
  "last_error": null
}
```

Source it from `systemctl --user show` (`ActiveState`, `Result`,
`ExecMainStatus`, `InactiveEnterTimestamp`, `NextElapseUSecRealtime`) plus a
bounded journal tail. Cache the answer for a few seconds, like diagnostics does,
so a polling UI cannot spawn `systemctl` storms.

`POST /v1/backup/run` starts `hearthdeck-backup.service` and returns `202
Accepted`; it must not wait for the run. The UI polls `status` until the service
is inactive.

### Systemd operations

Fixed argv only:

```
systemctl --user show hearthdeck-backup.service --property=...
systemctl --user show hearthdeck-backup.timer --property=...
systemctl --user start hearthdeck-backup.service
systemctl --user enable --now hearthdeck-backup.timer
systemctl --user disable --now hearthdeck-backup.timer
```

The schedule lives in a drop-in written by the daemon:

```ini
# ~/.config/systemd/user/hearthdeck-backup.timer.d/50-schedule.conf
[Timer]
OnCalendar=*-*-* 04:30:00
```

The daemon runs `systemctl --user daemon-reload` after writing it. A missing unit
(package not installed) returns a typed 503 "off-site backup is not installed",
not a 500.

### Error and UI states

| State | UI |
| --- | --- |
| Unit missing | "This Hearthdeck build does not include off-site backup." Enable disabled. |
| Remote missing | "Add the remote in rclone first." Enable disabled. |
| Disabled | Show destination + schedule, Enable action. |
| Enabled, idle | Show next run, Run now, Disable. |
| Running | Spinner/status, Run now disabled. |
| Failed | Last error from the service result, Run now, journal hint. |
| Run succeeded | Finish time and next run. |

Errors come from the API as `{"error": "..."}`; map 409 to the precondition
caption and 503 to the not-installed state. Never show a credential.

## Frontend integration notes

When the new Settings screen lands:

- Add `screens/settings/backup.rs` with its own `Message`, `Flow`/`State` and
  `update`-returned effects. Do not put daemon I/O in the view.
- Register the section in `screens/settings/mod.rs` and route effects through
  `app/settings_source.rs`, like RomM and Stremio.
- Add daemon client methods in `providers/daemon.rs`.
- Settings is currently non-walkable by the controller, so an interactive
  section needs the focus walk extended first. The backup section is not
  allowed to be pointer-only on a TV-first product.
- Reuse `titled_section`, `described_item`, `action_button`, `summary`,
  `error_line`, and the `toggler` pattern from categorization. There is no
  dropdown widget; schedule editing should use the existing `◀ value ▶`
  step control or two numeric rows.

## Migration and safety notes

- The old Settings screen gets no backup section; `deploy/backup/install-backup.sh`
  is the supported path until the rewrite is ready.
- `jobs.conf` is sourced as bash. The daemon must not write it from API input.
  Future free-form job editing requires moving the runner to a structured
  format first.
- The timer must stay disabled after a package install. A nightly job with no
  remote is noise, not a feature.
- The runner and timer are package-owned so `pacman -Syu` updates them. Only the
  destination, credentials, job overrides and enablement are per user.
