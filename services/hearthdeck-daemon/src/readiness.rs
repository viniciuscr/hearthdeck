//! Whether the platforms in the RomM library can actually play: their libretro
//! core on disk, and the BIOS files their emulators look for.
//!
//! Readiness is computed from the same platform-to-core table `retro.rs`
//! resolves launches from, so a green report is exactly what a launch would
//! use. BIOS files are deliberately a separate concern: Hearthdeck never ships
//! firmware and never fetches it from a source the user did not name. The
//! importer copies only the files the platforms *with roms* need, out of a
//! GitHub project the user configures (`HEARTHDECK_BIOS_SOURCE_URL`, laid out
//! like the Libretro BIOS pack convention `bios/<Company>/<Console>/<file>`),
//! and verifies every copy against the identities in `BIOS_SLOTS` before
//! placing it under the RetroArch system directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha1::{Digest, Sha1};

use hearthdeck_protocol::{
    BridgeRequest, BridgeResponse, RetroBiosFile, RetroBiosSyncFailure, RetroBiosSyncReport,
    RetroCoreInstallReport, RetroCoreReadiness, RetroSystemReadiness, paths,
};

use crate::bridge;
use crate::diagnostics::RommPlatform;
use crate::retro;

/// One known-good BIOS dump a slot accepts.
pub(crate) struct KnownBios {
    pub name: &'static str,
    /// Lowercase hex SHA-1 of the dump, where an authoritative identity is
    /// known. Absent for dumps whose identity is only name-deep.
    pub sha1: Option<&'static str>,
    /// Exact file size, where every accepted dump shares one.
    pub size: Option<u64>,
}

/// How a BIOS slot is satisfied, in the user's source tree and on disk.
pub(crate) enum BiosAccept {
    /// One of these dumps satisfies the slot, in preference order.
    Known(&'static [KnownBios]),
    /// Any file with this extension and exact size: the PS2 case, where every
    /// properly dumped BIOS is a 4 MiB `.bin` and the core finds it by scan,
    /// not by name.
    Pattern {
        /// What the UI shows while no file is placed yet.
        label: &'static str,
        extension: &'static str,
        size: u64,
        /// Name prefixes picked first when the source offers several.
        prefer: &'static [&'static str],
        /// Name prefixes skipped while another candidate exists.
        reject: &'static [&'static str],
    },
}

/// One file an emulator in `retro.rs`'s core table reads from the system
/// directory: where to find it in the user's source tree, where it goes, and
/// whether the emulator can work without it.
pub(crate) struct BiosSlot {
    /// RomM platform slugs this slot applies to, same alias breadth as
    /// `retro::CORE_BY_PLATFORM_SLUG`.
    pub platforms: &'static [&'static str],
    /// Folder under the source's `bios/` root, `Company/Console`.
    pub system: &'static str,
    /// Destination folder under the RetroArch system directory; empty is the
    /// root. Lowercase where the core demands it (`pcsx2/bios`).
    pub target_dir: &'static str,
    /// True when nothing runs at all without the file. Region files and
    /// accuracy extras are false: some games run without them.
    pub required: bool,
    pub accept: BiosAccept,
}

const PS1_SLOTTED_PLATFORMS: &[&str] = &["ps", "psx", "ps1", "playstation", "sony-playstation"];
const PS2_SLOTTED_PLATFORMS: &[&str] = &["ps2", "playstation-2", "sony-playstation-2"];
const PCE_SLOTTED_PLATFORMS: &[&str] = &[
    "pce",
    "pcengine",
    "pc-engine",
    "turbografx16",
    "turbografx-16",
    "nec-pc-engine",
    "supergrafx",
    "super-grafx",
];

