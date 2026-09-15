//! Bundled RetroArch joypad autoconfig profiles.
//!
//! `build.rs` scans `profiles/udev/` (vendored from
//! libretro/retroarch-joypad-autoconfig, see that folder's README) and emits
//! the `RETROARCH_PROFILES` constant this file includes, so the bridge binary
//! is self-contained: a pad plugged into a Hearthdeck-managed RetroArch
//! install gets a matching profile even though distro RetroArch ships none.
//!
//! RetroArch looks for profiles under `<joypad_autoconfig_dir>/<input
//! driver>/` (the upstream package installs them as
//! `.../autoconfig/udev/<device>.cfg`), so seeding targets the `udev`
//! subdirectory of Hearthdeck's autoconfig dir.
//!
//! Profiles are seeded with their menu-toggle binding unbound - see
//! `without_menu_toggle_binding`.

use std::path::Path;

use anyhow::{Context, Result};
use tracing::warn;

include!(concat!(env!("OUT_DIR"), "/retro_profiles.rs"));

/// The autoconfig key that binds a pad's Guide/Home button as a single-press
/// RetroArch menu toggle.
const MENU_TOGGLE_KEY: &str = "input_menu_toggle_btn";

/// The bundled profiles as `(file name, file contents)`, sorted by name for
/// deterministic builds. Contents are RetroArch config text: each file keys a
/// mapping on `input_vendor_id` / `input_product_id` / `input_device`.
pub fn bundled_retroarch_profiles() -> &'static [(&'static str, &'static str)] {
    RETROARCH_PROFILES
}

/// A profile's contents with its menu-toggle binding rewritten to unbound
/// (`"nul"`), leaving everything else byte-for-byte intact.
///
/// RetroArch applies joypad autoconfig when a pad is *detected*, which is after
/// it has read Hearthdeck's config files, so a binding that comes from a
/// profile cannot be undone from the config side: it has to not be in the
/// profile. Guide/Home is Hearthdeck's own button (it opens the quick-menu
/// overlay), so it is removed here, at the source. Only that exact key is
/// touched - the profiles also carry an `input_menu_toggle_btn_label`, and a
/// few have a typo'd `input_menu_toggle`, neither of which is a binding.
pub fn without_menu_toggle_binding(contents: &str) -> String {
    if !contents.contains(MENU_TOGGLE_KEY) {
        return contents.to_owned();
    }
    contents
        .split('\n')
        .map(|line| match line.split_once('=') {
            Some((key, _)) if key.trim() == MENU_TOGGLE_KEY => {
                format!("{MENU_TOGGLE_KEY} = \"nul\"")
            }
            _ => line.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Writes every bundled profile into `directory`, with its menu-toggle binding
/// unbound (see `without_menu_toggle_binding`).
///
/// A copy already on disk is rewritten only when it is recognisably
/// Hearthdeck's own: the vendored profile verbatim (what releases before the
/// menu-toggle rewrite wrote) or that profile rewritten. Anything else is the
/// user's - the autoconfig directory doubles as the target of RetroArch's own
/// "Save Controller Profile" wizard - so a re-saved or hand-fixed mapping still
/// survives the next launch. The trade-off is the same as it was when seeding
/// was strictly write-if-missing: a profile *Hearthdeck* corrects in a newer
/// release reaches only a machine whose copy still matches a shipped version.
pub async fn seed_retroarch_profiles(directory: &Path) -> Result<usize> {
    let mut seeded = 0;
    for (name, contents) in bundled_retroarch_profiles() {
        let unbound = without_menu_toggle_binding(contents);
        let profile_path = directory.join(name);
        let existing = match tokio::fs::read_to_string(&profile_path).await {
            Ok(existing) => Some(existing),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                // Not readable, so not ours to judge. Leave the file and let
                // the launch continue rather than fail it over a profile.
                warn!(profile = name, %error, "ignored unreadable RetroArch autoconfig profile");
                continue;
            }
        };
        let hearthdecks = match existing.as_deref() {
            None => true,
            Some(existing) => existing == *contents || existing == unbound.as_str(),
        };
        if !hearthdecks || existing.as_deref() == Some(unbound.as_str()) {
            continue;
        }
        tokio::fs::write(&profile_path, unbound)
            .await
            .with_context(|| {
                format!("could not write bundled RetroArch autoconfig profile {name}")
            })?;
        seeded += 1;
    }
    Ok(seeded)
}

#[cfg(test)]
mod tests {
    use super::{bundled_retroarch_profiles, without_menu_toggle_binding};

    #[test]
    fn unbinds_the_menu_toggle_binding_and_nothing_else() {
        let profile = concat!(
            "input_driver = \"udev\"\n",
            "input_menu_toggle_btn = \"8\"\n",
            "input_menu_toggle = \"8\"\n",
            "input_menu_toggle_btn_label = \"Guide\"\n",
        );

        assert_eq!(
            without_menu_toggle_binding(profile),
            concat!(
                "input_driver = \"udev\"\n",
                "input_menu_toggle_btn = \"nul\"\n",
                "input_menu_toggle = \"8\"\n",
                "input_menu_toggle_btn_label = \"Guide\"\n",
            )
        );
    }

    #[test]
    fn a_profile_without_the_binding_is_unchanged() {
        let profile = "input_driver = \"udev\"\ninput_a_btn = \"1\"\n";

        assert_eq!(without_menu_toggle_binding(profile), profile);
    }

    #[test]
    fn every_bundled_profile_with_the_binding_loses_it() {
        for (name, contents) in bundled_retroarch_profiles() {
            let unbound = without_menu_toggle_binding(contents);
            let bound = unbound.lines().any(|line| {
                line.split_once('=').map(|(key, _)| key.trim()) == Some(super::MENU_TOGGLE_KEY)
                    && line.trim_end() != format!("{} = \"nul\"", super::MENU_TOGGLE_KEY)
            });

            assert!(!bound, "{name} still binds the Guide button");
        }
    }

    #[test]
    fn binds_the_guide_button_in_the_platforms_microsoft_pad_profile() {
        // The pad this integration was built against is the interesting one to
        // pin down: its profile is *why* the rewrite above exists, so it must
        // actually be one of the profiles that binds Guide, or the rewrite is
        // being tested against nothing.
        let profile = bundled_retroarch_profiles()
            .iter()
            .find(|(name, _)| *name == "Microsoft_X-Box_Series_XS_pad.cfg")
            .map(|(_, contents)| *contents)
            .expect("the Xbox Series X|S profile is bundled");

        assert!(profile.contains("input_menu_toggle_btn = \"8\""));
    }

    #[test]
    fn bundles_profiles_for_the_platforms_microsoft_pads() {
        let profiles = bundled_retroarch_profiles();

        assert!(!profiles.is_empty());
        // The Xbox Series X|S pad over the kernel `xpad` driver (USB pid
        // 2834) is the pad this integration was built against; if its
        // profile ever disappears the vendored set regressed silently.
        assert!(
            profiles
                .iter()
                .any(|(name, _)| *name == "Microsoft_X-Box_Series_XS_pad.cfg")
        );
        assert!(
            profiles
                .iter()
                .any(|(_, contents)| contents.contains("input_product_id = \"2834\""))
        );
    }

    #[test]
    fn every_bundled_entry_is_a_profile_file() {
        for (name, contents) in bundled_retroarch_profiles() {
            assert!(name.ends_with(".cfg"), "unexpected file {name}");
            assert!(!contents.is_empty(), "profile {name} is empty");
        }
    }
}
