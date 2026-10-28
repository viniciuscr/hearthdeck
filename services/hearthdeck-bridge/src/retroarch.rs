//! Launching games under RetroArch: path trust boundaries, the managed
//! config Hearthdeck re-asserts every run, and the per-game save tree.
//!
//! The joypad autoconfig profiles a launch seeds live in `profiles`.

use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use tracing::info;

use crate::systemd::{LaunchedApplication, launch_with_systemd};

mod profiles;

/// Directories pacman installs libretro cores into on Arch. A core path must
/// resolve under one of these before the bridge will exec it; this is the
/// on-demand/curated-core-install question (see
/// docs/retroarch-integration.md) staying out of the bridge's trust
/// boundary regardless of how a core got there.
const RETRO_CORE_DIRECTORIES: [&str; 1] = ["/usr/lib/libretro"];

/// Where Arch's `libretro-shaders-slang` package installs its `.slangp`
/// presets. Hearthdeck only ever points RetroArch at an absolute preset
/// path under here, never at a user's own shader directory, so the applied
/// look is the same on every install and cannot drift from a desktop
/// RetroArch setup.
const RETRO_SHADER_DIRECTORY: &str = "/usr/share/libretro/shaders/shaders_slang";

/// Video driver Hearthdeck pins for a launch unless the core overrides it.
///
/// RetroArch's compiled default is `gl`. That is wrong for Hearthdeck in two
/// ways: `gl` cannot load slang shader presets at all, and it behaves
/// differently for hardware-rendered cores under the Gamescope/KMS sessions
/// Hearthdeck runs in. `vulkan` is the natural fit for Gamescope (itself a
/// Vulkan compositor) and works for every core in `retro.rs`'s table except the
/// ones named in [`VIDEO_DRIVER_BY_CORE`]. Pinned unconditionally so a launch's
/// behaviour does not depend on whether a shader happened to apply.
const RETRO_VIDEO_DRIVER_DEFAULT: &str = "vulkan";

/// Per-core driver overrides, keyed on the core filename like
/// `SHADER_PRESET_BY_CORE`.
///
/// Only Flycast is here. Its libretro Vulkan renderer has documented segfaults
/// (flyinghead/flycast#2082 and #2442, the latter right after the
/// resolution-change `SET_SYSTEM_AV_INFO` an FMV-to-gameplay transition
/// produces), so it is pinned to `glcore` -- the modern GL driver, which keeps
/// hardware rendering and a slang-capable context while avoiding the buggy
/// Vulkan path.
///
/// Dolphin was tried on `glcore` too, on the theory that its Vulkan renderer
/// was the GameCube black screen. It was not: `glcore` black-screened the same
/// way on real hardware, so the driver is not the cause and Dolphin stays on
/// the default rather than carrying an override that was never shown to help.
const VIDEO_DRIVER_BY_CORE: &[(&str, &str)] = &[("flycast_libretro.so", "glcore")];

/// Per-core shader preset, relative to `RETRO_SHADER_DIRECTORY`.
///
/// Keyed on the core filename rather than the platform: `retro.rs` already
/// owns the platform→core decision, and several consoles share one core
/// (Dolphin is GameCube and Wii; Genesis Plus GX covers Mega Drive, Master
/// System, Game Gear and Sega CD). The split below is the practical one -
/// CRT-era 2D consoles get a CRT mask, handhelds get an LCD grid, and the
/// 3D-era cores deliberately get no entry so they render unfiltered. If the
/// preset file is absent (shaders package not installed) the launch simply
/// proceeds unshaded, so this can never hard-fail a game.
const SHADER_PRESET_BY_CORE: &[(&str, &str)] = &[
    // CRT-era home consoles.
    ("fceumm_libretro.so", "crt/crt-guest-advanced.slangp"),
    ("snes9x_libretro.so", "crt/crt-guest-advanced.slangp"),
    (
        "genesis_plus_gx_libretro.so",
        "crt/crt-guest-advanced.slangp",
    ),
    ("picodrive_libretro.so", "crt/crt-guest-advanced.slangp"),
    ("mednafen_psx_libretro.so", "crt/crt-guest-advanced.slangp"),
    (
        "mupen64plus_next_libretro.so",
        "crt/crt-guest-advanced.slangp",
    ),
    ("parallel_n64_libretro.so", "crt/crt-guest-advanced.slangp"),
    (
        "mednafen_pce_fast_libretro.so",
        "crt/crt-guest-advanced.slangp",
    ),
    (
        "mednafen_supergrafx_libretro.so",
        "crt/crt-guest-advanced.slangp",
    ),
    ("kronos_libretro.so", "crt/crt-guest-advanced.slangp"),
    ("mame_libretro.so", "crt/crt-guest-advanced.slangp"),
    // Handhelds: an LCD pixel grid instead of a CRT mask.
    ("mgba_libretro.so", "handheld/lcd-grid-v2.slangp"),
    ("gambatte_libretro.so", "handheld/lcd-grid-v2.slangp"),
    ("desmume_libretro.so", "handheld/lcd-grid-v2.slangp"),
    ("melonds_libretro.so", "handheld/lcd-grid-v2.slangp"),
    ("ppsspp_libretro.so", "handheld/lcd-grid-v2.slangp"),
];