/// Every BIOS file the cores in `retro.rs` actually read, for the platforms
/// they cover. Identities come from the emulators' own documentation (LRPS2:
/// any properly dumped 4 MiB PS2 BIOS, `system/pcsx2/bios/`; Beetle PSX: the
/// three region files at the system root) and from published dump databases
/// for the SHA-1s.
static BIOS_SLOTS: &[BiosSlot] = &[
    // PlayStation 1. Beetle PSX runs its bundled OpenBIOS without any of
    // these, so each is an accuracy/region extra rather than a hard need.
    BiosSlot {
        platforms: PS1_SLOTTED_PLATFORMS,
        system: "Sony/PlayStation",
        target_dir: "",
        required: false,
        accept: BiosAccept::Known(&[KnownBios {
            name: "scph5500.bin",
            sha1: None,
            size: Some(524_288),
        }]),
    },
    BiosSlot {
        platforms: PS1_SLOTTED_PLATFORMS,
        system: "Sony/PlayStation",
        target_dir: "",
        required: false,
        accept: BiosAccept::Known(&[KnownBios {
            name: "scph5501.bin",
            sha1: None,
            size: Some(524_288),
        }]),
    },
    BiosSlot {
        platforms: PS1_SLOTTED_PLATFORMS,
        system: "Sony/PlayStation",
        target_dir: "",
        required: false,
        accept: BiosAccept::Known(&[KnownBios {
            name: "scph5502.bin",
            sha1: None,
            size: Some(524_288),
        }]),
    },
    // PlayStation 2. LRPS2 finds any properly dumped 4 MiB BIOS by scanning
    // `pcsx2/bios/`, so the slot is a shape, not a name. SCPH-10000 is
    // deprioritized: the core docs discourage it for compatibility.
    BiosSlot {
        platforms: PS2_SLOTTED_PLATFORMS,
        system: "Sony/PlayStation 2",
        target_dir: "pcsx2/bios",
        required: true,
        accept: BiosAccept::Pattern {
            label: "SCPH-*.bin",
            extension: "bin",
            size: 4_194_304,
            prefer: &["SCPH-"],
            reject: &["SCPH-10000"],
        },
    },
    // Sega Saturn. Kronos reads one of the two at the system root.
    BiosSlot {
        platforms: &["saturn", "sega-saturn"],
        system: "Sega/Saturn",
        target_dir: "",
        required: true,
        accept: BiosAccept::Known(&[
            KnownBios {
                name: "sega_101.bin",
                sha1: None,
                size: None,
            },
            KnownBios {
                name: "mpr-17933.bin",
                sha1: None,
                size: None,
            },
        ]),
    },
    // Sega Dreamcast. Flycast boots on its HLE BIOS; these are accuracy.
    BiosSlot {
        platforms: &["dc", "dreamcast", "sega-dreamcast"],
        system: "Sega/Dreamcast",
        target_dir: "dc",
        required: false,
        accept: BiosAccept::Known(&[
            KnownBios {
                name: "dc_boot.bin",
                sha1: Some("8951d1bb219ab2ff8583033d2119c899cc81f18c"),
                size: None,
            },
            KnownBios {
                name: "dc_flash.bin",
                sha1: None,
                size: None,
            },
        ]),
    },
    // Sega CD. Genesis Plus GX has no fallback per region: the game's region
    // file is the one that runs, so each row is a region extra.
    BiosSlot {
        platforms: &["segacd", "sega-cd", "mega-cd", "mega-cd-slash-sega-cd"],
        system: "Sega/Mega CD",
        target_dir: "",
        required: false,
        accept: BiosAccept::Known(&[KnownBios {
            name: "bios_CD_E.bin",
            sha1: Some("f891e0ea651e2232af0c5c4cb46a0cae2ee8f356"),
            size: None,
        }]),
    },
    BiosSlot {
        platforms: &["segacd", "sega-cd", "mega-cd", "mega-cd-slash-sega-cd"],
        system: "Sega/Mega CD",
        target_dir: "",
        required: false,
        accept: BiosAccept::Known(&[KnownBios {
            name: "bios_CD_U.bin",
            sha1: Some("f4f315adcef9b8feb0364c21ab7f0eaf5457f3ed"),
            size: None,
        }]),
    },
    BiosSlot {
        platforms: &["segacd", "sega-cd", "mega-cd", "mega-cd-slash-sega-cd"],
        system: "Sega/Mega CD",
        target_dir: "",
        required: false,
        accept: BiosAccept::Known(&[KnownBios {
            name: "bios_CD_J.bin",
            sha1: Some("4846f448160059a7da0215a5df12ca160f26dd69"),
            size: None,
        }]),
    },
    // PC Engine / TurboGrafx-16 System Card: one slot, because any card
    // version boots CD games. HuCard games run without one.
    BiosSlot {
        platforms: PCE_SLOTTED_PLATFORMS,
        system: "NEC/PC Engine",
        target_dir: "",
        required: false,
        accept: BiosAccept::Known(&[
            KnownBios {
                name: "syscard3.pce",
                sha1: Some("79f5ff55dd10187c7fd7b8daab0b3ffbd1f56a2c"),
                size: None,
            },
            KnownBios {
                name: "syscard3u.pce",
                sha1: Some("d02611d99921986147c753df14c7349b31d71950"),
                size: None,
            },
            KnownBios {
                name: "syscard2.pce",
                sha1: Some("88da02e2503f7c32810f5d93a34849d470742b6d"),
                size: None,
            },
            KnownBios {
                name: "syscard2u.pce",
                sha1: Some("2bea3dac98f84b2f2f469fa77ea720b8770d598d"),
                size: None,
            },
            KnownBios {
                name: "syscard1.pce",
                sha1: Some("a39a66da7de6ba94ab84d04eef7afeec7d4ee66a"),
                size: None,
            },
        ]),
    },
    // Neo Geo. MAME reads neogeo.zip at the system root, kept zipped.
    BiosSlot {
        platforms: &["neogeo", "neo-geo", "snk-neo-geo"],
        system: "SNK/Neo Geo",
        target_dir: "",
        required: true,
        accept: BiosAccept::Known(&[KnownBios {
            name: "neogeo.zip",
            sha1: None,
            size: None,
        }]),
    },
    // Famicom Disk System. Every game on this platform is a disk game.
    BiosSlot {
        platforms: &["fds"],
        system: "Nintendo/Famicom Disk System",
        target_dir: "",
        required: true,
        accept: BiosAccept::Known(&[KnownBios {
            name: "disksys.rom",
            sha1: Some("57fe1bdee955bb48d357e463ccbf129496930b62"),
            size: None,
        }]),
    },
    // Game Gear. Only the SMS mode of Genesis Plus GX reads it.
    BiosSlot {
        platforms: &["gg", "gamegear", "game-gear", "sega-game-gear"],
        system: "Sega/Game Gear",
        target_dir: "",
        required: false,
        accept: BiosAccept::Known(&[KnownBios {
            name: "bios.gg",
            sha1: Some("914aa165e3d879f060be77870d345b60cfeb4ede"),
            size: None,
        }]),
    },
];

/// Where readiness looks: the distro cores directory and the RetroArch system
/// directory Hearthdeck owns.
pub(crate) struct ReadinessPaths {
    pub cores_dir: PathBuf,
    pub system_dir: PathBuf,
    /// Where cores no distro package provides are placed by the sync.
    pub user_cores_dir: PathBuf,
}

