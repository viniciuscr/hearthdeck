# Stremio Integration

This is the working design doc and roadmap for reading a Stremio account's
**Continue Watching** list, linking that account by QR code, and putting the
result on the dashboard. It owns the Stremio decisions.
`backend-architecture.md` owns the provider/catalog shape these decisions plug
into; `kiosk-session.md` owns the launch rules the last phase must satisfy.

Everything under "Verified against the live service" was confirmed by a
read-only probe against a real account, not recalled from documentation or
community posts. Three widely repeated claims turned out to be wrong; they are
called out rather than quietly corrected.

## Goal

Three deliverables, in the order they unblock each other:

1. **Settings: link a Stremio account with a QR code.** No password field, ever.
2. **Dashboard: a Continue Watching rail**, showing unfinished films and series.
3. **A defensible home for the resulting credential** — which is *not* the Linux
   keyring, for the reasons below.

## Non-negotiable constraints (carried over from existing rules)

- **The frontend never talks to a third-party service.** It speaks only the
  paired daemon API (`backend-architecture.md`, "Frontend Catalog Boundary"), so
  all Stremio traffic lives in the daemon. This is also what keeps the session
  credential out of the UI process.
- **A source is a `DiscoveryProvider` with a stable `source_id`.** The
  architecture doc already reserves `stremio` as one (its provider list names
  `desktop-apps`, `steam`, `gog`, `epic`, `emulators`, `jellyfin`, `stremio`).
- **No command or URL field in the bridge protocol.** A deep link must therefore
  be assembled by the bridge from validated, typed components — never handed to
  it as a URL string. This mirrors how `LaunchRetroGame` carries a validated
  core path and ROM path instead of an `Exec` line.
- **A failed provider refresh must not delete rows it does not own.**
  `CatalogStore::replace_source` already gives this; do not bypass it.
- **Launch behaviour is not "done" until it is seen on real hardware.**
  `kiosk-session.md` is explicit that green CI proves only that code compiles.
- **Secrets are never logged.** The daemon logs launch plans and provider
  outcomes freely; the `authKey` must never appear in any of it.

## Why the keyring is not the answer

The obvious home for a session credential is the Secret Service. It does not
work here, and the reasons are worth writing down so nobody re-litigates it.

| Checked | Result |
| --- | --- |
| `gnome-keyring-daemon.socket` | **enabled** — the service is socket-activatable, so it *would* start on demand even in a bare `cosmic-comp` session |
| `org.freedesktop.secrets` on the session bus | present, exposes a `Default` collection, `default` aliases to it |
| `.../collection/Default` → `Locked` | **`true`** |
| `keyring\|kwallet` in `/etc/pam.d/*` | **no matches** — no keyring PAM module is configured anywhere, greetd's stack included |
| `/dev/tpm0`, `/dev/tpmrm0` | **absent** — no TPM on the reference host |
| `systemd-creds --user --with-key=host` | round-trips, but the man page documents that host key as **root-only** |

So the blocker is not starting the service — it is **unlocking** it. Nothing
unlocks the login collection at autologin, and an unattended daemon has no way
to answer a prompt. A locked collection is not a degraded store; it is a store
that fails.

`systemd-creds` is no better here: with no TPM the only key is the root-only
host key, which a **user** service cannot read without a root-side provisioning
step at install time. Encrypting with a key the daemon itself can read adds
nothing against anyone who already has that user's files.

**Decision:** store the `authKey` at mode `0600` in the daemon's own state, and
document it as bearer-equivalent. This is not a new low for the project — the
RomM integration already keeps its token in plaintext in the same SQLite
database. It is the only option that works unattended, and it is honest about
what it protects against.

## Verified against the live service

Read-only, against a real account. `datastorePut` was never called and must
never be: Hearthdeck reads this library, it does not write it.

### Endpoints

| Endpoint | Auth | Verified | Purpose |
| --- | --- | --- | --- |
| `POST https://api.strem.io/api/datastoreGet` | `authKey` | **live** | the library: `{authKey, collection:"libraryItem", ids:[], all:true}` |
| `POST https://api.strem.io/api/datastoreMeta` | `authKey` | **live** | `[id, epoch_ms]` pairs — cheap change detection |
| `POST https://api.strem.io/api/datastorePut` | `authKey` | not called | writing the library — **out of scope, do not use** |
| `POST https://api.strem.io/api/login` | email+password | error path only | password login — **deliberately unused** |
| `POST https://api.strem.io/api/loginWithToken` | token | error path only | alternate auth |
| `POST .../addonCollectionGet`, `GetUser`, `Events`, `Logout`, `Register` | `authKey` | source only | not needed by this feature |
| `GET https://link.stremio.com/api/create` | none | **live** | starts a link session |
| `GET https://link.stremio.com/api/read?code=XXXX` | none | **live** | yields the `authKey` once the user links |

