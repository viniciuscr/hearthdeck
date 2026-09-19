//! Resolves a RomM ROM into a local, launchable RetroArch core + ROM pair.
//!
//! This is Phase 2 of `docs/retroarch-integration.md`: platform-to-core
//! mapping and ROM fetch/caching. It does not talk to the bridge and does
//! not track sessions; `api.rs` handles that, the same way it already does
//! for Heroic and desktop-app launches.

use std::{
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};

use tokio::fs;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    diagnostics::{self, RommQueryError},
    settings::SettingsRepository,
};

/// Where pacman installs libretro cores on Arch. Kept in sync with the
/// bridge's own allowlist (`hearthdeck-bridge/src/platform/linux.rs`); the
/// bridge re-validates independently rather than trusting this module's
/// output, so a mismatch here fails safe as a rejected launch, not a
/// security hole.
const CORE_DIRECTORY: &str = "/usr/lib/libretro";

/// How long a cached rom survives without being played before the janitor
/// evicts it. This is the default only; `HEARTHDECK_ROM_CACHE_MAX_AGE_DAYS`
/// overrides it per deployment (see `Config::load`).
pub const DEFAULT_ROM_CACHE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// A `.<name>.<uuid>.part` file older than this cannot belong to a running
/// download — `download_rom_content` times out after two minutes — so it is
/// debris from a process that died mid-download.
const PARTIAL_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// How often the janitor sweeps the cache. A `tokio::time::interval`'s first
/// tick fires immediately, so every daemon start sweeps too.
const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Platform `fs_slug` (RomM's on-disk-folder-naming slug, e.g. "snes",
/// "ngc") to libretro core filename.
///
/// Each console lists every slug spelling we have seen RomM (or the
/// metadata providers behind it) emit for that platform, because a single
/// console is genuinely addressable by several names: RomM's own
/// folder-naming `fs_slug` ("ngc"), the provider slug ("gamecube"), and
/// hand-written variants. A miss here is the "core is not configured"
/// failure users hit even though the core is installed under
/// `/usr/lib/libretro`, so the aliases are deliberately broad.
///
/// Only cores Arch actually packages at the time of writing are listed;
/// `packaging/arch/PKGBUILD`'s `optdepends` mirrors this file and the
/// `every_pkgbuild_optdepend_core_has_a_platform_mapping` test keeps the two
/// in sync. See docs/retroarch-integration.md open question 2 for the plan to
/// make this on-demand and user-configurable instead of a fixed table.
const CORE_BY_PLATFORM_SLUG: &[(&str, &str)] = &[
    // Nintendo Entertainment System / Famicom / Famicom Disk System.
    ("nes", "fceumm_libretro.so"),
    ("famicom", "fceumm_libretro.so"),
    ("fds", "fceumm_libretro.so"),
    ("nintendo-entertainment-system", "fceumm_libretro.so"),
    // Super Nintendo / Super Famicom.
    ("snes", "snes9x_libretro.so"),
    ("sfc", "snes9x_libretro.so"),
    ("sfam", "snes9x_libretro.so"),
    ("super-famicom", "snes9x_libretro.so"),
    ("super-nintendo", "snes9x_libretro.so"),
    ("super-nintendo-entertainment-system", "snes9x_libretro.so"),
    // Sega Genesis / Mega Drive, Master System, Game Gear, Sega CD.
    ("genesis", "genesis_plus_gx_libretro.so"),
    ("genesis-slash-megadrive", "genesis_plus_gx_libretro.so"),
    ("megadrive", "genesis_plus_gx_libretro.so"),
    ("mega-drive", "genesis_plus_gx_libretro.so"),
    ("sega-mega-drive", "genesis_plus_gx_libretro.so"),
    ("sega-genesis", "genesis_plus_gx_libretro.so"),
    ("sms", "genesis_plus_gx_libretro.so"),
    ("mastersystem", "genesis_plus_gx_libretro.so"),
    ("master-system", "genesis_plus_gx_libretro.so"),
    ("sega-master-system", "genesis_plus_gx_libretro.so"),
    ("gg", "genesis_plus_gx_libretro.so"),
    ("gamegear", "genesis_plus_gx_libretro.so"),
    ("game-gear", "genesis_plus_gx_libretro.so"),
    ("sega-game-gear", "genesis_plus_gx_libretro.so"),
    ("segacd", "genesis_plus_gx_libretro.so"),
    ("sega-cd", "genesis_plus_gx_libretro.so"),
    ("mega-cd", "genesis_plus_gx_libretro.so"),
    ("mega-cd-slash-sega-cd", "genesis_plus_gx_libretro.so"),
    // Sega 32X (Genesis Plus GX does not emulate it).
    ("sega32x", "picodrive_libretro.so"),
    ("32x", "picodrive_libretro.so"),
    ("sega-32x", "picodrive_libretro.so"),
    // Game Boy / Game Boy Color / Game Boy Advance.
    ("gb", "mgba_libretro.so"),
    ("gameboy", "mgba_libretro.so"),
    ("game-boy", "mgba_libretro.so"),
    ("gbc", "mgba_libretro.so"),
    ("gameboy-color", "mgba_libretro.so"),
    ("game-boy-color", "mgba_libretro.so"),
    ("gba", "mgba_libretro.so"),
    ("gameboy-advance", "mgba_libretro.so"),
    ("game-boy-advance", "mgba_libretro.so"),
    // Nintendo 64.
    ("n64", "mupen64plus_next_libretro.so"),
    ("nintendo-64", "mupen64plus_next_libretro.so"),
    ("nintendo64", "mupen64plus_next_libretro.so"),
    // GameCube / Wii (one Dolphin core covers both).
    ("ngc", "dolphin_libretro.so"),
    ("gc", "dolphin_libretro.so"),
    ("gamecube", "dolphin_libretro.so"),
    ("game-cube", "dolphin_libretro.so"),
    ("nintendo-gamecube", "dolphin_libretro.so"),
    ("nintendo-game-cube", "dolphin_libretro.so"),
    ("wii", "dolphin_libretro.so"),
    ("nintendo-wii", "dolphin_libretro.so"),
    // PlayStation 1 / 2, PSP.
    ("ps", "mednafen_psx_libretro.so"),
    ("psx", "mednafen_psx_libretro.so"),
    ("ps1", "mednafen_psx_libretro.so"),
    ("playstation", "mednafen_psx_libretro.so"),
    ("sony-playstation", "mednafen_psx_libretro.so"),
    ("ps2", "play_libretro.so"),
    ("playstation-2", "play_libretro.so"),
    ("sony-playstation-2", "play_libretro.so"),
    ("psp", "ppsspp_libretro.so"),
    ("playstation-portable", "ppsspp_libretro.so"),
    ("sony-playstation-portable", "ppsspp_libretro.so"),
    // Nintendo DS.
    ("nds", "desmume_libretro.so"),
    ("nintendo-ds", "desmume_libretro.so"),
    ("ds", "desmume_libretro.so"),
    // Sega Dreamcast / Saturn.
    ("dc", "flycast_libretro.so"),
    ("dreamcast", "flycast_libretro.so"),
    ("sega-dreamcast", "flycast_libretro.so"),
    ("saturn", "kronos_libretro.so"),
    ("sega-saturn", "kronos_libretro.so"),
    // NEC PC Engine / TurboGrafx-16 and SuperGrafx.
    ("pce", "mednafen_pce_fast_libretro.so"),
    ("pcengine", "mednafen_pce_fast_libretro.so"),
    ("pc-engine", "mednafen_pce_fast_libretro.so"),
    ("turbografx16", "mednafen_pce_fast_libretro.so"),
    ("turbografx-16", "mednafen_pce_fast_libretro.so"),
    ("nec-pc-engine", "mednafen_pce_fast_libretro.so"),
    ("supergrafx", "mednafen_supergrafx_libretro.so"),
    ("super-grafx", "mednafen_supergrafx_libretro.so"),
    // Arcade / Neo Geo.
    ("arcade", "mame_libretro.so"),
    ("mame", "mame_libretro.so"),
    ("neogeo", "mame_libretro.so"),
    ("neo-geo", "mame_libretro.so"),
    ("snk-neo-geo", "mame_libretro.so"),
    // ScummVM point-and-click adventures.
    ("scummvm", "scummvm_libretro.so"),
];

#[derive(Debug)]
pub enum RetroLaunchError {
    Romm(RommQueryError),
    PlatformNotFound,
    UnsupportedPlatform { fs_slug: String },
    CoreNotInstalled { core_path: PathBuf },
    RomHasNoContentFile,
    InvalidContentFileName,
    Cache(std::io::Error),
}

impl std::fmt::Display for RetroLaunchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Romm(RommQueryError::NotConfigured) => {
                write!(formatter, "RomM is not configured")
            }
            Self::Romm(RommQueryError::Failed(error)) => write!(formatter, "{error}"),
            Self::PlatformNotFound => write!(formatter, "RomM platform was not found"),
            Self::UnsupportedPlatform { fs_slug } => write!(
                formatter,
                "no RetroArch core is configured for platform \"{fs_slug}\""
            ),
            Self::CoreNotInstalled { core_path } => write!(
                formatter,
                "RetroArch core is not installed at {}",
                core_path.display()
            ),
            Self::RomHasNoContentFile => write!(formatter, "rom has no content file to launch"),
            Self::InvalidContentFileName => write!(formatter, "rom content filename is invalid"),
            Self::Cache(error) => write!(formatter, "could not cache rom locally: {error}"),
        }
    }
}

