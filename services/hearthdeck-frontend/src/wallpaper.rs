//! What cosmic-bg is showing as the desktop wallpaper, read from cosmic-config.
//!
//! The dashboard draws this as its page background: the front door onto the
//! library wears the same picture as the desktop behind it. cosmic-bg keeps its
//! settings in cosmic-config under [`CONFIG_ID`]: a shared `all` entry, a
//! `same-on-all` flag, and one `output.<name>` entry per display when displays
//! carry different wallpapers. This module mirrors only the fields that decide
//! *which image* to draw and how to fit it - everything else in an entry is
//! accepted and ignored on read - and resolves them the way cosmic-bg itself
//! falls back.
//!
//! Picking a per-output entry needs the Wayland name of the output the window
//! is on, which a windowed client cannot cheaply learn. Resolution therefore
//! mirrors cosmic-bg's own default (`same-on-all`, one wallpaper everywhere):
//! the shared `all` entry, unless the config carries exactly one display entry
//! and says the displays differ. On a single-display machine that is always
//! the entry cosmic-bg is drawing.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use cosmic::cosmic_config::{Config, ConfigGet};
use cosmic::iced::ContentFit;
use cosmic::widget::icon::{self, Handle};
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// The app id cosmic-bg stores its configuration under, so the watcher and the
/// reader open the same store.
pub const CONFIG_ID: &str = "com.system76.CosmicBackground";
/// The config version `cosmic-bg-config::context` opens.
pub const CONFIG_VERSION: u64 = 1;

/// Key of the entry shared by every display.
const ALL: &str = "all";
/// Key of the flag saying whether the displays share one wallpaper.
const SAME_ON_ALL: &str = "same-on-all";
/// Key listing the displays that carry an entry of their own.
const BACKGROUNDS: &str = "backgrounds";
/// Prefix of a per-display entry's key: `output.<output name>`.
const OUTPUT_PREFIX: &str = "output.";

/// Raster formats a wallpaper can arrive in - the ones the renderer decodes.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tiff", "tif"];

/// The desktop wallpaper, ready to draw: one image and how it fills the page.
#[derive(Clone, Debug)]
pub struct Wallpaper {
    pub handle: Handle,
    pub fit: ContentFit,
}

/// One `Entry` as cosmic-bg writes it. Only the source and the scaling mode
/// decide what this app draws; the rest of the entry (`filter_by_theme`,
/// `rotation_frequency`, the filter and sampling methods) is accepted and
/// ignored, which is why nothing here is `deny_unknown_fields`.
#[derive(Debug, Deserialize, Clone, PartialEq)]
struct Entry {
    #[serde(default, deserialize_with = "bare_source")]
    source: Option<Source>,
    #[serde(default)]
    scaling_mode: ScalingMode,
}

/// Reads the source field the way cosmic-bg writes it: unadorned -
/// `source: Path("...")`, not `source: Some(Path("..."))` - as `None` only when
/// the entry carries no source at all. RON insists on the `Some` wrapper for a
/// plain option field, so the field reads itself bare instead.
fn bare_source<'de, D>(deserializer: D) -> Result<Option<Source>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Source::deserialize(deserializer)?))
}

/// The configured image source: a file, a folder to rotate through, or a
/// colour. The colour payload is ignored wholesale - this app draws nothing for
/// a colour background, whatever it holds.
#[derive(Debug, Deserialize, Clone, PartialEq)]
enum Source {
    Path(PathBuf),
    Color(serde::de::IgnoredAny),
}

/// How cosmic-bg scales the image to the output.
#[derive(Debug, Deserialize, Clone, Copy, Default, PartialEq)]
enum ScalingMode {
    /// Fit the image and fill the rest with a colour.
    Fit([f32; 3]),
    /// Stretch, ignoring the aspect ratio.
    Stretch,
    /// Zoom until the area is covered.
    #[default]
    Zoom,
}

/// What one entry resolves to before it becomes a [`Wallpaper`].
#[derive(Debug, Clone, PartialEq)]
struct Plan {
    path: PathBuf,
    fit: ContentFit,
}