/// Guide/Home is Hearthdeck's button - it opens the quick-menu overlay - so
/// RetroArch's single-button menu toggle is left unbound. Its pad route into
/// the menu is the combo below instead.
///
/// This line and the profile rewriting in `retro_profiles` are two halves of
/// one fix, because RetroArch runs joypad autoconfig when the pad is
/// *detected*, after it has read this file: a Guide binding that reached a pad
/// from an autoconfig profile survives anything written here. The profiles are
/// what actually keep Guide unbound; this is the belt to that braces, and
/// covers a pad autoconfigured from a profile outside Hearthdeck's directory
/// (say, one left over from a desktop RetroArch install).
const RETRO_MENU_TOGGLE_UNBOUND: &str = "input_menu_toggle_btn = \"nul\"";

/// RetroArch's other route into its menu, pinned for every launch: the value is
/// an index into RetroArch's own `input_combo_type` enum
/// (`input/input_defines.h` upstream), where `4` is `INPUT_COMBO_START_SELECT`
/// (Start + Select).
///
/// Set explicitly rather than left alone, because the default is
/// `INPUT_COMBO_NONE` on desktop Linux - so taking Guide away without this
/// would leave the pad no way into the menu at all. Start + Select needs no
/// Guide, and needs no button a core uses for anything unusual, which is what
/// lets it coexist with Hearthdeck's claim on the Guide button.
const RETRO_MENU_TOGGLE_COMBO: &str = "input_menu_toggle_gamepad_combo = \"4\"";

/// The preset to apply for a core, if the shaders package is installed.
/// Split from the filesystem check so the lookup is unit-testable against a
/// temp directory, mirroring `validate_retro_core_path_in`.
fn shader_preset_for_core(core_path: &Path) -> Option<PathBuf> {
    shader_preset_for_core_in(core_path, Path::new(RETRO_SHADER_DIRECTORY))
}

fn shader_preset_for_core_in(core_path: &Path, shader_directory: &Path) -> Option<PathBuf> {
    let core_filename = core_path.file_name()?.to_str()?;
    let (_, preset) = SHADER_PRESET_BY_CORE
        .iter()
        .find(|(core, _)| *core == core_filename)?;
    let preset_path = shader_directory.join(preset);
    preset_path.is_file().then_some(preset_path)
}

/// The video driver to pin for a core: its override if it has one, otherwise
/// the Hearthdeck default. Returns `&'static str` because every driver name is
/// a compile-time literal, so the caller can hold it for the rest of the
/// launch without owning it.
fn video_driver_for_core(core_path: &Path) -> &'static str {
    core_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|core_filename| {
            VIDEO_DRIVER_BY_CORE
                .iter()
                .find(|(core, _)| *core == core_filename)
                .map(|(_, driver)| *driver)
        })
        .unwrap_or(RETRO_VIDEO_DRIVER_DEFAULT)
}

/// Launches a RetroArch core against a ROM the daemon has already resolved
/// and cached locally. Mirrors `launch_application`'s discipline: the bridge
/// re-validates both paths itself rather than trusting the daemon's copy,
/// and RetroArch is exec'd directly as the transient unit's main process
/// (not handed off through a URI/IPC scheme the way Heroic is), so Kiosk
/// session tracking works the same way it does for desktop apps.
pub(crate) async fn launch_retro_game(
    core_path: &str,
    rom_path: &str,
    session_id: &str,
) -> Result<LaunchedApplication> {
    let core_path = validate_retro_core_path(core_path)?;
    let rom_path = validate_retro_rom_path(rom_path)?;
    // Resolved once and threaded through the managed config below. `None`
    // (no preset for this core, or the shaders package is not installed) is
    // the normal unshaded launch, not an error.
    let shader_preset = shader_preset_for_core(&core_path);
    let video_driver = video_driver_for_core(&core_path);
    let config_directory = retro_config_directory();
    tokio::fs::create_dir_all(&config_directory)
        .await
        .context("could not create Hearthdeck's RetroArch config directory")?;
    // Both directories are created before RetroArch reads the config: a
    // `savefile_directory` or `savestate_directory` that does not exist is
    // *ignored* (RetroArch logs one warning and silently falls back to its own
    // default), so writing the config alone would leave saves wherever the
    // desktop install keeps them on the first play of every game.
    let save_directories = create_save_directories(&retro_save_root(), &rom_path).await?;
    let config_path = config_directory.join("retroarch.cfg");
    // Regenerated on every launch so the settings Hearthdeck enforces can
    // never go stale from a previous session's in-game changes (the Knulli
    // strategy: the launcher rewrites what it must enforce each run, while
    // the persistent `retroarch.cfg` keeps user/RetroAchievements config
    // that has to survive). RetroArch layers it over `retroarch.cfg` via
    // `--appendconfig`, which has higher priority than `-c`.
    let managed_path = config_directory.join("retroarch.launch.cfg");
    let autoconfig_directory = config_directory.join("autoconfig");
    // RetroArch stores profiles under `<joypad_autoconfig_dir>/<input
    // driver>/`, mirroring how the upstream autoconfig package installs them
    // (`/usr/share/libretro/autoconfig/udev/`). Arch's `retroarch` package
    // ships none of those files, so without seeding, a Hearthdeck-managed
    // install has an empty directory and every pad is reported as "detected
    // but not configured" (the symptom this seeding fixes).
    let autoconfig_udev_directory = autoconfig_directory.join("udev");
    tokio::fs::create_dir_all(&autoconfig_udev_directory)
        .await
        .context("could not create Hearthdeck's RetroArch autoconfig directory")?;
    let seeded = profiles::seed_retroarch_profiles(&autoconfig_udev_directory)
        .await
        .context("could not seed Hearthdeck's RetroArch autoconfig profiles")?;
    if seeded > 0 {
        info!(seeded, "seeded RetroArch joypad autoconfig profiles");
    }
    tokio::fs::write(
        &managed_path,
        retroarch_managed_config(
            &autoconfig_directory,
            &save_directories,
            shader_preset.as_deref(),
            video_driver,
        ),
    )
    .await
    .context("could not write Hearthdeck's managed RetroArch config")?;
    if let Some(preset) = shader_preset.as_deref() {
        info!(preset = %preset.display(), "applying RetroArch shader preset");
    }
    let unit_name = format!("hearthdeck-app-{session_id}.service");
    launch_with_systemd(
        &unit_name,
        retroarch_command_args(&config_path, &managed_path, &core_path, &rom_path),
        None,
    )
    .await
}

