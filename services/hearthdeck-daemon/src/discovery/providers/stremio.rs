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

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tracing::debug;

use crate::{
    catalog::CatalogRecord,
    discovery::DiscoveryProvider,
    stremio::{SOURCE_ID, StremioRepository},
};

const API_URL: &str = "https://api.strem.io/api";

/// Stremio's home row is ten items (`CATALOG_PREVIEW_SIZE` in its own client), so
/// this is the same ten a Stremio user sees rather than an invented length.
const CONTINUE_WATCHING_LIMIT: usize = 10;

/// The app that opens a title, until the deep link lands.
///
/// Decision 8 of `docs/stremio-integration.md`: the launch the user actually wants
/// is `stremio://` at the exact episode, and that format is unverified until it has
/// been tried on the kiosk. Until then a card opens Stremio itself, which resumes
/// from its own state, rather than doing nothing.
const STREMIO_DESKTOP_ID: &str = "com.stremio.Stremio.desktop";

pub struct StremioProvider {
    settings: StremioRepository,
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
    pub fn new(settings: StremioRepository) -> Self {
        Self {
            settings,
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
        let records = continue_watching(&library)?;
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
    meta.get("result")
        .map(Value::to_string)
        .unwrap_or_else(String::new)
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
    // Stremio user sees.
    unfinished.sort_by(|left, right| mtime(right).cmp(&mtime(left)));
    unfinished.truncate(CONTINUE_WATCHING_LIMIT);

    Ok(unfinished
        .into_iter()
        .filter_map(|item| catalog_record(item))
        .collect())
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
        // The app-level launch, per decision 8; Phase 6 replaces this with a typed
        // deep link once the scheme is confirmed on hardware.
        launch_id: Some(STREMIO_DESKTOP_ID.to_owned()),
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
        CONTINUE_WATCHING_LIMIT, STREMIO_DESKTOP_ID, continue_watching, digest_of,
        in_continue_watching,
    };
    use crate::discovery::DiscoveryProvider;
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

        let times: Vec<&String> = records.iter().map(|record| &record.updated_at).collect();
        let mut sorted = times.clone();
        sorted.sort_by(|left, right| right.cmp(left));
        assert_eq!(times, sorted, "records must be newest first");
    }

    /// A record with no poster, or a small one, still yields a usable card.
    #[test]
    fn normalises_the_poster_size_and_the_app_level_launch() {
        let records = continue_watching(&library(vec![backrooms()])).unwrap();
        assert_eq!(
            records[0].icon.as_deref(),
            Some("https://images.metahub.space/poster/medium/tt26657236/img")
        );
        assert_eq!(records[0].launch_id.as_deref(), Some(STREMIO_DESKTOP_ID));

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
        assert_eq!(digest_of(&before), digest_of(&before.clone()));

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
        let provider = super::StremioProvider::new(StremioRepository::new(database.pool().clone()));

        assert!(provider.discover().await.unwrap().is_empty());
    }
}
