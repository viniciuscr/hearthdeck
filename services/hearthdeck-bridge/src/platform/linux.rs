use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use hearthdeck_protocol::{DiscoveredApplication, HeroicRunner};
use tokio::process::Command;
use tracing::{debug, info, warn};

use super::{DESKTOP_APPS_SOURCE, LaunchedApplication};

pub async fn discover_applications(source_id: &str) -> Result<Vec<DiscoveredApplication>> {
    if source_id != DESKTOP_APPS_SOURCE {
        anyhow::bail!("unsupported application source")
    }
    let directories = desktop_entry_directories();
    info!(directories = ?directories, "application discovery scanning desktop-entry directories");

    let mut entries = HashMap::new();
    for source_directory in directories {
        let mut directory = match tokio::fs::read_dir(&source_directory).await {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                warn!(directory = %source_directory.display(), %error, "application discovery could not read desktop-entry directory");
                continue;
            }
        };
        let mut desktop_entry_count = 0;
        let mut accepted_entry_count = 0;
        loop {
            let file = match directory.next_entry().await {
                Ok(Some(file)) => file,
                Ok(None) => break,
                Err(error) => {
                    warn!(%error, "application discovery could not enumerate desktop-entry directory");
                    break;
                }
            };
            let path = file.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "desktop")
            {
                desktop_entry_count += 1;
                match parse_desktop_entry(&path).await {
                    Ok(entry) => {
                        accepted_entry_count += 1;
                        debug!(
                            directory = %source_directory.display(),
                            application_id = %entry.application_id,
                            title = %entry.name,
                            "application discovery accepted desktop entry"
                        );
                        entries.entry(entry.application_id.clone()).or_insert(entry);
                    }
                    Err(error) => {
                        debug!(
                            desktop_entry = %path.display(),
                            %error,
                            "application discovery skipped desktop entry"
                        );
                    }
                }
            }
        }
        info!(
            directory = %source_directory.display(),
            desktop_entry_count,
            accepted_entry_count,
            "application discovery scanned desktop-entry directory"
        );
    }
    let mut entries: Vec<_> = entries.into_values().collect();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}

pub async fn launch_application(
    source_id: &str,
    application_id: &str,
    session_id: &str,
) -> Result<LaunchedApplication> {
    let path = desktop_entry_path(source_id, application_id).await?;
    let launch = command_for_desktop_entry(&path).await?;
    let unit_name = format!("hearthdeck-app-{session_id}.service");
    launch_with_systemd(&unit_name, launch.command, launch.working_directory).await
}

/// Heroic itself - not a specific game - is the resource Hearthdeck manages
/// here, under one stable (not per-launch) unit name. Electron single-instance
/// locking means a second `heroic://launch` URI while Heroic is still running
/// is handled by that *same* already-running process, not a fresh one we'd
/// separately track; reusing one unit name (and checking whether it's already
/// active before deciding whether to start it) is what keeps every launch
/// attributable to a unit Hearthdeck can reliably stop later, regardless of
/// how many games have been launched through it. Heroic is left running
/// between games on purpose (faster subsequent launches); closing it (and
/// whatever game it's running) is `stop_application(Some(HEROIC_UNIT_NAME))`,
/// which is cgroup-based via `systemctl --user stop` - see `launch_heroic_game`
/// below for why Heroic must be exec'd directly (not via `xdg-open`) for that
/// stop to actually reach the running game, not just Heroic's own launcher.
pub const HEROIC_UNIT_NAME: &str = "hearthdeck-heroic.service";

