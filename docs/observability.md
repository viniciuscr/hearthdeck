# Backend Observability

## Logging Model

Services use Rust `tracing`, not ad hoc prints. Every service writes one event
per line to stderr; systemd captures it in the user journal.

- Production default: newline-delimited JSON (`HEARTHDECK_LOG_FORMAT=json`).
- Development: readable output (`HEARTHDECK_LOG_FORMAT=pretty`).
- Filtering: `RUST_LOG`, for example `RUST_LOG=hearthdeck_daemon=debug,tower_http=info`.
- Service identity: systemd assigns `SYSLOG_IDENTIFIER` as `hearthdeck-daemon` or
  `hearthdeck-bridge`.

Do not log bearer tokens, pairing codes, certificate/private-key paths,
authorization headers, request bodies, desktop `Exec` strings, or media paths.

## Events

| Area | Fields |
| --- | --- |
| HTTP | `request_id`, `method`, `path`, `status_code`, `latency_ms` |
| Pairing | client ID and expiry, never code/token |
| Discovery | `source_id`, queue/coalesce state, `record_count`, `duration_ms` |
| Catalog | `source_id`, `record_count` |
| Bridge | socket lifecycle, desktop entry count, desktop ID launch outcome |
| Startup | service version, bind addresses, transport mode, database readiness |

The request trace intentionally excludes headers and bodies. It correlates API
logs with the `x-request-id` response header.

`GET /v1/health` is the current provider-health snapshot. It is the source of
truth for whether a catalog source is `starting`, `ready`, or `degraded`; use
logs for the associated operation detail.

For catalog synchronization, expect this lifecycle in order:

1. `all discovery providers refresh requested` or a source refresh event.
2. `discovery started` with `source_id`.
3. `catalog source replaced` with `record_count`.
4. `discovery completed` with `duration_ms`.
5. A `library_changed` WebSocket event to live Flutter clients.

## Operations

```sh
just services-logs
just logs-daemon
just logs-bridge
just logs-errors
```

For local readable development output:

```sh
HEARTHDECK_LOG_FORMAT=pretty RUST_LOG=hearthdeck_daemon=debug just daemon
HEARTHDECK_LOG_FORMAT=pretty RUST_LOG=hearthdeck_bridge=debug just bridge
```

To retain logs from the combined development target:

```sh
HEARTHDECK_LOG_FORMAT=pretty HEARTHDECK_DEV_LOG_DIR=/tmp/hearthdeck-logs just dev
tail -f /tmp/hearthdeck-logs/daemon.log /tmp/hearthdeck-logs/bridge.log
```

`just dev` follows bridge and daemon logs in the terminal by default. Disable
that only for quiet automation:

```sh
HEARTHDECK_DEV_SHOW_LOGS=false just dev
```

For a bounded incident window on Linux:

```sh
journalctl --user -u hearthdeck-daemon.service --since '15 minutes ago' -o json-pretty
```

## Desktop Discovery

Every bridge scan logs each directory, the number of `.desktop` candidates,
and the number of accepted application entries at the default `info` level.
This identifies whether a host has no desktop entries in a scanned location or
whether entries are rejected by the visibility/application checks:

```sh
journalctl --user -u hearthdeck-bridge.service --since '10 minutes ago' -o cat
```

For a temporary list of accepted application IDs and titles, add a user-unit
drop-in, reload, and restart the bridge:

```sh
systemctl --user edit hearthdeck-bridge.service
# Add: [Service]\nEnvironment=RUST_LOG=hearthdeck_bridge=debug
systemctl --user daemon-reload
systemctl --user restart hearthdeck-bridge.service hearthdeck-daemon.service
journalctl --user -u hearthdeck-bridge.service -f -o cat
```

The bridge never logs desktop-entry `Exec` values.

## Investigation Order

1. Filter daemon logs by `request_id` or `item_id`.
2. Check discovery records for `source_id`, `duration_ms`, and `record_count`.
3. Check bridge logs for the matching desktop ID outcome.
4. Check `systemctl --user status` if no lifecycle event exists.

Absence of a discovery completion event means the provider failed before
catalog persistence. A catalog completion event with zero records means the
provider completed successfully but found no source content.

With the Linux service graph, inspect the target and socket before restarting a
bridge process manually:

```sh
systemctl --user status hearthdeck.target hearthdeck-bridge.socket hearthdeck-daemon.service
```
