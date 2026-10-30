//! Deterministic identity rules shared by catalog serving and heuristic scans.

use hearthdeck_protocol::{CatalogClassification, FormFactor, Section};

const WATCH_SERVICES: &[&str] = &[
    "stremio",
    "netflix",
    "primevideo",
    "prime video",
    "prime-video",
    "disneyplus",
    "disney+",
    "hulu",
    "hbomax",
    "hbo max",
    "max.com",
    "crunchyroll",
    "paramountplus",
    "paramount+",
    "peacock",
    "tubitv",
    "tubi.tv",
    "plutotv",
    "pluto.tv",
    "appletv",
    "apple tv",
    "tv.apple",
    "plex",
    "jellyfin",
    "emby",
    "kodi",
];

const EMULATORS: &[&str] = &[
    "retroarch",
    "dolphin",
    "pcsx2",
    "duckstation",
    "rpcs3",
    "citra",
    "yuzu",
    "ryujinx",
    "mame",
    "mupen",
    "xemu",
    "melon",
    "ppsspp",
    "snes9x",
    "mgba",
    "mednafen",
    "desmume",
    "fsuae",
    "scummvm",
    "cemu",
    "ares",
    "bsnes",
    "limbo",
    "flycast",
    "redream",
    "standalone",
    "emulator",
];

const COUCH_LAUNCHERS: &[&str] = &[
    "heroic",
    "steam",
    "lutris",
    "retroarch",
    "emulationstation",
    "es-de",
    "moonlight",
];

#[must_use]
pub(super) fn emulator(signal: &str) -> bool {
    EMULATORS.iter().any(|name| signal.contains(name))
}

#[must_use]
pub(super) fn streaming(signal: &str) -> bool {
    WATCH_SERVICES.iter().any(|name| signal.contains(name))
}

/// Resolves library behavior without a model; a model may only promote Couch.
#[must_use]
pub fn classify_catalog(
    id: &str,
    exec: Option<&str>,
    section: Section,
    modeled_form: Option<FormFactor>,
) -> CatalogClassification {
    let signal = format!("hearthdeck:{id} {}", exec.unwrap_or_default()).to_lowercase();
    let streaming = section == Section::Applications && streaming(&signal);
    let section = match section {
        Section::PcGames if emulator(&signal) => Section::ConsoleGames,
        section => section,
    };
    let form_factor = (section == Section::Applications).then(|| {
        if streaming
            || COUCH_LAUNCHERS.iter().any(|name| signal.contains(name))
            || modeled_form == Some(FormFactor::Couch)
        {
            FormFactor::Couch
        } else {
            FormFactor::Desktop
        }
    });
    CatalogClassification {
        section,
        streaming,
        form_factor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_rules_preserve_library_placement_and_promotion() {
        let cases = [
            (
                "net.stremio.Stremio",
                None,
                false,
                false,
                None,
                Section::Applications,
                true,
                Some(FormFactor::Couch),
            ),
            (
                "web-app",
                Some("chromium --app=https://netflix.com"),
                false,
                false,
                None,
                Section::Applications,
                true,
                Some(FormFactor::Couch),
            ),
            (
                "org.gimp.GIMP",
                None,
                false,
                false,
                None,
                Section::Applications,
                false,
                Some(FormFactor::Desktop),
            ),
            (
                "heroic",
                None,
                false,
                false,
                None,
                Section::Applications,
                false,
                Some(FormFactor::Couch),
            ),
            (
                "retroarch",
                None,
                false,
                false,
                None,
                Section::Applications,
                false,
                Some(FormFactor::Couch),
            ),
            (
                "netflix-game",
                None,
                true,
                false,
                None,
                Section::PcGames,
                false,
                None,
            ),
            (
                "dolphin",
                None,
                true,
                false,
                None,
                Section::ConsoleGames,
                false,
                None,
            ),
            (
                "romm:42",
                None,
                true,
                true,
                None,
                Section::ConsoleGames,
                false,
                None,
            ),
            (
                "org.gimp.GIMP",
                None,
                false,
                false,
                Some(FormFactor::Couch),
                Section::Applications,
                false,
                Some(FormFactor::Couch),
            ),
            (
                "net.stremio.Stremio",
                None,
                false,
                false,
                Some(FormFactor::Desktop),
                Section::Applications,
                true,
                Some(FormFactor::Couch),
            ),
        ];
        for (id, exec, game, console, model, section, streaming, form_factor) in cases {
            let initial = if !game {
                Section::Applications
            } else if console {
                Section::ConsoleGames
            } else {
                Section::PcGames
            };
            assert_eq!(
                classify_catalog(id, exec, initial, model),
                CatalogClassification {
                    section,
                    streaming,
                    form_factor
                },
                "{id}"
            );
        }
    }

    #[test]
    fn media_categories_do_not_make_a_streaming_client() {
        for id in [
            "org.pulseaudio.pavucontrol",
            "qvidcap",
            "com.system76.CosmicPlayer",
        ] {
            assert!(
                !classify_catalog(id, None, Section::Applications, None).streaming,
                "{id}"
            );
        }
    }
}
