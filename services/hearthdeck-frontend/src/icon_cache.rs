// SPDX-License-Identifier: GPL-3.0-only

use cosmic::desktop::{IconSourceExt, fde::IconSource};
use cosmic::iced::core::{image as core_image, svg as core_svg};
use cosmic::iced::{Subscription, stream};
use cosmic::widget::icon;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::{SinkExt, StreamExt};
use resvg::{tiny_skia, usvg};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

/// The system fonts SVG text is drawn with, scanned once: loading them walks every
/// font directory, which is far too slow to repeat for each icon rasterized.
static SVG_FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut fonts = usvg::fontdb::Database::new();
    fonts.load_system_fonts();
    Arc::new(fonts)
});

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct IconCacheKey {
    name: String,
    size: u16,
}

pub struct IconCache {
    cache: HashMap<IconCacheKey, icon::Handle>,
}

impl IconCache {
    pub fn new() -> Self {
        let mut cache = HashMap::new();

        macro_rules! bundle {
            ($name:expr, $size:expr) => {
                let data: &'static [u8] = include_bytes!(concat!("../data/icons/", $name, ".svg"));
                cache.insert(
                    IconCacheKey {
                        name: $name.to_owned(),
                        size: $size,
                    },
                    icon::from_svg_bytes(data).symbolic($name.ends_with("-symbolic")),
                );
            };
        }

        bundle!("app-source-flatpak", 16);
        bundle!("app-source-local-symbolic", 16);
        bundle!("app-source-snap", 16);
        bundle!("app-source-nix", 16);
        bundle!("app-source-system-symbolic", 16);

        Self { cache }
    }

    pub fn get(&mut self, name: &str, size: u16) -> icon::Handle {
        self.cache
            .entry(IconCacheKey {
                name: name.to_owned(),
                size,
            })
            .or_insert_with(|| {
                icon::from_name(name)
                    .size(size)
                    .symbolic(name.ends_with("-symbolic"))
                    .handle()
            })
            .clone()
    }
}

static ICON_CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();