impl ReadinessPaths {
    pub(crate) fn discover() -> Self {
        let save_root = paths::retro_save_root();
        Self {
            cores_dir: PathBuf::from(retro::CORE_DIRECTORY),
            system_dir: paths::retro_system_dir(&save_root),
            user_cores_dir: paths::retro_cores_dir(&save_root),
        }
    }
}

/// The BIOS slots a platform's emulators read.
pub(crate) fn bios_slots_for(fs_slug: &str) -> impl Iterator<Item = &'static BiosSlot> {
    BIOS_SLOTS
        .iter()
        .filter(move |slot| slot.platforms.contains(&fs_slug))
}

/// Readiness for every platform RomM reports that has roms: what a launch
/// would resolve today, plus the BIOS files its emulators look for. Platforms
/// without a core mapping are left out — they cannot launch at all, which
/// `LaunchRetroGame` already reports as its own error.
pub(crate) async fn readiness(
    platforms: &[RommPlatform],
    paths: &ReadinessPaths,
) -> Vec<RetroSystemReadiness> {
    let mut report = Vec::new();
    for platform in active_platforms(platforms) {
        let fs_slug = platform_slug(platform);
        let Some(core_filename) = retro::core_filename_for(fs_slug, "") else {
            continue;
        };
        let installed = retro::find_core(
            core_filename,
            [paths.cores_dir.as_path(), paths.user_cores_dir.as_path()],
        )
        .await
        .is_some();
        let mut bios = Vec::new();
        for slot in bios_slots_for(fs_slug) {
            bios.push(bios_file_status(slot, &paths.system_dir).await);
        }
        report.push(RetroSystemReadiness {
            platform: fs_slug.to_owned(),
            display_name: platform
                .display_name
                .clone()
                .unwrap_or_else(|| platform.name.clone()),
            core: RetroCoreReadiness {
                core_filename: core_filename.to_owned(),
                package: retro::core_package_for(core_filename)
                    .unwrap_or_default()
                    .to_owned(),
                installed,
            },
            bios,
        });
    }
    report.sort_by(|left, right| left.display_name.cmp(&right.display_name));
    report
}

/// The distro packages whose cores the active platforms are missing, sorted
/// and deduped. These are the only names an install request may ever carry:
/// they come from this module's own platform-to-core table, never from a
/// client.
pub(crate) fn missing_core_packages(report: &[RetroSystemReadiness]) -> Vec<String> {
    let mut packages: Vec<String> = report
        .iter()
        .filter(|entry| !entry.core.installed)
        .map(|entry| entry.core.package.clone())
        .filter(|package| !package.is_empty())
        .collect();
    packages.sort();
    packages.dedup();
    packages
}

/// Installs the distro packages for the cores the active platforms are
/// missing, through the bridge's polkit-scoped helper. The package names come
/// from [`missing_core_packages`], never from a client, and the bridge
/// re-validates each one before the helper sees it.
pub(crate) async fn install_missing_cores(
    bridge_socket: &Path,
    platforms: &[RommPlatform],
    paths: &ReadinessPaths,
) -> anyhow::Result<RetroCoreInstallReport> {
    let packages = missing_core_packages(&readiness(platforms, paths).await);
    if packages.is_empty() {
        return Ok(RetroCoreInstallReport {
            installed: Vec::new(),
            failed: Vec::new(),
        });
    }
    match bridge::request(bridge_socket, BridgeRequest::InstallRetroCores { packages }).await? {
        BridgeResponse::RetroCoresInstalled { report } => Ok(report),
        BridgeResponse::Error { message, .. } => {
            anyhow::bail!("bridge rejected the core install: {message}")
        }
        _ => anyhow::bail!("bridge answered the core install with an unexpected response"),
    }
}

/// Core files the active platforms are missing that no distro package
/// provides: the sync fetches exactly these from the user's source, one
/// folder over from the BIOS files (`cores/<file name>`). The declared
/// `retro::SOURCE_ONLY_CORES` list gates what may be fetched at all — a core
/// missing both a package and a declaration is a mapping bug, not a fetch
/// candidate.
pub(crate) fn missing_source_cores(report: &[RetroSystemReadiness]) -> Vec<String> {
    let mut files: Vec<String> = report
        .iter()
        .filter(|entry| {
            !entry.core.installed
                && entry.core.package.is_empty()
                && retro::SOURCE_ONLY_CORES.contains(&entry.core.core_filename.as_str())
        })
        .map(|entry| entry.core.core_filename.clone())
        .collect();
    files.sort();
    files.dedup();
    files
}

/// The user's own BIOS repository: a GitHub project whose contents Hearthdeck
/// reads through the GitHub API and copies out of. Never a source Hearthdeck
/// picks — the user names it in `daemon.env`.
#[derive(Clone, Debug)]
pub(crate) struct BiosSource {
    owner: String,
    repo: String,
    branch: Option<String>,
    token: Option<String>,
}

