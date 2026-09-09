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

use std::path::Path;

use anyhow::{Context, Result};

include!(concat!(env!("OUT_DIR"), "/retro_profiles.rs"));

/// The bundled profiles as `(file name, file contents)`, sorted by name for
/// deterministic builds. Contents are RetroArch config text: each file keys a
/// mapping on `input_vendor_id` / `input_product_id` / `input_device`.
pub fn bundled_retroarch_profiles() -> &'static [(&'static str, &'static str)] {
    RETROARCH_PROFILES
}

/// Writes every bundled profile that is not already present in `directory`.
///
/// Seeding is intentionally write-if-missing rather than write-always: the
/// autoconfig directory doubles as the target of RetroArch's own "Save
/// Controller Profile" wizard, so a mapping the user re-saved (or fixed) must
/// survive the next launch instead of being reverted to the shipped default.
/// The trade-off is that a profile corrected in a newer Hearthdeck release
/// does not overwrite an already-seeded copy; refreshes only reach fresh
/// installs (or machines whose copy was deleted).
pub async fn seed_retroarch_profiles(directory: &Path) -> Result<usize> {
    let mut seeded = 0;
    for (name, contents) in bundled_retroarch_profiles() {
        let profile_path = directory.join(name);
        if tokio::fs::try_exists(&profile_path)
            .await
            .with_context(|| {
                format!("could not check for existing RetroArch autoconfig profile {name}")
            })?
        {
            continue;
        }
        tokio::fs::write(&profile_path, contents)
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
    use super::bundled_retroarch_profiles;

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