/// Delegate game startup to Heroic so its configured Wine, Proton, UMU,
/// wrappers, save sync, and per-game launch options remain authoritative.
pub async fn launch_heroic_game(
    runner: HeroicRunner,
    application_id: &str,
) -> Result<LaunchedApplication> {
    if !valid_heroic_application_id(application_id) {
        anyhow::bail!("Heroic application identifier is invalid")
    }
    let runner = match runner {
        HeroicRunner::Legendary => "legendary",
        HeroicRunner::Gog => "gog",
    };
    let uri = format!("heroic://launch?appName={application_id}&runner={runner}&gui=false");

    if super::application_is_running(Some(HEROIC_UNIT_NAME)).await? {
        // Heroic is already up and holds Electron's single-instance lock: a
        // direct `heroic <uri>` invocation here is detected as a second
        // instance and forwards this argv to the running primary over
        // Electron's own (non-D-Bus) IPC, then exits - matching the
        // cold-start branch's fast-return semantics below. Spawn, don't
        // await: tokio reaps the exited relay process in the background, so
        // this does not leak zombies.
        Command::new("heroic")
            .arg("--no-gui")
            .arg(&uri)
            .spawn()
            .context("could not ask the running Heroic instance to launch the game")?;
        return Ok(LaunchedApplication {
            unit_name: Some(HEROIC_UNIT_NAME.to_owned()),
        });
    }

    // Cold start: exec the `heroic` binary directly - NOT `xdg-open`.
    // Confirmed on real hardware (see docs/arch-package.md): `xdg-open
    // heroic://...` resolves this custom URI scheme through `gio
    // open`/`gio launch` (GLib's desktop-file launcher), which has its own
    // systemd integration (https://systemd.io/DESKTOP_ENVIRONMENTS/) and
    // unconditionally starts registered .desktop apps in a *new*
    // `app-<name>-<pid>.scope` under app.slice - migrating Heroic, and
    // everything it spawns (wineserver, the game itself), out of whatever
    // cgroup launched it. That silently orphaned the whole process tree
    // from hearthdeck-heroic.service's cgroup: only `xdg-open` itself and a
    // couple of early zygote helpers stayed put, so stopping the unit never
    // reached the actual game. Heroic reads the launch URI straight from
    // argv (see its own src/backend/protocol.ts), so it never needed
    // xdg-open/gio at all - invoking it directly keeps the whole tree
    // inside the cgroup we track, the same way desktop apps and RetroArch
    // already work. `--no-gui` is Heroic's own confirmed-supported CLI flag
    // for suppressing its window (belt-and-suspenders with the URL's
    // `gui=false`, and it also makes Heroic exit itself once the game
    // exits, instead of only the `gui=false` query param's window-hide).
    let command = vec![
        OsString::from("heroic"),
        OsString::from("--no-gui"),
        OsString::from(uri),
    ];
    launch_with_systemd(HEROIC_UNIT_NAME, command, None).await
}

/// Directories pacman installs libretro cores into on Arch. A core path must
/// resolve under one of these before the bridge will exec it; this is the
/// on-demand/curated-core-install question (see
/// docs/retroarch-integration.md) staying out of the bridge's trust
/// boundary regardless of how a core got there.
const RETRO_CORE_DIRECTORIES: [&str; 1] = ["/usr/lib/libretro"];