/// Reads what cosmic-bg is currently showing and resolves it to one image.
///
/// Returns `None` when cosmic-bg has no image configured - a colour background
/// counts as none here - and the dashboard then keeps the theme's own
/// background.
#[must_use]
pub fn load() -> Option<Wallpaper> {
    let config = Config::new(CONFIG_ID, CONFIG_VERSION).ok()?;
    let same_on_all: bool = get(&config, SAME_ON_ALL).unwrap_or(true);
    let all: Option<Entry> = get(&config, ALL);
    let outputs: Vec<Entry> = get::<Vec<String>>(&config, BACKGROUNDS)
        .unwrap_or_default()
        .iter()
        .filter_map(|output| get(&config, &format!("{OUTPUT_PREFIX}{output}")))
        .collect();
    let plan = resolve(same_on_all, all.as_ref(), &outputs)?;
    let path = pick_path(&plan.path)?;
    Some(Wallpaper {
        handle: icon::from_path(path),
        fit: plan.fit,
    })
}

/// One key of cosmic-bg's config, or `None` when it is absent or unreadable.
///
/// A key not being written yet is normal - `same-on-all` and the entries are
/// only created when Settings first saves a wallpaper - so absence is a
/// legitimate answer and only a parse failure is worth a log line.
fn get<T: DeserializeOwned>(config: &Config, key: &str) -> Option<T> {
    match config.get(key) {
        Ok(value) => Some(value),
        Err(error) => {
            log::debug!("cosmic background key {key} unreadable: {error:?}");
            None
        }
    }
}

/// The entry cosmic-bg would draw from, resolved to an image plan.
///
/// Displays sharing one wallpaper use the `all` entry. When they differ, a
/// windowed client cannot tell which display it is on, so it follows cosmic-bg's
/// own fallback for an unlisted output: the `all` entry - except when the config
/// carries exactly one display entry, which names the answer by elimination.
fn resolve(same_on_all: bool, all: Option<&Entry>, outputs: &[Entry]) -> Option<Plan> {
    let entry = match (same_on_all, outputs) {
        (false, [entry]) => entry,
        _ => all?,
    };
    plan(entry)
}

/// The image one entry names, with the fit its scaling mode asks for. A colour
/// background has no image to draw.
fn plan(entry: &Entry) -> Option<Plan> {
    let path = match entry.source.as_ref()? {
        Source::Path(path) => path.clone(),
        Source::Color(_) => return None,
    };
    let fit = match entry.scaling_mode {
        // The fit's fill colour bands behind the image; the page's own
        // background shows through instead.
        ScalingMode::Fit(_) => ContentFit::Contain,
        ScalingMode::Stretch => ContentFit::Fill,
        ScalingMode::Zoom => ContentFit::Cover,
    };
    Some(Plan { path, fit })
}

/// The file behind a source path. A folder source is one cosmic-bg rotates
/// through; the dashboard is a still image, so the folder resolves to its first
/// image in alphanumeric order - cosmic-bg's own default sampling.
fn pick_path(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        let entries: Vec<PathBuf> = std::fs::read_dir(path)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        first_image(entries)
    } else if path.is_file() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

/// The first image in alphanumeric order, ignoring anything that is not one.
fn first_image(mut paths: Vec<PathBuf>) -> Option<PathBuf> {
    paths.sort();
    paths.into_iter().find(|path| is_image(path))
}