Three corrections to what is commonly repeated:

- The link API is at **`/api/create`, not `/create`**. `GET /create` answers
  `{"success":false,"error":{"code":100,"message":"Unknown method"}}`.
- `api.strem.io` has **no origin restriction in practice**:
  `Access-Control-Allow-Origin: *` on preflight, and requests with no `Origin`
  succeed. (Irrelevant for a native daemon, but it retires a claim.)
- **Continue Watching is not gated on "watched".** See the traps below.

### Envelopes

`api.strem.io` answers **HTTP 200** for both success and failure, and the two
shapes are disjoint:

```jsonc
// success — note there is NO "error" key at all
{"result": [ /* libraryItem, ... */ ]}

// failure — and the HTTP status is still 200
{"error": {"code": 1, "message": "Session does not exist"}}
```

Check `body.get("error")`, not the presence of a key, and never trust the status
code. `link.stremio.com` differs: it sends `success`, `result` and `error`
together.

The link flow, verified end to end:

```jsonc
// GET /api/create
{"success":true, "code":"GYWV",
 "link":"https://link.stremio.com/GYWV",
 "qrcode":"https://link.stremio.com/qr?data=https%3A%2F%2Flink.stremio.com%2FGYWV",
 "result":{ /* same fields */ }, "error":null}

// GET /api/read?code=GYWV — before the user links
{"result":null, "error":{"code":101,"message":"Invalid or expired token"}}

// GET /api/read?code=GYWV — after the user links
{"result":{"authKey":"<44 chars>","success":true}}
```

The `authKey` is 44 opaque characters. No expiry field is returned, no refresh
token, and no user object — a re-link is the only recovery path. **A code is
single-use**: the first attempt consumed its code, so the settings screen must
request a fresh one per attempt rather than caching one.

### What the data actually looks like

One reference library: **62 records** (`movie` 51, `series` 10, `channel` 1),
of which exactly **10** satisfied the Continue Watching predicate — the same 10
Stremio's own home row uses (`CATALOG_PREVIEW_SIZE`).

```jsonc
// one datastoreGet entry, verbatim except for the poster URL
{"_id":"tt26657236","name":"Backrooms","type":"movie",
 "poster":"https://images.metahub.space/poster/small/tt26657236/img",
 "posterShape":"poster","removed":true,"temp":true,
 "_ctime":"...","_mtime":"2026-09-28T18:26:29.322320491Z","year":"...",
 "state":{"lastWatched":"2026-09-09T19:49:06.859Z","timeWatched":0,
          "timeOffset":2271644,"overallTimeWatched":0,"timesWatched":0,
          "flaggedWatched":0,"duration":6628768,"video_id":"tt26657236",
          "watched":"","noNotif":false}}
```

`timeOffset` and `duration` are **milliseconds** — confirmed, not assumed: 54
durations were ≥ 100 000 and **zero** fell in the seconds range.

## The traps

These were all observed in real data. Each one silently produces a wrong rail.

- **Most Continue Watching items are `removed: true, temp: true`.** 8 of the 10
  were. These are "in progress but not saved to my library" entries, and they
  are the normal case, not an edge case. The predicate's `(!removed || temp)`
  half is load-bearing: filtering on `!removed` alone would have shown **2 of
  10** items.
- **`flaggedWatched` does not exclude anything.** Two of the ten were marked
  watched, and one sat at **76 %** — above Stremio's own `WATCHED_THRESHOLD_COEF`
  of 0.7 — and still appeared. Do not add a "hide finished" filter; it would
  make the rail disagree with Stremio.
- **`state.season` / `state.episode` are unreliable.** Three of five series
  returned `0/0` while `video_id` held the real values (`tt2654620:2:10`). Parse
  `video_id` as `tt<id>:<season>:<episode>` and ignore those two fields.
- **`year` is sometimes a string range**, e.g. `"2021–2025"` (en dash), and is
  absent on most records. Type it loosely or drop it.
- **`_mtime` is not when it was watched.** One film's `_mtime` was 19 days after
  its `lastWatched`. `_mtime` is the last record write (often a sync from
  another device). Stremio sorts the row by `_mtime`; `lastWatched` is the more
  truthful "recently watched" and is the better sort for a dashboard rail. Pick
  one deliberately.
- **`_mtime` precision varies** — millisecond timestamps (`.281Z`) and
  nanosecond ones (`.322320491Z`) appear side by side, so parse as RFC 3339
  rather than by slicing the string.
