//! New arrivals: the newest titles on Stremio that this account can play.
//!
//! The title list is Cinemeta's "New" catalog — `catalog/<type>/year/genre=<year>`
//! for the current and the previous year — which is the feed Stremio itself shows
//! as new. That feed is a *release calendar*, though: it lists a film the day it
//! opens in cinemas, and no field in the meta says whether it can be played. So
//! the banner asks the next question before publishing a slide: does the linked
//! account have a stream for this title? Only the titles whose own addons answer
//! with at least one stream become arrivals. A banner is then something somebody
//! can put on tonight rather than a countdown to a cinema release.
//!
//! A refresh is four kinds of call: the year buckets for ids and art, the addon
//! collection once, one stream request per candidate (stopping as soon as the
//! banner is full), and the bridge for the desktop entry a slide opens.
//!
//! `SOURCE_ID` keeps the `stremio-arrivals` of the provider this replaces: the id
//! is what the catalog rows and the `stremio:new-arrivals` collection rule are
//! keyed by, so renaming it would orphan both for no gain a client can see.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{Datelike, Utc};
use futures_util::future::join_all;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::{
    catalog::CatalogRecord,
    discovery::DiscoveryProvider,
    discovery::providers::stremio::desktop_entry_for_stremio,
    stremio::{StremioRepository, addons::StreamAddons},
};

/// The catalog source this provider owns.
pub use hearthdeck_protocol::sources::STREMIO_ARRIVALS as SOURCE_ID;

/// Cinemeta's base URL. `v3-cinemeta.stremio.com` is the host everybody quotes
/// and it does not resolve at all; the addon lives on `v3-cinemeta.strem.io`, the
/// short registry-style host.
const CINEMETA_URL: &str = "https://v3-cinemeta.strem.io";

/// How many arrivals a banner rotates through. Ten is what the user asked for,
/// and a rotation of more than ten on a television is a playlist, not a banner.
const NEW_ARRIVALS_LIMIT: usize = 10;

/// How many of the newest candidates are asked about a stream.
///
/// The check is what decides, and it costs one request per addon per candidate,
/// so the budget is bounded: the newest forty titles, newest first. In practice
/// the first batch fills the banner; the budget exists so a week of cinema-only
/// releases cannot turn one refresh into hundreds of requests.
const CANDIDATES_TO_CHECK: usize = 40;

/// How many candidates are asked at once. Stream addons scrape indexers, so this
/// is a small number on purpose.
const STREAM_CHECKS_IN_FLIGHT: usize = 4;

/// How often the feed is re-read. Release data moves by the day, and a stream
/// list moves slower than that, so an hourly refresh is current for a kiosk
/// without re-asking every candidate's addons every few minutes.
const REFRESH_INTERVAL: Duration = Duration::from_secs(60 * 60);

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

pub struct NewArrivalsProvider {
    /// The linked account. Its session key is what asks the addons, so without a
    /// link there is no way to tell a playable arrival from an unplayable one,
    /// and the honest answer is no banner at all.
    account: StremioRepository,
    /// Where the host answers questions about installed applications: a banner
    /// slide launches the Stremio desktop entry, and only the bridge knows which
    /// entry that is.
    bridge_socket_path: PathBuf,
    client: reqwest::Client,
}

