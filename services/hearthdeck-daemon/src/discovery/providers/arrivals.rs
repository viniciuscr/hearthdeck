//! New arrivals: the newest titles that can be watched right now.
//!
//! The banner used to read Cinemeta's "New" catalog, and that catalog is a
//! release calendar: it lists a film the day it opens in cinemas, and no field in
//! the meta says whether it can be played yet. A banner built from it is mostly
//! slides nobody can watch. TMDB is asked instead, and it is asked the question
//! the banner actually means — which titles got a *home* release in this market
//! recently — so every slide is something somebody can put on tonight.
//!
//! A refresh is three kinds of call: `/discover` for candidate ids, one detail per
//! candidate (the availability, the IMDb id a launch needs and the wide art all
//! arrive in it), and the bridge for the desktop entry a slide opens. The market
//! is derived from the host's locale — see `crate::tmdb`.
//!
//! `SOURCE_ID` keeps the `stremio-arrivals` of the provider this replaces: the id
//! is what the catalog rows and the `stremio:new-arrivals` collection rule are
//! keyed by, so renaming it would orphan both for no gain a client can see. A
//! slide still launches Stremio, which is what the launch target below is for.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, Utc};
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::{
    catalog::CatalogRecord, discovery::DiscoveryProvider,
    discovery::providers::stremio::desktop_entry_for_stremio, tmdb::TmdbRepository,
};

/// The catalog source this provider owns.
pub use hearthdeck_protocol::sources::STREMIO_ARRIVALS as SOURCE_ID;

/// TMDB's versioned API root.
const TMDB_URL: &str = "https://api.themoviedb.org/3";

/// Where TMDB's artwork is served from.
const IMAGE_URL: &str = "https://image.tmdb.org/t/p";

/// TMDB's own release-type numbering: what a home release is.
const DIGITAL_RELEASE: u64 = 4;
const PHYSICAL_RELEASE: u64 = 5;

/// How many arrivals a banner rotates through. Ten is what the user asked for,
/// and a rotation of more than ten on a television is a playlist, not a banner.
const NEW_ARRIVALS_LIMIT: usize = 10;

/// How recent a home release has to be to count as an arrival.
///
/// This is the definition of "new" rather than a guess about availability:
/// anything inside the window has already been released at home, which is the
/// whole point of the change. Sixty days keeps a banner full without turning it
/// into a list of last year's films.
const AVAILABILITY_WINDOW_DAYS: u64 = 60;

/// How many candidates each type contributes before the availability check.
///
/// One page of a date-sorted discover is 20 titles and the check is what decides,
/// so a deeper page would only cost requests. What survives the window is ranked
/// by popularity, so a page this wide fills ten slides.
const CANDIDATES_PER_TYPE: usize = 20;

/// How often the release lists are re-read. Release data moves by the day, so a
/// refresh every few hours is current for a kiosk without polling a public API
/// like a live view — and each refresh costs one list call plus one per candidate.
const REFRESH_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

pub struct NewArrivalsProvider {
    /// The stored TMDB credential. Without one the rail publishes nothing, which
    /// is the honest answer: there is no keyless way to ask what is watchable.
    credentials: TmdbRepository,
    /// Where the host answers questions about installed applications: a banner
    /// slide launches the Stremio desktop entry, and only the bridge knows which
    /// entry that is.
    bridge_socket_path: PathBuf,
    client: reqwest::Client,
}