- **`state.watched` can be `""`**, and for films its prefix is the literal
  string `undefined` (`"undefined:1:eJwDAAAAAAE="`) — a client-side artifact.
  Treat it as opaque base64; do not parse the prefix strictly.
- **The library is not only films.** It contained a `yt_id:` YouTube item and a
  `channel`. Filter `type` deliberately.
- **Poster URLs are not uniform** — `/poster/small/` and `/poster/medium/`
  appear for the same shape of item. Rewrite the size segment for card art
  rather than trusting it.

## Schema drift against `stremio-core`

The Rust structs are a *subset* of what the service stores. Reading them as
authoritative would drop usable fields:

- **Stored but not in the structs:** `season`, `episode` (in `state`), plus
  `year`, `logo`, `background` (on the record). The extra art fields were absent
  from every Continue Watching record in the sample, so `poster` is the only
  dependable artwork — treat `logo`/`background` as a bonus when present.
- **In the structs but absent from all 62 records:** `behaviorHints` and
  `lastVidReleased`.

## Decisions

1. **The read path is the `api.strem.io` datastore, not the addon protocol.**
   The addon protocol is officially documented and completely useless here: an
   addon receives `{resource, type, id, extraArgs}` and no user identity at all,
   and there is no library or progress resource. Continue Watching cannot be
   reached as an addon.
2. **All Stremio I/O lives in the daemon**, as a `DiscoveryProvider` with
   `source_id: "stremio"`, registered in `discovery_providers()`.
3. **Authentication is the QR/link-code flow only.** The settings UI offers
   "Link with a code" and "Unlink" and nothing else — no email or password field
   exists. This is what keeps the password out of the system entirely rather
   than merely encrypted.
4. **The credential is the `authKey`, stored at mode `0600` in the daemon state,
   treated as a bearer token.** See "Why the keyring is not the answer".
5. **Change detection is `datastoreMeta`, not a full read.** Its `[id, epoch_ms]`
   pairs are tiny; only call `datastoreGet` when they move. A refresh interval of
   a few minutes is enough — this is a dashboard, not a live view.
6. **The rail is a feature-owned rule collection**, `stremio:continue-watching`,
   using the existing `owned_rule(owner, rule_id)` mechanism. It flows through
   the ordinary Collections → rail pipeline, and `CollectionItem` already carries
   the name and icon a card needs, so no rail code resolves Stremio itself.
7. **Season and episode come from `video_id`**, never from `state.season` /
   `state.episode`.
8. **Launch is a deep link, assembled by the bridge from typed fields** (`type`,
   media id, season, episode) — a validated `LaunchStremio` request, not a URL
   string. The app-level launch of `com.stremio.Stremio.desktop` is the fallback
   until the scheme is confirmed on hardware (Phase 1).
9. **The QR image is produced by the daemon** from a link URL it already holds,
   and served through the same authenticated asset path the frontend already
   renders RomM artwork from. The four-character code and the link are always
   shown as text as well, so the screen is useful even if the image fails.
10. **Unlinking removes both the credential and the catalog rows** that source
    owns, so a stale rail cannot outlive the account it came from.

## Protocol and data model changes

Schema changes are hand-written in `database.rs::migrate()` — there is no
`migrations/` directory — with an accompanying legacy-upgrade test, as the
existing `collections` table upgrade does.

- **Table `stremio_settings`**, single row (`id = 1`), mirroring
  `romm_settings`: `auth_key`, `linked_at`, `updated_at`. No email is stored
  (the link flow never returns one). Confirm the database file itself is `0600`
  — SQLite creates files according to the umask, so this may need an explicit
  `chmod` at creation.