pub fn icon_cache_handle(name: &str, size: u16) -> icon::Handle {
    let mut icon_cache = ICON_CACHE
        .get_or_init(|| Mutex::new(IconCache::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    icon_cache.get(name, size)
}

/// Where RomM's own artwork lives, as RomM reports it.
///
/// RomM runs on the user's machine and hands out paths inside its own URL space
/// rather than files, so a reference under this prefix is not something to open:
/// it is a path to ask the daemon's asset route to fetch, which reads it off the
/// host's library mount or from RomM itself.
const ROMM_ASSET_PREFIX: &str = "/assets/";

/// What a rail card with no artwork draws.
///
/// Symbolic, so it takes the card's own colour instead of a baked-in one - which
/// is also why nothing else is drawn this way: a rasterized glyph loses the
/// colour the theme would have given it.
pub const GENERIC_ICON: &str = "application-x-executable-symbolic";

/// What an icon reference names, and therefore where its bytes come from.
///
/// The daemon publishes a reference with every record and rail item, and a
/// reference is one of four things. This is the client reading it back: the
/// provider that fetches artwork and the views that draw it both go through
/// [`classify`], so a reference cannot mean one thing on the dashboard and
/// another in the library.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconReference<'a> {
    /// A poster or cover on the network, to be downloaded and cached.
    Remote(&'a str),
    /// A path inside RomM's library, which only the daemon can fetch.
    RommAsset(&'a str),
    /// A file on this machine.
    File(&'a str),
    /// A name the icon theme resolves.
    Name(&'a str),
}

/// Reads one icon reference. `None` is a reference that names nothing, which is
/// not the same as one that names something missing - both draw the same, but
/// only the second is worth fetching.
#[must_use]
pub fn classify(reference: &str) -> Option<IconReference<'_>> {
    let reference = reference.trim();
    if reference.is_empty() {
        return None;
    }
    Some(
        if reference.starts_with("http://") || reference.starts_with("https://") {
            IconReference::Remote(reference)
        } else if let Some(path) = reference.strip_prefix("file://") {
            // A launcher's artwork is often written as a URL rather than a path,
            // and the two are the same file once the scheme is off.
            IconReference::File(path)
        } else if reference.starts_with(ROMM_ASSET_PREFIX) {
            IconReference::RommAsset(reference)
        } else if reference.starts_with('/') {
            IconReference::File(reference)
        } else {
            IconReference::Name(reference)
        },
    )
}

/// How a surface wants its artwork drawn.
///
/// A portrait card is the library's grid tile and the dashboard's rail card: it
/// is taller than it is wide, and a cover drawn for a horizontal box bands inside
/// it. A details backdrop and the banner's own artwork are the opposite shape and
/// must keep the image as it is, so the surface says which it is rather than the
/// cache guessing from a size.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Orientation {
    /// The artwork as it is, for a surface shaped like the picture.
    AsIs,
    /// A portrait card: a clearly landscape cover is turned a quarter turn so it
    /// fills the card instead of banding inside it.
    Portrait,
}

/// How much wider than tall a cover has to be before it counts as horizontal box
/// art and is turned on its side for a portrait card. A third wider keeps
/// near-square artwork - an application's own icon, a cover only slightly off
/// square - out of the rule, so nothing straight is drawn crooked.
const LANDSCAPE_RATIO: f32 = 1.3;

/// Metahub's art kinds and the sizes it serves each in, in its URL segments.
const ART_KINDS: [&str; 3] = ["poster", "background", "logo"];
const ART_SIZES: [&str; 3] = ["small", "medium", "large"];

/// Every place one artwork is fetchable: the reference itself first, then the
/// same art at Metahub's other sizes, largest first.
///
/// Metahub's edge answers 200 for the art it has cached and 307s the rest to
/// an origin that is not reachable everywhere, so whether a fetch succeeds
/// depends on what has been asked for before - and a poster is the hottest art
/// there is while the banner's wide art is the coldest: it belongs to titles
/// that have just arrived. Retrying the same picture at its other sizes is what
/// keeps a new slide from silently drawing its poster instead of its backdrop.
#[must_use]
pub fn art_candidates(reference: &str) -> Vec<String> {
    let mut candidates = vec![reference.to_owned()];
    for kind in ART_KINDS {
        for size in ART_SIZES {
            let segment = format!("/{kind}/{size}/");
            if !reference.contains(&segment) {
                continue;
            }
            // The same art at its other sizes, largest first: this is a
            // fallback, so the quality of the art is worth more than its bytes.
            candidates.extend(
                ["large", "medium", "small"]
                    .into_iter()
                    .filter(|other| *other != size)
                    .map(|other| reference.replace(&segment, &format!("/{kind}/{other}/"))),
            );
            return candidates;
        }
    }
    candidates
}

/// Draws one icon reference: a file the provider resolved, or a name the icon
/// theme resolves.
///
/// The single place a view turns a reference into a widget, so every surface
/// draws a card the same way. A reference that is absent, blank, or still
/// unfetched - a poster URL or a RomM path the provider has not turned into a
/// file yet - draws [`GENERIC_ICON`], because a card is never blank and a path
/// that does not exist is not artwork.
pub fn reference_handle(
    reference: Option<&str>,
    size: u32,
    orientation: Orientation,
) -> icon::Handle {
    match reference.and_then(classify) {
        Some(IconReference::File(path)) => {
            entry_icon_handle(&IconSource::Path(PathBuf::from(path)), size, orientation)
        }
        // A symbolic name is monochrome and the theme decides its colour, which
        // a rasterized copy loses (it comes out black), so it is drawn through
        // the theme lookup like the generic mark.
        Some(IconReference::Name(name)) if name.ends_with("-symbolic") => {
            icon_cache_handle(name, size as u16)
        }
        Some(IconReference::Name(name)) => {
            entry_icon_handle(&IconSource::Name(name.to_owned()), size, orientation)
        }
        Some(IconReference::Remote(_) | IconReference::RommAsset(_)) | None => {
            icon_cache_handle(GENERIC_ICON, size as u16)
        }
    }
}

/// Rasterize an SVG into a raw RGBA [`icon::Handle`] at the given size.
///
/// Mirrors the renderer's own pipeline (iced_wgpu `image::vector`), but runs
/// off the render thread so scrolling never blocks on vector rasterization.
fn rasterize_svg(svg: &core_svg::Handle, width: u32, height: u32) -> Option<icon::Handle> {
    let bytes: Vec<u8> = match svg.data() {
        core_svg::Data::Path(path) => std::fs::read(path).ok()?,
        core_svg::Data::Bytes(bytes) => bytes.to_vec(),
    };

    let options = usvg::Options {
        fontdb: Arc::clone(&SVG_FONTS),
        ..usvg::Options::default()
    };

    let tree = usvg::Tree::from_data(&bytes, &options).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;

    let tree_size = tree.size().to_int_size();
    let target_size = if width > height {
        tree_size.scale_to_width(width)
    } else {
        tree_size.scale_to_height(height)
    };

    let transform = match target_size {
        Some(target_size) => {
            let tree_size = tree_size.to_size();
            let target_size = target_size.to_size();
            tiny_skia::Transform::from_scale(
                target_size.width() / tree_size.width(),
                target_size.height() / tree_size.height(),
            )
        }
        None => tiny_skia::Transform::default(),
    };

    // SVG rendering can panic on malformed or complex vectors; the caller then
    // falls back to the unrendered handle rather than caching a blank tile.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        resvg::render(&tree, transform, &mut pixmap.as_mut());
    }))
    .ok()?;

    Some(icon::from_raster_pixels(width, height, pixmap.take()))
}