impl NewArrivalsProvider {
    pub fn new(account: StremioRepository, bridge_socket_path: PathBuf) -> Self {
        Self {
            account,
            bridge_socket_path,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl DiscoveryProvider for NewArrivalsProvider {
    fn source_id(&self) -> &'static str {
        SOURCE_ID
    }

    fn refresh_interval(&self) -> Option<Duration> {
        Some(REFRESH_INTERVAL)
    }

    async fn discover(&self) -> Result<Vec<CatalogRecord>> {
        let Some(credentials) = self.account.credentials().await? else {
            // Not linked. No addons can be asked, so no arrival can be
            // confirmed; publishing nothing is the answer rather than a guess.
            debug!("the stremio account is not linked; publishing no arrivals");
            return Ok(Vec::new());
        };

        // The year the buckets are asked for. A release year is a fact about the
        // clock, kept out of the mapping so that stays a pure function.
        let year = Utc::now().year();
        let mut metas = Vec::new();
        let mut failures = 0;
        for kind in ["movie", "series"] {
            for bucket in [year - 1, year] {
                match self.new_catalog(kind, bucket).await {
                    Ok(mut page) => metas.append(&mut page),
                    Err(error) => {
                        failures += 1;
                        warn!(
                            kind,
                            bucket,
                            %error,
                            "the Cinemeta new-arrivals bucket could not be read"
                        );
                    }
                }
            }
        }
        if metas.is_empty() && failures > 0 {
            // Nothing could be read at all. Refusing rather than publishing an
            // empty snapshot keeps the last good arrivals on the banner instead
            // of wiping it because one addon call failed.
            bail!("no Cinemeta new-arrivals bucket could be read");
        }

        let addons = StreamAddons::installed(&credentials.auth_key).await?;
        if addons.is_empty() {
            // The account is linked but has no addon that serves streams. Asking
            // nothing and publishing an unconfirmed list would put unplayable
            // slides on the banner, which is the failure this provider exists to
            // stop.
            warn!("the Stremio account has no addon that serves streams; publishing no arrivals");
            return Ok(Vec::new());
        }

        let candidates = newest_candidates(&metas);
        let streamed = self.confirmed(&addons, &candidates).await?;
        let mut records = arrivals(&candidates, &streamed);
        // The launch target is resolved once per refresh, like the account
        // provider does: which desktop entry opens a title is a fact about this
        // host, not about the feed.
        let launch_id = desktop_entry_for_stremio(&self.bridge_socket_path).await;
        if launch_id.is_none() {
            warn!(
                "no Stremio desktop entry was discovered, so the banner will draw without \
                 anything to launch"
            );
        }
        for record in &mut records {
            record.launch_id = launch_id.clone();
        }
        debug!(
            records = records.len(),
            candidates = candidates.len(),
            addons = addons.len(),
            "stremio new arrivals refreshed"
        );
        Ok(records)
    }
}

impl NewArrivalsProvider {
    /// `GET <cinemeta>/catalog/<kind>/year/genre=<year>.json` — one bucket of the
    /// "New" catalog, as the raw metas it answered with.
    async fn new_catalog(&self, kind: &str, year: i32) -> Result<Vec<Value>> {
        let url = format!("{CINEMETA_URL}/catalog/{kind}/year/genre={year}.json");
        let response = self
            .client
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/json")
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|error| {
                anyhow!(
                    "could not reach Cinemeta for {kind} {year}: {}",
                    error.without_url()
                )
            })?;
        let status = response.status();
        let text = response.text().await.map_err(|error| {
            anyhow!(
                "could not read Cinemeta's answer for {kind} {year}: {}",
                error.without_url()
            )
        })?;
        if !status.is_success() {
            bail!("Cinemeta answered HTTP {status} for {kind} {year}");
        }
        let parsed: Value = serde_json::from_str(&text).map_err(|_| {
            anyhow!(
                "Cinemeta answered HTTP {status} for {kind} {year} with a body that is not JSON"
            )
        })?;
        // The addon protocol answers `{ metas: [...] }` for a catalog resource.
        let metas = parsed
            .get("metas")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("Cinemeta answered without a metas list for {kind} {year}"))?;
        Ok(metas.clone())
    }

    /// The ids among the newest candidates that at least one addon can play.
    ///
    /// Checked newest first, a few at a time, and abandoned once the banner is
    /// full: asking all forty candidates when the first twelve had streams would
    /// spend three times the requests to fill the same ten slides.
    ///
    /// A title whose addons all failed is left out of the set rather than
    /// published unconfirmed. If *nothing* could be confirmed because every
    /// lookup failed, the refresh refuses instead of publishing an empty
    /// snapshot, so a broken addon cannot wipe a banner that was working.
    async fn confirmed(
        &self,
        addons: &StreamAddons,
        candidates: &[Value],
    ) -> Result<HashSet<String>> {
        let mut streamed = HashSet::new();
        let mut checked = 0;
        let mut answered = 0;
        for batch in candidates.chunks(STREAM_CHECKS_IN_FLIGHT) {
            let answers = join_all(batch.iter().map(|meta| {
                let id = meta_id(meta).unwrap_or_default();
                addons.has_stream(meta_type(meta), id)
            }))
            .await;
            for (meta, answer) in batch.iter().zip(answers) {
                checked += 1;
                match answer {
                    Ok(true) => {
                        answered += 1;
                        if let Some(id) = meta_id(meta) {
                            streamed.insert(id.to_owned());
                        }
                    }
                    Ok(false) => answered += 1,
                    Err(error) => {
                        debug!(
                            id = meta_id(meta).unwrap_or_default(),
                            %error,
                            "a stream could not be confirmed for an arrival"
                        );
                    }
                }
            }
            if streamed.len() >= NEW_ARRIVALS_LIMIT {
                break;
            }
        }
        if checked > 0 && answered == 0 {
            bail!("no stream addon answered for any of the {checked} newest arrivals");
        }
        Ok(streamed)
    }
}

