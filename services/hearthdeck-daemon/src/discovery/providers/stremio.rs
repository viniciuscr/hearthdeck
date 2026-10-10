//! The Stremio discovery provider: an account's unfinished titles, read from the
//! Stremio account datastore and published as catalog records.
//!
//! Two calls make up a refresh. `datastoreMeta` returns one `[id, epoch_ms]` pair
//! per library item and costs nothing to read, so it decides whether the library
//! itself has to be fetched at all; `datastoreGet` returns the records. The digest
//! is the metadata's own serialization — exact, and a few kilobytes, so there is
//! nothing to gain by hashing it.
//!
//! The shapes below are not from Stremio's documentation, which does not exist for
//! this API: they are what a real account returned. `docs/stremio-integration.md`
//! records the full findings, and the traps the mapping has to absorb.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hearthdeck_protocol::{BridgeRequest, BridgeResponse, DiscoveredApplication};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::{
    bridge,
    catalog::CatalogRecord,
    discovery::DiscoveryProvider,
    stremio::{SOURCE_ID, StremioRepository},
};

const API_URL: &str = "https://api.strem.io/api";

/// Stremio's home row is ten items (`CATALOG_PREVIEW_SIZE` in its own client), so
/// this is the same ten a Stremio user sees rather than an invented length.
const CONTINUE_WATCHING_LIMIT: usize = 10;

/// The URL scheme Stremio's own desktop entry declares.
///
/// It is how the entry is found: `x-scheme-handler/stremio` is what the installed
/// entry claims to own `stremio://`, so the scheme identifies the application far
/// more reliably than its file name, which differs between a distribution package
/// and a Flatpak. Decision 8 of `docs/stremio-integration.md` wants a `stremio://`
/// deep link once that format is confirmed on hardware; until then a card opens the
/// app itself, and this is the entry it opens.
const STREMIO_SCHEME: &str = "stremio";

pub struct StremioProvider {
    settings: StremioRepository,
    /// Where the host answers questions about installed applications. A card
    /// launches a desktop entry, and only the bridge knows which entry that is.
    bridge_socket_path: PathBuf,
    /// The snapshot this provider last published, with the datastore metadata it
    /// came from. In memory only: after a restart the first refresh reads the
    /// library once, which is a fair price for not persisting a second copy of it.
    cached: Mutex<Option<CachedLibrary>>,
}

struct CachedLibrary {
    /// The `datastoreMeta` result this snapshot was built from.
    digest: String,
    records: Vec<CatalogRecord>,
}

impl StremioProvider {
    pub fn new(settings: StremioRepository, bridge_socket_path: PathBuf) -> Self {
        Self {
            settings,
            bridge_socket_path,
            cached: Mutex::new(None),
        }
    }
}

#[async_trait]
impl DiscoveryProvider for StremioProvider {
    fn source_id(&self) -> &'static str {
        SOURCE_ID
    }

    fn refresh_interval(&self) -> Option<Duration> {
        // Watching something is what moves this list, and `datastoreMeta` makes the
        // check one small request. Five minutes keeps the rail current without
        // turning a dashboard into a poller.
        Some(Duration::from_secs(5 * 60))
    }

    async fn discover(&self) -> Result<Vec<CatalogRecord>> {
        let Some(credentials) = self.settings.credentials().await? else {
            // Not linked. An empty snapshot is the honest answer and not an error:
            // it also clears anything a previous link left behind. Logged because
            // "the rail is empty" and "no account is linked" look identical from
            // the dashboard.
            debug!("the stremio account is not linked; publishing nothing");
            self.cached.lock().await.take();
            return Ok(Vec::new());
        };

        let meta = datastore_meta(&credentials.auth_key).await?;
        let digest = digest_of(&meta);
        if let Some(cached) = self.cached.lock().await.as_ref()
            && cached.digest == digest
        {
            // Nothing has moved, so the snapshot already published is still the
            // right one and the library is not re-read.
            return Ok(cached.records.clone());
        }

        let library = datastore_get(&credentials.auth_key).await?;
        let mut records = continue_watching(&library)?;
        // The launch target is resolved here, once per refresh, rather than in the
        // mapping: which desktop entry opens a title is a fact about this host, not
        // about the account, and keeping it out of the mapping leaves that a pure
        // function of the account's own data.
        let launch_id = desktop_entry_for_stremio(&self.bridge_socket_path).await;
        if launch_id.is_none() {
            warn!(
                "no Stremio desktop entry was discovered, so the rail will draw without \
                 anything to launch"
            );
        }
        for record in &mut records {
            record.launch_id = launch_id.clone();
        }
        *self.cached.lock().await = Some(CachedLibrary {
            digest,
            records: records.clone(),
        });
        Ok(records)
    }
}

