# Provider credentials

How a cloud provider's credential gets into Hearthdeck, who owns it, and what
rclone is allowed to see. Status: **the daemon half is implemented** — `backup/providers.rs`, the four routes and their tests. The settings-page card and the unit's `EnvironmentFile` line are next. pCloud is the
first provider; the layer is a registry so a second one is an entry, not a second
flow.

This supersedes the rule in [`backup-settings.md`](backup-settings.md) that said
Hearthdeck must never collect a cloud credential. That rule existed so the shell
never held an account secret. It is replaced by a narrower one, below, which keeps
the property that mattered — rclone's config never holds a secret — while giving a
television a way to set a provider up at all.

## Decisions

- **The daemon accepts a credential once**, validates it, and stores it in a
  `0600` file it owns: `~/.config/hearthdeck-backup/credentials.env`.
- **rclone never sees a credential in a config file.** The daemon runs rclone with
  `--config=""` and the credential in the environment
  (`RCLONE_CONFIG_PCLOUD_TYPE`, `…_TOKEN`, `…_HOSTNAME`).
- **The settings page sends the credential and forgets it.** It never writes a
  file, never keeps the value after the request, and never receives it back.
- **pCloud is authenticated with email and password**: the daemon calls pCloud's
  `getauth` and stores the token it returns.
- **A provider is configured only when it has been verified**, not when a file was
  written: setup ends with a real `rclone lsd pcloud:`.

## Why not rclone.conf

Checked against rclone's documentation while designing this:

- rclone's `pcloud` backend is **OAuth2-only** — `client_id`, `client_secret`,
  `token` (an OAuth JSON blob), `hostname`, `root_folder_id`. There is no
  `auth_token` option and rclone never sends pCloud's `auth=` parameter.
- rclone can encrypt its config, but the password then needs a home of its own, so
  on a single-user box the secret is still at rest — in two places instead of one.
- systemd *user*-unit credentials (`systemd-creds --user`) derive their key from
  UID, user name and machine id: they obfuscate, they do not protect.
- A desktop keyring is available on the dev box (`org.freedesktop.secrets`), but a
  kiosk session may start with the login keyring locked, which is the wrong thing
  to discover during a restore.

So the choice is the honest one: a `0600` file with the same protection as today's
`rclone.conf`, owned by the daemon, read only when a run or a verification needs
it, and never mirrored into rclone's own configuration.

## Threat model, stated once

- Anyone who can read the user's home reads the credential. That is true of
  `rclone.conf` today and is not made worse here.
- No defence is claimed against a root compromise, a compromised `hearthdeck`
  binary, or a malicious pCloud client. The rule below is about *our* mistakes,
  not about an attacker who already owns the box.

## Non-negotiables

- The password travels in a **POST body**, never in a URL, never in argv (argv is
  world-readable in `ps`), and never in a log line.
- Error text is redacted before it reaches the API or the page: a refusal must not
  echo the credential.
- `GET` responses never contain the secret — only `configured`, `verified_at`,
  `hostname`, `remote_available`.
- The stored file is written atomically (`0600`, temp file in the same directory,
  rename), the way `backup.env` already is.

## The token question — answered by rclone's own docs

`rclone.org/pcloud/` settles what was an open risk here:

- rclone's `token` option is "OAuth Access Token as a JSON blob", and its example
  is `{"access_token":"XXX","token_type":"bearer","expiry":"0001-01-01T00:00:00Z"}`.
  pCloud's `getauth` token **is** that access token, so the blob we store matches
  rclone's own shape (the zero expiry included, because pCloud's tokens are not
  time-bound).
- `--pcloud-username` / `--pcloud-password` exist "only … when you want to use the
  cleanup command. Due to a bug in the pcloud API the required API does not support
  OAuth authentication so we have to rely on user password authentication for it."
  Password authentication is therefore a supported pCloud path, not a workaround.
- `hostname` is `api.pcloud.com`, or `eapi.pcloud.com` for EU accounts, and must be
  set by hand for a remote configured outside `rclone config` — which is why the
  store keeps it next to the token.

Setup still verifies the token against `userinfo` in the Bearer form before
storing it, and still fails loudly:

- setup validates the token against `userinfo` using the same header rclone will
  send, and **fails loudly** with a message naming the alternative if pCloud
  refuses it;
- the OAuth-blob paste path (`rclone authorize pcloud` on a machine with a
  browser, paste the JSON into a masked field) stays available as the fallback;
- accounts with 2FA may not complete `getauth` at all; the same fallback covers
  them.

## API (protected router, paired token)

```
GET    /v1/backup/providers                 → [{id, configured, verified_at, hostname, remote_available}]
PUT    /v1/backup/providers/pcloud          {email, password} → {configured, verified_at, hostname}
DELETE /v1/backup/providers/pcloud          → 204
POST   /v1/backup/providers/pcloud/verify   → {verified_at} | 502
```

Typed failures: `400` when a field is not valid, `401`/`502` with a redacted reason
when pCloud refuses the credential, `503` when the units or `rclone` are not
installed. None of them quotes the credential.

## Modules

- `services/hearthdeck-daemon/src/backup/providers.rs` — the registry, a
  `PcloudApi` trait (`getauth`, `userinfo`) with a reqwest implementation and a
  fake for tests, and `ProviderStore` for the `0600` file.
- `services/hearthdeck-daemon/src/backup/control.rs` — `CommandRunner` grows
  environment support so verification can run
  `rclone --config="" lsd pcloud:` with the credential in the child's environment.
- `packaging/arch/hearthdeck-backup.service` — gains
  `EnvironmentFile=-%h/.config/hearthdeck-backup/credentials.env`, which is how a
  scheduled run inherits the credential. The runner itself is unchanged.
- `services/hearthdeck-settings` Backup page — a "pCloud account" card: email, a
  masked password, `Connect`, `Forget`, a state chip (`Connected`, `Not set`,
  `Needs verification`) and the redacted error line. The password field is cleared
  after a submit and is never prefilled.

## Tests

- A fake `PcloudApi` drives the daemon: success stores and reports `verified_at`;
  a refused password reports a redacted 502; a bearer-refused token returns the
  typed error that points at the paste fallback; `DELETE` forgets.
- The store's file is `0600` and the listing never contains the token.
- `CommandRunner` tests assert the argv (`--config=""`) and that the credential
  reaches the child only through the environment.
- A render test for the card: the password field is masked, `Connect` disabled
  while a write is in flight, and the state chip reads correctly for each state.

## Rollout

1. Daemon: `providers.rs`, the four routes, OpenAPI entries, tests.
2. Packaging: the unit now reads `credentials.env` with `EnvironmentFile=-`, so a
   scheduled run inherits the token; a test pins that line. Still to do here: one
   real pCloud account on the box.
3. Settings page card, then the TV.
