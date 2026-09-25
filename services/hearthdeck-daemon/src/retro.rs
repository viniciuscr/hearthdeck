//! Resolves a RomM ROM into a launchable RetroArch core + ROM pair.
//!
//! This is Phase 2 of `docs/retroarch-integration.md`: platform-to-core
//! mapping and locating the ROM itself. RomM runs on this machine and reports
//! library-relative paths, so a rom is launched where RomM already keeps it and
//! nothing is copied into a Hearthdeck cache. It does not talk to the bridge and
//! does not track sessions; `api.rs` handles that, the same way it already does
//! for Heroic and desktop-app launches.

use std::path::{Component, Path, PathBuf};

use tokio::fs;

use crate::{
    config::RommPaths,
    diagnostics::{self, RommQueryError},
    settings::SettingsRepository,
};

/// Where pacman installs libretro cores on Arch. Kept in sync with the
/// bridge's own allowlist (`hearthdeck-bridge/src/platform/linux.rs`); the
/// bridge re-validates independently rather than trusting this module's
/// output, so a mismatch here fails safe as a rejected launch, not a
/// security hole.
const CORE_DIRECTORY: &str = "/usr/lib/libretro";

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

/// Cores a ROM's *content* forces regardless of the platform it is filed under,
/// keyed on the file extension.
///
/// Sega 32X is the case that matters. A 32X cartridge is a `.32x` file for an
/// add-on to the Mega Drive, and RomM libraries routinely keep 32X roms in the
/// Mega Drive platform's folder. The platform slug then says `genesis`, which
/// maps to Genesis Plus GX - a core that cannot run 32X at all - so the launch
/// fails with the core looking like the problem. The extension is the only
/// signal that tells a 32X cartridge from a Mega Drive one, so a `.32x` file is
/// routed to PicoDrive whatever platform claims it.
const CORE_BY_CONTENT_EXTENSION: &[(&str, &str)] = &[("32x", "picodrive_libretro.so")];

#[derive(Debug)]
pub enum RetroLaunchError {
    Romm(RommQueryError),
    PlatformNotFound,
    UnsupportedPlatform {
        fs_slug: String,
    },
    CoreNotInstalled {
        core_path: PathBuf,
    },
    RomHasNoContentFile,
    InvalidContentFileName,
    /// The rom is not where RomM's metadata says it is: the library mount is
    /// missing or unreadable, or RomM's own index is stale.
    RomNotOnDisk {
        path: PathBuf,
    },
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
            Self::RomNotOnDisk { path } => {
                write!(formatter, "rom is not on disk at {}", path.display())
            }
        }
    }
}

impl std::error::Error for RetroLaunchError {}

pub struct RetroLaunchPlan {
    pub core_path: PathBuf,
    pub rom_path: PathBuf,
    pub game: diagnostics::RommGame,
}

/// Resolves the core and the ROM's own path for a RomM rom ID.
///
/// The rom is launched where RomM already keeps it: RomM is local and reports a
/// library-relative path, so nothing is copied into a Hearthdeck cache first.
/// Returns only validated local paths; the bridge re-validates the rom and core
/// independently before launch (see
/// `hearthdeck-bridge/src/platform/linux.rs`).
pub async fn prepare_launch(
    settings: &SettingsRepository,
    paths: &RommPaths,
    rom_id: i64,
) -> Result<RetroLaunchPlan, RetroLaunchError> {
    let rom = diagnostics::romm_rom(settings, rom_id)
        .await
        .map_err(RetroLaunchError::Romm)?;
    let rom_path = library_rom_path(paths, &rom).await?;

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

    // The content extension can override the platform: a `.32x` file is a 32X
    // cartridge even when it is filed under the Mega Drive platform.
    let content_name = rom.fs_name.as_deref().unwrap_or_default();
    let core_filename = core_filename_for(&fs_slug, content_name).ok_or_else(|| {
        RetroLaunchError::UnsupportedPlatform {
            fs_slug: fs_slug.clone(),
        }
    })?;
    let core_path = core_path_for(core_filename).await?;
    // Logged because "the wrong core started" is otherwise invisible: the core
    // is chosen here, the launch itself is silent, and the only symptom is the
    // emulator misbehaving.
    tracing::info!(
        platform = %fs_slug,
        content = content_name,
        core = core_filename,
        "resolved RetroArch core for launch"
    );

    Ok(RetroLaunchPlan {
        core_path,
        rom_path,
        game: rom,
    })
}

/// The on-disk path of a rom inside RomM's library.
///
/// Both halves come from RomM's metadata, so neither is trusted: `fs_path` has
/// to be an ordinary relative directory (no absolute paths, no `..`) and
/// `fs_name` a single file name. `fs_name` is deliberately still required to
/// name a *file*: RomM reports a folder there for a multi-file rom, which a core
/// cannot load as content, so that case is refused here rather than failing
/// obscurely inside RetroArch.
async fn library_rom_path(
    paths: &RommPaths,
    rom: &diagnostics::RommGame,
) -> Result<PathBuf, RetroLaunchError> {
    let fs_name = rom
        .fs_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or(RetroLaunchError::RomHasNoContentFile)?;
    validate_content_filename(fs_name)?;

    let mut directory = PathBuf::new();
    let fs_path = rom.fs_path.as_deref().unwrap_or_default();
    for component in Path::new(fs_path.trim_matches('/')).components() {
        match component {
            Component::Normal(part) => directory.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(RetroLaunchError::InvalidContentFileName);
            }
        }
    }

    let rom_path = paths.library_root.join(directory).join(fs_name);
    if !fs::metadata(&rom_path)
        .await
        .is_ok_and(|metadata| metadata.is_file())
    {
        return Err(RetroLaunchError::RomNotOnDisk { path: rom_path });
    }
    Ok(rom_path)
}