/// `post /api/datastoreMeta` — one `[id, epoch_ms]` pair per library item.
async fn datastore_meta(auth_key: &str) -> Result<Value> {
    datastore(
        "datastoreMeta",
        auth_key,
        json!({ "collection": "libraryItem" }),
    )
    .await
}

/// `post /api/datastoreGet` — the library itself.
async fn datastore_get(auth_key: &str) -> Result<Value> {
    datastore(
        "datastoreGet",
        auth_key,
        json!({ "collection": "libraryItem", "ids": [], "all": true }),
    )
    .await
}

/// One account-datastore call.
///
/// The account API answers HTTP 200 whether it succeeded or failed, so the body is
/// what decides. Neither the request body (which carries the session key) nor an
/// unexpected response body is ever quoted into an error: both can reach a log line
/// and a client, and one of them holds the credential.
async fn datastore(path: &str, auth_key: &str, body: Value) -> Result<Value> {
    let mut payload = body;
    payload["authKey"] = Value::String(auth_key.to_owned());

    let url = format!("{API_URL}/{path}");
    let response = reqwest::Client::new()
        .post(&url)
        .json(&payload)
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .with_context(|| format!("could not reach {url}"))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("could not read the answer from {url}"))?;
    let parsed: Value = serde_json::from_str(&text)
        .map_err(|_| anyhow!("{url} answered with HTTP {status} and a body that is not JSON"))?;

    if let Some(error) = parsed.get("error").filter(|error| !error.is_null()) {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        // A message from Stremio describes the failure without naming the key.
        bail!("the Stremio account API refused {path}: {message}");
    }
    Ok(parsed)
}

/// The identity of a datastore state: the metadata's own serialization.
fn digest_of(meta: &Value) -> String {
    meta.get("result").map(Value::to_string).unwrap_or_default()
}

/// The unfinished titles, newest first, capped at Stremio's own row length.
fn continue_watching(library: &Value) -> Result<Vec<CatalogRecord>> {
    let items = library
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("the Stremio datastore answered without a result list"))?;

    let mut unfinished: Vec<&Value> = items
        .iter()
        .filter(|item| in_continue_watching(item))
        .collect();
    // Sorted by `_mtime`, which is what Stremio sorts its own row by. `lastWatched`
    // is when the title was actually played and can be weeks earlier — on one
    // account it differed by nineteen days — so only `_mtime` reproduces what a
    // Stremio user sees. Reversed rather than compared by hand, which also leaves an
    // unreadable `_mtime` last rather than first.
    unfinished.sort_by_key(|item| std::cmp::Reverse(mtime(item)));
    unfinished.truncate(CONTINUE_WATCHING_LIMIT);

    Ok(unfinished.into_iter().filter_map(catalog_record).collect())
}

/// Stremio's own predicate, from `library_item.rs::is_in_continue_watching`.
///
/// The `temp` half is not a technicality. Eight of the ten items on the account
/// this was built against were `removed` *and* `temp* — "in progress but not saved
/// to the library" — so requiring `!removed` alone would show two of ten.
fn in_continue_watching(item: &Value) -> bool {
    if item.get("type").and_then(Value::as_str) == Some("other") {
        return false;
    }
    let removed = flag(item, "removed");
    let temp = flag(item, "temp");
    if removed && !temp {
        return false;
    }
    // `flaggedWatched` is deliberately not consulted: an item can be marked watched
    // and still be unfinished, and Stremio shows it.
    time_offset(item) > 0
}

