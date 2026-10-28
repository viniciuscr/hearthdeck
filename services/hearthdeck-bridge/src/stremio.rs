//! Opening a title's own page in Stremio.
//!
//! A Stremio launch is a desktop-application launch underneath - the entry's
//! own command with a `stremio://` URI appended - so it reuses `desktop_apps`
//! and `systemd` rather than carrying its own launch machinery.

use std::ffi::OsString;

use anyhow::{Result, bail};
use hearthdeck_protocol::StremioMediaType;

use crate::desktop_apps::{DESKTOP_APPS_SOURCE, command_for_desktop_entry, desktop_entry_path};
use crate::systemd::{LaunchedApplication, launch_with_systemd};

/// Opens a title's own page in Stremio.
///
/// The entry comes from the caller rather than being searched for here: the caller
/// resolved it against this host's application list already, and a second lookup with
/// its own matching rules is how a working launch turned into "no desktop entry
/// declares the stremio scheme". The id is still validated exactly as a plain
/// application launch validates it — a bare file name inside the desktop-entry
/// directories, never a path — so nothing new is trusted.
///
/// The URI is appended to the entry's own command rather than going through `xdg-open`:
/// that finds no desktop environment to delegate to on this session and gives up, which
/// is the same reason Heroic is exec'd directly.
pub(crate) async fn launch_stremio_title(
    application_id: &str,
    video_id: &str,
    media_type: StremioMediaType,
    session_id: &str,
) -> Result<LaunchedApplication> {
    let uri = stremio_detail_uri(video_id, media_type)?;
    let path = desktop_entry_path(DESKTOP_APPS_SOURCE, application_id).await?;
    let launch = command_for_desktop_entry(&path).await?;
    // The entry's command, with the URI where its `%u` field code was. Stremio reads it
    // from its own argv and forwards it to the running instance if there is one, so this
    // works whether the app is already open or not.
    let mut command = launch.command;
    command.push(OsString::from(uri));
    launch_with_systemd(
        &format!("hearthdeck-app-{session_id}.service"),
        command,
        launch.working_directory,
    )
    .await
}

/// The `stremio://` URI for a video id, built from that id and what the title is.
///
/// `tt0373074` is a film; `tt0149460:4:7` is season 4, episode 7 of a series. The shape
/// is the one Stremio's own web UI parses, and it was verified against the installed
/// app: both forms open the title's page rather than the home screen. A bare id names a
/// film or a series only as `media_type` says — the same number is a different page for
/// each — so that form carries the type into the route instead of guessing it.
fn stremio_detail_uri(video_id: &str, media_type: StremioMediaType) -> Result<String> {
    let mut parts = video_id.split(':');
    let base = parts.next().unwrap_or_default();
    if !is_imdb_id(base) {
        bail!("a video id must be an IMDb id")
    }
    let season = parts.next();
    let episode = parts.next();
    if parts.next().is_some() {
        bail!("a video id has at most a season and an episode")
    }
    match (season, episode) {
        (None, None) => Ok(format!("stremio:///detail/{}/{base}", media_type.as_str())),
        (Some(season), Some(episode)) => {
            if !is_small_number(season) || !is_small_number(episode) {
                bail!("a season and an episode must be numbers")
            }
            Ok(format!(
                "stremio:///detail/series/{base}/{base}:{season}:{episode}"
            ))
        }
        // A season without an episode names no video, so it is refused rather than
        // guessed at.
        _ => bail!("a video id needs both a season and an episode"),
    }
}

/// Whether `value` is an IMDb id: `tt` and digits, nothing else.
fn is_imdb_id(value: &str) -> bool {
    value.strip_prefix("tt").is_some_and(|digits| {
        !digits.is_empty() && digits.len() <= 10 && digits.bytes().all(|b| b.is_ascii_digit())
    })
}

/// Whether `value` is a season or episode number: digits, and not absurdly large.
fn is_small_number(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value != "0"
}

#[cfg(test)]
mod tests {
    use super::stremio_detail_uri;
    use hearthdeck_protocol::StremioMediaType;

    #[test]
    fn a_film_opens_its_own_page_and_a_series_opens_its_episode() {
        assert_eq!(
            stremio_detail_uri("tt0373074", StremioMediaType::Movie).unwrap(),
            "stremio:///detail/movie/tt0373074"
        );
        assert_eq!(
            stremio_detail_uri("tt0149460:4:7", StremioMediaType::Series).unwrap(),
            "stremio:///detail/series/tt0149460/tt0149460:4:7"
        );
        // A season with a multi-digit episode, which Stremio itself produces.
        assert_eq!(
            stremio_detail_uri("tt2654620:2:10", StremioMediaType::Series).unwrap(),
            "stremio:///detail/series/tt2654620/tt2654620:2:10"
        );
    }

    /// A new arrival has no episode to name, and is a film or a series only as its
    /// record says: the same bare number is a different page for each, so the type
    /// travels into the route's own segment. An id that does carry an episode is a
    /// series whatever the type says — the episode half is the stronger claim.
    #[test]
    fn a_bare_id_opens_the_kind_of_title_the_record_says() {
        assert_eq!(
            stremio_detail_uri("tt0373074", StremioMediaType::Series).unwrap(),
            "stremio:///detail/series/tt0373074"
        );
        assert_eq!(
            stremio_detail_uri("tt0149460:4:7", StremioMediaType::Movie).unwrap(),
            "stremio:///detail/series/tt0149460/tt0149460:4:7"
        );
    }

    /// The id arrives from a catalog record, so nothing about it is trusted: it becomes
    /// the path of a URI the host will hand to an application.
    #[test]
    fn refuses_anything_that_is_not_a_video_id() {
        for refused in [
            "",
            "tt",
            "ttabc",
            "0149460",
            "tt0149460:4",
            "tt0149460:4:7:9",
            "tt0149460:0:7",
            "tt0149460::7",
            "../../etc/passwd",
            "tt0149460/../../x",
            "stremio:///detail/movie/tt0149460",
        ] {
            assert!(
                stremio_detail_uri(refused, StremioMediaType::Movie).is_err(),
                "{refused} should be refused"
            );
        }
    }
}