/// The candidates worth checking, newest release first.
///
/// The buckets are merged rather than trusted one by one: one live answer put a
/// series inside the `movie` bucket, so what a meta says it is decides whether it
/// belongs, and the same title arriving through both buckets lands once. The
/// order is the metas' own `released` date, falling back to the addon's order —
/// a stable sort keeps that recency weighting for the many metas the field is
/// absent on rather than inventing an order for them.
fn newest_candidates(metas: &[Value]) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut candidates: Vec<&Value> = metas
        .iter()
        .filter(|meta| {
            let Some(id) = meta_id(meta) else {
                return false;
            };
            // Both a slide and a launch need this: the bridge builds the deep
            // link from an IMDb id and refuses anything else, and a stream addon
            // is asked by that same id.
            is_title_id(id) && seen.insert(id)
        })
        .filter(|meta| matches!(meta_type(meta), "movie" | "series"))
        .collect();
    candidates.sort_by_key(|meta| std::cmp::Reverse(released_at(meta)));
    candidates
        .into_iter()
        .take(CANDIDATES_TO_CHECK)
        .cloned()
        .collect()
}

/// The confirmed candidates that make the banner, newest first.
fn arrivals(candidates: &[Value], streamed: &HashSet<String>) -> Vec<CatalogRecord> {
    candidates
        .iter()
        .filter(|meta| meta_id(meta).is_some_and(|id| streamed.contains(id)))
        .filter_map(arrival_record)
        .take(NEW_ARRIVALS_LIMIT)
        .collect()
}

