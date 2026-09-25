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
tracked as a session, and stoppable, connecting directly to the Kiosk session
the way every other launch does (see `kiosk-session.md`; the earlier nested
Gamescope-per-launch was removed as it was never shown on hardware).

## Non-negotiable constraints (carried over from existing product rules)

- Every launch goes through the bridge's typed protocol
  (`hearthdeck-protocol`). No new request type gets a free-form command,
  path, or URL field — same discipline as `LaunchApplication` and
  `LaunchHeroicGame` today.
- Every game launch connects **directly to the Kiosk session**, the same
  `DISPLAY`/`WAYLAND_DISPLAY` Hearthdeck itself uses, rather than a nested
  Gamescope instance. This is not this document's rule to set — it is
  `kiosk-session.md`'s, and it applies to RetroArch exactly as it does to
  desktop apps and Heroic. RetroArch is not a background daemon; it is a
  per-session process, started on demand and torn down when the game ends,
  never resident when nothing is playing.
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

- RomM: read-only settings plus live console/game browsing
  (`/v1/retro/consoles`, `/v1/retro/roms`) and dedicated managed launch
  (`/v1/retro/roms/{id}/launch`). Listing asks RomM to group by metadata id,
  so a game's regions/revisions/discs collapse into one tile whose alternatives
  are offered in the context menu. No catalog materialization or saves.
- RetroArch: `retroarch` plus a curated set of `libretro-*` cores ship as
  package dependencies/optional dependencies (`packaging/arch/PKGBUILD`);
  the daemon resolves platform→core and the bridge launches the core under
  Hearthdeck's own config directory. `libretro-shaders-slang` is optional and
  supplies the per-console shader presets the bridge applies (decision 8).
- Discovery/catalog: RomM is *not* a `DiscoveryProvider`/`CatalogRecord`
  source (only Heroic and desktop apps are). It is a separate
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
   no pagination (`CatalogStore::list()`, `api.rs:list_library`) and the
   frontend loads the whole thing into memory and filters it per keystroke.
   RomM libraries routinely run into the thousands of ROMs; treating RomM
   as a `DiscoveryProvider` would mean `replace_source` reinserting
   thousands of rows on a timer and shipping them all in one unpaginated
   response — a direct violation of `product-foundations.md`'s "bound
   payload sizes... lazy-load and page large collections" rule. Session
   tracking (`ApplicationSession`, `ServerEvent`, the bridge protocol) is
   already source-agnostic and doesn't require catalog-table membership, so
   nothing about launching is actually lost by staying out of the catalog.
   RomM keeps its own dedicated, paginated launch route
  (`POST /v1/retro/roms/{id}/launch`), reusing the session/bridge
   machinery only. "Feels unified" UX (search, dashboard) stays a
   client-side concern rather than a server-side catalog merge — no server
   storage unification required. (The removed Flutter client blended
   catalog results with live RomM search results client-side.) Catalog
   pagination is real, worth doing
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
   Rust structs and client plumbing per capability forever, or forcing every
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
   `RommDiagnostic` block that lives outside it. The client-side equivalent
   (a generic live-library interface for the frontend to iterate over)
   is deliberately deferred until a second live source exists
   — building that abstraction for one case would be guessing at its
   shape.