/// Launches a RetroArch core against a ROM the daemon has already resolved
/// and cached locally. Mirrors `launch_application`'s discipline: the bridge
/// re-validates both paths itself rather than trusting the daemon's copy,
/// and RetroArch is exec'd directly as the transient unit's main process
/// (not handed off through a URI/IPC scheme the way Heroic is), so Kiosk
/// session tracking works the same way it does for desktop apps.
pub async fn launch_retro_game(
    core_path: &str,
    rom_path: &str,
    session_id: &str,
) -> Result<LaunchedApplication> {
    let core_path = validate_retro_core_path(core_path)?;
    let rom_path = validate_retro_rom_path(rom_path)?;
    let config_directory = retro_config_directory();
    tokio::fs::create_dir_all(&config_directory)
        .await
        .context("could not create Hearthdeck's RetroArch config directory")?;
    let config_path = config_directory.join("retroarch.cfg");
    let unit_name = format!("hearthdeck-app-{session_id}.service");
    launch_with_systemd(
        &unit_name,
        vec![
            OsString::from("retroarch"),
            OsString::from("-c"),
            config_path.into_os_string(),
            OsString::from("-L"),
            core_path.into_os_string(),
            rom_path.into_os_string(),
        ],
        None,
    )
    .await
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

/// The directory Hearthdeck caches ROMs fetched from RomM into. The bridge
/// only launches ROM files that resolve under this directory; the nested
/// layout inside it is the daemon's concern.
fn retro_rom_cache_directory() -> PathBuf {
    let cache_home = env::var_os("XDG_CACHE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|path| PathBuf::from(path).join(".cache")));
    cache_home
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("hearthdeck/romm")
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
    let cache_root = fs::canonicalize(retro_rom_cache_directory())
        .context("Hearthdeck rom cache directory does not exist")?;
    validate_retro_rom_path_in(rom_path, &cache_root)
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

async fn launch_with_systemd(
    unit_name: &str,
    application_command: Vec<OsString>,
    working_directory: Option<PathBuf>,
) -> Result<LaunchedApplication> {
    let mut command = Command::new("systemd-run");
    command
        .args([
            "--user",
            "--collect",
            "--quiet",
            "--service-type=exec",
            "--unit",
        ])
        .arg(unit_name)
        .arg("--working-directory")
        .arg(working_directory.unwrap_or_else(|| PathBuf::from("/")));
    if is_kiosk_session() {
        command.arg("--slice=hearthdeck-kiosk.slice");
    }
    // Forward the display/session environment the launched process needs to
    // connect to whatever session Hearthdeck itself is running in -
    // `systemd-run` does not inherit the caller's environment, and a
    // `systemd --user` unit's own default environment can be stale or
    // missing these entirely (Gamescope only ever assigns DISPLAY/
    // WAYLAND_DISPLAY to its own direct child - Hearthdeck itself - which is
    // why Hearthdeck's own startup imports them into systemd --user; see
    // linux/runner/my_application.cc). Reading them from the bridge's own
    // process environment and forwarding them explicitly is correct both
    // inside and outside the Kiosk session, so this no longer branches on
    // is_kiosk_session() the way it used to.
    for name in [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "GAMESCOPE_WAYLAND_DISPLAY",
        "XAUTHORITY",
        "GDK_BACKEND",
        "SDL_VIDEODRIVER",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "XDG_SESSION_TYPE",
        "DBUS_SESSION_BUS_ADDRESS",
    ] {
        if let Some(value) = env::var_os(name) {
            command
                .arg("--setenv")
                .arg(format!("{name}={}", value.to_string_lossy()));
        }
    }
    command.arg("--").args(application_command);
    let status = command
        .status()
        .await
        .context("could not start supervised application launch")?;
    if !status.success() {
        anyhow::bail!("systemd user manager rejected application launch")
    }
    Ok(LaunchedApplication {
        unit_name: Some(unit_name.to_owned()),
    })
}

fn is_kiosk_session() -> bool {
    current_desktops()
        .iter()
        .any(|desktop| desktop.eq_ignore_ascii_case("hearthdeck"))
}

fn valid_heroic_application_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, b'.' | b'_' | b'-')
        })
}

struct DesktopLaunch {
    command: Vec<OsString>,
    working_directory: Option<PathBuf>,
}

async fn desktop_entry_path(source_id: &str, application_id: &str) -> Result<PathBuf> {
    if source_id != DESKTOP_APPS_SOURCE {
        anyhow::bail!("unsupported application source")
    }
    for directory in desktop_entry_directories() {
        let path = directory.join(application_id);
        if path.is_file() && parse_desktop_entry(&path).await.is_ok() {
            return Ok(path);
        }
    }
    anyhow::bail!("desktop entry is not registered")
}

async fn command_for_desktop_entry(path: &Path) -> Result<DesktopLaunch> {
    let content = tokio::fs::read_to_string(path).await?;
    let values = desktop_entry_values(&content);
    validate_desktop_entry(&values)?;
    if desktop_entry_boolean(&values, "Terminal") {
        anyhow::bail!("terminal desktop entries are not supported in Kiosk mode")
    }
    if desktop_entry_boolean(&values, "DBusActivatable") {
        anyhow::bail!("D-Bus-activated desktop entries are not supported in a managed session")
    }
    let exec = values.get("Exec").context("desktop entry has no Exec")?;
    let command = parse_exec(
        exec,
        path,
        values.get("Name").copied(),
        values.get("Icon").copied(),
    )?;
    if command.is_empty() {
        anyhow::bail!("desktop entry has an empty Exec command")
    }
    if let Some(try_exec) = values.get("TryExec")
        && !executable_exists(try_exec)
    {
        anyhow::bail!("desktop entry TryExec is not executable")
    }
    Ok(DesktopLaunch {
        command: command.into_iter().map(OsString::from).collect(),
        working_directory: values
            .get("Path")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from),
    })
}