impl BiosSource {
    /// From `HEARTHDECK_BIOS_SOURCE_URL` plus the optional
    /// `HEARTHDECK_BIOS_SOURCE_TOKEN` for a private project. Unset is
    /// [`ReadinessError::NoSource`]: Hearthdeck never picks a source itself.
    pub(crate) fn from_env() -> Result<Self, ReadinessError> {
        let url = std::env::var("HEARTHDECK_BIOS_SOURCE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or(ReadinessError::NoSource)?;
        let token = std::env::var("HEARTHDECK_BIOS_SOURCE_TOKEN")
            .ok()
            .filter(|value| !value.is_empty());
        Self::parse(&url, token)
    }

    /// Accepts `https://github.com/<owner>/<repo>` with an optional
    /// `/tree/<branch>`. Anything else is refused rather than guessed at.
    pub(crate) fn parse(url: &str, token: Option<String>) -> Result<Self, ReadinessError> {
        let trimmed = url.trim().trim_end_matches('/');
        let rest = trimmed
            .strip_prefix("https://github.com/")
            .ok_or(ReadinessError::InvalidSourceUrl)?;
        let mut parts = rest.split('/');
        let owner = parts
            .next()
            .filter(|part| !part.is_empty())
            .ok_or(ReadinessError::InvalidSourceUrl)?;
        let repo = parts
            .next()
            .filter(|part| !part.is_empty())
            .ok_or(ReadinessError::InvalidSourceUrl)?;
        let branch = match (parts.next(), parts.next()) {
            (Some("tree"), Some(branch)) if !branch.is_empty() => Some(branch.to_owned()),
            (None, None) => None,
            _ => return Err(ReadinessError::InvalidSourceUrl),
        };
        Ok(Self {
            owner: owner.to_owned(),
            repo: repo.trim_end_matches(".git").to_owned(),
            branch,
            token,
        })
    }

    fn listing_url(&self, folder: &str) -> String {
        let mut url = format!(
            "https://api.github.com/repos/{}/{}/contents/{}",
            self.owner,
            self.repo,
            encode_path(folder)
        );
        if let Some(branch) = &self.branch {
            url.push_str(&format!("?ref={}", encode_path(branch)));
        }
        url
    }

    fn apply_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let request = request
            .header("User-Agent", "hearthdeck")
            .header("Accept", "application/vnd.github+json");
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }
}

/// What one BIOS sync run did, and the readiness that holds afterwards. Every
/// file fails on its own: one missing dump never stops the rest of the sync.
pub(crate) async fn sync_bios(
    platforms: &[RommPlatform],
    paths: &ReadinessPaths,
    source: &BiosSource,
) -> RetroBiosSyncReport {
    let mut fetcher = SourceFetcher::new(source);
    let mut fetched = Vec::new();
    let mut failed = Vec::new();

    for slot in active_slots(platforms) {
        if matches!(
            slot_state(slot, &paths.system_dir).await,
            SlotState::Present { verified: true, .. }
        ) {
            continue;
        }
        match place_bios(&mut fetcher, slot, &paths.system_dir).await {
            Ok(name) => fetched.push(name),
            Err(failure) => failed.push(failure),
        }
    }
    // Cores no distro package provides come from the same source, one folder
    // over: `cores/<file name>`.
    for core_file in missing_source_cores(&readiness(platforms, paths).await) {
        match place_core(&mut fetcher, &core_file, &paths.user_cores_dir).await {
            Ok(()) => fetched.push(core_file),
            Err(reason) => failed.push(RetroBiosSyncFailure {
                filename: core_file,
                reason,
            }),
        }
    }

    RetroBiosSyncReport {
        fetched,
        failed,
        readiness: readiness(platforms, paths).await,
    }
}

/// Fetches, verifies and places one slot's file; the placed name on success.
async fn place_bios(
    fetcher: &mut SourceFetcher<'_>,
    slot: &BiosSlot,
    system_dir: &Path,
) -> Result<String, RetroBiosSyncFailure> {
    let failure = |filename: &str, reason: String| RetroBiosSyncFailure {
        filename: filename.to_owned(),
        reason,
    };
    let display_name = slot_display_name(slot);
    let folder = format!("bios/{}", slot.system);
    let entries = fetcher
        .listing(&folder)
        .await
        .map_err(|reason| failure(&display_name, reason))?;
    let candidate = pick_candidate(entries, &slot.accept)
        .cloned()
        .ok_or_else(|| {
            failure(
                &display_name,
                format!("no matching file in the source's {folder}"),
            )
        })?;
    let name = canonical_name(slot, &candidate.name);
    let bytes = fetcher
        .download(&candidate)
        .await
        .map_err(|reason| failure(name, reason))?;
    if !file_is_verified(&candidate.name, &bytes, &slot.accept) {
        return Err(failure(
            name,
            "the downloaded file failed verification".to_owned(),
        ));
    }
    write_file(&target_path(slot, name, system_dir), &bytes)
        .await
        .map_err(|reason| failure(name, reason))?;
    Ok(name.to_owned())
}

/// Fetches one source-only core into the user's cores directory.
async fn place_core(
    fetcher: &mut SourceFetcher<'_>,
    core_file: &str,
    user_cores_dir: &Path,
) -> Result<(), String> {
    let candidate = fetcher
        .listing("cores")
        .await?
        .iter()
        .find(|entry| entry.name == core_file)
        .cloned()
        .ok_or_else(|| format!("no {core_file} in the source's cores/ folder"))?;
    let bytes = fetcher.download(&candidate).await?;
    if !is_elf(&bytes) {
        return Err("the downloaded core is not a shared library".to_owned());
    }
    write_file(&user_cores_dir.join(core_file), &bytes).await
}

/// The user's source plus the folder listings one sync has already read, so a
/// folder serving several slots is listed once.
struct SourceFetcher<'a> {
    client: reqwest::Client,
    source: &'a BiosSource,
    listings: HashMap<String, Result<Vec<SourceEntry>, String>>,
}

impl<'a> SourceFetcher<'a> {
    fn new(source: &'a BiosSource) -> Self {
        Self {
            client: reqwest::Client::new(),
            source,
            listings: HashMap::new(),
        }
    }

    async fn listing(&mut self, folder: &str) -> Result<&[SourceEntry], String> {
        if !self.listings.contains_key(folder) {
            let fresh = list_folder(&self.client, self.source, folder).await;
            self.listings.insert(folder.to_owned(), fresh);
        }
        match &self.listings[folder] {
            Ok(entries) => Ok(entries),
            Err(reason) => Err(reason.clone()),
        }
    }

