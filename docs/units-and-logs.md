# Units, Sessions, and Logs: the traps we keep falling into

This exists because the same class of failure has cost us more than once: a unit
that silently does nothing, a log line that never reaches `~/hearthdeck.log`, a
feature that looks missing because the running binary predates it. Each section is
a trap we actually hit, the rule that avoids it, and how to check.

Read this before changing a systemd unit, the session script, the log collector,
the SQLite schema, or `packaging/arch/PKGBUILD`.

## The graph

```
greetd / display manager
  -> /usr/lib/hearthdeck/hearthdeck-session          (packaging/arch/hearthdeck-session)
       -> systemctl --user start hearthdeck.target
            Wants= hearthdeck-log.service, hearthdeck-bridge.socket,
                   hearthdeck-daemon.service, hearthdeck-input.service,
                   romm.service, romm.path, hearthdeck-overlay.service
```

- Units live in `/usr/lib/systemd/user/` (packaged) or `~/.config/systemd/user/`
  (a checkout via `just install-services`).
- `hearthdeck.target` is globally disabled and started explicitly: by
  `default.target.wants/hearthdeck.target` from the package install, and by the
  session script. Starting it twice is a no-op.
- `PartOf=hearthdeck.target` means stopping/restarting the target stops/restarts
  the unit with it. That is why the whole stack dies at logout.

## Rule 1 — a non-zero `ExecCondition=` is a silent skip, not an error

`ExecCondition=` returning non-zero puts the unit in `inactive`, not `failed`. It
is not retried. `ExecStopPost=` **still runs**, which is why the log shows a
misleading `... stopped` line and nothing else.

Only encode *permanent* facts in a condition ("this machine has no RomM, so
there is no compose file"). A fact that can become true later — a mount still
coming up, a file not written yet, a socket not listening — must **not** be a
condition. Give it ordering, or a `.path` watcher.

Check: `systemctl --user show <unit> -p ConditionResult`.

## Rule 2 — a user unit cannot order itself after a system mount

`RequiresMountsFor=/mnt/external` on a `systemd --user` unit does not do what it
does for a system unit: the mount unit lives in the system manager. `%h`-relative
paths under the user's home do work.

So "start when the file appears" is a `.path` unit:

```ini
[Path]
PathExists=/mnt/external/romM/podman-compose.yaml
Unit=romm.service
```

A `.path` unit is inert when the file never appears, so it costs nothing on a
machine that does not use the feature, and it fires the moment the mount lands.

Limitation: a `.path` unit cannot read an `EnvironmentFile`, so it can only watch
a fixed path. If the path is overridden at runtime, the watcher has to be told the
same default.

## Rule 3 — arm a helper from the unit that is actually wanted

A host running an older package has an older `hearthdeck.target`. If the new
`Wants=` line lives only in the packaged target, that host never starts the new
unit — its RomM dependency comes from a drop-in instead.

If a helper exists only to help `X`, make `X` want it:

```ini
# romm.service
[Unit]
Wants=romm.path
```

Then it is armed wherever `X` is pulled in, whatever target did the pulling.

## Rule 4 — `~/.config/systemd/user/` shadows the package

A full copy of a unit there overrides `/usr/lib/systemd/user/` and silently masks
package updates. `just install-services` writes full copies; the package writes
`/usr/lib`. To extend a unit without shadowing it, use a drop-in
(`<unit>.d/*.conf`) — `Wants=` in a drop-in merges, it does not replace.

## Rule 5 — know which binary is actually running

`/usr/bin/hearthdeck-frontend` (package) is not
`services/target/release/hearthdeck-frontend` (checkout). A stale one presents as
"the feature is missing" or as a UI that reports success for something the daemon
no longer does.

Check before believing a bug report:

```sh
grep -a 'a string unique to the change' "$(command -v hearthdeck-frontend)"
ls -l --time-style=long-iso /usr/bin/hearthdeck-frontend services/target/release/hearthdeck-frontend
```

## Rule 6 — never report success for a no-op

HTTP 2xx means "accepted", not "changed". If an operation's useful outcome can be
zero — "create the dashboard collections" with a report that implies none — return
the outcome (a count) and let the caller say "nothing to create". `204 No Content`
for a no-op is a lie the UI will repeat.

## Logging: how `~/hearthdeck.log` is built, and how to add a source

`hearthdeck-log.service` truncates the file at session start and runs
`journalctl --user --follow`. It matches two ways, in **two separate streams**:

```
journalctl --user --follow --since=-30s --identifier=hearthdeck-session &
journalctl --user --follow --since=-30s -u <unit> -u <unit> ...
```

- Matches of different kinds are **ANDed**. `--identifier=X -u Y` matches nothing.
  This `journalctl` also rejects `+`, so a source is either an identifier or a
  unit — never mixed in one stream. That is why there are two.
- Use `-u <unit>` for anything that is a unit: it captures the unit's own output
  **and** systemd's messages about it (`Skipped due to 'exec-condition'`,
  `start operation timed out`, `Failed with result`, `Stopped`). `--identifier=`
  only captures the process's own lines.
