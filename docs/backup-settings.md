# Off-site backup: tooling and Settings contract

Optional nightly upload of game saves, database dumps and Hearthdeck metadata to
an rclone remote. This doc owns the split between the packaged tooling, the
per-user configuration, and the Settings section that will expose it.

**Status:** the tooling is packaged and installed **disabled**, and the settings
window's **Backup** page is what turns it on. There is no install script and no
manual enable step: package installation makes the feature available, and that
page is the only activation path.

## What the user configures

The page has four controls and a read-only last-run card:

| Control | Meaning | Default |
| --- | --- | --- |
| Nightly copy (switch) | Arms the timer. It is a **draft** until Save, and the card's chip reads `Unsaved` until then | off |
| Remote | Name of an existing rclone remote (`rclone listremotes`) | `pcloud` |
| Destination root | Folder under that remote | `hearthdeck-backups` |
| Schedule | Local hour and minute, stepped with the d-pad | 04:30 |
| Save and apply | Writes both files, reloads the units and moves the timer | — |
| Last run | Result, finish time, next run | read-only |
| Run now | Starts one run immediately | — |

Credentials are entered **once**, in the Backup page, and the daemon stores them
in a `0600` file it owns (`~/.config/hearthdeck-backup/credentials.env`).
rclone's own configuration never holds one: a run receives it through the
environment (`RCLONE_CONFIG_PCLOUD_*`) with `--config=""`. Who may see what, and
why this replaced "configure it in rclone first", is in
[`provider-credentials.md`](provider-credentials.md).

The page shows the precondition plainly: if the selected remote is not in
`rclone listremotes`, a "Remote not found" row sits directly above the switch, the
switch is disabled, and the save is refused with 409 rather than stored.
Enabling a timer with no remote would only produce nightly failures.

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
| `~/.config/hearthdeck-backup/credentials.env` | user, written by the daemon | provider credential (`0600`); the unit reads it with `EnvironmentFile=-` |
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

## Backend contract

The daemon module is `backup` (`backup.rs` for the files and the schedule,
`backup/control.rs` for the systemd adapter), following the
`categorizer/control.rs`, `romm_link/control.rs` and `stremio/control.rs` split.
The daemon spawns fixed `systemctl --user` and `journalctl --user` commands and
never accepts a unit name or argv from the API.

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

`next_run` is computed from the schedule the daemon wrote, in local time and
converted to UTC, rather than read back from systemd: the timer's own
next-elapse property is a localized string, and the schedule is the value this
daemon owns. `last_finished_at` comes from the journal (`__REALTIME_TIMESTAMP`),
because `InactiveEnterTimestamp` is localized too. `last_result` is the service's
`Result`, with `ExecMainStatus` appended when it is not zero -
`failed (exit 1)`.

Source it from one `systemctl --user show` per unit (`ActiveState`, `Result`,
`ExecMainStatus`) plus a bounded journal tail. A manager that cannot answer is a
502, not an idle timer: an empty answer is not the same as an inactive unit.
Cache the answer for a few seconds, like diagnostics does, so a polling UI cannot
spawn `systemctl` storms.

`POST /v1/backup/run` starts `hearthdeck-backup.service` and returns `202
Accepted`; it must not wait for the run. The UI polls `status` until the service
is inactive.

### Systemd operations

Fixed argv only:

```
systemctl --user cat hearthdeck-backup.service                       # is it installed?
systemctl --user show hearthdeck-backup.timer --property=ActiveState
systemctl --user show hearthdeck-backup.service --property=ActiveState \
    --property=Result --property=ExecMainStatus
systemctl --user start --no-block hearthdeck-backup.service
systemctl --user enable --now hearthdeck-backup.timer
systemctl --user disable --now hearthdeck-backup.timer
systemctl --user daemon-reload
journalctl --user -u hearthdeck-backup.service -n 1 --no-pager --output=json
journalctl --user -u hearthdeck-backup.service -n 20 --no-pager --output=cat
rclone listremotes
```

`start` is `--no-block` on purpose: the unit is `Type=oneshot` with a six-hour
start timeout, and `POST /v1/backup/run` promises 202 immediately. A blocking
start would hold the request for the whole backup and make the client time out
while the run is going fine.

`ActiveState` is read instead of `is-active`, and `active`, `activating` and
`reloading` all mean "running": a oneshot unit is `activating` for the entire
run, so an equality check on `active` would report the busy state as idle.

The schedule lives in a drop-in written by the daemon:

```ini
# ~/.config/systemd/user/hearthdeck-backup.timer.d/50-schedule.conf
[Timer]
OnCalendar=
OnCalendar=*-*-* 04:30:00
```

The empty `OnCalendar=` is not decoration: the setting is a list, and it
accumulates across the packaged unit and every drop-in. Without the reset the
user's time would be *added* to the packaged 04:30, so a schedule of 02:05 would
fire twice a night while the API reported only 02:05.

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
| Service manager unreachable | "The user service manager could not …" (502) — the read or the write failed at systemd, not at Hearthdeck. |

Errors come from the API as `{"error": "..."}`; map 409 to the precondition
caption and 503 to the not-installed state. Never show a credential.

## Where the UI lives

The section is the **Backup** page of the GPUI settings window
(`services/hearthdeck-settings`; see `docs/settings-window.md`), not the iced
page: a card of rows for the destination and the schedule, a `TimeField` for the
clock, and `Save and apply` / `Run now` actions. The iced page carries no backup
section, and there is no second activation path.

### Where failures show up

Every refusal is also a line in the daemon's own log, naming the command or the
unit that failed: `journalctl --user -u hearthdeck-daemon.service`. That is the
first place to look when a save is refused or the card says the manager could not
be reached.

The runner's own words — rclone's error, the job it skipped and why — are in the
backup unit's journal, which the session log collector already follows:

```bash
journalctl --user -u hearthdeck-backup.service -n 50
grep -i backup ~/hearthdeck.log
```

The settings window logs three things itself: a write it could not make, a state
it could not re-read, and a run it stopped following. It is a child of the
library app, so those lines land in the library app's output rather than the
daemon's — `journalctl --user -b | grep -i backup` finds them.

## Migration and safety notes

- The old Settings screen gets no backup section and there is no fallback
  installer. Installation only ships the disabled units; the new Settings
  toggle is what enables the timer, writes `backup.env` and the schedule
  drop-in. Do not add a second activation path.
- `jobs.conf` is sourced as bash. The daemon must not write it from API input.
  Future free-form job editing requires moving the runner to a structured
  format first.
- The timer must stay disabled after a package install. A nightly job with no
  remote is noise, not a feature.
- The runner and timer are package-owned so `pacman -Syu` updates them. Only the
  destination, credentials, job overrides and enablement are per user.
