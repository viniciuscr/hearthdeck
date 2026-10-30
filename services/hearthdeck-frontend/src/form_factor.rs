//! Whether an application is made for the couch or for a desktop.
//!
//! Applications are split in two: the ones a person can use from a sofa with a
//! controller (streaming clients, media centres, the game launchers themselves)
//! and everything that wants a keyboard and a pointer. The split rides on each
//! record as a namespaced category, the way a store or an install source does,
//! so the tabs, the filters and the launch profile all read the same tag.
//!
//! The rules here are deliberately plain: they decide without any model, so the
//! library is usable with smart categorization switched off, and a model that
//! later knows better only moves an app across. What the user chose always wins.

use crate::library_item::LibraryItem;
use serde::{Deserialize, Serialize};

use crate::app_group::is_watch_entry;

/// The record-category prefix carrying an application's form factor.
const FORM_CATEGORY_PREFIX: &str = "hearthdeck-form:";

/// Launchers and front-ends whose whole job is to be driven from a couch. Matched
/// against the id and exec line, so each name is specific enough not to appear in
/// an unrelated path.
const COUCH_LAUNCHERS: &[&str] = &[
    "heroic",
    "steam",
    "lutris",
    "retroarch",
    "emulationstation",
    "es-de",
    "moonlight",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormFactor {
    Couch,
    Desktop,
}

impl FormFactor {
    fn tag(self) -> String {
        let name = match self {
            Self::Couch => "couch",
            Self::Desktop => "desktop",
        };
        format!("{FORM_CATEGORY_PREFIX}{name}")
    }

    /// The other one, which is what moving an application does.
    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::Couch => Self::Desktop,
            Self::Desktop => Self::Couch,
        }
    }
}

/// What the rules say `entry` is, before anyone has overridden them.
#[must_use]
pub fn classify(entry: &LibraryItem) -> FormFactor {
    if is_watch_entry(entry) || is_couch_launcher(entry) {
        FormFactor::Couch
    } else {
        FormFactor::Desktop
    }
}

fn is_couch_launcher(entry: &LibraryItem) -> bool {
    let haystack =
        format!("{} {}", entry.id, entry.exec.as_deref().unwrap_or_default()).to_lowercase();
    COUCH_LAUNCHERS
        .iter()
        .any(|launcher| haystack.contains(launcher))
}

/// Writes the rules' answer onto a record that carries none yet.
pub fn tag(entry: &mut LibraryItem) {
    let form = classify(entry);
    set(entry, form);
}

/// Makes `form` the only form tag the record carries.
pub fn set(entry: &mut LibraryItem, form: FormFactor) {
    entry
        .categories
        .retain(|category| !category.starts_with(FORM_CATEGORY_PREFIX));
    entry.categories.push(form.tag());
}

/// The form a record carries, which is what every reader goes by. A record with
/// no tag - one that never went through [`tag`] - is read by the rules.
#[must_use]
pub fn of(entry: &LibraryItem) -> FormFactor {
    [FormFactor::Couch, FormFactor::Desktop]
        .into_iter()
        .find(|form| entry.categories.contains(&form.tag()))
        .unwrap_or_else(|| classify(entry))
}

/// The category a tab filtering on `form` matches.
#[must_use]
pub fn category(form: FormFactor) -> String {
    form.tag()
}

#[cfg(test)]
mod tests {
    use super::{FormFactor, classify, of, set, tag};
    use crate::library_item::LibraryItem;
    use cosmic::desktop::fde::IconSource;

    fn entry(id: &str, exec: &str) -> LibraryItem {
        LibraryItem {
            id: id.into(),
            name: id.into(),
            wm_class: None,
            exec: Some(exec.into()),
            icon: IconSource::Name(String::new()),
            categories: Vec::new(),
        }
    }

    #[test]
    fn streaming_clients_and_launchers_are_for_the_couch() {
        assert_eq!(
            classify(&entry("com.stremio.Stremio.desktop", "stremio")),
            FormFactor::Couch
        );
        assert_eq!(
            classify(&entry("com.heroicgameslauncher.hgl.desktop", "heroic")),
            FormFactor::Couch
        );
    }

    #[test]
    fn anything_else_wants_a_desktop() {
        assert_eq!(
            classify(&entry("org.gnome.Terminal.desktop", "gnome-terminal")),
            FormFactor::Desktop
        );
        assert_eq!(
            classify(&entry("org.gimp.GIMP.desktop", "gimp")),
            FormFactor::Desktop
        );
    }

    #[test]
    fn a_record_carries_exactly_one_form_and_the_tag_is_what_is_read() {
        let mut app = entry("org.gimp.GIMP.desktop", "gimp");
        tag(&mut app);
        assert_eq!(of(&app), FormFactor::Desktop);

        set(&mut app, FormFactor::Couch);

        assert_eq!(of(&app), FormFactor::Couch);
        assert_eq!(
            app.categories
                .iter()
                .filter(|category| category.starts_with("hearthdeck-form:"))
                .count(),
            1
        );
    }

    #[test]
    fn an_untagged_record_is_read_by_the_rules() {
        assert_eq!(
            of(&entry("org.gimp.GIMP.desktop", "gimp")),
            FormFactor::Desktop
        );
    }
}