impl std::error::Error for RetroLaunchError {}

pub struct RetroLaunchPlan {
    pub core_path: PathBuf,
    pub rom_path: PathBuf,
    pub game: diagnostics::RommGame,
}

/// Resolves the core and locally cached ROM path for a RomM rom ID,
/// downloading the ROM into Hearthdeck's own cache directory if it is not
/// already there. Returns only validated local paths; the bridge
/// re-validates both independently before launch (see
/// `hearthdeck-bridge/src/platform/linux.rs`).
pub async fn prepare_launch(
    settings: &SettingsRepository,
    rom_id: i64,
) -> Result<RetroLaunchPlan, RetroLaunchError> {
    let rom = diagnostics::romm_rom(settings, rom_id)
        .await
        .map_err(RetroLaunchError::Romm)?;
    let fs_name = rom
        .fs_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .ok_or(RetroLaunchError::RomHasNoContentFile)?;

    let platforms = diagnostics::romm_platforms(settings)
        .await
        .map_err(RetroLaunchError::Romm)?;
    let platform = platforms
        .into_iter()
        .find(|platform| platform.id == rom.platform_id)
        .ok_or(RetroLaunchError::PlatformNotFound)?;
    let fs_slug = platform.fs_slug.or(platform.slug).ok_or_else(|| {
        RetroLaunchError::UnsupportedPlatform {
            fs_slug: platform.name.clone(),
        }
    })?;

    let core_path = resolve_core_path(&fs_slug).await?;
    let rom_path = ensure_rom_cached(settings, rom_id, fs_name).await?;

    Ok(RetroLaunchPlan {
        core_path,
        rom_path,
        game: rom,
    })
}