- **`GET /v1/stremio/settings`** → connection status **without the credential**,
  following the `getRommSettings` precedent ("RomM connection status without its
  credential"): `{linked_at, item_count, in_progress_count, last_refresh}`.
- **`POST /v1/stremio/link`** → starts a session: `{code, link, expires_in}`.
- **`GET /v1/stremio/link`** → `{status: "pending" | "linked"}`, never the key.
- **`DELETE /v1/stremio/settings`** → unlink, per decision 10.
- **`GET /v1/stremio/qr`** → the QR image (decision 9).
- **Refresh** reuses `POST /v1/discovery/stremio/refresh`, which already exists.
- **`CatalogRecord` mapping**: `id: "stremio:<imdbid>"` (namespaced, as the
  provider contract requires), `title: name`, `icon: poster` with the size
  segment normalised, and `metadata` carrying `type`, `video_id`, `season`,
  `episode`, `timeOffset`, `duration`, `progress`, `lastWatched`. **Confirm the
  `kind` and `metadata.categories` values the frontend classifier accepts**, so
  these cards land in a video rail rather than being classified as Games or Apps.

## Open questions

1. **Does `stremio://` resume actually work, and in what exact form?** The
   shell's own README documents no URL scheme, so this is undocumented and must
   be established empirically: check that the installed desktop entry declares
   `MimeType=x-scheme-handler/stremio;`, then try `xdg-open` with a
   film and an episode, then confirm Stremio resumes at the right position
   rather than merely opening. This is Phase 1 and it gates Phase 6.
2. **Does Stremio launch cleanly inside this session at all?** It is a
   Qt/CEF-style application; `kiosk-session.md`'s hardware rule applies before
   this is called working.
3. **Progress on a card.** The rail cards have no progress affordance today; the
   rail needs one (a bar, or a "23 min left" line) or the data is wasted.
4. **Rail length.** Stremio's home row is 10 (`CATALOG_PREVIEW_SIZE`) and its
   Library view is 200 (`LIBRARY_RECENT_COUNT`). Take 10 for parity with what a
   Stremio user sees.
5. **What refreshes when.** Only `datastoreMeta` polling on a timer, or also on
   dashboard focus? Decide before wiring the interval, and back off on failures
   so an offline box is not retrying every few seconds.
6. **Is `datastoreMeta` callable per-collection, or does it always return the
   whole store?** It returned all 62 records when asked about `libraryItem`, so
   treat its output as store-wide.
7. **`docs/code-review-roadmap.md` is referenced from three places but does not
   exist.** Unrelated to this feature, but the docs map should stop pointing at
   a missing file.

## Phased roadmap

Each phase is scoped to be doable in one sitting and independently useful.

- **Phase 0 — Doc and seam registration. This change.** Write this doc, register
  it in the docs map, and record the verified shapes so the mapping phases do not
  have to re-derive them.
- **Phase 1 — Hardware verification gate. Blocking, no code.** On the kiosk:
  confirm the `stremio://` scheme is registered, find its exact working form, and
  confirm it resumes at the right position. The output decides whether Phase 6
  ships deep links or falls back to an app-level launch. Do not start Phase 6
  before this answers.
- **Phase 2 — Daemon: credential and link session.** `stremio_settings` in
  `migrate()` with an upgrade test; the link state machine over
  `link.stremio.com/api/create` + `/api/read`; one-shot codes; the `authKey`
  never logged; `0600` confirmed on both the row and the database file.
- **Phase 3 — Daemon: provider and mapping.** `StremioProvider` with
  `source_id: "stremio"`: `datastoreMeta` change detection, `datastoreGet`, the
  Continue Watching predicate, `video_id` parsing, millisecond progress, poster
  normalisation, and a refresh interval. Independence from the other providers
  comes free from the worker set.
- **Phase 4 — API and contract.** The routes above plus their schemas in
  `contracts/openapi.yaml`, keeping the credential out of every response, in the
  style `getRommSettings` already sets.
- **Phase 5 — Frontend: settings and rail.** A Settings section (link with a
  code → QR + code + link, poll status, unlink) and a dashboard rail. The rail
  needs a `DashboardShelf` variant and a card source, since today's shelves are
  built around desktop entries; the existing `CollectionItem` → card conversion
  is the path to reuse.
- **Phase 6 — Launch.** The typed `LaunchStremio` request, the bridge building
  the URI from validated components, and the app-level launch kept as the
  fallback path.
- **Phase 7 — Stretch: keyring-backed `SecretStore`.** Only worth doing if the
  session grows a way to unlock a collection unattended, or the hardware gains a
  TPM. Until then this is the decision above, not a task.

## Re-verifying by hand

The probe that produced the numbers above is throwaway and lives outside the
repo. The parts worth re-running are small enough to inline; none of them need
or print the credential:

```sh
# start a link session (no secret needed)
curl -s https://link.stremio.com/api/create

# after linking, read the library (key from the daemon's own store)
curl -s -X POST https://api.strem.io/api/datastoreGet \
  -H 'Content-Type: application/json' \
  -d "{\"authKey\":\"$KEY\",\"collection\":\"libraryItem\",\"ids\":[],\"all\":true}"

# cheap change detection
curl -s -X POST https://api.strem.io/api/datastoreMeta \
  -H 'Content-Type: application/json' -d "{\"authKey\":\"$KEY\",\"collection\":\"libraryItem\"}"
```

Treat `$KEY` as a bearer token: do not paste it into a shell history you keep,
and do not leave it in a transcript.