impl NewArrivalsProvider {
    pub fn new(credentials: TmdbRepository, bridge_socket_path: PathBuf) -> Self {
        Self {
            credentials,
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
        let Some(credentials) = self.credentials.credentials().await? else {
            debug!("no TMDB credential is stored; publishing no arrivals");
            return Ok(Vec::new());
        };
        // Both are facts about the host and the clock rather than about the feed,
        // so they are read once here and kept out of the mapping.
        let market = Market {
            region: crate::tmdb::region(),
            language: crate::tmdb::language(),
        };
        let today = Utc::now().date_naive();

        let mut available = Vec::new();
        let mut failures = 0;
        match self.films(&credentials.api_key, &market, today).await {
            Ok(mut films) => available.append(&mut films),
            Err(error) => {
                failures += 1;
                warn!(%error, "the TMDB film releases could not be read");
            }
        }
        match self.series(&credentials.api_key, &market, today).await {
            Ok(mut series) => available.append(&mut series),
            Err(error) => {
                failures += 1;
                warn!(%error, "the TMDB series releases could not be read");
            }
        }
        if failures == 2 {
            // Nothing could be read at all. Refusing rather than publishing an
            // empty snapshot keeps the last good arrivals on the banner instead
            // of wiping it because one API call failed.
            bail!("no TMDB release list could be read");
        }

        let mut records = newest_arrivals(available);
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
            region = %market.region,
            "new arrivals refreshed"
        );
        Ok(records)
    }
}

impl NewArrivalsProvider {
    /// The films that have a home release in this market.
    async fn films(
        &self,
        api_key: &str,
        market: &Market,
        today: NaiveDate,
    ) -> Result<Vec<Available>> {
        let page = self
            .get_json(
                self.request(
                    "/discover/movie",
                    &[
                        ("sort_by", "primary_release_date.desc".to_owned()),
                        ("include_adult", "false".to_owned()),
                        // A film has left the cinemas when it has either of these.
                        ("with_release_type", "4|5".to_owned()),
                        ("region", market.region.clone()),
                        ("language", market.language.clone()),
                        // Bounded to the window so the list is only what could be
                        // new; the per-title check below is what decides.
                        ("release_date.gte", window_start(today)),
                        ("release_date.lte", today.to_string()),
                    ],
                    api_key,
                ),
                "the film release list",
            )
            .await?;

        let mut available = Vec::new();
        for id in candidate_ids(&page) {
            // One title's detail failing is not the list failing: TMDB answers a
            // 404 for a title it has since removed, and the other nineteen
            // candidates are still worth a slide.
            match self.detail("movie", id, api_key, market).await {
                Ok(detail) => available.extend(available_film(&detail, market, today)),
                Err(error) => debug!(id, %error, "a TMDB film detail could not be read"),
            }
        }
        Ok(available)
    }

    /// The series that have started airing in this window.
    ///
    /// A series has no cinema window to clear, so its own air date is the
    /// availability: once the first episode is out there is something to watch.
    async fn series(
        &self,
        api_key: &str,
        market: &Market,
        today: NaiveDate,
    ) -> Result<Vec<Available>> {
        let page = self
            .get_json(
                self.request(
                    "/discover/tv",
                    &[
                        ("sort_by", "first_air_date.desc".to_owned()),
                        ("include_adult", "false".to_owned()),
                        ("language", market.language.clone()),
                        ("first_air_date.gte", window_start(today)),
                        ("first_air_date.lte", today.to_string()),
                    ],
                    api_key,
                ),
                "the series release list",
            )
            .await?;

        let mut available = Vec::new();
        for id in candidate_ids(&page) {
            match self.detail("tv", id, api_key, market).await {
                Ok(detail) => available.extend(available_series(&detail, market, today)),
                Err(error) => debug!(id, %error, "a TMDB series detail could not be read"),
            }
        }
        Ok(available)
    }

    /// One title with its availability, its IMDb id and its wide art.
    async fn detail(&self, kind: &str, id: u64, api_key: &str, market: &Market) -> Result<Value> {
        let preferred = market
            .language
            .split('-')
            .next()
            .unwrap_or(market.language.as_str())
            .to_owned();
        // `release_dates` is a film-side resource: a series is release-dated by its
        // own air dates, which the detail already carries.
        let append = if kind == "movie" {
            "external_ids,images,release_dates"
        } else {
            "external_ids,images"
        };
        self.get_json(
            self.request(
                &format!("/{kind}/{id}"),
                &[
                    ("language", market.language.clone()),
                    ("append_to_response", append.to_owned()),
                    // The logo the slide draws with, in this host's language first,
                    // English second, and textless art as the last resort.
                    ("include_image_language", format!("{preferred},en,null")),
                ],
                api_key,
            ),
            "a title's detail",
        )
        .await
    }