async fn resolve_core_path(fs_slug: &str) -> Result<PathBuf, RetroLaunchError> {
    let core_filename = CORE_BY_PLATFORM_SLUG
        .iter()
        .find(|(slug, _)| *slug == fs_slug)
        .map(|(_, core)| *core)
        .ok_or_else(|| RetroLaunchError::UnsupportedPlatform {
            fs_slug: fs_slug.to_owned(),
        })?;
    let core_path = Path::new(CORE_DIRECTORY).join(core_filename);
    fs::metadata(&core_path)
        .await
        .map_err(|_| RetroLaunchError::CoreNotInstalled {
            core_path: core_path.clone(),
        })?;
    Ok(core_path)
}

async fn ensure_rom_cached(
    settings: &SettingsRepository,
    rom_id: i64,
    fs_name: &str,
) -> Result<PathBuf, RetroLaunchError> {
    validate_content_filename(fs_name)?;
    let rom_path = rom_cache_directory().join(rom_id.to_string()).join(fs_name);
    if fs::metadata(&rom_path).await.is_ok() {
        touch_cached_rom(&rom_path).await;
        return Ok(rom_path);
    }
    if let Some(parent) = rom_path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(RetroLaunchError::Cache)?;
    }
    let temporary_path =
        rom_path.with_file_name(format!(".{}.{}.part", fs_name, Uuid::new_v4().simple()));
    if let Err(error) =
        diagnostics::download_rom_content(settings, rom_id, fs_name, &temporary_path).await
    {
        let _ = fs::remove_file(&temporary_path).await;
        return Err(RetroLaunchError::Romm(error));
    }
    if let Err(error) = fs::rename(&temporary_path, &rom_path).await {
        let _ = fs::remove_file(&temporary_path).await;
        return Err(RetroLaunchError::Cache(error));
    }
    Ok(rom_path)
}