/// Settings Hearthdeck re-asserts at the start of every RetroArch launch.
/// Kept as a plain string builder so the exact contents are unit-testable
/// without touching the filesystem.
fn retroarch_managed_config(
    autoconfig_directory: &Path,
    save_directories: &RetroSaveDirectories,
    shader_preset: Option<&Path>,
    _video_driver: &str,
) -> String {
    let mut config = [
        // Start in fullscreen, never a desktop window. `video_windowed_fullscreen`
        // keeps it borderless on the current output instead of switching
        // modes, which is what compositors (Gamescope, cosmic-comp) expect.
        "video_fullscreen = \"true\"",
        "video_windowed_fullscreen = \"true\"",
        // Bind the physical pad Hearthdeck sees to Player 1 explicitly, so a
        // second device (e.g. the input service's virtual controller) cannot
        // take RetroPad slot 1 away from it.
        "input_player1_joypad_index = \"0\"",
        "input_autodetect_enable = \"true\"",
        // Lock the joypad driver to udev: RetroArch looks for autoconfig
        // profiles under `<joypad_autoconfig_dir>/<driver>/`, and Hearthdeck
        // only seeds the `udev/` subdirectory, so another driver would find an
        // empty folder and report every pad "not configured" again.
        "input_joypad_driver = \"udev\"",
        // Guide/Home belongs to Hearthdeck, so the menu moves to a Start combo.
        // Both constants carry the reasoning; neither is worth much without
        // the other.
        RETRO_MENU_TOGGLE_UNBOUND,
        RETRO_MENU_TOGGLE_COMBO,
    ]
    .join("\n");
    // Point RetroArch at a Hearthdeck-owned autoconfig directory. RetroArch
    // resolves its autoconfig dir from its own default config location, not
    // from the `-c` path Hearthdeck passes, so without this line it looks in
    // `~/.config/retroarch/autoconfig` (empty on a machine that only runs
    // RetroArch through Hearthdeck) and every pad shows up "not configured"
    // even though it is detected. `launch_retro_game` seeds the bundled udev
    // profiles into `.../udev/` before RetroArch starts, so the override is
    // backed by real profiles rather than an empty directory. The wizard's
    // "Save Controller Profile" writes into Hearthdeck's config dir too,
    // where it persists across launches and is safe from the managed-config
    // reset.
    config.push('\n');
    config.push_str(&format!(
        "joypad_autoconfig_dir = \"{}\"\n",
        autoconfig_directory.display()
    ));
    // One folder per game under a root the user can point a backup or cloud
    // sync client at without knowing anything about RetroArch.
    //
    // The paths are pinned here rather than left to `sort_*`, because neither
    // RetroArch sort flag produces a per-game folder: `sort_savefiles_enable`
    // appends the *core's* library name, and `sort_savefiles_by_content_enable`
    // appends the ROM's parent directory name, which inside RomM's library is
    // the platform folder. All four sort flags are still written out as
    // `false` so a value saved from RetroArch's own menu cannot file this
    // launch's saves somewhere else. The `*_in_content_dir` flags matter more:
    // `true` would write saves next to the ROM, i.e. into RomM's library mount.
    config.push_str(&format!(
        "savefile_directory = \"{}\"\n\
         savestate_directory = \"{}\"\n\
         savefiles_in_content_dir = \"false\"\n\
         savestates_in_content_dir = \"false\"\n\
         sort_savefiles_enable = \"false\"\n\
         sort_savefiles_by_content_enable = \"false\"\n\
         sort_savestates_enable = \"false\"\n\
         sort_savestates_by_content_enable = \"false\"\n",
        save_directories.saves.display(),
        save_directories.states.display(),
    ));
    // Pinned for every launch, not just shaded ones: RetroArch's default `gl`
    // driver cannot load slang presets and mis-sizes hardware-rendered cores
    // under Gamescope. Resolved per core because Flycast needs `glcore`; see
    // `RETRO_VIDEO_DRIVER_DEFAULT` and `VIDEO_DRIVER_BY_CORE`.
    // config.push_str(&format!("video_driver = \"{video_driver}\"\n"));
    if let Some(preset) = shader_preset {
        config.push_str("video_shader_enable = \"true\"\n");
        config.push_str(&format!("video_shader = \"{}\"\n", preset.display()));
        config.push_str(&format!(
            "video_shader_dir = \"{RETRO_SHADER_DIRECTORY}\"\n"
        ));
    }
    config
}