    /// A request carrying the credential, in whichever form it is one.
    ///
    /// TMDB accepts two. A v3 key is a 32-character string read from the
    /// `api_key` parameter; a v4 read token is a JWT — so it carries dots — and
    /// belongs in a bearer header. Sending one as the other is a 401 that says
    /// nothing about what went wrong.
    fn request(
        &self,
        path: &str,
        params: &[(&str, String)],
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        let request = self.client.get(format!("{TMDB_URL}{path}")).query(params);
        if api_key.contains('.') {
            request.bearer_auth(api_key)
        } else {
            request.query(&[("api_key", api_key)])
        }
    }

    async fn get_json(&self, request: reqwest::RequestBuilder, what: &str) -> Result<Value> {
        let response = request
            .timeout(REQUEST_TIMEOUT)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            // `without_url` is load-bearing: a v3 key rides in the query string,
            // so a `reqwest::Error`'s own message contains the credential, and
            // this text reaches the provider's health report and the log.
            .map_err(|error| anyhow!("could not reach TMDB for {what}: {}", error.without_url()))?;
        let status = response.status();
        let text = response.text().await.map_err(|error| {
            anyhow!(
                "could not read TMDB's answer for {what}: {}",
                error.without_url()
            )
        })?;
        let parsed: Value = serde_json::from_str(&text).map_err(|_| {
            anyhow!("TMDB answered HTTP {status} for {what} with a body that is not JSON")
        })?;
        if !status.is_success() {
            let reason = parsed
                .get("status_message")
                .and_then(Value::as_str)
                .unwrap_or("no status message");
            bail!("TMDB refused {what} with HTTP {status}: {reason}");
        }
        Ok(parsed)
    }
}

/// The market the feed is asked about, derived from the host's locale.
struct Market {
    region: String,
    language: String,
}

/// The oldest date the list calls ask for.
fn window_start(today: NaiveDate) -> String {
    (today - chrono::Days::new(AVAILABILITY_WINDOW_DAYS))
        .format("%Y-%m-%d")
        .to_string()
}

/// The ids a discover page offers, in the order it offered them.
fn candidate_ids(page: &Value) -> Vec<u64> {
    page.get("results")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|candidate| candidate.get("id").and_then(Value::as_u64))
        .take(CANDIDATES_PER_TYPE)
        .collect()
}

/// One title TMDB dates as watchable, with everything a slide needs.
struct Available {
    kind: &'static str,
    imdb_id: String,
    title: String,
    /// The day it became watchable at home.
    since: NaiveDate,
    /// How it became watchable, for the record's own metadata.
    how: &'static str,
    /// The market that date was read for, which is not always the host's.
    market: String,
    /// The title's own release year, for the slide's caption.
    year: Option<i32>,
    popularity: f64,
    poster: Option<String>,
    background: Option<String>,
    logo: Option<String>,
    description: Option<String>,
    rating: Option<f64>,
}

/// A film is watchable once TMDB has a home release for it in the past.
///
/// The host's market is the answer whenever TMDB has a release row for it, home
/// release or not: a film with only cinema dates there really is still in the
/// cinemas. The fallback exists for the other case — no row at all, which is a gap
/// in TMDB's release data rather than a statement about the film, and common in
/// smaller markets — because a banner that empties itself over a data gap serves
/// nobody. Whatever market the date came from is recorded on the slide.
fn available_film(detail: &Value, market: &Market, today: NaiveDate) -> Option<Available> {
    let (since, how, country) = match country_releases(detail, &market.region) {
        // TMDB tracks this market, so this is the answer: a film with no home
        // release here is a film in cinemas here, and nothing else is consulted.
        Some(releases) => {
            let (since, how) = latest_home_release(releases, today)?;
            (since, how, market.region.clone())
        }
        // No row for this market at all, which is a gap in TMDB's release data
        // rather than a statement about the film.
        None => home_release_elsewhere(detail, &market.region, today)?,
    };
    within_window(since, today)?;
    assembled(detail, "movie", since, how, &country, market)
}