/// Re-stamps a cache hit so the janitor measures "last played", not "first
/// downloaded". Deliberately not atime: Linux mounts default to `relatime`,
/// which often leaves a read-only access without updating the timestamp.
///
/// Best-effort by design: a filesystem that refuses the update only costs an
/// early eviction, and an evicted rom is re-downloaded on the next launch.
async fn touch_cached_rom(rom_path: &Path) {
    let path = rom_path.to_path_buf();
    // Tokio has no `set_modified`, and the timestamp syscall is blocking, so
    // it runs on the blocking pool rather than stalling an async worker.
    let touched = tokio::task::spawn_blocking(move || {
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_modified(SystemTime::now()))
    })
    .await;
    match touched {
        Ok(Ok(())) => {}
        Ok(Err(error)) => warn!(
            path = %rom_path.display(),
            %error,
            "could not refresh rom cache timestamp"
        ),
        Err(error) => warn!(
            path = %rom_path.display(),
            %error,
            "rom cache timestamp update did not run"
        ),
    }
}

fn validate_content_filename(fs_name: &str) -> Result<(), RetroLaunchError> {
    let mut components = Path::new(fs_name).components();
    if matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none() {
        Ok(())
    } else {
        Err(RetroLaunchError::InvalidContentFileName)
    }
}

/// The directory Hearthdeck caches ROMs fetched from RomM into. Matches the
/// bridge's own `retro_rom_cache_directory` (same env-derived path, same
/// user); the bridge re-validates any rom it is asked to launch resolves
/// under this directory before exec'ing RetroArch.
fn rom_cache_directory() -> PathBuf {
    let cache_home = std::env::var_os("XDG_CACHE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|path| PathBuf::from(path).join(".cache")));
    cache_home
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("hearthdeck/romm")
}

/// What one cache sweep removed, in entries.
#[derive(Debug, Default, PartialEq, Eq)]
struct CacheSweep {
    roms: usize,
    partials: usize,
}

/// Spawns the background task that keeps the rom cache bounded: at startup
/// and every `SWEEP_INTERVAL` it deletes roms untouched for longer than
/// `max_age` and abandoned partial downloads.
///
/// Failing is not fatal. The cache is re-creatable by definition — an evicted
/// rom is downloaded again the next time it is launched — so a failed sweep
/// logs and waits for the next tick rather than disturbing playback.
pub fn start_cache_janitor(max_age: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            timer.tick().await;
            match sweep_rom_cache(&rom_cache_directory(), max_age).await {
                Ok(sweep) if sweep != CacheSweep::default() => info!(
                    evicted_roms = sweep.roms,
                    evicted_partials = sweep.partials,
                    max_age_secs = max_age.as_secs(),
                    "rom cache swept"
                ),
                Ok(_) => {}
                Err(error) => warn!(%error, "rom cache sweep failed"),
            }
        }
    })
}