- Use `--identifier=<id>` only for streams that are not a unit — the session
  script tags its output (and everything it launches) with
  `systemd-cat -t hearthdeck-session`.
- `--since=-30s` means the **previous** session's teardown can appear at the top
  of a new file. Do not read the first lines as this session's.
- Every new packaged unit must be added to the collector's `-u` list. The test
  `the_session_log_collector_follows_every_packaged_user_unit` parses the PKGBUILD
  and fails if you forget.

## Runbook: a unit that did not start

```sh
systemctl --user status <unit> --no-pager
systemctl --user show <unit> -p ConditionResult -p ExecCondition -p Wants -p After -p Result -p ExecMainStatus
systemctl --user cat <unit>           # what is actually loaded, drop-ins included
journalctl --user -u <unit> -b --no-pager
grep -i <unit> ~/hearthdeck.log
```

- `ConditionResult=no` → skipped by a condition (Rule 1).
- `Result=timeout` → the command outran `TimeoutStartSec`.
- `Wants=`/`After=` missing the unit → nothing pulled it in (Rule 3).
- `could not be found` → the package you are running predates the unit.

## SQLite migrations

- `CREATE TABLE IF NOT EXISTS` does **not** add columns to an existing table.
- `ALTER TABLE ... ADD COLUMN` has **no** `IF NOT EXISTS`; the duplicate-column
  error is swallowed on purpose.
- Therefore any statement that *names* a new column must run **after** its ALTER.
  A seed `INSERT` that listed `owner` before the `ALTER TABLE ... ADD COLUMN owner`
  made every pre-existing database fail to migrate, and the daemon exited.
- Test a **legacy** schema, not only a fresh database. A fresh `CREATE` builds the
  table with the new column and hides the bug.

## PKGBUILD bash

- Inside `'...'`, `\'` does not escape the quote: it ends the string and eats the
  closing `)`. Never put an apostrophe in a single-quoted array — reword it.
- Check before pushing: `bash -n packaging/arch/PKGBUILD`.

## Checklist

- [ ] `systemd-analyze verify <unit>` is clean.
- [ ] New unit is in the PKGBUILD, in `just install-services`, **and** in the log
      collector.
- [ ] A helper is armed from the unit that is actually wanted (Rule 3).
- [ ] No transient fact in an `ExecCondition=` (Rule 1).
- [ ] Conditions, `Wants=`, and `After=` inspected with `systemctl --user show` on a
      real host, not assumed.
- [ ] The deployed binary contains the change (Rule 5).
- [ ] `bash -n packaging/arch/PKGBUILD` and `cargo test -p hearthdeck-daemon` pass.
- [ ] `just format` has been run. CI (`just ci-check`) begins with
      `cargo fmt --all -- --check` and fails the entire run on a single diff, even
      when the code is otherwise correct. `just check` covers it locally.
