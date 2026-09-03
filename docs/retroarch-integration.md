# RetroArch Integration

This is the working design doc and roadmap for turning RetroArch into Hearthdeck's
console-emulation launch backend, with RomM staying the ROMs/metadata provider. It
is a living document: decisions here are the current best answer, not permanent
law. Update it as we learn more, the same way `docs/backend-architecture.md` and
`docs/kiosk-session.md` get amended when reality disagrees with the plan.

Status: design phase. Nothing described as "Decided" below is implemented yet
unless a phase is marked done.

## Goal

RomM stays the console/game catalog and metadata source (already live, read-only,
in **Settings > System > Retro & RomM**). RetroArch becomes the thing that
actually runs a game when the user presses "Play" on a RomM item, launched and
supervised the same disciplined way Hearthdeck already launches desktop apps:
typed request in, no shell string ever crosses the daemon/bridge boundary,
tracked as a session, stoppable, and (per the standing kiosk rule) wrapped in
Gamescope with the smallest possible memory/CPU footprint.

## Non-negotiable constraints (carried over from existing product rules)

- Every launch goes through the bridge's typed protocol
  (`hearthdeck-protocol`). No new request type gets a free-form command,
  path, or URL field — same discipline as `LaunchApplication` and
  `LaunchHeroicGame` today.
- Gamescope wraps every game launch in the Kiosk session, exactly like
  `launch_with_systemd(..., wrap_in_gamescope: true)` does for desktop apps
  today (`hearthdeck-bridge/src/platform/linux.rs:121`). RetroArch is not a
  background daemon; it is a per-session process, started on demand and torn
  down when the game ends, never resident when nothing is playing.
- RetroArch must be **directly supervised by the transient systemd unit**,
  not handed off via a URI/IPC scheme to some other already-running process.
  This is the one thing to explicitly *not* copy from Heroic's launch path:
  `services/README.md` documents that Heroic's `heroic://launch` URI hands
  off to whichever Heroic process is already running rather than exec'ing
  the game directly, which is why Heroic itself (not each game) has to be
  tracked as one stable, reused unit instead of a fresh one per launch.
  RetroArch has no such indirection problem if we exec it directly as the
  unit's main process — so, unlike Heroic, a RetroArch launch is fully torn
  down (and its unit garbage-collected) the moment the game exits, with no
  extra bookkeeping needed to reuse or explicitly close anything afterward.

## Current state (baseline, as of this doc)

- RomM: read-only settings + live console/game browsing only
  (`lib/retro.dart`, `diagnostics.rs`, `/v1/retro/consoles`,
  `/v1/retro/games`). No launch, no catalog integration, no saves.
- RetroArch: not installed by the package, not referenced anywhere in
  `services/` or `packaging/`.
- Discovery/catalog: RomM is *not* a `DiscoveryProvider`/`CatalogRecord`
  source (only Heroic and desktop/macOS apps are). It is a separate
  direct-proxy surface.
- Install requests: the host advertises `install_requests: false`, and
  `POST /v1/install-requests` returns `501` until a privileged approval path
  exists. No pacman/Flatpak call exists in the daemon or bridge.

## Research findings that shape the decisions below

