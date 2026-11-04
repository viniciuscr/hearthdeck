//! Library menu actions, shared by rendering and controller confirmation.

use crate::app_group::{AppLibraryConfig, Section};
use crate::fl;
use crate::form_factor::{self, FormFactor};
use crate::library_item::LibraryItem;
use crate::widgets::menu::Row;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Run,
    ToggleControllerCompatibility,
    MoveToForm(FormFactor),
}

pub fn actions(section: Section, entry: &LibraryItem) -> impl Iterator<Item = Action> {
    [Action::Run, Action::ToggleControllerCompatibility]
        .into_iter()
        .chain(
            (section == Section::Applications)
                .then(|| Action::MoveToForm(form_factor::of(entry).other())),
        )
}

pub fn rows(
    section: Section,
    entry: &LibraryItem,
    config: &AppLibraryConfig,
) -> impl Iterator<Item = Row<Action>> {
    actions(section, entry).map(|action| {
        let label = match action {
            Action::Run => fl!("run"),
            Action::ToggleControllerCompatibility => {
                if config.desktop_input_for(entry) {
                    fl!("disable-controller-compatibility")
                } else {
                    fl!("enable-controller-compatibility")
                }
            }
            Action::MoveToForm(FormFactor::Couch) => fl!("move-to-couch"),
            Action::MoveToForm(FormFactor::Desktop) => fl!("move-to-desktop"),
        };
        Row::new(label, action)
    })
}

#[cfg(test)]
mod tests {
    use super::{Action, actions, rows};
    use crate::app_group::{AppLibraryConfig, Section};
    use crate::form_factor::FormFactor;
    use crate::library_item::LibraryItem;

    #[test]
    fn only_applications_offer_a_move_to_the_other_form() {
        for form in [FormFactor::Couch, FormFactor::Desktop] {
            let entry = LibraryItem {
                id: "writer".to_owned(),
                name: "Writer".to_owned(),
                exec: None,
                icon: cosmic::desktop::fde::IconSource::Name("writer".to_owned()),
                categories: Vec::new(),
                wm_class: None,
                form_factor: Some(form),
                store: None,
                sources: Vec::new(),
                console: None,
                section: Section::Applications,
            };
            for section in Section::ALL {
                let expected = if section == Section::Applications {
                    vec![
                        Action::Run,
                        Action::ToggleControllerCompatibility,
                        Action::MoveToForm(form.other()),
                    ]
                } else {
                    vec![Action::Run, Action::ToggleControllerCompatibility]
                };
                assert_eq!(
                    actions(section, &entry).collect::<Vec<_>>(),
                    expected,
                    "{section:?}, {form:?}"
                );
                assert_eq!(
                    rows(section, &entry, &AppLibraryConfig::default())
                        .map(|row| row.message)
                        .collect::<Vec<_>>(),
                    expected,
                    "rendered actions for {section:?}, {form:?}"
                );
            }
        }
    }
}