/// A series is watchable once its first episode has aired.
fn available_series(detail: &Value, market: &Market, today: NaiveDate) -> Option<Available> {
    let since = day(detail.get("first_air_date"))?;
    within_window(since, today)?;
    assembled(detail, "series", since, "aired", &market.region, market)
}

/// One country's release list, when TMDB has a row for it.
fn country_releases<'a>(detail: &'a Value, country: &str) -> Option<&'a Vec<Value>> {
    detail
        .get("release_dates")?
        .get("results")?
        .as_array()?
        .iter()
        .find(|entry| {
            entry
                .get("iso_3166_1")
                .and_then(Value::as_str)
                .is_some_and(|iso| iso.eq_ignore_ascii_case(country))
        })?
        .get("release_dates")?
        .as_array()
}

/// The latest digital or physical release that has already happened, if any.
///
/// The latest rather than the first: a physical edition that follows the digital
/// one is the arrival a person would recognise, and both are past-dated by the
/// time they are returned here. A future date is not an arrival yet, and a
/// theatrical one — TMDB's type 2 and 3 — is exactly what this feature exists to
/// exclude.
fn latest_home_release(releases: &[Value], today: NaiveDate) -> Option<(NaiveDate, &'static str)> {
    releases
        .iter()
        .filter_map(|release| {
            let how = match release.get("type").and_then(Value::as_u64)? {
                DIGITAL_RELEASE => "digital",
                PHYSICAL_RELEASE => "physical",
                _ => return None,
            };
            let released = day(release.get("release_date"))?;
            (released <= today).then_some((released, how))
        })
        .max_by_key(|(released, _)| *released)
}

/// The latest home release from any market other than `market`.
///
/// Only consulted when TMDB has no row for the host's market, and reported with
/// the market it was found in so the record says where it is watchable without a
/// second lookup.
fn home_release_elsewhere(
    detail: &Value,
    market: &str,
    today: NaiveDate,
) -> Option<(NaiveDate, &'static str, String)> {
    detail
        .get("release_dates")?
        .get("results")?
        .as_array()?
        .iter()
        .filter_map(|entry| {
            let iso = entry.get("iso_3166_1").and_then(Value::as_str)?;
            if iso.eq_ignore_ascii_case(market) {
                return None;
            }
            let (since, how) = latest_home_release(country_releases(detail, iso)?, today)?;
            Some((since, how, iso.to_owned()))
        })
        .max_by_key(|(since, _, _)| *since)
}

/// Whether a release date is inside the window that counts as new.
///
/// A date in the future is not an arrival at all, and one older than the window
/// has stopped being one; both are `None` rather than an error, because a
/// candidate list is expected to be mostly neither.
fn within_window(since: NaiveDate, today: NaiveDate) -> Option<()> {
    let age = today.signed_duration_since(since).num_days();
    (0..=i64::try_from(AVAILABILITY_WINDOW_DAYS).ok()?)
        .contains(&age)
        .then_some(())
}

/// One meta's worth of record-building, shared by films and series.
fn assembled(
    detail: &Value,
    kind: &'static str,
    since: NaiveDate,
    how: &'static str,
    country: &str,
    market: &Market,
) -> Option<Available> {
    Some(Available {
        kind,
        imdb_id: imdb_id(detail)?.to_owned(),
        title: title(detail, kind)?,
        since,
        how,
        market: country.to_owned(),
        year: original_year(detail, kind),
        popularity: number(detail, "popularity").unwrap_or_default(),
        poster: image(detail.get("poster_path"), "w342"),
        background: image(detail.get("backdrop_path"), "w1280"),
        logo: logo(detail, &market.language),
        description: text(detail, "overview"),
        rating: number(detail, "vote_average").filter(|rating| *rating > 0.0),
    })
}