fn parse_exec(
    value: &str,
    desktop_path: &Path,
    application_name: Option<&str>,
    icon: Option<&str>,
) -> Result<Vec<String>> {
    let mut arguments = Vec::new();
    let mut argument = String::new();
    let mut quoted = false;
    let mut argument_was_quoted = false;
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if quoted {
            if character == '"' {
                quoted = false;
            } else if character == '\\' {
                let escaped = characters
                    .next()
                    .context("desktop entry has an invalid quoted Exec command")?;
                if !matches!(escaped, '"' | '`' | '$' | '\\') {
                    anyhow::bail!("desktop entry has an invalid quoted Exec escape")
                }
                argument.push(escaped);
            } else {
                argument.push(character);
            }
        } else if character == '\\' {
            let escaped = characters
                .next()
                .context("desktop entry has an invalid Exec command")?;
            argument.push(escaped);
        } else if character == '"' {
            quoted = true;
            argument_was_quoted = true;
        } else if character.is_whitespace() && !quoted {
            if !argument.is_empty() {
                arguments.push((std::mem::take(&mut argument), argument_was_quoted));
                argument_was_quoted = false;
            }
        } else {
            argument.push(character);
        }
    }
    if quoted {
        anyhow::bail!("desktop entry has an invalid Exec command")
    }
    if !argument.is_empty() {
        arguments.push((argument, argument_was_quoted));
    }
    let mut expanded = Vec::new();
    for (argument, was_quoted) in arguments {
        expand_field_codes(
            &mut expanded,
            &argument,
            was_quoted,
            desktop_path,
            application_name,
            icon,
        )?;
    }
    Ok(expanded)
}

fn expand_field_codes(
    arguments: &mut Vec<String>,
    argument: &str,
    was_quoted: bool,
    desktop_path: &Path,
    application_name: Option<&str>,
    icon: Option<&str>,
) -> Result<()> {
    if matches!(argument, "%f" | "%F" | "%u" | "%U") {
        if was_quoted {
            anyhow::bail!("desktop entry has a quoted file field code")
        }
        return Ok(());
    }
    if argument == "%i" {
        if was_quoted {
            anyhow::bail!("desktop entry has a quoted icon field code")
        }
        if let Some(icon) = icon {
            arguments.push("--icon".to_owned());
            arguments.push(icon.to_owned());
        }
        return Ok(());
    }
    let mut expanded = String::new();
    let mut characters = argument.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            expanded.push(character);
            continue;
        }
        if was_quoted {
            anyhow::bail!("desktop entry has a field code inside quoted text")
        }
        match characters
            .next()
            .context("desktop entry has an incomplete field code")?
        {
            '%' => expanded.push('%'),
            'c' => expanded.push_str(application_name.unwrap_or_default()),
            'k' => expanded.push_str(&desktop_path.to_string_lossy()),
            'f' | 'F' | 'u' | 'U' | 'i' => {
                anyhow::bail!("desktop entry has a field code in an invalid position")
            }
            _ => anyhow::bail!("desktop entry has an unknown field code"),
        }
    }
    if !expanded.is_empty() {
        arguments.push(expanded);
    }
    Ok(())
}

fn executable_exists(command: &str) -> bool {
    let path = Path::new(command);
    if path.is_absolute() || command.contains('/') {
        return path.is_file()
            && fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0);
    }
    env::var_os("PATH")
        .as_deref()
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .map(|directory| directory.join(command))
        .any(|candidate| {
            candidate.is_file()
                && fs::metadata(candidate)
                    .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
        })
}

fn validate_desktop_entry(values: &HashMap<&str, &str>) -> Result<()> {
    if values.get("Type") != Some(&"Application") {
        anyhow::bail!("not an application desktop entry")
    }
    if desktop_entry_boolean(values, "NoDisplay") || desktop_entry_boolean(values, "Hidden") {
        anyhow::bail!("not a visible application entry")
    }
    let desktops = current_desktops();
    if let Some(only_show_in) = values.get("OnlyShowIn")
        && !visible_in_desktop(only_show_in, &desktops)
    {
        anyhow::bail!("desktop entry is hidden in the current desktop")
    }
    if let Some(not_show_in) = values.get("NotShowIn")
        && visible_in_desktop(not_show_in, &desktops)
    {
        anyhow::bail!("desktop entry is hidden in the current desktop")
    }
    Ok(())
}

fn current_desktops() -> Vec<String> {
    env::var("XDG_CURRENT_DESKTOP")
        .ok()
        .map(|desktop| desktop.split(':').map(ToOwned::to_owned).collect())
        .unwrap_or_default()
}