/// The libretro core filename for a rom: its content extension if one is known,
/// otherwise the platform slug.
///
/// Content wins over platform on purpose. A platform slug is the weaker signal
/// for anything that is an add-on or shares a folder (32X in a Mega Drive
/// library), and getting it wrong launches a core that cannot run the rom.
fn core_filename_for(fs_slug: &str, content_name: &str) -> Option<&'static str> {
    if let Some(extension) = Path::new(content_name)
        .extension()
        .and_then(|ext| ext.to_str())
        && let Some((_, core)) = CORE_BY_CONTENT_EXTENSION
            .iter()
            .find(|(ext, _)| ext.eq_ignore_ascii_case(extension))
    {
        return Some(core);
    }
    CORE_BY_PLATFORM_SLUG
        .iter()
        .find(|(slug, _)| *slug == fs_slug)
        .map(|(_, core)| *core)
}

async fn core_path_for(core_filename: &str) -> Result<PathBuf, RetroLaunchError> {
    let core_path = Path::new(CORE_DIRECTORY).join(core_filename);
    fs::metadata(&core_path)
        .await
        .map_err(|_| RetroLaunchError::CoreNotInstalled {
            core_path: core_path.clone(),
        })?;
    Ok(core_path)
}

fn validate_content_filename(fs_name: &str) -> Result<(), RetroLaunchError> {
    let mut components = Path::new(fs_name).components();
    if matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none() {
        Ok(())
    } else {
        Err(RetroLaunchError::InvalidContentFileName)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CORE_BY_PLATFORM_SLUG, RetroLaunchError, core_filename_for, core_path_for,
        validate_content_filename,
    };

    /// Every libretro core the PKGBUILD packages must be reachable from at least
    /// one platform slug, or a user who installed that core cannot launch its
    /// games.
    ///
    /// The list is read from the PKGBUILD rather than restated here, so adding a
    /// core package upstream is caught — the previous version iterated a
    /// hand-maintained copy of the list and could not notice.
    #[test]
    fn every_pkgbuild_optdepend_core_has_a_platform_mapping() {
        let cores_with_mappings: std::collections::HashSet<&str> = CORE_BY_PLATFORM_SLUG
            .iter()
            .map(|(_, core)| *core)
            .collect();

        let pkgbuild = include_str!("../../../packaging/arch/PKGBUILD");
        let packages = packaged_libretro_cores(pkgbuild);
        assert!(
            !packages.is_empty(),
            "found no libretro cores in the PKGBUILD optdepends"
        );
        for package in packages {
            let core = core_file_for_package(&package);
            assert!(
                cores_with_mappings.contains(core.as_str()),
                "PKGBUILD packages {package} (core {core}) but no platform slug maps to it"
            );
        }
    }

    /// The `libretro-*` core packages in a PKGBUILD's `optdepends`, minus the ones
    /// that are not emulator cores.
    fn packaged_libretro_cores(pkgbuild: &str) -> Vec<String> {
        let mut packages = Vec::new();
        let mut in_optdepends = false;
        for line in pkgbuild.lines() {
            let line = line.trim();
            if line.starts_with("optdepends=(") {
                in_optdepends = true;
                continue;
            }
            if !in_optdepends {
                continue;
            }
            if line == ")" {
                break;
            }
            // Each entry is `'libretro-name: description'`. The shader preset
            // package shares the prefix but is not an emulator core.
            if let Some(rest) = line.strip_prefix('\'')
                && let Some((name, _)) = rest.split_once(':')
                && name.starts_with("libretro-")
                && !name.contains("shaders")
            {
                packages.push(name.to_owned());
            }
        }
        packages
    }

    /// The `.so` file Arch installs for a `libretro-<name>` package: `<name>` with
    /// dashes as underscores and `_libretro.so` appended, except the `beetle-*`
    /// packages, which bundle the upstream `mednafen_*` cores under a different
    /// name.
    fn core_file_for_package(package: &str) -> String {
        let name = package.strip_prefix("libretro-").unwrap_or(package);
        let name = match name {
            "beetle-psx" => "mednafen_psx",
            "beetle-pce-fast" => "mednafen_pce_fast",
            "beetle-supergrafx" => "mednafen_supergrafx",
            other => other,
        };
        format!("{}_libretro.so", name.replace('-', "_"))
    }

    #[test]
    fn a_platform_slug_with_no_core_and_no_content_override_is_unsupported() {
        assert_eq!(
            core_filename_for("some-platform-nobody-mapped-yet", "game.bin"),
            None
        );
    }

    #[tokio::test]
    async fn a_missing_core_file_is_reported_as_not_installed() {
        let error = core_path_for("definitely_not_installed_libretro.so")
            .await
            .unwrap_err();

        assert!(matches!(error, RetroLaunchError::CoreNotInstalled { .. }));
    }

    #[test]
    fn a_32x_rom_uses_picodrive_even_under_a_genesis_platform() {
        // 32X roms routinely live in the Mega Drive platform, whose slug maps to
        // Genesis Plus GX - a core that cannot run 32X. The extension decides.
        assert_eq!(
            core_filename_for("genesis-slash-megadrive", "Knuckles Chaotix (USA).32x"),
            Some("picodrive_libretro.so")
        );
        // An ordinary Mega Drive rom still gets the platform's core.
        assert_eq!(
            core_filename_for("genesis-slash-megadrive", "Sonic (USA).md"),
            Some("genesis_plus_gx_libretro.so")
        );
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
