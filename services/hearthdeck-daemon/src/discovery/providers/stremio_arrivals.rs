//! Stremio's new arrivals: the newest titles on Stremio, read from the Cinemeta
//! addon's "New" catalog and published as catalog records.
//!
//! The account datastore cannot answer "what just arrived" — it holds what the
//! person already has. What Discover shows as new is an addon catalog instead,
//! and Cinemeta (`com.linvo.cinemeta`, the metadata addon Stremio itself ships
//! against) is the one that answers anonymously, with no account and no key:
//! addon identity is the manifest URL and every call is a plain GET.
//!
//! Its "New" catalog is `catalog/<type>/year/genre=<year>.json` — a per release
//! year bucket whose extra is *required* and whose options are years. There is no
//! `last` catalog id and the addon protocol has no `sort` extra at all, so
//! "recently added" is purely an addon-side convention and this one is a year
//! bucket. The current and the previous year are both read, so a January
//! dashboard is not an empty banner, and the merge is sorted by the metas' own
//! `released` date rather than trusting the bucket's order: one live page listed
//! an October 9 title before October 14/15 ones, so the catalog is
//! recency-weighted, not strictly newest first.
//!
//! `docs/stremio-integration.md` records the verified shapes and the traps.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{Datelike, Utc};
use serde_json::Value;
use tracing::{debug, warn};

use crate::{
    catalog::CatalogRecord, discovery::DiscoveryProvider,
    discovery::providers::stremio::desktop_entry_for_stremio,
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

/// How often the bucket is re-read. A release calendar moves by the day, so
/// half an hour keeps a long-running kiosk current without polling a public
/// addon like a live view.
const REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);

pub struct StremioArrivalsProvider {
    /// Where the host answers questions about installed applications: a banner
    /// slide launches the Stremio desktop entry, and only the bridge knows which
    /// entry that is.
    bridge_socket_path: PathBuf,
}

impl StremioArrivalsProvider {
    pub fn new(bridge_socket_path: PathBuf) -> Self {
        Self { bridge_socket_path }
    }
}

#[async_trait]
impl DiscoveryProvider for StremioArrivalsProvider {
    fn source_id(&self) -> &'static str {
        SOURCE_ID
    }

    fn refresh_interval(&self) -> Option<Duration> {
        Some(REFRESH_INTERVAL)
    }

    async fn discover(&self) -> Result<Vec<CatalogRecord>> {
        // The year the buckets are asked for. A release year is a fact about the
        // clock, kept out of the mapping so that stays a pure function.
        let year = Utc::now().year();
        let mut metas = Vec::new();
        let mut failures = 0;
        for kind in ["movie", "series"] {
            for bucket in [year - 1, year] {
                match new_catalog(kind, bucket).await {
                    Ok(mut page) => metas.append(&mut page),
                    Err(error) => {
                        failures += 1;
                        warn!(kind, bucket, %error, "the Cinemeta new-arrivals bucket could not be read");
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

        let mut records = newest_arrivals(&metas);
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
        debug!(records = records.len(), "stremio new arrivals refreshed");
        Ok(records)
    }
}

/// `GET <cinemeta>/catalog/<kind>/year/genre=<year>.json` — one bucket of the
/// "New" catalog, as the raw metas it answered with.
async fn new_catalog(kind: &str, year: i32) -> Result<Vec<Value>> {
    let url = format!("{CINEMETA_URL}/catalog/{kind}/year/genre={year}.json");
    let response = reqwest::Client::new()
        .get(&url)
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

    // The addon protocol answers `{ metas: [...] }` for a catalog resource.
    let metas = parsed
        .get("metas")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("{url} answered without a metas list"))?;
    Ok(metas.clone())
}

/// The arrivals worth a banner, newest release first.
///
/// The buckets are merged rather than trusted one by one: one live answer put a
/// series inside the `movie` bucket, so what a meta says it is decides whether it
/// belongs, and the same title arriving through both buckets lands once. The
/// order is the metas' own `released` date, falling back to the addon's order —
/// a stable sort keeps that recency weighting for the many metas the field is
/// absent on rather than inventing an order for them.
fn newest_arrivals(metas: &[Value]) -> Vec<CatalogRecord> {
    let mut seen = std::collections::HashSet::new();
    let mut arrivals: Vec<&Value> = metas
        .iter()
        .filter(|meta| {
            let Some(id) = meta_id(meta) else {
                return false;
            };
            // Both a place in the banner and a launch need this: the bridge
            // builds the deep link from an IMDb id and refuses anything else,
            // so a meta carrying something else is not an arrival at all.
            is_title_id(id) && seen.insert(id)
        })
        .filter(|meta| matches!(meta_type(meta), "movie" | "series"))
        .collect();
    // Newest release first, with the undated metas last: `None` sorts below any
    // `Some`, so `Reverse` on the Option's own ordering puts a date ahead of no
    // date. A stable sort then keeps the addon's recency weighting for the metas
    // the feed never dated.
    arrivals.sort_by_key(|meta| std::cmp::Reverse(released_at(meta)));
    arrivals.truncate(NEW_ARRIVALS_LIMIT);
    arrivals.into_iter().filter_map(arrival_record).collect()
}

/// One meta as a catalog record: the id the launch route resolves, the art the
/// banner draws with, and the title's own type so a series opens as a series.
fn arrival_record(meta: &Value) -> Option<CatalogRecord> {
    let id = meta_id(meta)?;
    let kind = meta_type(meta);
    let metadata = serde_json::json!({
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
    use super::{arrival_record, meta_id, newest_arrivals, released_at};
    use serde_json::{Value, json};

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

    #[test]
    fn the_banner_holds_the_ten_most_recently_released_arrivals() {
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

        let records = newest_arrivals(&metas);

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

        let records = newest_arrivals(&metas);

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

        let records = newest_arrivals(&metas);

        assert_eq!(records.len(), 1, "arrivals: {records:?}");
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

        let records = newest_arrivals(&metas);

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

        let records = newest_arrivals(&metas);

        let titles: Vec<&str> = records.iter().map(|record| record.title.as_str()).collect();
        assert_eq!(titles, vec!["Dated", "Undated"], "arrivals: {titles:?}");
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