fn catalog_record(item: &Value) -> Option<CatalogRecord> {
    let id = item.get("_id").and_then(Value::as_str)?.trim();
    if id.is_empty() {
        return None;
    }
    let state = state(item);
    let offset = time_offset(item);
    let duration = duration(item);

    let mut metadata = json!({
        "source": SOURCE_ID,
        "type": item.get("type").and_then(Value::as_str).unwrap_or("other"),
        "progress": progress(offset, duration),
        "time_offset_ms": offset,
        "duration_ms": duration,
        "minutes_left": minutes_left(offset, duration),
        "video_id": state.get("video_id").cloned().unwrap_or(Value::Null),
        "last_watched": state.get("lastWatched").cloned().unwrap_or(Value::Null),
        "times_watched": state.get("timesWatched").cloned().unwrap_or(Value::Null),
        // Carried verbatim, and sometimes a range: one record held "2021-2025" as a
        // string rather than a number, so nothing may assume an integer here.
        "year": item.get("year").cloned().unwrap_or(Value::Null),
    });
    // Read from the video id rather than from `state.season`/`state.episode`: those
    // two exist but were 0 on every record an older client had written, while the
    // video id was right on all of them.
    if let Some((season, episode)) = season_episode(item, id) {
        metadata["season"] = json!(season);
        metadata["episode"] = json!(episode);
    }

    Some(CatalogRecord {
        id: format!("{SOURCE_ID}:{id}"),
        title: item
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(id)
            .to_owned(),
        kind: item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("other")
            .to_owned(),
        // Filled in by the provider once per refresh, from the desktop entry the
        // bridge found; see `desktop_entry_for_stremio`. The mapping has no way to
        // know it, and a guess that names nothing is refused at launch time.
        launch_id: None,
        icon: poster(item),
        metadata,
        // The record's own change time, so a snapshot is stable between refreshes
        // rather than every card looking freshly updated.
        updated_at: item
            .get("_mtime")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

/// `tt0149460:4:7` with id `tt0149460` is season 4, episode 7. A film's video id is
/// its own id (`tt26657236`), which yields `None` — films have no episode.
fn season_episode(item: &Value, id: &str) -> Option<(u64, u64)> {
    let video_id = state(item).get("video_id").and_then(Value::as_str)?;
    let rest = video_id.strip_prefix(id)?.trim_start_matches(':');
    let mut parts = rest.split(':');
    let season = parts.next()?.parse().ok()?;
    let episode = parts.next()?.parse().ok()?;
    Some((season, episode))
}

/// The poster as stored, with the size segment normalised.
///
/// Records in one library carried both `/poster/small/` and `/poster/medium/` for
/// the same kind of item, and a dashboard card wants the larger one.
fn poster(item: &Value) -> Option<String> {
    let poster = item.get("poster").and_then(Value::as_str)?.trim();
    if poster.is_empty() {
        return None;
    }
    Some(poster.replace("/poster/small/", "/poster/medium/"))
}

/// The desktop entry that opens a title, as the host's application list has it.
///
/// Asked of the bridge rather than hardcoded. The entry's file name differs between a
/// distribution package and a Flatpak, and an id that names nothing is refused at
/// launch time — which the user sees only as a launch that failed, with nothing
/// saying why. Resolved once per refresh, so an app installed later is picked up on
/// the next one.
async fn desktop_entry_for_stremio(bridge_socket_path: &Path) -> Option<String> {
    let response = bridge::request(
        bridge_socket_path,
        BridgeRequest::DiscoverApplications {
            source_id: crate::DESKTOP_APPS_SOURCE.to_owned(),
        },
    )
    .await
    .ok()?;
    let BridgeResponse::Applications { applications, .. } = response else {
        return None;
    };
    pick_stremio(&applications)
}

/// The Stremio entry among the host's applications, if it is installed at all.
///
/// Matched on the URL scheme the entry declares first, because that is what
/// identifies the application rather than what it happens to be called, and only
/// then on the id and name.
fn pick_stremio(applications: &[DiscoveredApplication]) -> Option<String> {
    let by_scheme = applications
        .iter()
        .find(|application| application.launch_scheme.as_deref() == Some(STREMIO_SCHEME));
    let by_name = applications.iter().find(|application| {
        application
            .application_id
            .to_ascii_lowercase()
            .contains(STREMIO_SCHEME)
            || application
                .name
                .to_ascii_lowercase()
                .contains(STREMIO_SCHEME)
    });
    by_scheme
        .or(by_name)
        .map(|application| application.application_id.clone())
}

fn state(item: &Value) -> &Value {
    item.get("state").unwrap_or(&Value::Null)
}

fn flag(item: &Value, field: &str) -> bool {
    item.get(field).and_then(Value::as_bool).unwrap_or(false)
}

fn time_offset(item: &Value) -> u64 {
    state(item)
        .get("timeOffset")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn duration(item: &Value) -> u64 {
    state(item)
        .get("duration")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

/// How far in, from 0 to 1, in milliseconds. Zero when the length is unknown,
/// rather than a division that invents a position.
fn progress(offset: u64, duration: u64) -> f64 {
    if duration == 0 {
        return 0.0;
    }
    (offset as f64 / duration as f64).clamp(0.0, 1.0)
}

fn minutes_left(offset: u64, duration: u64) -> u64 {
    duration.saturating_sub(offset) / 60_000
}

/// `_mtime` as a time, for ordering.
///
/// The precision varies between records — millisecond timestamps and nanosecond
/// ones appear side by side — so this parses RFC 3339 rather than slicing the
/// string. An unparsable value sorts last instead of silently winning.
fn mtime(item: &Value) -> Option<DateTime<Utc>> {
    item.get("_mtime")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::{
        CONTINUE_WATCHING_LIMIT, STREMIO_SCHEME, continue_watching, digest_of,
        in_continue_watching, pick_stremio,
    };
    use crate::discovery::DiscoveryProvider;
    use hearthdeck_protocol::{ApplicationSource, DiscoveredApplication};
    use serde_json::{Value, json};

    /// The observed film record, `removed` and `temp` both true, 34.3% in.
    fn backrooms() -> Value {
        json!({
            "_id": "tt26657236", "name": "Backrooms", "type": "movie",
            "poster": "https://images.metahub.space/poster/small/tt26657236/img",
            "removed": true, "temp": true, "year": "2021-2025",
            "_mtime": "2026-09-28T18:26:29.322320491Z",
            "state": {
                "lastWatched": "2026-09-09T19:49:06.859Z", "timeOffset": 2271644,
                "duration": 6628768, "timesWatched": 0, "flaggedWatched": 0,
                "video_id": "tt26657236", "watched": ""
            }
        })
    }

    /// The observed series record: mark it watched and it still counts; the state
    /// season/episode fields are 0 while the video id holds the truth.
    fn the_strain() -> Value {
        json!({
            "_id": "tt2654620", "name": "The Strain", "type": "series",
            "poster": "https://images.metahub.space/poster/small/tt2654620/img",
            "removed": true, "temp": true,
            "_mtime": "2025-09-29T20:09:53.046321912Z",
            "state": {
                "lastWatched": "2025-09-29T20:09:53.046199078Z", "timeOffset": 857070,
                "duration": 2505558, "timesWatched": 21, "flaggedWatched": 1,
                "season": 0, "episode": 0, "video_id": "tt2654620:2:10"
            }
        })
    }

    fn library(items: Vec<Value>) -> Value {
        json!({ "result": items })
    }

    /// The two halves of the predicate that the real data forced: `temp` keeps an
    /// in-progress item that was never saved, and `flaggedWatched` excludes nothing.
    #[test]
    fn keeps_removed_temp_items_and_watched_ones_but_drops_finished_and_other() {
        assert!(in_continue_watching(&backrooms()));
        assert!(in_continue_watching(&the_strain()));

        // Removed and genuinely gone from the library.
        let mut gone = backrooms();
        gone["temp"] = json!(false);
        assert!(!in_continue_watching(&gone));

        // Not a film or a series.
        let mut other = backrooms();
        other["type"] = json!("other");
        assert!(!in_continue_watching(&other));

        // Nothing played yet.
        let mut unstarted = backrooms();
        unstarted["state"]["timeOffset"] = json!(0);
        assert!(!in_continue_watching(&unstarted));
    }

    /// A film's video id is its own id, so it has no episode; a series reads
    /// season and episode from the video id, not from the state fields that are 0.
    #[test]
    fn takes_the_episode_from_the_video_id_not_the_state_fields() {
        let film = continue_watching(&library(vec![backrooms()])).unwrap();
        let film = &film[0];
        assert_eq!(film.id, "stremio:tt26657236");
        assert_eq!(film.kind, "movie");
        assert_eq!(film.metadata["season"], Value::Null);
        assert_eq!(film.metadata["episode"], Value::Null);

        let series = continue_watching(&library(vec![the_strain()])).unwrap();
        assert_eq!(series[0].metadata["season"], json!(2));
        assert_eq!(series[0].metadata["episode"], json!(10));
        assert_eq!(series[0].metadata["video_id"], json!("tt2654620:2:10"));
    }

    /// Progress comes out of millisecond values, and a year that is a range stays a
    /// string rather than being coerced.
    #[test]
    fn reports_progress_in_milliseconds_and_leaves_a_range_year_alone() {
        let records = continue_watching(&library(vec![backrooms()])).unwrap();
        let metadata = &records[0].metadata;
        assert_eq!(metadata["time_offset_ms"], json!(2271644));
        assert_eq!(metadata["duration_ms"], json!(6628768));
        assert_eq!(metadata["minutes_left"], json!(72));
        let progress = metadata["progress"].as_f64().unwrap();
        assert!((progress - 0.3426).abs() < 0.001, "progress was {progress}");
        assert_eq!(metadata["year"], json!("2021-2025"));
    }

    /// The row is Stremio's own length and its own order: newest write first.
    #[test]
    fn caps_the_row_at_ten_and_sorts_by_the_record_time() {
        let items = (0..12)
            .map(|index| {
                let mut item = backrooms();
                item["_id"] = json!(format!("tt{index:07}"));
                item["_mtime"] = json!(format!("2026-01-{:02}T00:00:00Z", index + 1));
                item["state"]["timeOffset"] = json!(1000 + index);
                item
            })
            .collect();
        let records = continue_watching(&library(items)).unwrap();

        assert_eq!(records.len(), CONTINUE_WATCHING_LIMIT);
        // Index 11 is the newest write, so it leads.
        assert_eq!(records[0].id, "stremio:tt0000011");
        assert_eq!(records[9].id, "stremio:tt0000002");

        // Newest first, checked pairwise rather than by sorting a copy: the assertion
        // then names the pair that broke the order instead of showing two lists.
        let times: Vec<&String> = records.iter().map(|record| &record.updated_at).collect();
        assert!(
            times.windows(2).all(|pair| pair[0] >= pair[1]),
            "records must be newest first: {times:?}"
        );
    }

    /// A record with no poster, or a small one, still yields a usable card.
    #[test]
    fn normalises_the_poster_size_and_the_app_level_launch() {
        let records = continue_watching(&library(vec![backrooms()])).unwrap();
        assert_eq!(
            records[0].icon.as_deref(),
            Some("https://images.metahub.space/poster/medium/tt26657236/img")
        );
        // No launch target: the mapping cannot know one, and the provider fills it in
        // from the desktop entry the bridge found.
        assert_eq!(records[0].launch_id, None);

        let mut bare = backrooms();
        bare["poster"] = json!("");
        let records = continue_watching(&library(vec![bare])).unwrap();
        assert_eq!(records[0].icon, None);
    }

    /// The digest is what decides whether the library is read at all, so it has to
    /// move when an item does and hold still when nothing has.
    #[test]
    fn the_digest_tracks_the_metadata_and_ignores_what_it_does_not_carry() {
        let before = json!({ "result": [["tt1", 1], ["tt2", 2]] });
        // Pinned, not merely self-consistent: the digest is how a refresh decides
        // there is nothing to fetch.
        assert_eq!(digest_of(&before), r#"[["tt1",1],["tt2",2]]"#);

        let moved = json!({ "result": [["tt1", 1], ["tt2", 3]] });
        assert_ne!(digest_of(&before), digest_of(&moved));

        let arrived = json!({ "result": [["tt1", 1], ["tt2", 2], ["tt3", 2]] });
        assert_ne!(digest_of(&before), digest_of(&arrived));
    }

    /// A film's `watched` bitfield carries the literal string `undefined` in its
    /// prefix, and can be empty. The mapping reads it for nothing, so neither may
    /// disturb it — this is a regression guard on a value that looks like a bug in
    /// the data and is.
    #[test]
    fn an_undefined_or_empty_watched_bitfield_does_not_disturb_the_mapping() {
        let mut film = backrooms();
        film["state"]["watched"] = json!("undefined:1:eJwDAAAAAAE=");
        let records = continue_watching(&library(vec![film])).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "stremio:tt26657236");
        assert_eq!(records[0].metadata["time_offset_ms"], json!(2271644));

        let mut empty = backrooms();
        empty["state"]["watched"] = json!("");
        assert_eq!(continue_watching(&library(vec![empty])).unwrap().len(), 1);
    }

    /// The host's Stremio entry is found by the scheme it declares, not by a guessed
    /// file name: the same application ships as `com.stremio.Stremio.desktop` from a
    /// distribution package and under another id as a Flatpak, and a guess that names
    /// nothing is refused at launch time.
    #[test]
    fn finds_the_stremio_entry_by_its_scheme_before_its_name() {
        let application = |id: &str, name: &str, scheme: Option<&str>| DiscoveredApplication {
            application_id: id.to_owned(),
            name: name.to_owned(),
            comment: None,
            icon: None,
            categories: Vec::new(),
            launch_scheme: scheme.map(str::to_owned),
            // Which entry is picked is decided by the scheme, not the origin.
            source: ApplicationSource::System,
        };

        assert_eq!(pick_stremio(&[]), None);
        assert_eq!(
            pick_stremio(&[application("org.videolan.VLC.desktop", "VLC", Some("vlc"))]),
            None
        );

        // The scheme wins even when another entry merely mentions the name.
        let applications = [
            application("com.example.stremio-helper.desktop", "Stremio Helper", None),
            application(
                "net.stremio.Stremio.desktop",
                "Stremio",
                Some(STREMIO_SCHEME),
            ),
        ];
        assert_eq!(
            pick_stremio(&applications).as_deref(),
            Some("net.stremio.Stremio.desktop")
        );

        // A name is still enough for an entry that declares no scheme at all.
        assert_eq!(
            pick_stremio(&[application("com.stremio.Stremio.desktop", "Stremio", None)]).as_deref(),
            Some("com.stremio.Stremio.desktop")
        );
    }

    /// An account that is not linked publishes nothing, and does not fail: an error
    /// here would show up as a degraded provider on every refresh forever.
    #[tokio::test]
    async fn an_unlinked_account_publishes_nothing_rather_than_failing() {
        use crate::database::Database;
        use crate::stremio::StremioRepository;

        let directory = tempfile::tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let provider = super::StremioProvider::new(
            StremioRepository::new(database.pool().clone()),
            // A socket that is not there: an unlinked account must return before it
            // ever asks the bridge anything.
            directory.path().join("bridge.sock"),
        );

        assert!(provider.discover().await.unwrap().is_empty());
    }
}
