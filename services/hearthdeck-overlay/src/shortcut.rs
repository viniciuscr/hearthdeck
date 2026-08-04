use std::error::Error;

use cosmic_config::{Config, ConfigGet, ConfigSet};
use cosmic_settings_config::Binding;
use cosmic_settings_config::shortcuts::{Action, Modifiers};

const SHORTCUT_CONFIG_ID: &str = "com.system76.CosmicSettings.Shortcuts";
const SHORTCUT_CONFIG_VERSION: u64 = 1;
const SHORTCUT_KEY: &str = "custom";
const TOGGLE_COMMAND: &str = "/usr/lib/hearthdeck/hearthdeck-overlay --toggle";

/// Adds Hearthdeck's default shortcut without replacing a user-owned binding.
pub fn install() -> Result<(), Box<dyn Error>> {
    let config = Config::new(SHORTCUT_CONFIG_ID, SHORTCUT_CONFIG_VERSION)?;
    let mut shortcuts = config.get_local(SHORTCUT_KEY).unwrap_or_default();
    let binding = Binding {
        modifiers: Modifiers::new().logo().shift(),
        key: Some(xkbcommon::xkb::Keysym::from_char('h')),
        keycode: None,
        description: Some("Toggle Hearthdeck overlay".into()),
    };
    let action = Action::Spawn(TOGGLE_COMMAND.into());

    match shortcuts.0.get(&binding) {
        Some(existing) if existing == &action => return Ok(()),
        Some(existing) => {
            return Err(format!(
                "Super+Shift+H is already assigned to {existing:?}; Hearthdeck will not replace it"
            )
            .into());
        }
        None => {}
    }

    shortcuts.0.insert(binding, action);
    config.set(SHORTCUT_KEY, shortcuts)?;
    Ok(())
}