fn retroarch_command_args(
    config_path: &Path,
    managed_path: &Path,
    core_path: &Path,
    rom_path: &Path,
) -> Vec<OsString> {
    vec![
        OsString::from("retroarch"),
        OsString::from("-c"),
        config_path.as_os_str().to_owned(),
        OsString::from("--appendconfig"),
        managed_path.as_os_str().to_owned(),
        OsString::from("-L"),
        core_path.as_os_str().to_owned(),
        rom_path.as_os_str().to_owned(),
    ]
}

/// Hearthdeck's own RetroArch config directory, never the user's default
/// `~/.config/retroarch`. Keeping it separate means RetroAchievements login,
/// input configuration, and video settings persist across every launch
/// without fighting a separately configured desktop RetroArch installation
/// on a dual-purpose machine.
fn retro_config_directory() -> PathBuf {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|path| PathBuf::from(path).join(".config")));
    config_home
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("hearthdeck/retroarch")
}

/// Root of the save/state tree Hearthdeck pins for every launch:
/// `<root>/saves/<game>/` and `<root>/states/<game>/`.
///
/// Under the XDG data home by default, not the config directory above: these
/// are user data, and backing them up should not mean backing up settings.
/// `HEARTHDECK_RETRO_SAVE_ROOT` replaces the whole root, which is how a box
/// points the tree straight at a Dropbox or pCloud folder instead of relying on
/// a symlink inside the provider's directory.
const RETRO_SAVE_ROOT_ENV: &str = "HEARTHDECK_RETRO_SAVE_ROOT";

fn retro_save_root() -> PathBuf {
    retro_save_root_from(
        env::var_os(RETRO_SAVE_ROOT_ENV).map(PathBuf::from),
        env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    )
}