/// An [`IconSource`], hashable.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum SourceKey {
    Name(String),
    Path(PathBuf),
}

impl SourceKey {
    fn of(source: &IconSource) -> Self {
        match source {
            IconSource::Name(name) => Self::Name(name.clone()),
            IconSource::Path(path) => Self::Path(path.clone()),
        }
    }

    fn source(&self) -> IconSource {
        match self {
            Self::Name(name) => IconSource::Name(name.clone()),
            Self::Path(path) => IconSource::Path(path.clone()),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RasterIconKey {
    source: SourceKey,
    size: u32,
    /// A cover turned to fill a portrait card is a different picture from the
    /// same cover drawn as it is, so the two are cached apart.
    portrait: bool,
}

/// Cap on the rasterized-icon cache.
///
/// Each entry is a full RGBA tile - hundreds of KiB at the sizes the grid uses -
/// so an unbounded map is a slow leak on a kiosk that browses a large RomM
/// library.
const RASTER_ICON_CACHE_CAP: usize = 256;

/// The icons resolved so far, and the ones a view asked for that the worker has
/// not resolved yet.
#[derive(Default)]
struct RasterIcons {
    /// Each handle with the tick it was last drawn at, so a full cache drops
    /// what has been off screen longest rather than everything at once.
    resolved: HashMap<RasterIconKey, (icon::Handle, u64)>,
    tick: u64,
    queued: HashSet<RasterIconKey>,
}

impl RasterIcons {
    /// The resolved handle for `key`, marked as just drawn.
    fn get(&mut self, key: &RasterIconKey) -> Option<icon::Handle> {
        self.tick += 1;
        let tick = self.tick;
        let (handle, used) = self.resolved.get_mut(key)?;
        *used = tick;
        Some(handle.clone())
    }

    fn insert(&mut self, key: RasterIconKey, handle: icon::Handle) {
        self.queued.remove(&key);
        if self.resolved.len() >= RASTER_ICON_CACHE_CAP
            && let Some(oldest) = self
                .resolved
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| key.clone())
        {
            self.resolved.remove(&oldest);
        }
        self.tick += 1;
        self.resolved.insert(key, (handle, self.tick));
    }
}

static RASTER_ICONS: LazyLock<Mutex<RasterIcons>> = LazyLock::new(Mutex::default);

/// The queue from the views to the worker. The worker's end waits here until
/// [`rendered_icons`] takes it, so a request made before it starts is kept.
type Jobs = (
    UnboundedSender<RasterIconKey>,
    Mutex<Option<UnboundedReceiver<RasterIconKey>>>,
);
static JOBS: LazyLock<Jobs> = LazyLock::new(|| {
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    (sender, Mutex::new(Some(receiver)))
});

fn raster_icons() -> std::sync::MutexGuard<'static, RasterIcons> {
    RASTER_ICONS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Draws an entry icon without touching the disk.
///
/// A raster file drawn as it is needs no work: the renderer decodes it. Anything
/// else - an icon-theme lookup, an SVG to rasterize, a cover to turn - is too
/// slow for `view`, so a miss is handed to [`rendered_icons`] and drawn as the
/// untouched picture, or [`GENERIC_ICON`] when there is none, until the worker
/// answers.
pub fn entry_icon_handle(source: &IconSource, size: u32, orientation: Orientation) -> icon::Handle {
    let picture = match source {
        IconSource::Path(path) if !is_svg(path) => {
            let picture = icon::from_path(path.clone());
            if orientation == Orientation::AsIs {
                return picture;
            }
            Some(picture)
        }
        _ => None,
    };
    let key = RasterIconKey {
        source: SourceKey::of(source),
        size,
        portrait: orientation == Orientation::Portrait,
    };
    let mut icons = raster_icons();
    if let Some(handle) = icons.get(&key) {
        return handle;
    }
    if icons.queued.insert(key.clone()) && JOBS.0.unbounded_send(key).is_err() {
        log::warn!("icon worker is gone; icons stay generic");
    }
    drop(icons);
    picture
        .unwrap_or_else(|| icon_cache_handle(GENERIC_ICON, u16::try_from(size).unwrap_or(u16::MAX)))
}

fn is_svg(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
}

/// Resolves the icons views asked for, off the UI thread, and emits once per
/// batch so the views that asked draw again.
pub fn rendered_icons() -> Subscription<()> {
    Subscription::run_with("hearthdeck-icon-worker", |_| {
        stream::channel(1, async move |mut output| {
            let taken = JOBS.1.lock().unwrap_or_else(|e| e.into_inner()).take();
            let Some(mut jobs) = taken else {
                log::warn!("icon worker started twice; the first one keeps the queue");
                return;
            };
            while let Some(first) = jobs.next().await {
                let mut batch = vec![first];
                while let Ok(Some(key)) = jobs.try_next() {
                    batch.push(key);
                }
                let resolved = tokio::task::spawn_blocking(move || {
                    batch
                        .into_iter()
                        .map(|key| {
                            let handle = resolve(&key);
                            (key, handle)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
                match resolved {
                    Ok(resolved) => {
                        let mut icons = raster_icons();
                        for (key, handle) in resolved {
                            icons.insert(key, handle);
                        }
                    }
                    Err(error) => log::warn!("icon worker failed: {error}"),
                }
                if output.send(()).await.is_err() {
                    return;
                }
            }
        })
    })
}

/// Resolves one icon: SVGs pre-rendered to raw RGBA at its size so the renderer
/// never rasterizes vectors on the render thread (which froze page scrolling),
/// PNGs kept as they are (cheap to decode) - except on a portrait card, where a
/// clearly landscape cover is turned a quarter turn so it fills the card.
fn resolve(key: &RasterIconKey) -> icon::Handle {
    let base = key.source.source().as_cosmic_icon();
    match (&base.data, key.portrait) {
        (icon::Data::Svg(svg), _) => rasterize_svg(svg, key.size, key.size).unwrap_or(base),
        // A cover drawn for a horizontal box, on a card taller than it is wide:
        // turned, so the picture fills the card instead of banding inside it.
        (icon::Data::Image(image), true) => turned_for_portrait(image).unwrap_or(base),
        (icon::Data::Image(_), false) => base,
    }
}

/// A landscape cover turned a quarter turn clockwise, as an icon of its own.
///
/// `None` is every case the rule does not apply to - the picture is not clearly
/// landscape, its bytes cannot be read or decoded, or it is already a raster in
/// memory - and the caller then draws the artwork as it is, so a cover nothing is
/// known about still draws.
fn turned_for_portrait(image: &core_image::Handle) -> Option<icon::Handle> {
    let core_image::Handle::Path(_, path) = image else {
        return None;
    };
    let bytes = std::fs::read(path).ok()?;
    let decoded = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;

    let turned = turned_portrait(&decoded.to_rgba8())?;
    let (width, height) = turned.dimensions();
    Some(icon::from_raster_pixels(width, height, turned.into_raw()))
}

/// Whether a raster is horizontal box art: at least [`LANDSCAPE_RATIO`] times as
/// wide as it is tall.
#[must_use]
fn is_landscape(width: u32, height: u32) -> bool {
    height > 0 && width as f32 / height as f32 >= LANDSCAPE_RATIO
}

/// The quarter turn clockwise, or `None` when the picture is not clearly
/// landscape. Clockwise, so the cover's left edge becomes the card's top edge.
fn turned_portrait(raster: &image::RgbaImage) -> Option<image::RgbaImage> {
    let (width, height) = raster.dimensions();
    if !is_landscape(width, height) {
        return None;
    }
    Some(image::imageops::rotate90(raster))
}

#[cfg(test)]
mod tests {
    use super::{
        GENERIC_ICON, IconReference, Orientation, RASTER_ICON_CACHE_CAP, RasterIconKey,
        RasterIcons, SourceKey, art_candidates, classify, entry_icon_handle, is_landscape,
        raster_icons, reference_handle, resolve, turned_portrait,
    };
    use cosmic::desktop::fde::IconSource;
    use cosmic::widget::icon;
    use std::path::PathBuf;

    fn key(path: &str) -> RasterIconKey {
        RasterIconKey {
            source: SourceKey::Path(PathBuf::from(path)),
            size: 64,
            portrait: false,
        }
    }

    /// A reference is the daemon's, and it means one of four things. Reading it
    /// wrong is the whole of the bug this vocabulary exists to stop: a poster URL
    /// read as a theme name draws nothing, and a path that is not a file (RomM's
    /// own artwork) is not something to open.
    #[test]
    fn a_reference_is_read_as_the_thing_its_bytes_come_from() {
        let poster = "https://images.metahub.space/poster/medium/tt0149460/img";
        assert_eq!(classify(poster), Some(IconReference::Remote(poster)));
        assert_eq!(
            classify("http://images.example.com/cover.jpg"),
            Some(IconReference::Remote("http://images.example.com/cover.jpg"))
        );

        // RomM hands out paths in its own URL space, not files on this machine.
        let romm = "/assets/romm/resources/roms/9/4014/cover/small.png";
        assert_eq!(classify(romm), Some(IconReference::RommAsset(romm)));
        assert_eq!(
            classify("/assets/platforms/snes.png"),
            Some(IconReference::RommAsset("/assets/platforms/snes.png"))
        );

        // A file is a file whether it was written as a path or as a URL.
        assert_eq!(
            classify("file:///home/user/cover.png"),
            Some(IconReference::File("/home/user/cover.png"))
        );
        let cached = "/home/user/.cache/hearthdeck/icons/1a2b3c4d.png";
        assert_eq!(classify(cached), Some(IconReference::File(cached)));

        // Everything else is a name for the icon theme to resolve.
        assert_eq!(
            classify("org.example.App"),
            Some(IconReference::Name("org.example.App"))
        );
        assert_eq!(
            classify("  application-x-executable-symbolic  "),
            Some(IconReference::Name("application-x-executable-symbolic"))
        );

        // A reference that names nothing is nothing to fetch or draw.
        assert_eq!(classify(""), None);
        assert_eq!(classify("   "), None);
    }

    /// A card with nothing drawable draws the generic mark, and the mark is
    /// symbolic so the theme colours it rather than a glyph baked at one colour.
    /// A poster URL and a RomM path are both undrawable as they stand - the
    /// provider is what turns them into files - so arriving here means the mark.
    #[test]
    fn nothing_drawable_draws_the_generic_mark() {
        for reference in [
            None,
            Some(""),
            Some("https://images.metahub.space/poster/medium/tt0149460/img"),
            Some("/assets/romm/resources/roms/9/4014/cover/small.png"),
        ] {
            assert!(
                reference_handle(reference, 64, Orientation::AsIs).symbolic,
                "{reference:?} is not artwork, so it draws the mark"
            );
        }
        assert!(GENERIC_ICON.ends_with("-symbolic"));
    }

    /// Metahub serves part of the poster catalogue as webp, and iced decodes a
    /// cached poster through the `image` crate. With libcosmic's feature set
    /// alone (bmp, ico, jpeg, png) the decoder answers "the image format WebP is
    /// not supported", which is why only the titles whose poster is webp drew
    /// the placeholder in Continue Watching. The frontend's own `image`
    /// dependency turns that decoder on; this guesses the format from the bytes
    /// and decodes, which is what iced's loader does for a cached poster.
    #[test]
    fn a_webp_poster_decodes_the_way_the_renderer_loads_one() {
        let poster = include_bytes!("../data/test/poster.webp");

        let image = image::ImageReader::new(std::io::Cursor::new(poster))
            .with_guessed_format()
            .expect("a webp poster has a recognisable signature")
            .decode()
            .expect("a webp poster must decode");

        assert_eq!((image.width(), image.height()), (4, 4));
    }

    /// A file the provider resolved is drawn as the image it is, and a symbolic
    /// name keeps the theme's own colouring instead of being rasterized into a
    /// fixed one.
    #[test]
    fn a_resolved_file_is_drawn_as_an_image() {
        let cached = resolve(&key("/tmp/hearthdeck-cache/cover.png"));
        assert!(!cached.symbolic);

        assert!(
            reference_handle(Some("app-source-flatpak-symbolic"), 16, Orientation::AsIs).symbolic
        );
    }

    /// A cover drawn for a horizontal box bands inside a portrait card, so the
    /// card turns it a quarter turn - and only when it is clearly landscape. A
    /// square application icon and a portrait cover are left exactly as they are.
    #[test]
    fn only_clearly_landscape_art_is_turned_for_a_portrait_card() {
        assert!(is_landscape(52, 40), "exactly a third wider is box art");
        assert!(is_landscape(1920, 1080), "16:9 is box art");
        assert!(!is_landscape(12, 10), "a fifth wider is not");
        assert!(!is_landscape(10, 10), "a square icon is never turned");
        assert!(
            !is_landscape(10, 20),
            "a portrait cover is already the card's shape"
        );
        assert!(!is_landscape(10, 0), "a height of nothing is not a picture");
    }

    /// The quarter turn swaps the sides and carries the picture's own corner with
    /// it: clockwise, so the cover's left edge becomes the card's top edge and the
    /// title reads with the head tilted left.
    #[test]
    fn the_quarter_turn_swaps_the_sides_and_keeps_the_pictures_corner() {
        let mut raster = image::RgbaImage::new(8, 4);
        raster.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));

        let turned = turned_portrait(&raster).expect("a 2:1 picture is landscape");

        assert_eq!(turned.dimensions(), (4, 8), "the sides swap");
        assert_eq!(
            turned.get_pixel(3, 0),
            &image::Rgba([255, 0, 0, 255]),
            "the top-left corner moved to the top-right"
        );
        assert!(
            turned_portrait(&image::RgbaImage::new(4, 4)).is_none(),
            "a square is drawn as it is"
        );
    }

    /// A reference the edge will not serve is retried at the same art's other
    /// sizes before it is given up on: it is the same picture, and one of
    /// Metahub's sizes is usually cached when another is not. A reference that
    /// is not Metahub's art has no other sizes to try.
    #[test]
    fn a_cold_art_reference_falls_back_to_the_same_art_at_its_other_sizes() {
        assert_eq!(
            art_candidates("https://images.metahub.space/background/medium/tt0149460/img"),
            vec![
                "https://images.metahub.space/background/medium/tt0149460/img",
                "https://images.metahub.space/background/large/tt0149460/img",
                "https://images.metahub.space/background/small/tt0149460/img",
            ]
        );
        assert_eq!(
            art_candidates("/home/user/.cache/hearthdeck/icons/1a2b3c4d.png"),
            vec!["/home/user/.cache/hearthdeck/icons/1a2b3c4d.png".to_owned()]
        );
    }

    /// `view` never resolves an icon itself: a miss draws the generic mark and
    /// is queued once for the worker, however many frames ask for it.
    #[test]
    fn a_miss_draws_the_generic_mark_and_queues_the_icon_once() {
        let path = "/tmp/hearthdeck-test/never-resolved.svg";
        let source = IconSource::Path(PathBuf::from(path));

        let first = entry_icon_handle(&source, 64, Orientation::AsIs);
        let again = entry_icon_handle(&source, 64, Orientation::AsIs);

        assert!(first.symbolic && again.symbolic, "the mark stands in");
        let icons = raster_icons();
        assert!(icons.queued.contains(&key(path)), "queued for the worker");
        assert!(!icons.resolved.contains_key(&key(path)));
    }

    /// A raster cover is drawn at once: as it is, with nothing to queue, and on
    /// a portrait card as it is until the worker has turned it.
    #[test]
    fn a_raster_cover_is_drawn_while_it_waits() {
        let path = "/tmp/hearthdeck-test/cover.png";
        let source = IconSource::Path(PathBuf::from(path));

        assert!(!entry_icon_handle(&source, 64, Orientation::AsIs).symbolic);
        assert!(!raster_icons().queued.contains(&key(path)), "nothing to do");

        assert!(!entry_icon_handle(&source, 64, Orientation::Portrait).symbolic);
        let portrait = RasterIconKey {
            portrait: true,
            ..key(path)
        };
        assert!(
            raster_icons().queued.contains(&portrait),
            "the turn is queued"
        );
    }

    /// A full cache drops what has been off screen longest, so the tiles on
    /// screen never fall back to the mark while the grid scrolls.
    #[test]
    fn a_full_cache_drops_the_icon_drawn_longest_ago() {
        let mut icons = RasterIcons::default();
        let handle = || icon::from_raster_pixels(1, 1, vec![0; 4]);
        for index in 0..RASTER_ICON_CACHE_CAP {
            icons.insert(key(&format!("/icon-{index}.png")), handle());
        }
        assert!(icons.get(&key("/icon-0.png")).is_some(), "drawn again");

        icons.insert(key("/new.png"), handle());

        assert_eq!(icons.resolved.len(), RASTER_ICON_CACHE_CAP);
        assert!(
            icons.get(&key("/icon-0.png")).is_some(),
            "recently drawn stays"
        );
        assert!(icons.get(&key("/icon-1.png")).is_none(), "the oldest goes");
        assert!(icons.get(&key("/new.png")).is_some());
    }
}