fn visible_in_desktop(desktops: &str, current_desktops: &[String]) -> bool {
    desktops
        .split(';')
        .filter(|desktop| !desktop.is_empty())
        .any(|desktop| current_desktops.iter().any(|current| current == desktop))
}

fn desktop_entry_boolean(values: &HashMap<&str, &str>, key: &str) -> bool {
    values.get(key).is_some_and(|value| *value == "true")
}

fn desktop_entry_directories() -> Vec<PathBuf> {
    desktop_entry_directories_for(
        env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
        env::var("XDG_DATA_DIRS").ok(),
    )
}

fn desktop_entry_directories_for(
    data_home: Option<PathBuf>,
    home: Option<PathBuf>,
    data_dirs: Option<String>,
) -> Vec<PathBuf> {
    let data_home = data_home
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| home.map(|path| path.join(".local/share")));
    let data_dirs = data_dirs
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());

    let mut directories = Vec::new();
    let mut seen = HashSet::new();
    if let Some(data_home) = data_home {
        push_unique(&mut directories, &mut seen, data_home.join("applications"));
        // Flatpak exports launchers outside the normal XDG data root.
        push_unique(
            &mut directories,
            &mut seen,
            data_home.join("flatpak/exports/share/applications"),
        );
    }
    for data_dir in data_dirs
        .split(':')
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        push_unique(&mut directories, &mut seen, data_dir.join("applications"));
    }
    push_unique(
        &mut directories,
        &mut seen,
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
    );
    directories
}

fn push_unique(directories: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, path: PathBuf) {
    if seen.insert(path.clone()) {
        directories.push(path);
    }
}

