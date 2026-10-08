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
- **A launch names the bridge's own source namespace, not the record's provider.**
  `LaunchApplication` means "start a desktop entry", and the bridge resolves it
  inside `desktop-apps` and refuses every other source id — forwarding the record's
  own `source_id` is refused with "unsupported application source", which reached the
  user as a bare 502 on every Continue Watching card. For the same reason the Stremio
  desktop entry is resolved from the host's application list by the URL scheme it
  declares, never hardcoded: the id differs between a distribution package and a
  Flatpak, and a guess that names nothing fails identically.
- **Do not depend on the desktop entry having registered the URI scheme.** The
  `stremio://` URIs work because Stremio parses its own argv, not because
  `x-scheme-handler/stremio` is registered — the entry does not appear to declare it, and
  that is why `xdg-open` never had a method for one either. Matching the entry *by*
  scheme is therefore a nicety that needs a name fallback, which is where the two
  implementations of that lookup diverged: the provider fell back to the name and found
  the app, the bridge did not and refused every launch. The bridge no longer looks the
  entry up at all — the request carries the id discovery already produced, and the
  bridge validates it exactly as it validates any application id.

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
6. **The rail is a feature-owned rule collection**, `stremio:continue-watching`, using
   the same mechanism as the model's own rails. **Implemented.** The row is seeded in
   `database.rs::migrate()` rather than published on link — an empty rule costs
   nothing, and a seeded row saves the link path a lifecycle to maintain — and
   `list_collections` resolves it from the records the provider published, newest
   change first. Its items carry the `hearthdeck:`-prefixed ids the model's rails use,
   and that prefix is load-bearing rather than cosmetic: the client strips it and hands
   the remainder to `/v1/apps/{id}/launch`. Sending the bare catalog id instead leaves
   the client's own launch gate refusing it, so the card would draw and then do
   nothing when pressed. (That is the first thing that broke in testing, and the
   reason `activate_dashboard_app` now also looks in the rail's own entry lists.)
7. **Season and episode come from `video_id`**, never from `state.season` /
   `state.episode`.
8. **Launch is a deep link, assembled by the bridge from a validated id** —
   `LaunchStremioTitle { video_id }`, not a URL string: the bridge checks the id and
   builds the `stremio://` URI itself. **Implemented, and verified on real hardware.**
   The shape is `stremio:///detail/movie/<tt-id>` for a film and
   `stremio:///detail/series/<tt-id>/<tt-id>:<season>:<episode>` for an episode; both
   open the title's own page rather than Stremio's home screen. The id arrives as
   `argv[1]`, which Stremio's shell forwards to its web UI as an `open-media` event —
   queued until the UI is ready, so a cold start works exactly like one where Stremio
   is already running. It opens the title's *page*, not the player: playing needs a
   stream chosen, which is the user's step in Stremio's own UI, and this is the
   deliberate stopping point. A record with no video id launches the plain application
   instead, so a rail keeps working for anything published before the ids existed.
9. **The QR image is produced by the daemon from a link URL it already holds**, and
   served through the same authenticated asset path the frontend already renders
   RomM artwork from. The four-character code and the link are always shown as text
   as well, so the screen is useful even if the image fails.
   **Not implemented yet.** The daemon can either generate the image (a `qrcode`
   dependency) or proxy the PNG Stremio already returns for the link, and the second
   needs no new crate. The screen currently ships the code and the link only, which
   is the half that makes the flow work without typing.
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
- **`CatalogRecord` mapping** (implemented): `id: "stremio:<imdbid>"` (namespaced,
  as the provider contract requires), `title: name`, `icon: poster` with the size
  segment normalised, `launch_id` set to `com.stremio.Stremio.desktop` as the
  decision-8 fallback, and `metadata` carrying `type`, `video_id`, `season`,
  `episode`, `progress`, `time_offset_ms`, `duration_ms`, `minutes_left`,
  `last_watched`, `times_watched` and `year`.

  **Answered, and the answer is not a mapping choice.** No `kind` or category
  value can put these cards on a video rail, because no such value exists:
  `Section` has only PC Games, Console Games and Applications, and
  `app_group.rs::is_watch_entry` identifies streaming *services* by id and exec
  line — deliberately not by category, since `AudioVideo`/`Video` also match
  volume mixers and capture tools. So a Stremio film lands in Applications
  whatever it carries, which is why the rail is its own shelf rather than a value to
  pick here (Phase 5, built).

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
- **Phase 1 — Hardware verification gate. ANSWERED.** The scheme is registered and the
  working form is known: `stremio:///detail/movie/<tt-id>` and
  `stremio:///detail/series/<tt-id>/<tt-id>:<s>:<e>` each open the title's page in the
  installed app, whether Stremio was closed or already running. It does not resume a
  position and cannot: the URI opens a page, and starting playback needs a stream
  picked first. So Phase 6 shipped the details-page launch and the position half is
  dropped rather than waiting on a check nobody needs any more.
- **Phase 2 — Daemon: credential and link session. DONE.** `stremio_settings` with
  an upgrade test; the link flow over `link.stremio.com/api/create` + `/api/read`;
  the session key never serializable, never logged, and never quoted in an error;
  and `Database::connect` now narrows the database file to `0600`. The endpoints
  are `POST`/`GET /v1/stremio/link` and `GET`/`DELETE /v1/stremio/settings`.
- **Phase 3 — Daemon: provider and mapping. DONE.** `StremioProvider` with
  `source_id: "stremio"`, a five-minute refresh, `datastoreMeta` change detection
  (the digest is the metadata's own serialization), the Continue Watching
  predicate, `video_id`-derived season and episode, millisecond progress, poster
  normalisation, and the app-level launch as the decision-8 fallback. An account
  that is not linked publishes an empty snapshot rather than failing, so an unlink
  cannot leave a degraded provider behind. `CatalogRecord` gained `Clone` for the
  snapshot cache.
- **Phase 4 — API and contract.** The routes above plus their schemas in
  `contracts/openapi.yaml`, keeping the credential out of every response, in the
  style `getRommSettings` already sets.
- **Phase 5 — Frontend: settings and rail. Settings half DONE, rail not started.**
  The Settings screen has a Stremio section: it links with a code, draws that code
  and the link with a countdown, polls the daemon every two seconds until the link
  resolves, asks the source to refresh so the catalog fills immediately, and unlinks.
  Because both the code and the link are on screen, authenticating needs no typing on
  a television — the link opens on any device already signed in. The controls go
  through the same `actions`/`pressable` list the categorization half uses, so the
  controller reaches them, and the section sits *below* categorization so the screen
  a user already knows, and its pad behaviour, are unchanged.

  The dashboard **rail** is in as well: a Continue Watching shelf, drawn from the
  daemon's `stremio:continue-watching` collection and placed above the streaming apps.
  It deliberately has no fallback, unlike the Watch shelf — an empty rail means no
  account is linked or nothing on it is unfinished, and a shelf guessed from names
  would be a claim about somebody else's watch history. A link re-reads the collection
  once the daemon's refresh has had time to publish, because that refresh is a network
  round trip on the daemon's side and reading it straight away would find the row still
  empty.

  Two pieces remain, both small. The **QR image**: decision 9 called for the daemon to
  produce it, which means either a `qrcode` dependency in the daemon plus a route, or
  proxying the PNG Stremio already returns through a daemon route — the second adds no
  crate. Until one of them lands, the screen shows the code and the link, which is what
  makes the flow work without typing at all. And the **progress affordance**: the cards
  are name and artwork only, because `CollectionItem` carries no progress and neither
  does the shelf's card type. "23 min left" therefore needs the value to reach the card
  — either by extending `CollectionItem` (the daemon already has it in the catalog
  record's metadata) or by correlating the card with the catalog item on the client.
- **Phase 6 — Launch. DONE, to the details page.** `LaunchStremioTitle { video_id }`
  added to the protocol; the bridge validates that id, finds the Stremio entry by the
  scheme it declares rather than by a guessed file name, builds the URI and appends it
  to that entry's own command. Not through `xdg-open`: on this session that finds no
  desktop environment to delegate to and gives up, which is the same reason Heroic is
  exec'd directly. The plain application launch stays as the fallback for a record with
  no video id. Auto-play is deliberately out of scope — it needs a stream chosen, which
  belongs to Stremio's own UI and the person watching.
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