    async fn download(&self, entry: &SourceEntry) -> Result<Vec<u8>, String> {
        let url = entry
            .download_url
            .as_deref()
            .ok_or_else(|| "the source offers no download URL for this file".to_owned())?;
        download(&self.client, self.source, url).await
    }
}

#[derive(Debug)]
pub(crate) enum ReadinessError {
    NoSource,
    InvalidSourceUrl,
}

impl std::fmt::Display for ReadinessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSource => write!(
                formatter,
                "no BIOS source is configured (HEARTHDECK_BIOS_SOURCE_URL)"
            ),
            Self::InvalidSourceUrl => {
                write!(
                    formatter,
                    "HEARTHDECK_BIOS_SOURCE_URL is not a GitHub project link"
                )
            }
        }
    }
}

impl std::error::Error for ReadinessError {}

/// One file as the GitHub contents API reports it.
#[derive(Clone, Debug, Deserialize)]
struct SourceEntry {
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    download_url: Option<String>,
}

/// BIOS files are a few MiB and core files run larger; anything beyond this
/// is a mistake upstream, not something Hearthdeck should buffer.
const MAX_SOURCE_FILE_BYTES: usize = 128 * 1024 * 1024;

enum SlotState {
    Missing,
    Present { name: String, verified: bool },
}

/// The platforms with roms — the only ones whose BIOS files are "necessary".
fn active_platforms(platforms: &[RommPlatform]) -> impl Iterator<Item = &RommPlatform> {
    platforms
        .iter()
        .filter(|platform| platform.rom_count > 0 && !platform_slug(platform).is_empty())
}

fn platform_slug(platform: &RommPlatform) -> &str {
    platform
        .fs_slug
        .as_deref()
        .or(platform.slug.as_deref())
        .unwrap_or("")
}

/// The distinct BIOS slots the active platforms need. Distinct by table
/// position: one slot can serve several platform slugs (the PC Engine System
/// Card serves both PCE and SuperGrafx) and must be fetched only once.
fn active_slots(platforms: &[RommPlatform]) -> Vec<&'static BiosSlot> {
    let mut slots: Vec<&'static BiosSlot> = Vec::new();
    for platform in active_platforms(platforms) {
        for slot in bios_slots_for(platform_slug(platform)) {
            if !slots.iter().any(|seen| std::ptr::eq(*seen, slot)) {
                slots.push(slot);
            }
        }
    }
    slots
}

fn slot_display_name(slot: &BiosSlot) -> String {
    match &slot.accept {
        BiosAccept::Known(known) => known
            .first()
            .map(|entry| entry.name.to_owned())
            .unwrap_or_default(),
        BiosAccept::Pattern { label, .. } => (*label).to_owned(),
    }
}

/// The slot's destination relative to the system directory, with the file name
/// the slot would place (or did place).
fn slot_target_display(slot: &BiosSlot, name: &str) -> String {
    if slot.target_dir.is_empty() {
        name.to_owned()
    } else {
        format!("{}/{}", slot.target_dir, name)
    }
}

fn target_path(slot: &BiosSlot, name: &str, system_dir: &Path) -> PathBuf {
    if slot.target_dir.is_empty() {
        system_dir.join(name)
    } else {
        system_dir.join(slot.target_dir).join(name)
    }
}

/// The spelling a placed file gets: the known dump's own name for `Known`
/// slots, the source's name for `Pattern` slots.
fn canonical_name<'a>(slot: &BiosSlot, source_name: &'a str) -> &'a str {
    match &slot.accept {
        BiosAccept::Known(known) => known
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(source_name))
            .map(|entry| entry.name)
            .unwrap_or(source_name),
        BiosAccept::Pattern { .. } => source_name,
    }
}

async fn bios_file_status(slot: &'static BiosSlot, system_dir: &Path) -> RetroBiosFile {
    let (filename, present, verified) = match slot_state(slot, system_dir).await {
        SlotState::Missing => (slot_display_name(slot), false, false),
        SlotState::Present { name, verified } => (name, true, verified),
    };
    let target = slot_target_display(slot, &filename);
    RetroBiosFile {
        system: slot.system.to_owned(),
        filename,
        required: slot.required,
        present,
        verified,
        target,
    }
}

async fn slot_state(slot: &BiosSlot, system_dir: &Path) -> SlotState {
    let directory = if slot.target_dir.is_empty() {
        system_dir.to_path_buf()
    } else {
        system_dir.join(slot.target_dir)
    };
    let Ok(mut entries) = tokio::fs::read_dir(&directory).await else {
        return SlotState::Missing;
    };
    let mut found: Option<(String, bool)> = None;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name_satisfies(&name, &slot.accept) {
            continue;
        }
        let verified = match tokio::fs::read(entry.path()).await {
            Ok(bytes) => file_is_verified(&name, &bytes, &slot.accept),
            Err(_) => false,
        };
        match &found {
            Some((_, true)) => {}
            _ => found = Some((name, verified)),
        }
    }
    match found {
        Some((name, verified)) => SlotState::Present { name, verified },
        None => SlotState::Missing,
    }
}

/// Whether a file name is one the slot would take at all (identity, before
/// content checks).
fn name_satisfies(name: &str, accept: &BiosAccept) -> bool {
    match accept {
        BiosAccept::Known(known) => known
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(name)),
        BiosAccept::Pattern { extension, .. } => Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case(extension)),
    }
}

