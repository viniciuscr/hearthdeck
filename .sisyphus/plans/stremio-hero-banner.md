# Stremio hero banner — plan

Goal: dashboard hero banner (rotating, 30% window height, full width) over the existing rails, showing the last-added movies+series from Stremio, capped at 10, auto-rotating and focus-aware.

Status: implemented end to end. Validation and the report are the steps left (see the task list).

## Done — daemon
- `discovery/providers/stremio_arrivals.rs`: Cinemeta (`https://v3-cinemeta.strem.io`; `.stremio.com` does not resolve) `GET /catalog/{movie,series}/year/genre={year}.json` for year-1 and year. Pure `newest_arrivals` keeps IMDb ids only, movie|series only, dedupes, sorts by `released` desc (undated last), caps at 10; `arrival_record` maps id `stremio-arrivals:<tt>`, metadata `{source,type,video_id,background,logo,description,year,release_info,released,imdb_rating}`, `updated_at` = released; the provider resolves the Stremio desktop entry per refresh. Source `stremio-arrivals`, 30-minute refresh, registered in `main.rs::discovery_providers`.
- Typed launch: protocol `StremioMediaType` (`#[serde(default)]` = Movie) + `media_type` on `BridgeRequest::LaunchStremioTitle`; bridge `stremio_detail_uri(video_id, media_type)` — bare `tt` → `stremio:///detail/<type>/<tt>`, `tt:s:e` unchanged; daemon `stremio_title_target(item)` reads `metadata.discovery.{video_id,type}` for both Stremio sources.
- Collections: `RULE_NEW_ARRIVALS = "stremio:new-arrivals"` builtin (owner stremio, position 99); `CollectionItem` += `background`/`logo`/`description`; `api::new_arrival_items` resolves it; `contracts/openapi.yaml` updated.

## Done — frontend
- `CollectionItem` carries the art (serde default); `resolve_rail_icons` resolves background and logo through `resolve_icon`, the one place a reference becomes artwork.
- `screens/dashboard/hero.rs`: `Slide` art on `Card`, pure `rotate`/`stepped`/`landed_from_above`/`rotates`/`synopsis`, and the banner view — full window width, `hero_height(WINDOW_HEIGHT)` (30 %), backdrop `ContentFit::Cover`, caption scrim, wordmark or name, one line of synopsis, slide dots, and the shared tile button class for the accent ring.
- The banner is the page's first row (`Shelf::banner`), so `focus` walks its slides with no change to the cursor model: left/right move the slide with the pad, up/down leave the row, Confirm launches the slide (`launch_card_message`). `Message::HeroTick` + a subscription gated on `rotates(hero_len, cursor)` (paused while the pad is on the banner, page-scoped, >1 slide) drive the 8-second rotation from `style::HERO_SLIDE_INTERVAL`. `hero_index` is clamped when collections reload, and the arrivals collection is excluded from the rail list so it is not drawn twice.

## Done — docs and tests
- `docs/stremio-integration.md`: new "New arrivals: the Cinemeta catalog" section (live-verified shapes, the `.stremio.com` miss, recency-weighted ordering, `skip=` 504s), four new traps, decisions 11–13, Phase 8, the `CollectionItem`/`media_type` contract changes, and open question 8 (the bare-id series URI still needs the hardware check).
- Tests: 8 provider unit tests + 2 API seam tests (banner art, series-typed launch) in the daemon; the bridge's typed URI test; hero unit tests and a shell test (banner is row 0, pauses under the pad, wraps on rotation) in the frontend; client art tests.

## Left
- plan_step_08: `just check-fast` for daemon/bridge/protocol (frontend already green), then `just check` for the multi-crate diff, then re-read `git diff`.
- plan_step_09: the report block (`reporting` skill): Done / Watch out / Checked / Next.