/// Deletes the cached roms under `root` that have not been touched for
/// `max_age`, plus the partial files old enough to be crash debris.
///
/// A rom's age is the newest modification time among the files inside its
/// directory, not the directory's own time: `touch_cached_rom` stamps the
/// content file on every launch, so a rom still in rotation never ages out,
/// while one downloaded and never launched again does. A directory holding no
/// content at all falls back to its own timestamp, which is when its last
/// entry was written or removed — that is, when the failed download happened.
/// Partials count as activity only while they are fresh: a fresh `.part` is an
/// in-flight download and must protect its directory, a stale one is debris
/// that is deleted and counted against nothing.
async fn sweep_rom_cache(root: &Path, max_age: Duration) -> std::io::Result<CacheSweep> {
    let cutoff = SystemTime::now()
        .checked_sub(max_age)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let partial_cutoff = SystemTime::now()
        .checked_sub(PARTIAL_MAX_AGE)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut sweep = CacheSweep::default();
    let mut roms = match fs::read_dir(root).await {
        Ok(roms) => roms,
        // The cache directory only exists once something has been downloaded.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(sweep),
        Err(error) => return Err(error),
    };
    while let Some(rom) = roms.next_entry().await? {
        let path = rom.path();
        let Ok(metadata) = rom.metadata().await else {
            warn!(path = %path.display(), "skipping unreadable rom cache entry");
            continue;
        };
        if !metadata.is_dir() {
            // Not a shape `ensure_rom_cached` writes, but sweep it rather than
            // let an unexpected file claim space forever.
            if modified_at(&metadata) < cutoff && fs::remove_file(&path).await.is_ok() {
                sweep.roms += 1;
            }
            continue;
        }
        let mut newest: Option<SystemTime> = None;
        let mut contents = fs::read_dir(&path).await?;
        while let Some(content) = contents.next_entry().await? {
            let content_path = content.path();
            let Ok(content_metadata) = content.metadata().await else {
                continue;
            };
            // Nested directories are not a shape this cache produces; skip
            // them rather than recurse, so no unexpected layout can turn the
            // janitor into an unbounded tree walk.
            if content_metadata.is_dir() {
                continue;
            }
            let content_modified = modified_at(&content_metadata);
            if is_partial(&content_path)
                && content_modified < partial_cutoff
                && fs::remove_file(&content_path).await.is_ok()
            {
                sweep.partials += 1;
                continue;
            }
            newest = Some(newest.map_or(content_modified, |newest| newest.max(content_modified)));
        }
        let age = newest.unwrap_or_else(|| modified_at(&metadata));
        if age < cutoff {
            match fs::remove_dir_all(&path).await {
                Ok(()) => sweep.roms += 1,
                Err(error) => warn!(
                    path = %path.display(),
                    %error,
                    "could not evict rom cache entry"
                ),
            }
        }
    }
    Ok(sweep)
}

/// Matches the `.<fs_name>.<uuid>.part` names `ensure_rom_cached` downloads to.
fn is_partial(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.') && name.ends_with(".part"))
}

