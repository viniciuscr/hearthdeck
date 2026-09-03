// SPDX-License-Identifier: GPL-3.0-only

use cosmic::desktop::{IconSourceExt, fde::IconSource};
use cosmic::iced::core::{image as core_image, svg as core_svg};
use cosmic::widget::icon;
use resvg::{tiny_skia, usvg};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IconCacheKey {
    name: &'static str,
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
                        name: $name,
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

    pub fn get(&mut self, name: &'static str, size: u16) -> icon::Handle {
        self.cache
            .entry(IconCacheKey { name, size })
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

pub fn icon_cache_handle(name: &'static str, size: u16) -> icon::Handle {
    let mut icon_cache = ICON_CACHE
        .get_or_init(|| Mutex::new(IconCache::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    icon_cache.get(name, size)
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

    let mut fontdb = usvg::fontdb::Database::new();
    fontdb.load_system_fonts();
    let options = usvg::Options {
        fontdb: Arc::new(fontdb),
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

    // SVG rendering can panic on malformed or complex vectors.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        resvg::render(&tree, transform, &mut pixmap.as_mut());
    }));

    Some(icon::from_raster_pixels(width, height, pixmap.take()))
}

/// Stable identity for a resolved icon handle, used as the cache key.
fn handle_identity(base: &icon::Handle) -> String {
    match &base.data {
        icon::Data::Svg(svg) => format!("svg-{:x}", svg.id()),
        icon::Data::Image(image) => match image {
            core_image::Handle::Path(_, path) => format!("img-{}", path.to_string_lossy()),
            core_image::Handle::Bytes(_, _) => "img-bytes".into(),
            core_image::Handle::Rgba { id, .. } => format!("img-rgba-{id:?}"),
        },
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RasterIconKey {
    identity: String,
    size: u32,
}

static RASTER_ICON_CACHE: OnceLock<Mutex<HashMap<RasterIconKey, icon::Handle>>> = OnceLock::new();

/// Resolve an entry icon into an [`icon::Handle`], pre-rendering SVG icons to
/// raw RGBA at `size` so the renderer never rasterizes vectors on the render
/// thread (which froze page scrolling). PNGs are kept as-is (cheap to decode),
/// and results are cached by handle identity.
pub fn entry_icon_handle(source: &IconSource, size: u32) -> icon::Handle {
    let base = source.as_cosmic_icon();
    let key = RasterIconKey {
        identity: handle_identity(&base),
        size,
    };

    let mut cache = RASTER_ICON_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    if let Some(handle) = cache.get(&key) {
        return handle.clone();
    }

    let handle = match &base.data {
        icon::Data::Svg(svg) => rasterize_svg(svg, size, size).unwrap_or(base),
        icon::Data::Image(_) => base,
    };

    cache.insert(key, handle.clone());

    handle
}