/// The arrivals worth a banner, best-known first.
///
/// The window has already answered "is this new", so the rank is popularity: inside
/// sixty days of home releases, the titles somebody has heard of are the ones a
/// hero banner should rotate through. The collection re-sorts them by release date
/// on the way out (`list_source` orders on `updated_at`), so this order decides
/// only who makes the ten.
fn newest_arrivals(mut available: Vec<Available>) -> Vec<CatalogRecord> {
    available.sort_by(|left, right| right.popularity.total_cmp(&left.popularity));
    available.truncate(NEW_ARRIVALS_LIMIT);
    available.into_iter().map(arrival_record).collect()
}

/// One title as a catalog record: the IMDb id the launch resolves, the art the
/// banner draws with, and the title's own type so a series opens as a series.
fn arrival_record(available: Available) -> CatalogRecord {
    let since = available.since.format("%Y-%m-%d").to_string();
    let record_id = format!("{SOURCE_ID}:{}", available.imdb_id);
    let metadata = json!({
        "source": SOURCE_ID,
        "type": available.kind,
        // The bridge builds the `stremio://` detail link from this. An arrival
        // has no watch state, so it is the bare IMDb id and the type above is
        // what makes a series a series.
        "video_id": available.imdb_id,
        // The hero banner's own artwork: a backdrop is the wide art a 30%-tall
        // slide needs and a poster cannot carry it.
        "background": available.background,
        "logo": available.logo,
        "description": available.description,
        "year": available.year,
        // When it became watchable at home — the date this banner is about. The
        // cinema date is what the feed this replaces was keyed by, and it is
        // precisely what made a slide unplayable.
        "released": &since,
        "availability": {
            "since": &since,
            "how": available.how,
            "market": available.market,
        },
        // TMDB's own score, named for what it is: it is not an IMDb rating and
        // must not be shown as one.
        "tmdb_rating": available.rating,
    });
    CatalogRecord {
        id: record_id,
        title: available.title,
        kind: available.kind.to_owned(),
        // Filled in by the provider once per refresh from the desktop entry the
        // bridge found; the mapping has no way to know it.
        launch_id: None,
        icon: available.poster,
        metadata,
        // The availability date, so `list_source`'s newest-first order is the
        // banner's own and a title released at home today outranks last month's.
        updated_at: format!("{since}T00:00:00Z"),
    }
}