/// A file whose modification time cannot be read is treated as ancient, so
/// unreadable debris is evicted rather than kept forever.
fn modified_at(metadata: &std::fs::Metadata) -> SystemTime {
    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::{
        CORE_BY_PLATFORM_SLUG, CacheSweep, RetroLaunchError, resolve_core_path, sweep_rom_cache,
        touch_cached_rom, validate_content_filename,
    };

    const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

    /// Backdates a file's modification time, standing in for "last used".
    /// Read-only is enough: the tests own the files, and Unix allows an owner
    /// to set timestamps through a read-only descriptor. Directories need it,
    /// since they cannot be opened for writing.
    fn backdate(path: &std::path::Path, age: Duration) {
        let file = std::fs::File::open(path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    fn write_rom(directory: &std::path::Path, name: &str, age: Duration) -> std::path::PathBuf {
        std::fs::create_dir_all(directory).unwrap();
        let path = directory.join(name);
        std::fs::write(&path, b"rom").unwrap();
        backdate(&path, age);
        path
    }

    #[tokio::test]
    async fn evicts_only_roms_unused_for_longer_than_the_retention_window() {
        let cache = tempfile::tempdir().unwrap();
        let played = write_rom(
            &cache.path().join("1"),
            "played.sfc",
            Duration::from_secs(60),
        );
        let stale = write_rom(
            &cache.path().join("2"),
            "stale.sfc",
            Duration::from_secs(30 * 86_400),
        );

        let sweep = sweep_rom_cache(cache.path(), RETENTION).await.unwrap();

        assert_eq!(sweep.roms, 1);
        assert!(played.exists());
        assert!(!cache.path().join("2").exists());
        assert!(!stale.exists());
    }

    #[tokio::test]
    async fn a_fresh_download_partial_keeps_its_rom_directory_alive() {
        let cache = tempfile::tempdir().unwrap();
        let rom_directory = cache.path().join("42");
        write_rom(&rom_directory, "old.sfc", Duration::from_secs(30 * 86_400));
        // An in-flight download: fresh `.part`, older content file.
        std::fs::write(rom_directory.join(".new.sfc.abc.part"), b"partial").unwrap();

        let sweep = sweep_rom_cache(cache.path(), RETENTION).await.unwrap();

        assert_eq!(sweep, CacheSweep::default());
        assert!(rom_directory.join(".new.sfc.abc.part").exists());
    }

    #[tokio::test]
    async fn removes_a_partial_older_than_a_day_without_evicting_the_rom() {
        let cache = tempfile::tempdir().unwrap();
        let rom_directory = cache.path().join("42");
        let rom = write_rom(&rom_directory, "game.sfc", Duration::from_secs(60));
        let partial = rom_directory.join(".game.sfc.abc.part");
        std::fs::write(&partial, b"partial").unwrap();
        backdate(&partial, Duration::from_secs(3 * 86_400));

        let sweep = sweep_rom_cache(cache.path(), RETENTION).await.unwrap();

        assert_eq!(
            sweep,
            CacheSweep {
                roms: 0,
                partials: 1,
            }
        );
        assert!(!partial.exists());
        assert!(rom.exists());
    }

    #[tokio::test]
    async fn a_stale_partial_does_not_keep_an_unused_rom_alive() {
        let cache = tempfile::tempdir().unwrap();
        let rom_directory = cache.path().join("42");
        write_rom(&rom_directory, "game.sfc", Duration::from_secs(30 * 86_400));
        let partial = rom_directory.join(".game.sfc.abc.part");
        std::fs::write(&partial, b"partial").unwrap();
        backdate(&partial, Duration::from_secs(3 * 86_400));

        let sweep = sweep_rom_cache(cache.path(), RETENTION).await.unwrap();

        assert_eq!(
            sweep,
            CacheSweep {
                roms: 1,
                partials: 1,
            }
        );
        assert!(!rom_directory.exists());
    }

    #[tokio::test]
    async fn evicts_an_empty_rom_directory_older_than_the_retention_window() {
        let cache = tempfile::tempdir().unwrap();
        let rom_directory = cache.path().join("42");
        std::fs::create_dir_all(&rom_directory).unwrap();
        backdate(&rom_directory, Duration::from_secs(30 * 86_400));

        let sweep = sweep_rom_cache(cache.path(), RETENTION).await.unwrap();

        assert_eq!(sweep.roms, 1);
        assert!(!rom_directory.exists());
    }

    #[tokio::test]
    async fn a_missing_cache_directory_is_not_an_error() {
        let cache = tempfile::tempdir().unwrap();

        let sweep = sweep_rom_cache(&cache.path().join("absent"), RETENTION)
            .await
            .unwrap();

        assert_eq!(sweep, CacheSweep::default());
    }

    #[tokio::test]
    async fn a_cache_hit_refreshes_the_timestamp_the_janitor_measures() {
        let cache = tempfile::tempdir().unwrap();
        let rom = write_rom(cache.path(), "game.sfc", Duration::from_secs(30 * 86_400));

        touch_cached_rom(&rom).await;

        let modified = std::fs::metadata(&rom).unwrap().modified().unwrap();
        assert!(modified > SystemTime::now() - Duration::from_secs(60));
    }

    #[test]
    fn every_pkgbuild_optdepend_core_has_a_platform_mapping() {
        let cores_with_mappings: std::collections::HashSet<&str> = CORE_BY_PLATFORM_SLUG
            .iter()
            .map(|(_, core)| *core)
            .collect();
        for core in [
            "fceumm_libretro.so",
            "snes9x_libretro.so",
            "genesis_plus_gx_libretro.so",
            "picodrive_libretro.so",
            "mgba_libretro.so",
            "mupen64plus_next_libretro.so",
            "mednafen_psx_libretro.so",
            "play_libretro.so",
            "ppsspp_libretro.so",
            "desmume_libretro.so",
            "dolphin_libretro.so",
            "flycast_libretro.so",
            "kronos_libretro.so",
            "mednafen_pce_fast_libretro.so",
            "mednafen_supergrafx_libretro.so",
            "mame_libretro.so",
            "scummvm_libretro.so",
        ] {
            assert!(
                cores_with_mappings.contains(core),
                "{core} is packaged in PKGBUILD but has no platform slug mapped to it"
            );
        }
    }

    #[tokio::test]
    async fn rejects_a_platform_slug_with_no_configured_core() {
        let error = resolve_core_path("some-platform-nobody-mapped-yet")
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            RetroLaunchError::UnsupportedPlatform { .. }
        ));
    }

    #[test]
    fn accepts_a_single_rom_content_filename() {
        assert!(validate_content_filename("game (USA).chd").is_ok());
    }

    #[test]
    fn rejects_rom_content_paths_and_urls() {
        for unsafe_name in [
            "../game.chd",
            "/tmp/game.chd",
            "disc/game.chd",
            "https://example.com/game.chd",
        ] {
            assert!(matches!(
                validate_content_filename(unsafe_name),
                Err(RetroLaunchError::InvalidContentFileName)
            ));
        }
    }
}