/// The strictest identity the slot knows: SHA-1 where a dump's hash is
/// published, exact size where every accepted dump shares one, name-only
/// otherwise.
fn file_is_verified(name: &str, bytes: &[u8], accept: &BiosAccept) -> bool {
    match accept {
        BiosAccept::Known(known) => {
            let Some(entry) = known
                .iter()
                .find(|entry| entry.name.eq_ignore_ascii_case(name))
            else {
                return false;
            };
            if let Some(size) = entry.size
                && bytes.len() as u64 != size
            {
                return false;
            }
            match entry.sha1 {
                Some(sha1) => sha1_hex(bytes) == sha1,
                None => true,
            }
        }
        BiosAccept::Pattern { size, .. } => bytes.len() as u64 == *size,
    }
}

/// The one cheap identity a core file has before it is placed: it is an ELF
/// object, not an error page or a truncated transfer.
fn is_elf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x7fELF")
}

fn sha1_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The source-listing entry a slot would take: `Known` slots by name in
/// preference order, `Pattern` slots by shape with the preferred name prefix
/// first and deprioritized names last.
fn pick_candidate<'a>(entries: &'a [SourceEntry], accept: &BiosAccept) -> Option<&'a SourceEntry> {
    match accept {
        BiosAccept::Known(known) => known.iter().find_map(|entry| {
            entries
                .iter()
                .find(|candidate| candidate.name.eq_ignore_ascii_case(entry.name))
        }),
        BiosAccept::Pattern {
            extension,
            size,
            prefer,
            reject,
            ..
        } => entries
            .iter()
            .filter(|candidate| {
                name_satisfies(
                    &candidate.name,
                    &BiosAccept::Pattern {
                        label: "",
                        extension,
                        size: *size,
                        prefer,
                        reject,
                    },
                ) && candidate.size == *size
            })
            .min_by_key(|candidate| {
                let lowered = candidate.name.to_ascii_lowercase();
                // A rejected name loses even to an unpreferred one when it also
                // matches a preferred prefix: `SCPH-10000` starts with
                // `SCPH-`, so prefer alone cannot demote it.
                if reject
                    .iter()
                    .any(|prefix| lowered.starts_with(&prefix.to_ascii_lowercase()))
                {
                    2u8
                } else if prefer
                    .iter()
                    .any(|prefix| lowered.starts_with(&prefix.to_ascii_lowercase()))
                {
                    0u8
                } else {
                    1u8
                }
            }),
    }
}

async fn list_folder(
    client: &reqwest::Client,
    source: &BiosSource,
    folder: &str,
) -> Result<Vec<SourceEntry>, String> {
    let response = source
        .apply_auth(client.get(source.listing_url(folder)))
        .send()
        .await
        .map_err(|error| format!("could not reach the BIOS source: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "the BIOS source returned {} for {folder}",
            response.status()
        ));
    }
    response
        .json::<Vec<SourceEntry>>()
        .await
        .map_err(|error| format!("the BIOS source sent an unreadable listing: {error}"))
}