8. **Shaders are per-core presets from `libretro-shaders-slang`, not a
   user-supplied shader directory.** Arch packages the upstream slang shader
   collection; the bridge maps a core filename to one preset under
   `/usr/share/libretro/shaders/shaders_slang` (CRT mask for 2D CRT-era
   consoles, an LCD grid for handhelds, no preset for 3D-era cores). The
   managed config pins `video_driver = "vulkan"` for **every** launch (not
   only shaded ones), with a per-core override table `VIDEO_DRIVER_BY_CORE`
   that pins Flycast to `glcore`.
   Rationale: RetroArch's compiled default is `gl`, which cannot load slang
   presets at all and mis-sizes hardware-rendered cores under the
   Gamescope/KMS sessions Hearthdeck runs in. `vulkan` fits Gamescope (itself
   a Vulkan compositor) and is supported by every core in `retro.rs`'s table.
   Flycast is the exception: its libretro Vulkan renderer segfaults on the
   resolution change an FMV-to-gameplay transition produces
   (flyinghead/flycast#2082, #2442), so it is pinned to `glcore` — the modern
   GL driver, which keeps hardware rendering and a slang-capable context
   without the buggy Vulkan path.
   Default and override are named constants (`RETRO_VIDEO_DRIVER_DEFAULT`,
   `VIDEO_DRIVER_BY_CORE`), so each is one explicit decision, and a missing
   preset still degrades to an unshaded launch instead of failing. Keying
   presets on the core keeps the platform→core decision in one place while
   naturally covering consoles that share a core (Dolphin = GC/Wii, Genesis
   Plus GX = MD/MS/GG/Sega CD). Per-user preset and per-core core-option
   overrides are future work, alongside the core-install configurability in
   open question 2.
   **Not the variable for GameCube:** Dolphin was tried on `glcore` here too,
   on the theory that its Vulkan renderer was the GameCube black screen. It
   black-screened identically on hardware, so the driver is not the cause and
   Dolphin is deliberately left on the default rather than carrying an
   override that was never shown to help. See open question 6.

9. **Guide/Home belongs to Hearthdeck, so RetroArch's menu opens with
   Start + Select.** Every bundled joypad autoconfig profile binds the pad's
   Guide button as RetroArch's single-press menu toggle — the same button the
   quick-menu overlay uses (`docs/arch-package.md`) — so the binding is removed
   in the profiles the bridge seeds (`retro_profiles::without_menu_toggle_binding`)
   and the managed config pins `input_menu_toggle_gamepad_combo` to
   Start + Select for the route into the menu that the pad keeps
   (`RETRO_MENU_TOGGLE_UNBOUND`, `RETRO_MENU_TOGGLE_COMBO` in
   `platform/linux.rs`).
   Rationale: both halves are needed, and only one of them is a config
   change. RetroArch applies autoconfig when a pad is *detected*, which is
   after it has read its config files, so a Guide binding coming from a profile
   would survive any config-side unbind — the profile is what actually keeps
   the button free. The combo has to be set explicitly rather than left alone
   because RetroArch's default is `INPUT_COMBO_NONE` on desktop Linux, so
   taking Guide away without it would leave the controller with no way into
   RetroArch's menu at all. The trade-off is that Hearthdeck now rewrites part
   of a vendored upstream file on the way to disk (the vendored copies stay
   byte-identical and refreshable — see `profiles/udev/README.md`), and that the
   rewrite is applied only to copies recognised as Hearthdeck's own (upstream
   verbatim, or its rewrite), so a mapping re-saved from RetroArch's controller
   wizard is still never clobbered.