async fn parse_desktop_entry(path: &Path) -> Result<DiscoveredApplication> {
    let content = tokio::fs::read_to_string(path).await?;
    let values = desktop_entry_values(&content);
    validate_desktop_entry(&values)?;
    command_for_desktop_entry(path).await?;
    let name = values
        .get("Name")
        .context("desktop entry has no Name")?
        .to_string();
    let application_id = path
        .file_name()
        .context("desktop entry has no file name")?
        .to_string_lossy()
        .to_string();
    Ok(DiscoveredApplication {
        application_id,
        name,
        comment: values.get("Comment").map(ToString::to_string),
        icon: values.get("Icon").map(ToString::to_string),
        categories: values
            .get("Categories")
            .map(|value| {
                value
                    .split(';')
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        launch_scheme: values
            .get("Exec")
            .and_then(|exec| exec.contains("heroic://").then_some("heroic".to_owned())),
    })
}

fn desktop_entry_values(content: &str) -> HashMap<&str, &str> {
    let mut values = HashMap::new();
    let mut in_desktop_entry = false;
    for line in content.lines().map(str::trim) {
        if line.starts_with('[') {
            if in_desktop_entry {
                break;
            }
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if in_desktop_entry
            && !line.starts_with('#')
            && let Some((key, value)) = line.split_once('=')
        {
            values.insert(key.trim(), value.trim());
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        desktop_entry_directories_for, parse_desktop_entry, parse_exec,
        valid_heroic_application_id, validate_retro_core_path_in, validate_retro_rom_path_in,
        visible_in_desktop,
    };

    #[tokio::test]
    async fn parses_only_the_desktop_entry_group() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("example.desktop");
        tokio::fs::write(
            &path,
            "[Desktop Entry]\nType=Application\nName=Example\nExec=example --open\nCategories=Utility;\n[Desktop Action other]\nName=Wrong name\n",
        )
        .await
        .unwrap();

        let entry = parse_desktop_entry(&path).await.unwrap();

        assert_eq!(entry.name, "Example");
        assert_eq!(entry.categories, ["Utility"]);
        assert_eq!(entry.launch_scheme, None);
    }

    #[tokio::test]
    async fn identifies_heroic_shortcuts_for_dedicated_game_discovery() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("heroic-game.desktop");
        tokio::fs::write(
            &path,
            "[Desktop Entry]\nType=Application\nName=Example\nExec=xdg-open heroic://launch?appName=Example&runner=legendary\nCategories=Game;\n",
        )
        .await
        .unwrap();

        let entry = parse_desktop_entry(&path).await.unwrap();

        assert_eq!(entry.launch_scheme.as_deref(), Some("heroic"));
    }

    #[test]
    fn parses_exec_without_forwarding_runtime_file_arguments() {
        let command = parse_exec(
            "example --title %c %U",
            std::path::Path::new("/tmp/example.desktop"),
            Some("Example"),
            None,
        )
        .unwrap();

        assert_eq!(command, ["example", "--title", "Example"]);
    }

    #[test]
    fn expands_icon_field_code_as_two_arguments() {
        let command = parse_exec(
            "example %i",
            std::path::Path::new("/tmp/example.desktop"),
            Some("Example"),
            Some("example-icon"),
        )
        .unwrap();

        assert_eq!(command, ["example", "--icon", "example-icon"]);
    }

    #[test]
    fn rejects_field_codes_inside_quoted_arguments() {
        let result = parse_exec(
            "example \"%c\"",
            std::path::Path::new("/tmp/example.desktop"),
            Some("Example"),
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn recognizes_hearthdeck_cosmic_kiosk_desktop_names() {
        let current_desktops = vec!["hearthdeck".to_owned(), "COSMIC".to_owned()];

        assert!(visible_in_desktop("hearthdeck;", &current_desktops));
        assert!(visible_in_desktop("COSMIC;", &current_desktops));
        assert!(!visible_in_desktop("gamescope;", &current_desktops));
        assert!(!visible_in_desktop("KDE;", &current_desktops));
    }

    #[test]
    fn uses_xdg_defaults_and_flatpak_exports_when_environment_is_empty() {
        let directories = desktop_entry_directories_for(
            Some(PathBuf::new()),
            Some(PathBuf::from("/home/tester")),
            Some(String::new()),
        );

        assert_eq!(
            directories,
            vec![
                PathBuf::from("/home/tester/.local/share/applications"),
                PathBuf::from("/home/tester/.local/share/flatpak/exports/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
            ],
        );
    }

    #[test]
    fn ignores_relative_xdg_data_directories() {
        let directories = desktop_entry_directories_for(
            None,
            Some(PathBuf::from("/home/tester")),
            Some("relative:/opt/share:/usr/share".to_owned()),
        );

        assert!(!directories.contains(&PathBuf::from("relative/applications")));
        assert!(directories.contains(&PathBuf::from("/opt/share/applications")));
        assert_eq!(
            directories
                .iter()
                .filter(|path| path == &&PathBuf::from("/usr/share/applications"))
                .count(),
            1,
        );
    }

    #[test]
    fn validates_heroic_application_identifiers_before_uri_construction() {
        assert!(valid_heroic_application_id("Fortnite"));
        assert!(valid_heroic_application_id("1091500"));
        assert!(!valid_heroic_application_id("Fortnite&runner=gog"));
        assert!(!valid_heroic_application_id("../Fortnite"));
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
    fn accepts_a_rom_inside_the_hearthdeck_cache_directory() {
        let cache_directory = tempfile::tempdir().unwrap();
        let rom_path = cache_directory.path().join("42.sfc");
        std::fs::write(&rom_path, b"").unwrap();
        let cache_root = std::fs::canonicalize(cache_directory.path()).unwrap();

        let resolved = validate_retro_rom_path_in(rom_path.to_str().unwrap(), &cache_root).unwrap();

        assert_eq!(resolved, std::fs::canonicalize(&rom_path).unwrap());
    }

    #[test]
    fn rejects_a_rom_outside_the_hearthdeck_cache_directory() {
        let cache_directory = tempfile::tempdir().unwrap();
        let other_directory = tempfile::tempdir().unwrap();
        let rom_path = other_directory.path().join("42.sfc");
        std::fs::write(&rom_path, b"").unwrap();
        let cache_root = std::fs::canonicalize(cache_directory.path()).unwrap();

        assert!(validate_retro_rom_path_in(rom_path.to_str().unwrap(), &cache_root).is_err());
    }

    #[test]
    fn rejects_a_rom_path_that_does_not_exist() {
        let cache_directory = tempfile::tempdir().unwrap();
        let rom_path = cache_directory.path().join("missing.sfc");
        let cache_root = std::fs::canonicalize(cache_directory.path()).unwrap();

        assert!(validate_retro_rom_path_in(rom_path.to_str().unwrap(), &cache_root).is_err());
    }
}