/// One meta as a catalog record: the id the launch route resolves, the art the
/// banner draws with, and the title's own type so a series opens as a series.
fn arrival_record(meta: &Value) -> Option<CatalogRecord> {
    let id = meta_id(meta)?;
    let kind = meta_type(meta);
    let metadata = json!({
        "source": SOURCE_ID,
        "type": kind,
        // The bridge builds the `stremio://` detail link from this. An arrival
        // has no watch state, so it is the bare IMDb id and the type above is
        // what makes a series a series.
        "video_id": id,
        // The hero banner's own artwork: Cinemeta fills `background` and `logo`
        // on every meta it serves, which is the one dependable wide art this
        // feed has — a portrait poster cannot carry a 30%-tall banner.
        "background": meta.get("background").cloned().unwrap_or(Value::Null),
        "logo": meta.get("logo").cloned().unwrap_or(Value::Null),
        "description": meta.get("description").cloned().unwrap_or(Value::Null),
        // Carried loosely, exactly as the feed sent it: `year` is a string on
        // some records and `releaseInfo` a range.
        "year": meta.get("year").cloned().unwrap_or(Value::Null),
        "release_info": meta.get("releaseInfo").cloned().unwrap_or(Value::Null),
        "released": meta.get("released").cloned().unwrap_or(Value::Null),
        "imdb_rating": meta.get("imdbRating").cloned().unwrap_or(Value::Null),
    });

    Some(CatalogRecord {
        id: format!("{SOURCE_ID}:{id}"),
        title: meta
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(id)
            .to_owned(),
        kind: kind.to_owned(),
        // Filled in by the provider once per refresh from the desktop entry the
        // bridge found; the mapping has no way to know it.
        launch_id: None,
        icon: crate::discovery::providers::stremio::poster(meta),
        metadata,
        // The release date, so `list_source`'s newest-first order is the banner's
        // own. A meta without one sorts last rather than looking freshly arrived.
        updated_at: meta
            .get("released")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

/// The IMDb id a meta carries, trimmed and non-empty.
fn meta_id(meta: &Value) -> Option<&str> {
    meta.get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

/// Whether an id is an IMDb title id: `tt` followed by digits. The same rule the
/// bridge validates a video id by, kept here so an arrival that could never be
/// launched is dropped at the door instead of drawn on the banner.
fn is_title_id(id: &str) -> bool {
    id.strip_prefix("tt").is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    })
}

/// What the meta says it is. Anything not spelled `series` is `movie`-shaped
/// here only if it is spelled `movie`; the caller filters the rest out.
fn meta_type(meta: &Value) -> &str {
    meta.get("type").and_then(Value::as_str).unwrap_or("other")
}

/// When the title was released, as a comparable instant.
///
/// `released` is an RFC 3339 timestamp on the metas seen so far, but the feed
/// types nothing strictly — one account's `year` arrived as `"2021-2025"` — so a
/// plain `YYYY-MM-DD` is accepted too and anything else is `None` rather than a
/// guess. `None` sorts last, which is where a title nobody dated belongs.
fn released_at(meta: &Value) -> Option<i64> {
    let released = meta.get("released").and_then(Value::as_str)?.trim();
    if let Ok(instant) = chrono::DateTime::parse_from_rfc3339(released) {
        return Some(instant.timestamp());
    }
    let date = chrono::NaiveDate::parse_from_str(released, "%Y-%m-%d").ok()?;
    Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::{Value, json};

    use super::{
        CANDIDATES_TO_CHECK, NEW_ARRIVALS_LIMIT, arrival_record, arrivals, meta_id,
        newest_candidates, released_at,
    };

    /// A Cinemeta meta as the feed serves them: full objects, not previews.
    fn meta(id: &str, kind: &str, name: &str, released: Option<&str>) -> Value {
        json!({
            "id": id,
            "type": kind,
            "name": name,
            "poster": format!("https://images.metahub.space/poster/medium/{id}/img"),
            "background": format!("https://images.metahub.space/background/medium/{id}/img"),
            "logo": format!("https://images.metahub.space/logo/medium/{id}/img"),
            "description": format!("About {name}"),
            "year": "2026",
            "releaseInfo": "2026",
            "released": released,
            "imdbRating": "7.1",
        })
    }

    fn streamed(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn the_banner_holds_the_ten_most_recently_released_confirmed_arrivals() {
        let metas: Vec<Value> = (1..=12)
            .map(|index| {
                meta(
                    &format!("tt{index:07}"),
                    "movie",
                    &format!("Film {index}"),
                    Some(&format!("2026-10-{index:02}T00:00:00.000Z")),
                )
            })
            .collect();
        let candidates = newest_candidates(&metas);
        let ids: Vec<String> = (3..=12).map(|index| format!("tt{index:07}")).collect();
        let streamed = streamed(&ids.iter().map(String::as_str).collect::<Vec<_>>());

        let records = arrivals(&candidates, &streamed);

        let titles: Vec<&str> = records.iter().map(|record| record.title.as_str()).collect();
        assert_eq!(
            titles,
            vec![
                "Film 12", "Film 11", "Film 10", "Film 9", "Film 8", "Film 7", "Film 6", "Film 5",
                "Film 4", "Film 3",
            ],
            "arrivals: {titles:?}"
        );
    }

    /// The point of the provider: a title the feed calls new is not an arrival
    /// unless something can play it.
    #[test]
    fn an_arrival_with_no_stream_is_not_on_the_banner() {
        let metas = vec![
            meta(
                "tt0000001",
                "movie",
                "No stream",
                Some("2026-10-20T00:00:00.000Z"),
            ),
            meta(
                "tt0000002",
                "movie",
                "Playable",
                Some("2026-10-19T00:00:00.000Z"),
            ),
        ];
        let candidates = newest_candidates(&metas);

        let records = arrivals(&candidates, &streamed(&["tt0000002"]));

        let titles: Vec<&str> = records.iter().map(|record| record.title.as_str()).collect();
        assert_eq!(titles, vec!["Playable"], "arrivals: {titles:?}");
    }

    #[test]
    fn the_ten_arrivals_mix_films_and_series() {
        let metas = vec![
            meta(
                "tt0000001",
                "movie",
                "A film",
                Some("2026-10-20T00:00:00.000Z"),
            ),
            meta(
                "tt0000002",
                "series",
                "A series",
                Some("2026-10-19T00:00:00.000Z"),
            ),
        ];
        let candidates = newest_candidates(&metas);

        let records = arrivals(&candidates, &streamed(&["tt0000001", "tt0000002"]));

        let kinds: Vec<&str> = records.iter().map(|record| record.kind.as_str()).collect();
        assert_eq!(kinds, vec!["movie", "series"], "arrivals: {records:?}");
    }

    #[test]
    fn one_title_lands_once_when_both_buckets_return_it() {
        let metas = vec![
            meta(
                "tt0000001",
                "movie",
                "Twice",
                Some("2026-10-20T00:00:00.000Z"),
            ),
            meta(
                "tt0000001",
                "movie",
                "Twice",
                Some("2026-10-20T00:00:00.000Z"),
            ),
        ];

        let candidates = newest_candidates(&metas);

        assert_eq!(candidates.len(), 1, "candidates: {candidates:?}");
    }

    #[test]
    fn a_meta_that_is_neither_a_film_nor_a_series_is_not_an_arrival() {
        let metas = vec![
            meta(
                "tt0000001",
                "other",
                "A channel",
                Some("2026-10-20T00:00:00.000Z"),
            ),
            meta(
                "yt_id:abc",
                "movie",
                "A video",
                Some("2026-10-19T00:00:00.000Z"),
            ),
            meta(
                "tt0000002",
                "movie",
                "Kept",
                Some("2026-10-18T00:00:00.000Z"),
            ),
        ];

        let candidates = newest_candidates(&metas);
        let records = arrivals(&candidates, &streamed(&["tt0000001", "tt0000002"]));

        let ids: Vec<&str> = records.iter().map(|record| record.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["stremio-arrivals:tt0000002"],
            "arrivals: {records:?}"
        );
    }

    #[test]
    fn a_title_nobody_dated_sorts_after_one_that_was() {
        let metas = vec![
            meta("tt0000001", "movie", "Undated", None),
            meta(
                "tt0000002",
                "movie",
                "Dated",
                Some("2026-10-20T00:00:00.000Z"),
            ),
        ];
        let candidates = newest_candidates(&metas);

        let records = arrivals(&candidates, &streamed(&["tt0000001", "tt0000002"]));

        let titles: Vec<&str> = records.iter().map(|record| record.title.as_str()).collect();
        assert_eq!(titles, vec!["Dated", "Undated"], "arrivals: {titles:?}");
    }

    /// The stream check is bounded: only the newest candidates are asked about,
    /// and the ordering is the one the banner will use.
    #[test]
    fn only_the_newest_candidates_are_checked_for_a_stream() {
        let metas: Vec<Value> = (1..=CANDIDATES_TO_CHECK + 5)
            .map(|index| {
                meta(
                    &format!("tt{index:07}"),
                    "movie",
                    &format!("Film {index}"),
                    Some(&format!("2026-10-{:02}T00:00:00.000Z", index % 28 + 1)),
                )
            })
            .collect();

        let candidates = newest_candidates(&metas);

        assert_eq!(candidates.len(), CANDIDATES_TO_CHECK);
        let dates: Vec<&str> = candidates
            .iter()
            .filter_map(|meta| meta.get("released").and_then(Value::as_str))
            .collect();
        assert!(
            dates.windows(2).all(|pair| pair[0] >= pair[1]),
            "candidates must be newest first: {dates:?}"
        );
    }

    #[test]
    fn the_banner_never_exceeds_its_length() {
        let metas: Vec<Value> = (1..=25)
            .map(|index| {
                meta(
                    &format!("tt{index:07}"),
                    "movie",
                    &format!("Film {index}"),
                    Some(&format!("2026-10-{:02}T00:00:00.000Z", index % 28 + 1)),
                )
            })
            .collect();
        let candidates = newest_candidates(&metas);
        let all: Vec<String> = (1..=25).map(|index| format!("tt{index:07}")).collect();
        let streamed = streamed(&all.iter().map(String::as_str).collect::<Vec<_>>());

        let records = arrivals(&candidates, &streamed);

        assert_eq!(records.len(), NEW_ARRIVALS_LIMIT);
    }

    #[test]
    fn a_release_date_is_read_whether_the_feed_sends_a_timestamp_or_a_date() {
        let timestamp = json!({ "released": "2026-10-20T00:00:00.000Z" });
        let date = json!({ "released": "2026-10-20" });

        assert_eq!(released_at(&timestamp), released_at(&date));
        assert_eq!(released_at(&json!({ "released": "2021-2025" })), None);
    }

    #[test]
    fn an_arrival_record_carries_the_art_and_the_title_type_the_banner_launches_by() {
        let record = arrival_record(&meta(
            "tt0000001",
            "series",
            "A series",
            Some("2026-10-20T00:00:00.000Z"),
        ))
        .expect("a well-formed meta maps to a record");

        assert_eq!(record.id, "stremio-arrivals:tt0000001");
        assert_eq!(record.kind, "series");
        assert_eq!(record.updated_at, "2026-10-20T00:00:00.000Z");
        let discovery = &record.metadata;
        assert_eq!(discovery["video_id"], "tt0000001");
        assert_eq!(discovery["type"], "series");
        assert_eq!(
            discovery["background"],
            "https://images.metahub.space/background/medium/tt0000001/img"
        );
        assert_eq!(
            discovery["logo"],
            "https://images.metahub.space/logo/medium/tt0000001/img"
        );
        assert_eq!(discovery["description"], "About A series");
    }

    #[test]
    fn a_meta_without_an_id_maps_to_nothing() {
        assert!(meta_id(&json!({ "name": "No id" })).is_none());
        assert!(arrival_record(&json!({ "name": "No id" })).is_none());
    }
}