- **RetroAchievements config is set up once, not per launch.** It's a
  username/token pair under Settings > Achievements in `retroarch.cfg`,
  checked at login and cached; it is not something that needs recreating
  per game. ([libretro docs](https://docs.libretro.com/guides/retroachievements/))
  This directly answers your Knulli/Lakka question: there is no reason to
  regenerate RetroArch's config from scratch per game. If those distros
  appear to do that, it's most likely because their frontend always invokes
  RetroArch fresh per game (RetroArch is not long-running — it exits after
  the game closes, same as we'd do), while the *config directory* persists
  across those invocations. Process-per-launch and config-per-launch are
  different things; we only need the former.
- **Distro-packaged RetroArch normally disables the in-app Core Downloader**
  and expects cores from the system package manager instead
  ([libretro docs](https://docs.libretro.com/guides/download-cores/)).
  Arch ships cores as individual `libretro-*` packages. This lines up
  cleanly with pacman being the install mechanism, not a custom
  buildbot-fetching downloader.
- **RomM has an official, documented save-file sync protocol** (server-side
  "sync orchestrator": client hashes local saves + filenames, sends the
  list, server returns an upload/download/conflict/noop plan per file),
  used by RomM's own official client, Grout
  ([grout save-sync guide](https://raw.githubusercontent.com/rommapp/grout/main/docs/usage/save-sync.md)).
  Match is by platform + filename (PSP by game ID). This is a solved
  problem we should reuse as a client, not reinvent.
- **Correction: RomM's server absolutely stores and serves save states —
  it is not saves-only.** Checked the actual source
  (`backend/endpoints/states.py` in `rommapp/romm`, verified against
  `saves.py`): there is a full `/api/states` resource with upload, list,
  get, download, update, visibility toggle, and delete, structurally
  parallel to `/api/saves` and tied to `rom_id` + `emulator`. What's true is
  narrower than my first read of Grout's docs suggested: `saves.py` has an
  extra bookkeeping layer — `DeviceSaveSync`, `db_sync_session_handler`,
  `device_id`/`session_id` params — that implements the automatic
  multi-device hash-compare/conflict-plan orchestration Grout's Save Sync
  feature uses. `states.py` has none of that layer yet: no device tracking,
  no sync sessions, no conflict plan, just plain CRUD. So: RomM can be the
  storage/backup/portability home for states today via plain upload/download
  calls; it just doesn't (yet) do RomM's fancy automatic per-device
  conflict resolution for them the way it does for saves. For Hearthdeck,
  which is one install talking to one RomM server, that automatic
  orchestration isn't a hard requirement anyway — a simple
  "upload the state(s) when a session ends, fetch the latest before
  launch" flow against the plain endpoints gets state backup without
  needing it.
- **RetroArch also has its own built-in Cloud Sync** (WebDAV, or iCloud on
  Apple platforms) that covers save states as well as save files and
  configs, using a three-way-merge manifest system
  ([libretro docs](https://docs.libretro.com/guides/retroarch-cloud-sync/)).
  This is a second, independent sync mechanism, orthogonal to RomM's, and
  is now a lower priority given RomM can already do the job end to end.

## Decisions

These are settled for now; each has a one-line rationale. Revisit if reality
disagrees.

1. **Launch = direct process supervision.** The bridge execs
   `retroarch` itself as the `systemd-run --user` unit's main process
   (reusing `launch_with_systemd`), the same as desktop apps — not a
   URI/IPC hand-off. Rationale: avoids the exact Kiosk-mode tracking problem
   Heroic already hit.
2. **One persistent, Hearthdeck-owned RetroArch config directory**, not the
   user's default `~/.config/retroarch/`. Launch with an explicit config
   path (`retroarch -c <path>/retroarch.cfg` or `$RETROARCH_CONFIG_DIRECTORY`).
   Rationale: guarantees RetroAchievements login, input config, and video
   settings survive across every launch without fighting a separately
   configured desktop RetroArch on dual-purpose machines; mirrors the
   existing `~/.config/hearthdeck/daemon.env` "we own our own config"
   pattern already used for LAN/TLS settings.
3. **Core installation goes through the OS package manager (pacman),
   not a custom libretro-buildbot downloader.** Rationale: matches how
   distro-packaged RetroArch expects cores to arrive, keeps cores
   security-patched by the distro, and reuses the already-designed (if
   currently stub) typed install-request boundary instead of inventing a
   new privileged download-and-place-`.so`-files mechanism.
4. **Save files sync through RomM's own sync-orchestrator API**, implemented
   as a client the same way Grout is one. Rationale: it's an existing,
   documented, hash-based, conflict-aware protocol; reimplementing it worse
   inside Hearthdeck has no upside, and it keeps saves living alongside the
   ROMs library that already is the source of truth.
5. **Save states live in RomM too, via plain upload/download against
   `/api/states`**, not RetroArch's own WebDAV Cloud Sync. Rationale: RomM
   already stores states as a first-class asset type; standing up or
   depending on a separate WebDAV backend just to get state portability
   would duplicate storage Hearthdeck already has access to. We don't get
   RomM's automatic multi-device conflict resolution for states (that layer
   only exists for saves today), but for a single Hearthdeck install talking
   to one RomM server that isn't a requirement — upload on session end,
   fetch latest before launch is enough. Revisit only if RomM later adds
   the same device-sync orchestration to states, or if multi-device
   conflict resolution turns out to matter in practice.
6. **RomM does not join the catalog/`DiscoveryProvider` registry. Launch
   and catalog-membership are separate decisions.** `GET /v1/library` has
   no pagination (`CatalogStore::list()`, `api.rs:list_library`) and Flutter
   loads the whole thing into memory per search keystroke (`search.dart`).
   RomM libraries routinely run into the thousands of ROMs; treating RomM
   as a `DiscoveryProvider` would mean `replace_source` reinserting
   thousands of rows on a timer and shipping them all in one unpaginated
   response — a direct violation of `product-foundations.md`'s "bound
   payload sizes... lazy-load and page large collections" rule. Session
   tracking (`ApplicationSession`, `ServerEvent`, the bridge protocol) is
   already source-agnostic and doesn't require catalog-table membership, so
   nothing about launching is actually lost by staying out of the catalog.
   RomM keeps its own dedicated, paginated launch route
   (`POST /v1/retro/games/{id}/launch`), reusing the session/bridge
   machinery only. "Feels unified" UX (search, dashboard) stays a
   client-side merge, the same pattern `search.dart` already uses to blend
   catalog results with live RomM search results — no server storage
   unification required. Catalog pagination is real, worth doing
   eventually for any future large-library source (Steam will hit the same
   wall), but it is an independent project, not a blocker here.
7. **Live pass-through sources (RomM today, others later) get a formal
   `RemoteLibraryAdapter` trait, parallel to `DiscoveryProvider`.**
   Confirmed RomM's real `/api/roms` capability surface is much richer than
   what Hearthdeck forwards today (only `limit`/`offset`/`platform_ids`/
   `search_term`, hardcoded `order_by=name`): it also supports arbitrary
   `order_by`/`order_dir`, `updated_after`, and many boolean/multi-value
   filters (`favorite`, `playable`, `missing`, `has_ra`, `has_saves`,
   `has_states`, `genres`, `regions`, `tags`, each with a per-field
   `any`/`all`/`none` logic operator). Rather than hand-writing bespoke
   Rust structs and Dart plumbing per capability forever, or forcing every
   future live source through one flattened generic query language and
   losing fidelity, each adapter declares its own capabilities (supported
   sort fields, filter fields and their kind) and implements one shared
   trait; only the page envelope (`items`/`total`/`limit`/`offset`) and
   health reporting are standardized, not the filter vocabulary itself.
   RomM's own dedicated route stays RomM-shaped; `RommAdapter` is simply
   the one place that knows how to translate a generic
   `filters: BTreeMap<String, Vec<String>>` into RomM's real query params.
   Health for this adapter joins the same `/v1/health` array discovery
   providers use (tagged `kind: "live_proxy"`), replacing today's bespoke
   `RommDiagnostic` block that lives outside it. The Dart-side equivalent
   (a `LiveLibrarySource` interface for `search.dart` to iterate over
   generically) is deliberately deferred until a second live source exists
   — building that abstraction for one case would be guessing at its
   shape.

## Starting the RomM server itself

RomM is an external self-hosted server (commonly a podman/docker-compose
stack), not something Hearthdeck packages or manages the lifecycle of.
Whoever runs it needs it started before it's useful — at session start or at
boot, not by hand after every login.

**Decided: this is a systemd unit, not daemon code.** Every other
"start/stop/supervise a host process" concern in this project is a systemd
unit (`hearthdeck.target`, `hearthdeck-bridge.socket`, the Kiosk session
scripts) — never an imperative shell-out embedded in the daemon or bridge.
`docs/product-foundations.md`'s own rule is explicit: "the daemon... must not
contain host-specific command construction." Wrapping `podman-compose up -d`
in a daemon startup routine would be exactly that, plus it would reinvent
what systemd already does correctly for free: restart-on-failure, proper
start/stop ordering, and boot/session integration.

`deploy/systemd/romm.service.example` is a `systemd --user` oneshot unit
(`RemainAfterExit=yes`) wrapping `podman-compose up -d`/`down` in the user's
own compose project directory, `WantedBy=hearthdeck.target` — so it starts
at the same point `hearthdeck.target` already does ("starts for users at
their next login", per the README), not tied to system boot. Rootless
podman wants the user's session (runtime dir, D-Bus) anyway, so tying it to
the user-session target instead of `multi-user.target` avoids fighting
rootless podman's own expectations. It is a `.example` template, not
auto-installed by the Arch package: the compose project's location
(`/mnt/external/romM/` or wherever) is specific to each install, the same
reason `daemon.env.example` is shipped as documentation rather than a live
config.

**"A place in the UI to do this kind of stuff":** rather than build a new
control surface, `service_statuses()` in `diagnostics.rs` — the same
function that already reports `hearthdeck.target`/daemon/bridge status via
`systemctl --user show` — now also queries `romm.service`. If the unit isn't
installed, it reports the same neutral `unavailable` state the function
already returns for any unit the user's systemd instance doesn't know about,
so this is safe to query unconditionally. Once the template is installed,
RomM's container-stack status shows up in Settings' existing service status
view next to the daemon and bridge — no new screen, no separate place to go
looking for whether it's up.

**Done: a restart control from within Hearthdeck's own UI.**
`POST /v1/retro/service/restart` runs `systemctl --user restart
romm.service` — the unit name is a fixed constant in
`diagnostics::restart_romm_service`, never a request parameter, so this is
one narrowly scoped action, not a generic "restart any unit" capability
(same discipline as the existing install-request boundary). Threaded
through `CatalogRepository.restartRommService()` and surfaced as a
**Restart** button directly on the RomM card in Settings' existing service
status view, next to the daemon/bridge cards — reusing
`_refresh()`/snackbar UX already established for the library-rescan and
provider-refresh controls on the same screen. Only shown/wired for the
`romm_container` card; every other service card is unaffected.

## Open questions (need a decision before/at the relevant phase)

1. **Install-request privilege model.** Supporting install requests that
  actually install a `libretro-*` package needs a real privileged path — most
   likely a polkit rule scoped to an allowlisted pacman transaction pattern,
   invoked from a small helper, never the daemon calling `pacman` directly
   as the user. This is real, separate security work and deserves its own
   design pass, not a quick shortcut.
2. **Which cores ship by default vs. install on demand.** Fully
   on-demand (install a core only the first time a platform's first ROM is
   seen) is the ideal you described, and is achievable *if* (1) above is
   solved — the daemon already sees RomM platform data, so "first ROM seen
   for platform X, no matching core file on disk" is a detectable, one-time
   event, not something that needs to happen every launch. Until (1) is
   solved, fall back to bundling a small curated set of common-platform
   cores via `depends`/`optdepends` in `PKGBUILD`, with the rest manual.
3. **State storage location for Hearthdeck's own RetroArch config.**
   RomM's `/api/states` endpoints exist and work; the open part is purely
   Hearthdeck-side: where RetroArch writes states locally (per decision 2,
   inside the Hearthdeck-owned config/save directory, likely sorted by
   core and content directory the same way saves are, per RetroArch's own
   directory-organization settings), and when Hearthdeck uploads/downloads
   them relative to a play session (after `StopApplicationSession`, before
   `LaunchRetroGame`). No RomM-side blocker remains; this is scheduling and
   local path bookkeeping, not a protocol design question.
4. **Graceful stop → save flush.** Stopping a game today is
   `systemctl --user stop <unit>` (SIGTERM). Need to confirm RetroArch
   flushes `.srm` saves on SIGTERM alone, or whether we need its network
   command interface (`QUIT` command) for a clean shutdown before the save
   sync step runs. Verify empirically in Phase 3.
5. **Catalog pagination (independent project, not a blocker).** `GET
   /v1/library` and `CatalogStore::list()` are unpaginated. Worth fixing
   before any future large-library `DiscoveryProvider` (Steam, GOG) gets
   added, but out of scope for RetroArch/RomM launch work per decision 6.

## Phased roadmap

Each phase is scoped to be doable in one sitting and independently useful.

- **Phase 0 — Packaging. DONE.** Added `retroarch` to `depends`, plus
  `optdepends` for a starter set of common-console `libretro-*` cores in
  `packaging/arch/PKGBUILD`.
- **Phase 1 — Protocol + capability plumbing. DONE.** Added
  `BridgeRequest::LaunchRetroGame { core_path, rom_path, session_id }` to
  `hearthdeck-protocol` (no command/URL field, matching the crate's own
  test discipline). Added `retro_launch` to `HostCapabilities`
  (`true` on Linux, `false` on macOS/other, matching
  `application_sessions`'s pattern), threaded through `openapi.yaml` and the
  Dart client. Implemented the bridge side ahead of schedule since the
  match on `BridgeRequest` is exhaustive: `launch_retro_game` in
  `platform/linux.rs` execs `retroarch` directly (decision 1) via
  `launch_with_systemd(..., wrap_in_gamescope: true)`, against a
  Hearthdeck-owned config directory (decision 2) and a validated,
  allowlisted core path plus a validated, Hearthdeck-cache-scoped ROM path
  — the daemon-side resolution that produces those two paths is still
  Phase 2.
- **Phase 2 — Daemon: core + ROM resolution. DONE.** New `retro.rs` module:
  static `fs_slug` → libretro core filename table (covering every core added
  in Phase 0, including Dreamcast/`flycast` and N64/`mupen64plus-next`),
  validated against `/usr/lib/libretro`; ROM fetch/cache from RomM's
  `/api/roms/{id}` and `/api/roms/{id}/content/{fs_name}` (added to
  `diagnostics.rs`, same authenticated-proxy pattern as `romm_asset` —
  credentials never leave the daemon) into `$XDG_CACHE_HOME/hearthdeck/romm/`,
  the same path the bridge's own allowlist expects. User-configurable core
  overrides remain future work (open question 2).
- **Phase 3 — Bridge: launch + stop. Code done, hardware verification
  pending.** `launch_retro_game`, path validation, and the systemd-run/
  gamescope wrapping landed in Phase 1's implementation above. What's left:
  confirm on real Linux hardware that SIGTERM-based stop (`stop_application`,
  unchanged, reused as-is) actually flushes `.srm` saves (open question 4);
  switch to RetroArch's network command interface for a clean quit first if
  it doesn't.
- **Phase 4 — Daemon: dedicated retro-launch route. Route DONE,
  `RemoteLibraryAdapter` not started.** `POST /v1/retro/roms/{id}/launch`
  landed: resolves the launch plan via `retro::prepare_launch`, issues
  `BridgeRequest::LaunchRetroGame`, and reuses `ApplicationSession`/
  `ServerEvent` tracking exactly like `launch_app` does. No catalog/
  `DiscoveryProvider` work, per decision 6. Still open: the
  `RemoteLibraryAdapter` trait (decision 7) and `RommAdapter`, which would
  replace `list_retro_roms`'s current hardcoded 4-param passthrough with
  full forwarding of RomM's real filter/sort capabilities, and move RomM
  health into the shared `/v1/health` array.
- **Phase 5 — Flutter. DONE (dedicated Retro tab only).** Added
  `HearthdeckApiClient.launchRetroRom`, a "Play" primary action
  (`ContentAction(id: 'launch', ...)`) on every RomM game's
  `ContentDetails`, and wired `ContentDetailsPage.onPrimaryAction` in
  `retro.dart` to call it, with the same request/error snackbar UX
  `full_library.dart` already uses for catalog launches. Not wired in
  `search.dart`'s merged results yet — that screen has no
  `onPrimaryAction` for any source today, catalog or RomM, so this isn't a
  RomM-specific gap. This is also the intended manual test surface: open
  the app, go to **Retro**, pick a console and a game, press **Play**.
- **Phase 6 — Save-file sync.** RomM sync-orchestrator client (hash local
  saves in the Hearthdeck-owned RetroArch save directory, POST to RomM,
  execute the returned plan).
- **Phase 7 — Save-state backup.** Simpler than Phase 6 on purpose: no
  conflict orchestration available server-side yet, so just upload the
  state(s) for a ROM to RomM's `/api/states` when a play session ends, and
  fetch the latest one before a launch if the local copy is missing or
  older. Revisit if RomM adds device-sync orchestration for states later.
- **Phase 8 — On-demand core install (stretch).** Only after open question 2
  has a real answer: detect first-ROM-for-new-platform, trigger the
  install-request flow end-to-end (approval UI + privileged helper), replace
  the Phase 0 curated core list with true on-demand installation.

## Next concrete step

Phases 0, 1, and 2 are done, along with the code for Phase 3 and the launch
route for Phase 4. Next up is Phase 5 (Flutter "Play" action wired to
`POST /v1/retro/roms/{id}/launch`), or the `RemoteLibraryAdapter`
half of Phase 4 if richer RomM search/filter/sort is wanted first — both are
independent of each other.