/// Whether an id is an IMDb title id: `tt` followed by digits. The same rule the
/// bridge validates a video id by, kept here so a title that could never be
/// launched is dropped at the door instead of drawn on the banner.
fn is_title_id(id: &str) -> bool {
    id.strip_prefix("tt").is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn imdb_id(detail: &Value) -> Option<&str> {
    let id = detail.get("external_ids")?.get("imdb_id")?.as_str()?.trim();
    is_title_id(id).then_some(id)
}

fn title(detail: &Value, kind: &str) -> Option<String> {
    let key = if kind == "series" { "name" } else { "title" };
    text(detail, key)
}

fn text(detail: &Value, key: &str) -> Option<String> {
    detail
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn number(detail: &Value, key: &str) -> Option<f64> {
    detail.get(key).and_then(Value::as_f64)
}

/// The title's own release year — the cinema or premiere year, not the year it
/// reached a living room.
fn original_year(detail: &Value, kind: &str) -> Option<i32> {
    let key = if kind == "series" {
        "first_air_date"
    } else {
        "release_date"
    };
    day(detail.get(key)).map(|date| date.year())
}

/// A TMDB date: `2026-10-03` on a detail, `2026-10-03T00:00:00.000Z` inside a
/// release list, and an empty string for the ones TMDB never dated.
fn day(value: Option<&Value>) -> Option<NaiveDate> {
    let value = value?.as_str()?.trim();
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(date);
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|instant| instant.date_naive())
}

fn image(path: Option<&Value>, size: &str) -> Option<String> {
    let path = path?.as_str()?.trim();
    (!path.is_empty()).then(|| format!("{IMAGE_URL}/{size}{path}"))
}

/// The logo a slide draws with, in the host's language when TMDB has one.
///
/// Ranked rather than filtered: a title with no Portuguese logo still deserves a
/// slide, and a textless one is a better answer than a poster's dead space.
fn logo(detail: &Value, language: &str) -> Option<String> {
    let preferred = language.split('-').next().unwrap_or(language);
    detail
        .get("images")?
        .get("logos")?
        .as_array()?
        .iter()
        .filter_map(|logo| {
            let path = logo
                .get("file_path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|path| !path.is_empty())?;
            let rank = match logo.get("iso_639_1").and_then(Value::as_str) {
                Some(tag) if tag.eq_ignore_ascii_case(preferred) => 0,
                None => 1,
                Some("en") => 2,
                Some(_) => 3,
            };
            Some((rank, path.to_owned()))
        })
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, path)| format!("{IMAGE_URL}/w500{path}"))
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use serde_json::{Value, json};

    use super::{
        Market, arrival_record, available_film, available_series, logo, newest_arrivals,
        window_start,
    };

    fn market() -> Market {
        Market {
            region: "BR".to_owned(),
            language: "pt-BR".to_owned(),
        }
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 6).expect("a real date")
    }

    /// A film detail as `/movie/{id}?append_to_response=release_dates,external_ids,images`
    /// answers: the availability, the IMDb id a launch needs and the art in one
    /// object.
    fn film(imdb_id: &str, popularity: f64, releases: Value) -> Value {
        json!({
            "id": 550,
            "title": format!("Film {imdb_id}"),
            "release_date": "2026-09-18",
            "overview": "  About it  ",
            "popularity": popularity,
            "vote_average": 7.4,
            "poster_path": "/poster.jpg",
            "backdrop_path": "/backdrop.jpg",
            "external_ids": { "imdb_id": imdb_id },
            "images": { "logos": [
                { "iso_639_1": "en", "file_path": "/logo-en.png" },
                { "iso_639_1": "pt", "file_path": "/logo-pt.png" }
            ] },
            "release_dates": { "results": releases },
        })
    }

    fn series(imdb_id: &str, first_air_date: &str) -> Value {
        json!({
            "id": 1399,
            "name": format!("Series {imdb_id}"),
            "first_air_date": first_air_date,
            "overview": "About it",
            "popularity": 12.0,
            "vote_average": 8.1,
            "poster_path": "/series-poster.jpg",
            "backdrop_path": "/series-backdrop.jpg",
            "external_ids": { "imdb_id": imdb_id },
            "images": { "logos": [] },
        })
    }

    fn country(iso: &str, releases: Vec<Value>) -> Value {
        json!({ "iso_3166_1": iso, "release_dates": releases })
    }

    /// One TMDB release entry: `type` is its own numbering, where 3 is theatrical,
    /// 4 digital and 5 physical.
    fn release(kind: u64, date: &str) -> Value {
        json!({ "type": kind, "release_date": format!("{date}T00:00:00.000Z") })
    }

    #[test]
    fn a_film_still_in_cinemas_is_not_an_arrival() {
        let detail = film(
            "tt0000001",
            10.0,
            json!([country("BR", vec![release(3, "2026-10-16")])]),
        );

        assert!(
            available_film(&detail, &market(), today()).is_none(),
            "a theatrical date is the cinema run this feature exists to exclude"
        );
    }

    #[test]
    fn a_film_with_a_home_release_here_is_an_arrival_from_that_date() {
        let detail = film(
            "tt0000001",
            10.0,
            json!([country(
                "BR",
                vec![release(3, "2026-08-01"), release(4, "2026-10-27")]
            )]),
        );

        let available = available_film(&detail, &market(), today()).expect("released at home");

        assert_eq!(
            available.since,
            NaiveDate::from_ymd_opt(2026, 10, 27).unwrap()
        );
        assert_eq!(available.how, "digital");
        assert_eq!(available.market, "BR");
    }

    #[test]
    fn the_latest_home_release_wins_when_a_physical_edition_follows_the_digital_one() {
        let detail = film(
            "tt0000001",
            10.0,
            json!([country(
                "BR",
                vec![release(4, "2026-10-01"), release(5, "2026-10-29")]
            )]),
        );

        let available = available_film(&detail, &market(), today()).expect("released at home");

        assert_eq!(
            available.since,
            NaiveDate::from_ymd_opt(2026, 10, 29).unwrap()
        );
        assert_eq!(available.how, "physical");
    }

    #[test]
    fn a_home_release_that_has_not_happened_yet_is_not_an_arrival() {
        let detail = film(
            "tt0000001",
            10.0,
            json!([country("BR", vec![release(4, "2026-12-01")])]),
        );

        assert!(available_film(&detail, &market(), today()).is_none());
    }

    #[test]
    fn a_market_with_no_row_at_all_falls_back_to_the_one_that_has_a_home_release() {
        let detail = film(
            "tt0000001",
            10.0,
            json!([country("US", vec![release(4, "2026-11-01")])]),
        );

        let available = available_film(&detail, &market(), today()).expect("released at home");

        assert_eq!(
            available.market, "US",
            "the slide says where it is watchable"
        );
    }

    #[test]
    fn an_old_home_release_is_no_longer_an_arrival() {
        let detail = film(
            "tt0000001",
            10.0,
            json!([country("BR", vec![release(4, "2026-01-10")])]),
        );

        assert!(available_film(&detail, &market(), today()).is_none());
    }

    #[test]
    fn a_film_with_no_imdb_id_cannot_be_launched_so_is_not_an_arrival() {
        let mut detail = film(
            "tt0000001",
            10.0,
            json!([country("BR", vec![release(4, "2026-11-01")])]),
        );
        detail["external_ids"] = json!({ "imdb_id": "" });

        assert!(available_film(&detail, &market(), today()).is_none());
    }

    #[test]
    fn a_series_is_an_arrival_once_its_first_episode_has_aired() {
        let available = available_series(&series("tt0000002", "2026-11-01"), &market(), today())
            .expect("premiered this week");
        assert_eq!(available.how, "aired");
        assert!(available_series(&series("tt0000003", "2026-12-24"), &market(), today()).is_none());
    }

    #[test]
    fn the_banner_keeps_the_ten_best_known_of_what_is_watchable() {
        let details: Vec<Value> = (1..=12)
            .map(|index| {
                film(
                    &format!("tt{index:07}"),
                    f64::from(index),
                    json!([country("BR", vec![release(4, "2026-11-01")])]),
                )
            })
            .collect();
        let available: Vec<_> = details
            .iter()
            .filter_map(|detail| available_film(detail, &market(), today()))
            .collect();

        let records = newest_arrivals(available);

        let titles: Vec<&str> = records.iter().map(|record| record.title.as_str()).collect();
        assert_eq!(records.len(), 10, "arrivals: {titles:?}");
        assert_eq!(titles[0], "Film tt0000012");
        assert_eq!(titles[9], "Film tt0000003");
    }

    #[test]
    fn an_arrival_record_carries_the_wide_art_and_the_home_release_date() {
        let detail = film(
            "tt0000001",
            10.0,
            json!([country("BR", vec![release(4, "2026-10-27")])]),
        );
        let available = available_film(&detail, &market(), today()).expect("released at home");

        let record = arrival_record(available);

        assert_eq!(record.id, "stremio-arrivals:tt0000001");
        assert_eq!(record.kind, "movie");
        assert_eq!(record.updated_at, "2026-10-27T00:00:00Z");
        assert_eq!(
            record.icon.as_deref(),
            Some("https://image.tmdb.org/t/p/w342/poster.jpg")
        );
        let discovery = &record.metadata;
        assert_eq!(discovery["video_id"], "tt0000001");
        assert_eq!(discovery["type"], "movie");
        assert_eq!(
            discovery["background"],
            "https://image.tmdb.org/t/p/w1280/backdrop.jpg"
        );
        assert_eq!(
            discovery["logo"],
            "https://image.tmdb.org/t/p/w500/logo-pt.png"
        );
        assert_eq!(discovery["description"], "About it");
        assert_eq!(discovery["released"], "2026-10-27");
        assert_eq!(discovery["availability"]["how"], "digital");
        assert_eq!(discovery["availability"]["market"], "BR");
    }

    #[test]
    fn the_logo_prefers_the_hosts_language_then_a_textless_one_then_english() {
        let images = |logos: Value| json!({ "images": { "logos": logos } });

        assert_eq!(
            logo(
                &images(json!([{ "iso_639_1": "en", "file_path": "/en.png" }])),
                "pt-BR"
            )
            .as_deref(),
            Some("https://image.tmdb.org/t/p/w500/en.png")
        );
        assert_eq!(
            logo(
                &images(json!([
                    { "iso_639_1": "en", "file_path": "/en.png" },
                    { "iso_639_1": null, "file_path": "/textless.png" }
                ])),
                "pt-BR"
            )
            .as_deref(),
            Some("https://image.tmdb.org/t/p/w500/textless.png")
        );
        assert_eq!(logo(&images(json!([])), "pt-BR"), None);
    }

    /// A daemon with no credential has nothing to ask TMDB with, and an empty
    /// snapshot is the honest answer — publishing the old cinema calendar instead
    /// is what this provider exists to stop.
    #[tokio::test]
    async fn no_credential_publishes_no_arrivals() {
        let directory = tempfile::tempdir().unwrap();
        let database = crate::database::Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let provider = super::NewArrivalsProvider::new(
            crate::tmdb::TmdbRepository::new(database.pool().clone()),
            directory.path().join("bridge.sock"),
        );

        let records = crate::discovery::DiscoveryProvider::discover(&provider)
            .await
            .expect("an unconfigured provider is not an error");

        assert!(records.is_empty(), "records: {records:?}");
    }

    /// The two credential forms TMDB accepts are not interchangeable: a v3 key in
    /// a bearer header is a 401, and a v4 token in the query is one too.
    #[tokio::test]
    async fn a_v3_key_rides_in_the_query_and_a_v4_token_in_the_header() {
        let directory = tempfile::tempdir().unwrap();
        let database = crate::database::Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let provider = super::NewArrivalsProvider::new(
            crate::tmdb::TmdbRepository::new(database.pool().clone()),
            directory.path().join("bridge.sock"),
        );

        let v3 = provider
            .request("/discover/movie", &[], "0123456789abcdef0123456789abcdef")
            .build()
            .unwrap();
        assert_eq!(
            v3.url().query(),
            Some("api_key=0123456789abcdef0123456789abcdef")
        );
        assert!(v3.headers().get("authorization").is_none());

        let v4 = provider
            .request("/discover/movie", &[], "header.payload.signature")
            .build()
            .unwrap();
        assert!(v4.url().query().is_none(), "url: {}", v4.url());
        assert_eq!(
            v4.headers().get("authorization").unwrap(),
            "Bearer header.payload.signature"
        );
    }

    #[test]
    fn the_list_calls_ask_for_the_same_window_the_check_applies() {
        assert_eq!(window_start(today()), "2026-09-07");
    }
}