async fn download(
    client: &reqwest::Client,
    source: &BiosSource,
    url: &str,
) -> Result<Vec<u8>, String> {
    let response = source
        .apply_auth(client.get(url))
        .send()
        .await
        .map_err(|error| format!("could not download from the BIOS source: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "the BIOS source returned {} for this file",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_SOURCE_FILE_BYTES as u64)
    {
        return Err("the file is too large to be a BIOS dump".to_owned());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("the download was cut short: {error}"))?;
    if bytes.len() > MAX_SOURCE_FILE_BYTES {
        return Err("the file is too large to be a BIOS dump".to_owned());
    }
    Ok(bytes.to_vec())
}

/// Writes through a sibling temp file and a rename, so an interrupted sync
/// never leaves a truncated BIOS or core where a launch would load it.
async fn write_file(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let failed = |error: std::io::Error| format!("could not write {}: {error}", target.display());
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(failed)?;
    }
    let partial = target.with_extension("partial");
    tokio::fs::write(&partial, bytes).await.map_err(failed)?;
    tokio::fs::rename(&partial, target).await.map_err(failed)
}

/// Path segments only need their spaces escaped for the GitHub API; every
/// other character in the folder names we know is already URL-safe.
fn encode_path(path: &str) -> String {
    path.replace(' ', "%20")
}

#[cfg(test)]
mod tests {
    use super::{
        BiosAccept, BiosSlot, BiosSource, KnownBios, ReadinessError, SourceEntry, bios_slots_for,
        file_is_verified, name_satisfies, pick_candidate, readiness, sha1_hex, slot_target_display,
    };
    use crate::diagnostics::RommPlatform;
    use crate::retro;
    use std::path::Path;

    fn platform(fs_slug: &str, rom_count: u64) -> RommPlatform {
        RommPlatform {
            id: 1,
            name: fs_slug.to_owned(),
            display_name: None,
            rom_count,
            slug: None,
            fs_slug: Some(fs_slug.to_owned()),
            url_logo: None,
            artwork_paths: Vec::new(),
        }
    }

    #[test]
    fn bios_slots_are_well_formed() {
        for slot in super::BIOS_SLOTS {
            assert!(!slot.platforms.is_empty(), "slot without a platform");
            assert!(!slot.system.is_empty(), "slot without a source folder");
            assert!(!slot.system.starts_with('/'), "absolute source folder");
            assert!(
                !slot.system.ends_with('/'),
                "source folder with trailing slash"
            );
            assert!(!slot.target_dir.starts_with('/'), "absolute target dir");
            match &slot.accept {
                BiosAccept::Known(known) => {
                    assert!(!known.is_empty(), "Known slot without a dump");
                    for entry in *known {
                        assert!(!entry.name.is_empty(), "dump without a name");
                        if let Some(sha1) = entry.sha1 {
                            assert_eq!(sha1.len(), 40, "{}: sha1 not 40 hex", entry.name);
                            assert!(
                                sha1.chars().all(|c| c.is_ascii_hexdigit()),
                                "{}: sha1 not hex",
                                entry.name
                            );
                        }
                    }
                }
                BiosAccept::Pattern {
                    label,
                    extension,
                    size,
                    ..
                } => {
                    assert!(!label.is_empty(), "Pattern slot without a label");
                    assert!(!extension.is_empty(), "Pattern slot without an extension");
                    assert!(*size > 0, "Pattern slot without a size");
                }
            }
        }
    }

    #[test]
    fn every_mapped_core_is_installable() {
        for (_, core) in retro::CORE_BY_PLATFORM_SLUG {
            assert!(
                retro::core_package_for(core).is_some() || retro::SOURCE_ONLY_CORES.contains(core),
                "core {core} has neither a package nor a source fallback"
            );
        }
    }

    #[test]
    fn sha1_hex_matches_the_published_hello_digest() {
        // Anchors the hash helper against a public, checkable digest.
        assert_eq!(
            sha1_hex(b"hello"),
            "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"
        );
    }

    #[tokio::test]
    async fn a_platform_with_no_roms_is_not_readied() {
        let paths = super::ReadinessPaths {
            cores_dir: Path::new("/nonexistent").to_path_buf(),
            system_dir: Path::new("/nonexistent").to_path_buf(),
            user_cores_dir: Path::new("/nonexistent").to_path_buf(),
        };
        let ready = readiness(&[platform("ps2", 0)], &paths).await;
        assert!(ready.is_empty());
    }

    #[tokio::test]
    async fn readiness_reports_missing_core_and_bios() {
        let paths = super::ReadinessPaths {
            cores_dir: Path::new("/nonexistent").to_path_buf(),
            system_dir: Path::new("/nonexistent").to_path_buf(),
            user_cores_dir: Path::new("/nonexistent").to_path_buf(),
        };
        let ready = readiness(&[platform("ps2", 3)], &paths).await;
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].platform, "ps2");
        assert_eq!(ready[0].core.core_filename, "pcsx2_libretro.so");
        // No package provides this core: readiness reports an empty package
        // and the sync fetches the file from the user's source instead.
        assert_eq!(ready[0].core.package, "");
        assert!(!ready[0].core.installed);
        assert_eq!(ready[0].bios.len(), 1);
        assert!(ready[0].bios[0].required);
        assert!(!ready[0].bios[0].present);
        assert_eq!(ready[0].bios[0].target, "pcsx2/bios/SCPH-*.bin");
    }

    #[tokio::test]
    async fn a_placed_pattern_file_is_seen_and_verified_by_size() {
        let system_directory = tempfile::tempdir().unwrap();
        let bios_directory = system_directory.path().join("pcsx2/bios");
        tokio::fs::create_dir_all(&bios_directory).await.unwrap();
        tokio::fs::write(bios_directory.join("SCPH-70004.BIN"), vec![0u8; 4_194_304])
            .await
            .unwrap();

        let paths = super::ReadinessPaths {
            cores_dir: Path::new("/nonexistent").to_path_buf(),
            system_dir: system_directory.path().to_path_buf(),
            user_cores_dir: Path::new("/nonexistent").to_path_buf(),
        };
        let ready = readiness(&[platform("ps2", 1)], &paths).await;
        assert!(ready[0].bios[0].present);
        assert!(ready[0].bios[0].verified);
        assert_eq!(ready[0].bios[0].filename, "SCPH-70004.BIN");
        assert_eq!(ready[0].bios[0].target, "pcsx2/bios/SCPH-70004.BIN");
    }

    #[test]
    fn known_dumps_verify_by_hash_and_size() {
        let accept = BiosAccept::Known(&[KnownBios {
            name: "hello.bin",
            sha1: Some("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"),
            size: Some(5),
        }]);
        assert!(file_is_verified("HELLO.BIN", b"hello", &accept));
        assert!(!file_is_verified("hello.bin", b"hell0", &accept));
        assert!(!file_is_verified("other.bin", b"hello", &accept));
    }

    #[test]
    fn pattern_files_verify_by_size() {
        let accept = BiosAccept::Pattern {
            label: "*.bin",
            extension: "bin",
            size: 4,
            prefer: &[],
            reject: &[],
        };
        assert!(name_satisfies("dump.BIN", &accept));
        assert!(!name_satisfies("dump.iso", &accept));
        assert!(file_is_verified("dump.bin", b"1234", &accept));
        assert!(!file_is_verified("dump.bin", b"123", &accept));
    }

    #[test]
    fn pattern_candidates_prefer_the_preferred_prefix() {
        let accept = BiosAccept::Pattern {
            label: "*.bin",
            extension: "bin",
            size: 4,
            prefer: &["SCPH-"],
            reject: &["SCPH-10000"],
        };
        let entries = vec![
            entry("ps2-0100jd.bin", 4),
            entry("SCPH-10000.bin", 4),
            entry("SCPH-70004.BIN", 4),
            entry("SCPH-70004.EROM", 4),
            entry("tiny.bin", 1),
        ];
        let picked = pick_candidate(&entries, &accept).unwrap();
        assert_eq!(picked.name, "SCPH-70004.BIN");

        // With no preferred candidate, the deprioritized one still beats none.
        let only_rejected = vec![entry("SCPH-10000.bin", 4)];
        let picked = pick_candidate(&only_rejected, &accept).unwrap();
        assert_eq!(picked.name, "SCPH-10000.bin");
    }

    #[test]
    fn known_candidates_follow_the_slots_preference_order() {
        let accept = BiosAccept::Known(&[
            KnownBios {
                name: "syscard3.pce",
                sha1: None,
                size: None,
            },
            KnownBios {
                name: "syscard2.pce",
                sha1: None,
                size: None,
            },
        ]);
        let entries = vec![entry("syscard2.pce", 10), entry("syscard3.pce", 10)];
        let picked = pick_candidate(&entries, &accept).unwrap();
        assert_eq!(picked.name, "syscard3.pce");
    }

    #[test]
    fn install_requests_name_only_the_missing_core_packages() {
        // One platform and one core package per row, so two missing rows
        // dedupe to one installable name and a ready row contributes nothing.
        let row = |installed: bool| hearthdeck_protocol::RetroSystemReadiness {
            platform: "snes".to_owned(),
            display_name: "Super Nintendo".to_owned(),
            core: hearthdeck_protocol::RetroCoreReadiness {
                core_filename: "snes9x_libretro.so".to_owned(),
                package: "libretro-snes9x".to_owned(),
                installed,
            },
            bios: Vec::new(),
        };
        let report = vec![row(true), row(false), row(false)];
        assert_eq!(
            super::missing_core_packages(&report),
            vec!["libretro-snes9x".to_owned()]
        );
        assert!(super::missing_core_packages(&[row(true)]).is_empty());
    }

    #[test]
    fn source_only_cores_belong_to_the_sync_and_not_the_install_button() {
        let row = |installed: bool, package: &str| hearthdeck_protocol::RetroSystemReadiness {
            platform: "ps2".to_owned(),
            display_name: "PlayStation 2".to_owned(),
            core: hearthdeck_protocol::RetroCoreReadiness {
                core_filename: "pcsx2_libretro.so".to_owned(),
                package: package.to_owned(),
                installed,
            },
            bios: Vec::new(),
        };
        // A missing core no package provides is fetched from the user's
        // source; a missing packaged core is the install button's job, and an
        // installed one is left alone.
        let report = vec![row(false, ""), row(false, "libretro-snes9x"), row(true, "")];
        assert_eq!(
            super::missing_source_cores(&report),
            vec!["pcsx2_libretro.so".to_owned()]
        );
    }

    #[test]
    fn a_downloaded_core_must_be_an_elf_object() {
        assert!(super::is_elf(b"\x7fELF\x02\x01\x01"));
        assert!(!super::is_elf(b"<html>404</html>"));
    }

    #[test]
    fn bios_slots_only_cover_platforms_with_bios() {
        assert!(bios_slots_for("ps2").count() > 0);
        assert!(bios_slots_for("gba").count() == 0);
    }

    #[test]
    fn source_links_parse_and_nothing_else_does() {
        let parsed = BiosSource::parse("https://github.com/someone/my-bios", None).unwrap();
        assert_eq!(parsed.owner, "someone");
        assert_eq!(parsed.repo, "my-bios");
        assert_eq!(parsed.branch, None);

        let parsed =
            BiosSource::parse("https://github.com/someone/my-bios/tree/nightly/", None).unwrap();
        assert_eq!(parsed.branch.as_deref(), Some("nightly"));

        for bad in [
            "https://example.com/someone/my-bios",
            "https://github.com/someone",
            "https://github.com/someone/repo/blob/main/x",
            "http://github.com/someone/repo",
        ] {
            assert!(
                matches!(
                    BiosSource::parse(bad, None),
                    Err(ReadinessError::InvalidSourceUrl)
                ),
                "{bad} should not parse"
            );
        }
    }

    #[test]
    fn target_display_keeps_the_emulators_lowercase_layout() {
        let slot = BiosSlot {
            platforms: &["ps2"],
            system: "Sony/PlayStation 2",
            target_dir: "pcsx2/bios",
            required: true,
            accept: BiosAccept::Pattern {
                label: "SCPH-*.bin",
                extension: "bin",
                size: 4,
                prefer: &[],
                reject: &[],
            },
        };
        assert_eq!(
            slot_target_display(&slot, "SCPH-70004.BIN"),
            "pcsx2/bios/SCPH-70004.BIN"
        );
    }

    fn entry(name: &str, size: u64) -> SourceEntry {
        SourceEntry {
            name: name.to_owned(),
            size,
            download_url: Some(format!("https://example.invalid/{name}")),
        }
    }

    /// Readiness and launch share one core lookup, so a core the sync placed in
    /// the user's cores directory reads as installed.
    #[tokio::test]
    async fn a_core_in_the_user_cores_directory_counts_as_installed() {
        let user_cores = tempfile::tempdir().unwrap();
        tokio::fs::write(user_cores.path().join("pcsx2_libretro.so"), b"\x7fELF")
            .await
            .unwrap();
        let paths = super::ReadinessPaths {
            cores_dir: Path::new("/nonexistent").to_path_buf(),
            system_dir: Path::new("/nonexistent").to_path_buf(),
            user_cores_dir: user_cores.path().to_path_buf(),
        };

        let ready = readiness(&[platform("ps2", 1)], &paths).await;

        assert!(ready[0].core.installed);
    }

    #[tokio::test]
    async fn a_placed_file_lands_whole_with_no_partial_left_behind() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("pcsx2/bios/SCPH-70004.BIN");

        super::write_file(&target, b"first").await.unwrap();
        super::write_file(&target, b"second").await.unwrap();

        assert_eq!(tokio::fs::read(&target).await.unwrap(), b"second");
        let names: Vec<_> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("SCPH-70004.BIN")]);
    }
}