10. **The ROM cache is bounded by age, not by size.** A cached rom is a
    copy of something that lives on the RomM server, so evicting it is never
    data loss — the next launch downloads it again. The daemon therefore keeps
    no size ledger at all: it re-stamps a cached rom's timestamp on every
    launch (`touch_cached_rom` in `retro.rs`, deliberately not atime, which
    Linux's `relatime` default often leaves un-updated on a read) and a
    background task (`start_cache_janitor`) sweeps
    `$XDG_CACHE_HOME/hearthdeck/romm/` at startup and every six hours, deleting
    any rom no launch has touched for `HEARTHDECK_ROM_CACHE_MAX_AGE_DAYS`
    (default 7), plus `.part` files old enough to be crash debris rather than
    an in-flight download. The alternative — a byte budget — needs RomM to
    report a size for everything on disk and still has to pick victims; age is
    one file timestamp and one comparison. The trade-off is that a game played
    less often than the window pays a re-download, tunable per deployment via
    the env var. Serving the file straight from RomM instead is not an option
    with the current design: libretro cores open a local path, and the bridge
    only accepts a launch path under this cache directory.

## Starting the RomM server itself

RomM is an external self-hosted server (here a Podman Compose stack), not
something Hearthdeck packages. Its optional Compose lifecycle is managed by
`systemd --user` units, and it has to be up whenever a Hearthdeck session is — a
TV box has no shell to run `podman-compose up` in by hand.

**Decided: this is a systemd unit, not daemon code.** Every other
"start/stop/supervise a host process" concern in this project is a systemd unit
(`hearthdeck.target`, `hearthdeck-bridge.socket`, the session script) — never an
imperative shell-out embedded in the daemon or bridge.
`docs/product-foundations.md`'s own rule is explicit: "the daemon... must not
contain host-specific command construction." Wrapping `podman-compose up -d` in a
daemon startup routine would be exactly that, plus it would reinvent what systemd
already does correctly for free: start/stop ordering and session integration. So
the daemon *reports* RomM's status and can ask systemd to restart it
(`diagnostics::restart_romm_service`, a fixed unit constant); it never starts the
stack itself.

### What triggers what, in order

```
hearthdeck-session
  └─ systemctl --user start hearthdeck.target
       ├─ romm.service                            (oneshot, RemainAfterExit=yes)
       │    Environment=ROMM_COMPOSE_FILE=<packaged default>
       │    EnvironmentFile=-~/.config/hearthdeck/romm.env   (overrides it)
       │    ExecStart=/usr/lib/hearthdeck/hearthdeck-romm up   # checks, then podman-compose up -d
       │    ExecStop =/usr/lib/hearthdeck/hearthdeck-romm down
       │    Wants=romm.path
       ├─ romm.path                               (watches the *default* path)
       │    Unit=romm.service  -> starts the stack when a late file appears
       └─ hearthdeck-daemon.service
            loads the same romm.env, so the RomM library and resource roots
            follow the configured path instead of the packaged default
```

- **`romm.service` wants `romm.path` itself**, not only the target. A host
  running an older package has an older `hearthdeck.target` whose `Wants=` line
  predates it; arming a helper from the unit that is actually wanted is
  `docs/units-and-logs.md` Rule 3.
- **`romm.path` watches only the hardcoded default path.** A `.path` unit cannot
  read an `EnvironmentFile`, so an override in `romm.env` is not watched. That is
  acceptable because a configured path is normally under the home directory,
  which is mounted before the session starts — there is no race to close.

### Where the compose file comes from

`romm.service` and the daemon both load `~/.config/hearthdeck/romm.env` with
`EnvironmentFile=`, so `ROMM_COMPOSE_FILE` in that file is the single place the
deployment path is configured, and both processes agree on it. With nothing set,
the packaged default is `/mnt/external/romM/podman-compose.yaml`. Copy
`/usr/share/doc/hearthdeck/romm.env.example` there to point at a deployment
somewhere else.

### Why there is no `ExecCondition`

The unit used to carry `ExecCondition=`. A non-zero `ExecCondition=` is *not* a
failure: it leaves the unit `inactive`, retries nothing, and — because condition
results are only logged at debug — writes nothing useful to `~/hearthdeck.log`.
"The external drive is two seconds late" and "this host has no RomM" were
indistinguishable, and on a host whose compose file was not where the condition
looked, it failed on every boot, silently, forever. That silence is why no
condition is used at all (`docs/units-and-logs.md` Rule 1).

Instead the driver does its checks and reports what it found on every run: no
compose file (a normal no-op that `romm.path` retries), podman missing,
podman-compose missing, or podman not running (`podman info` is what decides
that last one). A file that is merely late is `up`'s business, and `up` says so
rather than staying quiet.

### Why `up` never waits

`hearthdeck.target` lists `romm.service` in `Wants=`, and **starting a target
waits for its oneshot dependencies to exit**. Any wait inside `up` — for the
mount, for the file — is a wait before `cosmic-comp` launches, with a blank
screen as the symptom. Waiting is unnecessary anyway: `romm.path` fires
`romm.service` the moment the file appears. `TimeoutStartSec=900` covers the one
thing allowed to make the session wait: `podman-compose up -d` actually
converging the stack (a cold first `up` may need to pull images).

### Two details of `romm.service` are deliberate and easy to get wrong

- **It has no restart policy.** `podman-compose up -d` converges a stack by
  *recreating* any container whose config differs from the compose files —
  including containers an earlier `up` already got running. Auto-retrying a
  failed up therefore recreates, and can leave down, a stack that was fine,
  rather than fixing anything. Confirmed the hard way: a `Restart=on-failure`
  here turned one failed start into a loop that tore down a running stack and
  left its containers `Created`. A failure now stays failed and visible.
- **`ROMM_COMPOSE_ARGS` carries extra argv, not a file path.** It holds
  additional `podman-compose` arguments (typically extra `-f` override files).
  The driver passes it *unquoted* — `${ROMM_COMPOSE_ARGS:-}` — so the value is
  split on whitespace and dropped entirely when unset or empty. The no-override
  case therefore passes exactly `-f <base>` and nothing else; a quoted
  `"${VAR}"` would pass one empty argument and break it. Compose overrides are
  how a host whose podman cannot open `/dev/net/tun` (podman's default `pasta`
  networking needs it; `tun` is a module on most kernels) switches the stack to
  `network_mode: host` without forking the base compose file. Example
  `romm.env` line:

  ```sh
  ROMM_COMPOSE_ARGS=-f /home/you/.config/hearthdeck/romm-hostnet.yaml
  ```

**"A place in the UI to do this kind of stuff":** rather than build a new
control surface, `service_statuses()` in `diagnostics.rs` — the same
function that already reports `hearthdeck.target`/daemon/bridge status via
`systemctl --user show` — now also queries `romm.service`. If the unit isn't
installed, it reports the same neutral `unavailable` state the function
already returns for any unit the user's systemd instance doesn't know about,
so this is safe to query unconditionally. Once the environment file is configured,
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
6. **GameCube (Dolphin) black screen on launch. Unresolved.** Every other
   core starts; Dolphin's core loads, the picture stays black and RetroArch's
   own menu stops responding (the core's init blocks the main loop), so the
   session can only be force-quit. Ruled out so far on hardware: the video
   driver (`vulkan` and `glcore` black-screen identically - decision 8), and
   the platform→core mapping (the resolved core is logged at launch; it is
   `dolphin_libretro.so`). Not yet gathered: RetroArch's own output for the
   attempt, which is the next step and is available from the transient unit's
   journal (`journalctl --user -u 'hearthdeck-app-*.service'`) - a libretro
   core's failure prints there. Also worth testing: the same core + rom
   launched outside Hearthdeck (`retroarch -L
   /usr/lib/libretro/dolphin_libretro.so <rom>`), which separates "Hearthdeck's
   managed config and launch" from "this core under this session". Do not
   change the driver or the launch args again before one of those two shows
   the actual failure.

## Phased roadmap

Each phase is scoped to be doable in one sitting and independently useful.

- **Phase 0 — Packaging. DONE.** Added `retroarch` to `depends`, plus
  `optdepends` for a starter set of common-console `libretro-*` cores in
  `packaging/arch/PKGBUILD`.
- **Phase 1 — Protocol + capability plumbing. DONE.** Added
  `BridgeRequest::LaunchRetroGame { core_path, rom_path, session_id }` to
  `hearthdeck-protocol` (no command/URL field, matching the crate's own
  test discipline). Added `retro_launch` to `HostCapabilities`
  (`true` on Linux, matching
  `application_sessions`'s pattern), threaded through `openapi.yaml` and the
  frontend `DaemonClient`. Implemented the bridge side ahead of schedule since the
  match on `BridgeRequest` is exhaustive: `launch_retro_game` in
  `platform/linux.rs` execs `retroarch` directly (decision 1) via
  `launch_with_systemd`, connecting directly to the Kiosk session the same way
  every other launch does (see `kiosk-session.md`), against a
  Hearthdeck-owned config directory (decision 2) and a validated,
  allowlisted core path plus a validated, Hearthdeck-cache-scoped ROM path
  — the daemon-side resolution that produces those two paths is still
  Phase 2.
- **Phase 2 — Daemon: core + ROM resolution. DONE.** New `retro.rs` module:
  static `fs_slug` → libretro core filename table (covering every core added
  in Phase 0, including Dreamcast/`flycast` and N64/`mupen64plus-next`),
  validated against `/usr/lib/libretro`. A ROM's **content extension** can
  override the platform's core, because a platform slug cannot distinguish
  software that shares a folder: a `.32x` cartridge is a 32X rom even when
  RomM files it under the Mega Drive platform, whose slug maps to Genesis
  Plus GX — a core that cannot run 32X — so `.32x` routes to PicoDrive
  whatever platform claims it. The resolved platform, content name and core
  are logged at launch, since "the wrong core started" is otherwise silent.
  ROM paths are read from RomM's own library mount (`fs_path` + `fs_name`)
  rather than cached (decision 10's cache was retired); user-configurable
  core overrides remain future work (open question 2).
- **Phase 3 — Bridge: launch + stop. Code done, hardware verification
  pending.** `launch_retro_game`, path validation, and the direct systemd-run
  launch landed in Phase 1's implementation above. What's left:
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
- **Phase 5 — Flutter client. DONE, later removed.** Added a RomM launch
  call (a "Play" primary action) to every RomM game's detail view in the
  Flutter client's dedicated Retro tab, reusing the launch request/error
  UX it already had for catalog items. That client was removed when the
  frontend was replaced by the Rust/iced app; the Rust frontend browses
  RomM through its Console Games section, so this phase is historical.
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
route for Phase 4. Phase 5 targeted the now-removed Flutter client. Next up
is the `RemoteLibraryAdapter` half of Phase 4 if richer RomM
search/filter/sort is wanted.