/// Whether a path names a wallpaper this app can render.
fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape cosmic-bg writes through cosmic-config: every field of its
    /// `Entry`, including the ones this mirror ignores. The dashboard must read
    /// the real file, not just its own idea of one.
    const WRITTEN_BY_COSMIC_BG: &str = r#"(
        output: "HDMI-A-1",
        source: Path("/usr/share/backgrounds/cosmic/orion_nebula_nasa_heic0601a.jpg"),
        filter_by_theme: true,
        rotation_frequency: 900,
        filter_method: Lanczos,
        scaling_mode: Zoom,
        sampling_method: Alphanumeric,
    )"#;

    fn entry(source: Source, scaling_mode: ScalingMode) -> Entry {
        Entry {
            source: Some(source),
            scaling_mode,
        }
    }

    fn image(path: &str) -> Source {
        Source::Path(PathBuf::from(path))
    }

    #[test]
    fn entry_parses_from_the_config_cosmic_bg_writes() {
        let entry: Entry =
            ron::from_str(WRITTEN_BY_COSMIC_BG).expect("the entry cosmic-bg saves must parse");
        let plan = resolve(true, Some(&entry), &[]).expect("a wallpaper entry resolves");
        assert_eq!(
            plan.path,
            PathBuf::from("/usr/share/backgrounds/cosmic/orion_nebula_nasa_heic0601a.jpg"),
            "unexpected path from {WRITTEN_BY_COSMIC_BG}"
        );
        assert_eq!(plan.fit, ContentFit::Cover, "Zoom must cover the page");
    }

    #[test]
    fn shared_background_wins_whatever_the_displays_hold() {
        let shared = entry(image("/shared.jpg"), ScalingMode::Zoom);
        let one = entry(image("/one.jpg"), ScalingMode::Zoom);
        let two = entry(image("/two.jpg"), ScalingMode::Zoom);
        let plan = resolve(true, Some(&shared), &[one, two]).expect("the shared entry resolves");
        assert_eq!(plan.path, PathBuf::from("/shared.jpg"));
    }

    #[test]
    fn lone_display_entry_is_used_when_displays_differ() {
        let shared = entry(image("/shared.jpg"), ScalingMode::Zoom);
        let only = entry(image("/only.jpg"), ScalingMode::Zoom);
        let plan = resolve(false, Some(&shared), &[only]).expect("the lone entry resolves");
        assert_eq!(plan.path, PathBuf::from("/only.jpg"));
    }

    #[test]
    fn several_display_entries_fall_back_to_the_shared_background() {
        let shared = entry(image("/shared.jpg"), ScalingMode::Zoom);
        let one = entry(image("/one.jpg"), ScalingMode::Zoom);
        let two = entry(image("/two.jpg"), ScalingMode::Zoom);
        let plan = resolve(false, Some(&shared), &[one, two]).expect("the shared entry resolves");
        assert_eq!(plan.path, PathBuf::from("/shared.jpg"));
    }

    #[test]
    fn colour_background_parses_and_leaves_the_theme_background_in_place() {
        let written = r#"(
            output: "HDMI-A-1",
            source: Color(Single((0.1, 0.1, 0.1))),
            filter_by_theme: false,
            rotation_frequency: 900,
            filter_method: Lanczos,
            scaling_mode: Zoom,
            sampling_method: Alphanumeric,
        )"#;
        let entry: Entry = ron::from_str(written).expect("a colour background must parse");
        assert_eq!(
            resolve(true, Some(&entry), &[]),
            None,
            "a colour background has no image to draw, from {written}"
        );
    }

    #[test]
    fn missing_background_leaves_the_theme_background_in_place() {
        assert_eq!(resolve(true, None, &[]), None);
    }

    #[test]
    fn scaling_modes_follow_what_cosmic_bg_would_do() {
        let cases: &[(ScalingMode, ContentFit)] = &[
            (ScalingMode::Zoom, ContentFit::Cover),
            (ScalingMode::Stretch, ContentFit::Fill),
            (ScalingMode::Fit([0.0, 0.0, 0.0]), ContentFit::Contain),
        ];
        for (scaling_mode, expected) in cases {
            let plan = plan(&entry(image("/wall.jpg"), *scaling_mode)).expect("an image resolves");
            assert_eq!(plan.fit, *expected, "scaling mode: {scaling_mode:?}");
        }
    }

    #[test]
    fn rotating_folder_picks_the_first_image_in_alphanumeric_order() {
        let picked = first_image(vec![
            PathBuf::from("/wallpapers/c.png"),
            PathBuf::from("/wallpapers/notes.txt"),
            PathBuf::from("/wallpapers/a.jpg"),
            PathBuf::from("/wallpapers/B.webp"),
        ]);
        assert_eq!(picked, Some(PathBuf::from("/wallpapers/B.webp")));
    }

    #[test]
    fn folder_without_images_resolves_to_no_wallpaper() {
        assert_eq!(
            first_image(vec![PathBuf::from("/wallpapers/notes.txt")]),
            None
        );
    }
}