/// Split from the environment so the override-then-XDG chain is unit-testable
/// without mutating process environment from a parallel test.
fn retro_save_root_from(
    configured: Option<PathBuf>,
    data_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PathBuf {
    if let Some(configured) = configured.filter(|path| !path.as_os_str().is_empty()) {
        return configured;
    }
    data_home
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| home.map(|path| path.join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("hearthdeck/retroarch")
}

/// Where one launch's saves and save states go.
///
/// One folder per content file, named after the ROM's stem, so the synced tree
/// reads like a game list and a state and its save stay together. The folder
/// deliberately says nothing about the core: a save follows the game, not the
/// emulator, so changing which core runs a ROM does not orphan it.
#[derive(Debug, PartialEq, Eq)]
struct RetroSaveDirectories {
    saves: PathBuf,
    states: PathBuf,
}

impl RetroSaveDirectories {
    fn for_rom(save_root: &Path, rom_path: &Path) -> Self {
        let game = save_folder_name(rom_path);
        Self {
            saves: save_root.join("saves").join(&game),
            states: save_root.join("states").join(game),
        }
    }
}

/// Creates the directories a launch pins, returning them for the managed config.
///
/// RetroArch validates `savefile_directory` and `savestate_directory` when it
/// reads its config and *ignores* one that does not exist, falling back to its
/// own default with a single warning in the journal. So the directories have to
/// exist before the config is written, not when the first save happens.
async fn create_save_directories(
    save_root: &Path,
    rom_path: &Path,
) -> Result<RetroSaveDirectories> {
    let directories = RetroSaveDirectories::for_rom(save_root, rom_path);
    tokio::fs::create_dir_all(&directories.saves)
        .await
        .context("could not create this game's RetroArch save directory")?;
    tokio::fs::create_dir_all(&directories.states)
        .await
        .context("could not create this game's RetroArch save-state directory")?;
    Ok(directories)
}

/// The folder a ROM's saves and states go in: its file stem, with the bytes
/// that would break a RetroArch config line replaced by `_`.
///
/// The name is not the bridge's to trust — it arrives from RomM's metadata, and
/// `validate_content_filename` in the daemon accepts any single path component,
/// including quotes, backslashes and newlines. Those are harmless as a path
/// (the component cannot contain `/`) but not as an interpolated
/// `savefile_directory = "…"` value, where a quote ends the value early and a
/// newline starts a new key. Everything else - spaces, brackets, non-ASCII - is
/// left alone, so the tree still reads like the game list.
fn save_folder_name(rom_path: &Path) -> OsString {
    let stem = rom_path
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or(OsStr::new("game"));
    let sanitized: Vec<u8> = stem
        .as_bytes()
        .iter()
        .map(|byte| match byte {
            b'"' | b'\\' => b'_',
            0x00..=0x1f | 0x7f => b'_',
            byte => *byte,
        })
        .collect();
    OsString::from_vec(sanitized)
}

/// The directory Hearthdeck caches ROMs fetched from RomM into. The bridge
/// only launches ROM files that resolve under this directory; the nested
/// layout inside it is the daemon's concern.
/// Where RomM keeps the roms it serves, on this host.
///
/// Deliberately mirrors the daemon's `config::RommPaths` derivation instead of
/// taking a root over the socket: the bridge re-checks the rom it is asked to
/// launch against a root it resolves itself, so a mismatch between the two
/// fails safe as a rejected launch rather than widening what can be launched.
/// RomM's compose file sits at the root of its data directory, beside the
/// `library/` and `resources/` mounts.
fn retro_rom_library_root() -> PathBuf {
    if let Some(overridden) =
        env::var_os("HEARTHDECK_ROMM_LIBRARY_ROOT").filter(|path| !path.is_empty())
    {
        return PathBuf::from(overridden);
    }
    // Derive the root exactly as the daemon does rather than re-implementing it:
    // the bridge re-checks a rom against this root, and the two must agree on where
    // RomM's library is, not merely each be plausible.
    let compose_file = env::var_os("ROMM_COMPOSE_FILE")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(hearthdeck_protocol::paths::DEFAULT_ROMM_COMPOSE_FILE));
    hearthdeck_protocol::paths::romm_data_root(Some(&compose_file)).join("library")
}

fn validate_retro_core_path(core_path: &str) -> Result<PathBuf> {
    let allowed_directories: Vec<&Path> = RETRO_CORE_DIRECTORIES.iter().map(Path::new).collect();
    validate_retro_core_path_in(core_path, &allowed_directories)
}

fn validate_retro_core_path_in(core_path: &str, allowed_directories: &[&Path]) -> Result<PathBuf> {
    let path = Path::new(core_path);
    anyhow::ensure!(path.is_absolute(), "retro core path must be absolute");
    let canonical = fs::canonicalize(path).context("retro core does not exist")?;
    anyhow::ensure!(
        canonical
            .extension()
            .is_some_and(|extension| extension == "so"),
        "retro core must be a shared library"
    );
    anyhow::ensure!(
        allowed_directories
            .iter()
            .filter_map(|directory| fs::canonicalize(directory).ok())
            .any(|directory| canonical.starts_with(directory)),
        "retro core is not in an allowlisted cores directory"
    );
    Ok(canonical)
}

fn validate_retro_rom_path(rom_path: &str) -> Result<PathBuf> {
    let library_root = fs::canonicalize(retro_rom_library_root())
        .context("RomM library directory does not exist")?;
    validate_retro_rom_path_in(rom_path, &library_root)
}

fn validate_retro_rom_path_in(rom_path: &str, cache_root: &Path) -> Result<PathBuf> {
    let path = Path::new(rom_path);
    anyhow::ensure!(path.is_absolute(), "rom path must be absolute");
    let canonical = fs::canonicalize(path).context("cached rom does not exist")?;
    anyhow::ensure!(
        canonical.starts_with(cache_root),
        "rom path is not inside the Hearthdeck rom cache"
    );
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use super::{
        RETRO_VIDEO_DRIVER_DEFAULT, RetroSaveDirectories, SHADER_PRESET_BY_CORE,
        VIDEO_DRIVER_BY_CORE, create_save_directories, retro_save_root_from,
        retroarch_command_args, retroarch_managed_config, shader_preset_for_core_in,
        validate_retro_core_path_in, validate_retro_rom_path_in, video_driver_for_core,
    };

    /// The save/state directories a launch would get for a ROM in RomM's
    /// library, so config tests do not restate the layout.
    fn test_save_directories() -> RetroSaveDirectories {
        RetroSaveDirectories::for_rom(
            Path::new("/home/user/.local/share/hearthdeck/retroarch"),
            Path::new("/mnt/romM/library/roms/snes/Zelda - A Link to the Past (USA).sfc"),
        )
    }

    #[test]
    fn accepts_a_core_inside_an_allowlisted_directory() {
        let cores_directory = tempfile::tempdir().unwrap();
        let core_path = cores_directory.path().join("snes9x_libretro.so");
        std::fs::write(&core_path, b"").unwrap();

        let resolved =
            validate_retro_core_path_in(core_path.to_str().unwrap(), &[cores_directory.path()])
                .unwrap();

        assert_eq!(
            resolved,
            std::fs::canonicalize(&core_path).unwrap(),
            "resolved core path should match the canonicalized input"
        );
    }

    #[test]
    fn rejects_a_core_outside_every_allowlisted_directory() {
        let cores_directory = tempfile::tempdir().unwrap();
        let other_directory = tempfile::tempdir().unwrap();
        let core_path = other_directory.path().join("snes9x_libretro.so");
        std::fs::write(&core_path, b"").unwrap();

        assert!(
            validate_retro_core_path_in(core_path.to_str().unwrap(), &[cores_directory.path()])
                .is_err()
        );
    }

    #[test]
    fn rejects_a_core_that_is_not_a_shared_library() {
        let cores_directory = tempfile::tempdir().unwrap();
        let core_path = cores_directory.path().join("snes9x_libretro.txt");
        std::fs::write(&core_path, b"").unwrap();

        assert!(
            validate_retro_core_path_in(core_path.to_str().unwrap(), &[cores_directory.path()])
                .is_err()
        );
    }

    #[test]
    fn rejects_a_core_path_that_does_not_exist() {
        let cores_directory = tempfile::tempdir().unwrap();
        let core_path = cores_directory.path().join("missing_libretro.so");

        assert!(
            validate_retro_core_path_in(core_path.to_str().unwrap(), &[cores_directory.path()])
                .is_err()
        );
    }

    #[test]
    fn accepts_a_rom_inside_romms_library() {
        let library_root = tempfile::tempdir().unwrap();
        let rom_path = library_root.path().join("42.sfc");
        std::fs::write(&rom_path, b"").unwrap();
        let library_root = std::fs::canonicalize(library_root.path()).unwrap();

        let resolved =
            validate_retro_rom_path_in(rom_path.to_str().unwrap(), &library_root).unwrap();

        assert_eq!(resolved, std::fs::canonicalize(&rom_path).unwrap());
    }

    #[test]
    fn rejects_a_rom_outside_romms_library() {
        let library_root = tempfile::tempdir().unwrap();
        let other_directory = tempfile::tempdir().unwrap();
        let rom_path = other_directory.path().join("42.sfc");
        std::fs::write(&rom_path, b"").unwrap();
        let library_root = std::fs::canonicalize(library_root.path()).unwrap();

        assert!(validate_retro_rom_path_in(rom_path.to_str().unwrap(), &library_root).is_err());
    }

    #[test]
    fn rejects_a_rom_path_that_does_not_exist() {
        let library_root = tempfile::tempdir().unwrap();
        let rom_path = library_root.path().join("missing.sfc");
        let library_root = std::fs::canonicalize(library_root.path()).unwrap();

        assert!(validate_retro_rom_path_in(rom_path.to_str().unwrap(), &library_root).is_err());
    }

    #[test]
    fn managed_retroarch_config_forces_fullscreen_and_player_one() {
        let autoconfig_directory = tempfile::tempdir().unwrap();
        let config = retroarch_managed_config(
            autoconfig_directory.path(),
            &test_save_directories(),
            None,
            "vulkan",
        );

        assert!(config.contains("video_fullscreen = \"true\""));
        assert!(config.contains("video_windowed_fullscreen = \"true\""));
        assert!(config.contains("input_player1_joypad_index = \"0\""));
        assert!(config.contains("input_joypad_driver = \"udev\""));
        assert!(config.contains(&format!(
            "joypad_autoconfig_dir = \"{}\"",
            autoconfig_directory.path().display()
        )));
        assert!(config.ends_with('\n'));
    }

    #[test]
    fn managed_retroarch_config_pins_a_per_game_save_and_state_directory() {
        let autoconfig_directory = tempfile::tempdir().unwrap();
        let save_directories = test_save_directories();

        let config = retroarch_managed_config(
            autoconfig_directory.path(),
            &save_directories,
            None,
            "vulkan",
        );

        // One folder per game, named after the ROM - not the core, and not the
        // platform folder RomM files it in.
        assert_eq!(
            save_directories.saves.as_path(),
            Path::new(
                "/home/user/.local/share/hearthdeck/retroarch/saves/Zelda - A Link to the Past (USA)"
            )
        );
        assert_eq!(
            save_directories.states.as_path(),
            Path::new(
                "/home/user/.local/share/hearthdeck/retroarch/states/Zelda - A Link to the Past (USA)"
            )
        );
        assert!(config.contains(&format!(
            "savefile_directory = \"{}\"",
            save_directories.saves.display()
        )));
        assert!(config.contains(&format!(
            "savestate_directory = \"{}\"",
            save_directories.states.display()
        )));

        // Everything that could move a save out of those directories is off:
        // `sort_*_enable` appends a core-name folder, `sort_*_by_content_enable`
        // appends the ROM's parent directory (a platform folder here), and the
        // `*_in_content_dir` flags write into RomM's library mount.
        for key in [
            "savefiles_in_content_dir",
            "savestates_in_content_dir",
            "sort_savefiles_enable",
            "sort_savefiles_by_content_enable",
            "sort_savestates_enable",
            "sort_savestates_by_content_enable",
        ] {
            assert!(
                config.contains(&format!("{key} = \"false\"")),
                "{key} is not pinned off"
            );
        }
    }

    #[test]
    fn retro_save_root_lets_the_configured_root_win() {
        // A box pointing the tree straight at a cloud-sync folder: the override
        // is used as-is, with no `hearthdeck/retroarch` suffix.
        assert_eq!(
            retro_save_root_from(
                Some(PathBuf::from("/home/user/pCloudDrive/Hearthdeck")),
                Some(PathBuf::from("/home/user/.local/share")),
                Some(PathBuf::from("/home/user")),
            ),
            PathBuf::from("/home/user/pCloudDrive/Hearthdeck")
        );
        // Otherwise the XDG data home, not the config directory.
        assert_eq!(
            retro_save_root_from(
                None,
                Some(PathBuf::from("/data/home")),
                Some(PathBuf::from("/home/user")),
            ),
            PathBuf::from("/data/home/hearthdeck/retroarch")
        );
        // An empty variable counts as unset, and without XDG_DATA_HOME the
        // spec's `$HOME/.local/share` fallback applies.
        assert_eq!(
            retro_save_root_from(
                Some(PathBuf::new()),
                None,
                Some(PathBuf::from("/home/user")),
            ),
            PathBuf::from("/home/user/.local/share/hearthdeck/retroarch")
        );
    }

    #[test]
    fn a_rom_name_cannot_break_or_inject_a_config_line() {
        // Names come from RomM, so a quote or a newline in one must not end the
        // `savefile_directory` value early or start a key of its own.
        let save_directories = RetroSaveDirectories::for_rom(
            Path::new("/home/user/.local/share/hearthdeck/retroarch"),
            Path::new("/mnt/romM/library/roms/snes/Game \"(USA)\"\nsort_savefiles_enable.sfc"),
        );
        let config = retroarch_managed_config(
            Path::new("/home/user/.config/hearthdeck/retroarch/autoconfig"),
            &save_directories,
            None,
            "vulkan",
        );

        assert_eq!(config.matches("savefile_directory = ").count(), 1);
        assert_eq!(config.matches("sort_savefiles_enable = ").count(), 1);
        assert!(config.contains("sort_savefiles_enable = \"false\""));
        assert_eq!(
            save_directories.saves.as_path(),
            Path::new(
                "/home/user/.local/share/hearthdeck/retroarch/saves/Game _(USA)__sort_savefiles_enable"
            )
        );
    }

    #[tokio::test]
    async fn a_launch_creates_its_save_directories_before_retroarch_reads_the_config() {
        let save_root = tempfile::tempdir().unwrap();

        let directories = create_save_directories(
            save_root.path(),
            Path::new("/mnt/romM/library/roms/snes/Zelda (USA).sfc"),
        )
        .await
        .unwrap();

        // RetroArch ignores a configured directory that does not exist yet, so
        // these have to be on disk before the managed config is written rather
        // than when the first save happens.
        assert!(directories.saves.is_dir());
        assert!(directories.states.is_dir());
        assert_eq!(
            directories.saves,
            save_root.path().join("saves/Zelda (USA)")
        );
        assert_eq!(
            directories.states,
            save_root.path().join("states/Zelda (USA)")
        );
    }

    /// A packaged install writes saves where the docs say with no `daemon.env`
    /// at all, and the env file still wins (`EnvironmentFile=` overrides
    /// `Environment=`), which is how a box moves the tree to a synced folder.
    #[test]
    fn every_bridge_unit_presets_the_save_root_the_code_defaults_to() {
        let deploy_unit = include_str!("../../../deploy/systemd/hearthdeck-bridge.service");
        let packaged_unit = include_str!("../../../packaging/arch/hearthdeck-bridge.service");
        let env_example = include_str!("../../../deploy/systemd/daemon.env.example");

        let preset = |unit: &str| {
            unit.lines()
                .find_map(|line| line.strip_prefix("Environment=HEARTHDECK_RETRO_SAVE_ROOT="))
                .map(str::to_owned)
        };
        let deployed = preset(deploy_unit).expect("the deploy unit does not preset the save root");
        assert_eq!(
            preset(packaged_unit).as_deref(),
            Some(deployed.as_str()),
            "the packaged unit and the checkout unit disagree on the save root"
        );
        // `%h` is systemd's home specifier; the code resolves the same place
        // from $HOME for a dev run that has no unit at all.
        assert_eq!(
            PathBuf::from(deployed.replacen("%h", "/home/user", 1)),
            retro_save_root_from(None, None, Some(PathBuf::from("/home/user"))),
        );
        // The documented default is the same string, so uncommenting it in
        // daemon.env changes nothing instead of moving saves by surprise.
        assert!(
            env_example.contains(&format!("# HEARTHDECK_RETRO_SAVE_ROOT={deployed}")),
            "daemon.env.example documents a different default than the unit presets"
        );
        // It is a default only if the user's file can override it.
        assert!(deploy_unit.contains("EnvironmentFile=-%h/.config/hearthdeck/daemon.env"));
        assert!(packaged_unit.contains("EnvironmentFile=-%h/.config/hearthdeck/daemon.env"));
    }

    #[test]
    fn managed_retroarch_config_applies_a_shader_preset_when_given_one() {
        let autoconfig_directory = tempfile::tempdir().unwrap();
        let preset =
            Path::new("/usr/share/libretro/shaders/shaders_slang/crt/crt-guest-advanced.slangp");

        let config = retroarch_managed_config(
            autoconfig_directory.path(),
            &test_save_directories(),
            Some(preset),
            "vulkan",
        );

        assert!(config.contains("video_shader_enable = \"true\""));
        assert!(config.contains(&format!("video_shader = \"{}\"", preset.display())));
    }

    /// Enforcement of a video driver was turned off deliberately, so the
    /// managed config must not carry a `video_driver` line at all - only the
    /// shader lines depend on a preset applying.
    #[test]
    fn managed_retroarch_config_leaves_the_video_driver_to_retroarch() {
        let autoconfig_directory = tempfile::tempdir().unwrap();

        let config = retroarch_managed_config(
            autoconfig_directory.path(),
            &test_save_directories(),
            None,
            "vulkan",
        );

        assert!(!config.contains("video_driver = "));
        assert!(!config.contains("video_shader"));
    }

    #[test]
    fn resolves_a_per_core_video_driver_override() {
        // Flycast is the override: its libretro Vulkan renderer segfaults on the
        // FMV-to-gameplay resolution change, so it is pinned to `glcore`.
        assert_eq!(
            video_driver_for_core(Path::new("/usr/lib/libretro/flycast_libretro.so")),
            "glcore"
        );
        // Every other core gets the Hearthdeck default. Dolphin is deliberately
        // not overridden: `glcore` was tried on hardware and black-screened the
        // same as the default, so the driver is not its problem.
        assert_eq!(
            video_driver_for_core(Path::new("/usr/lib/libretro/dolphin_libretro.so")),
            RETRO_VIDEO_DRIVER_DEFAULT
        );
        assert_eq!(
            video_driver_for_core(Path::new("/usr/lib/libretro/snes9x_libretro.so")),
            RETRO_VIDEO_DRIVER_DEFAULT
        );
        // Every override names a real core and a driver that is not the default.
        for (core, driver) in VIDEO_DRIVER_BY_CORE {
            assert!(
                core.ends_with("_libretro.so"),
                "{core} is not a core filename"
            );
            assert_ne!(
                *driver, RETRO_VIDEO_DRIVER_DEFAULT,
                "{core} overrides to the default"
            );
        }
    }

    #[test]
    fn resolves_a_shader_preset_only_when_the_file_exists() {
        let shader_directory = tempfile::tempdir().unwrap();
        let preset = shader_directory
            .path()
            .join("crt/crt-guest-advanced.slangp");
        std::fs::create_dir_all(preset.parent().unwrap()).unwrap();
        std::fs::write(&preset, b"# shader preset\n").unwrap();

        let resolved = shader_preset_for_core_in(
            Path::new("/usr/lib/libretro/snes9x_libretro.so"),
            shader_directory.path(),
        );
        assert_eq!(resolved.as_deref(), Some(preset.as_path()));

        // A core the table deliberately leaves unfiltered (3D-era cores) gets
        // no preset even though the shaders package is installed.
        assert!(
            shader_preset_for_core_in(
                Path::new("/usr/lib/libretro/dolphin_libretro.so"),
                shader_directory.path(),
            )
            .is_none()
        );

        // A mapped core still gets no preset when the shaders package is not
        // installed, so the launch proceeds unshaded rather than failing.
        let empty_directory = tempfile::tempdir().unwrap();
        assert!(
            shader_preset_for_core_in(
                Path::new("/usr/lib/libretro/snes9x_libretro.so"),
                empty_directory.path(),
            )
            .is_none()
        );
    }

    #[test]
    fn every_shader_preset_references_a_known_shader_collection_file() {
        for (core, preset) in SHADER_PRESET_BY_CORE {
            assert!(
                preset.ends_with(".slangp"),
                "{core} maps to {preset}, which is not a slang preset"
            );
        }
    }

    #[test]
    fn retroarch_command_layers_the_managed_config_over_the_base_config() {
        let args = retroarch_command_args(
            Path::new("/home/user/.config/hearthdeck/retroarch/retroarch.cfg"),
            Path::new("/home/user/.config/hearthdeck/retroarch/retroarch.launch.cfg"),
            Path::new("/usr/lib/libretro/snes9x_libretro.so"),
            Path::new("/home/user/.cache/hearthdeck/romm/42/Game.sfc"),
        );

        assert_eq!(
            args,
            vec![
                OsString::from("retroarch"),
                OsString::from("-c"),
                OsString::from("/home/user/.config/hearthdeck/retroarch/retroarch.cfg"),
                OsString::from("--appendconfig"),
                OsString::from("/home/user/.config/hearthdeck/retroarch/retroarch.launch.cfg"),
                OsString::from("-L"),
                OsString::from("/usr/lib/libretro/snes9x_libretro.so"),
                OsString::from("/home/user/.cache/hearthdeck/romm/42/Game.sfc"),
            ]
        );
    }

    #[test]
    fn managed_retroarch_config_moves_the_menu_off_the_guide_button() {
        let autoconfig_directory = tempfile::tempdir().unwrap();

        let config = retroarch_managed_config(
            autoconfig_directory.path(),
            &test_save_directories(),
            None,
            "vulkan",
        );

        // Guide/Home opens Hearthdeck's overlay, so RetroArch must not also use
        // it as a menu button...
        assert!(config.contains("input_menu_toggle_btn = \"nul\""));
        // ...and a Start combo is the menu route the pad keeps instead.
        assert!(config.contains("input_menu_toggle_gamepad_combo = \"4\""));
    }
}
