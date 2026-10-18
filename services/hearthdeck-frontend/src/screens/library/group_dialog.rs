//! The group-management dialogs: naming a new group, and confirming a delete.
//!
//! The inherited shell stood these two up as layer surfaces - a window per
//! dialog, with `DELETE_GROUP_WINDOW_ID` and its autosize holding the card
//! together. This app has no layer surfaces: a dialog is a card over the page,
//! the way the tile menu is, and its state is plain data the shell holds
//! beside the menu's. What a confirmed dialog asks of the config is returned
//! as an [`Effect`] and applied in the shell, so the decision itself can be
//! tested with no app around it.
//!
//! [`view`] draws whichever card is up over the same dimming scrim the tile
//! menu uses.

use std::sync::LazyLock;

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::Length;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::theme;
use cosmic::widget::{Id, button, container, dialog, icon, mouse_area, text, text_input};

use crate::app::Message;
use crate::fl;
use crate::style::{
    DIALOG_ACTION_WIDTH, DIALOG_WIDTH, ICON_LARGE, TEXT_BODY, destructive_button_class, menu_scrim,
    primary_action_button_class, standard_button_class,
};

/// The dialog standing over the library, if one is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GroupDialog {
    /// Nothing stands between the pad and the grid.
    #[default]
    Closed,
    /// Naming the group about to be created - the only dialog with a field.
    Create {
        /// What its name field holds.
        name: String,
    },
    /// Confirming the deletion of one group of the current section.
    Delete {
        /// Which group, by its place in the section's tab strip.
        group: usize,
    },
}

/// What a confirmed dialog asks of the config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Create a group of this name in the current section.
    Create(String),
    /// Take the group at this index out of the current section.
    Delete(usize),
}

impl GroupDialog {
    /// Opens the dialog that names a new group.
    pub fn open_create(&mut self) {
        *self = Self::Create {
            name: String::new(),
        };
    }

    /// Opens the dialog that confirms taking the group at `group` away.
    pub fn open_delete(&mut self, group: usize) {
        *self = Self::Delete { group };
    }

    /// Whether a dialog stands over the page.
    pub fn is_open(&self) -> bool {
        !matches!(self, Self::Closed)
    }

    /// What the name field holds - empty unless the create dialog is up, which
    /// is the only one with a field to fill.
    pub fn name(&self) -> &str {
        match self {
            Self::Create { name } => name,
            Self::Closed | Self::Delete { .. } => "",
        }
    }

    /// Records what the name field was changed to. Only the create dialog has
    /// a field, so typing elsewhere in the shell cannot rename anything here.
    pub fn set_name(&mut self, name: String) {
        if let Self::Create { name: slot } = self {
            *slot = name;
        }
    }

    /// The confirm button: what to do about it, and the dialog stands down
    /// once it decides.
    ///
    /// A folder with no name is not a folder - confirming the empty field
    /// decides nothing and keeps the dialog up, asking.
    pub fn confirm(&mut self) -> Option<Effect> {
        match self.clone() {
            Self::Create { name } => {
                let name = name.trim().to_owned();
                if name.is_empty() {
                    return None;
                }
                *self = Self::Closed;
                Some(Effect::Create(name))
            }
            Self::Delete { group } => {
                *self = Self::Closed;
                Some(Effect::Delete(group))
            }
            Self::Closed => None,
        }
    }

    /// The cancel button: stand down without deciding anything.
    pub fn cancel(&mut self) {
        *self = Self::Closed;
    }
}

/// The widget id of the create dialog's name field, which the opening focuses.
static NAME_INPUT_ID: LazyLock<Id> = LazyLock::new(|| Id::new("group-name"));

/// The widget id of the name field in the create dialog.
pub fn name_input_id() -> Id {
    NAME_INPUT_ID.clone()
}

/// The dialog standing over the library: `None` while no dialog is up, and
/// otherwise the card centered on the same dimming scrim the tile menu uses,
/// with a press on the scrim standing it down.
///
/// `MouseArea` hands a press to its content first, so the buttons keep theirs.
pub fn view(state: &GroupDialog) -> Option<Element<'_, Message>> {
    let Spacing {
        space_xxs, space_s, ..
    } = theme::spacing();

    let card: Element<'_, Message> = match state {
        GroupDialog::Closed => return None,
        GroupDialog::Create { .. } => dialog::dialog()
            .icon(icon::from_name("list-add-symbolic").size(ICON_LARGE))
            .title(fl!("new-group"))
            .control(
                text_input(fl!("new-group-placeholder"), state.name())
                    .id(name_input_id())
                    .on_input(Message::GroupNameChanged)
                    .on_submit(|_| Message::ConfirmGroupDialog)
                    .width(Length::Fixed(DIALOG_WIDTH)),
            )
            .primary_action(
                button::custom(
                    text::body(fl!("create-new"))
                        .size(TEXT_BODY)
                        .center()
                        .width(Length::Fill),
                )
                .class(primary_action_button_class())
                .on_press(Message::ConfirmGroupDialog)
                .padding([space_xxs, space_s])
                .width(DIALOG_ACTION_WIDTH),
            )
            .secondary_action(
                button::custom(
                    text::body(fl!("cancel"))
                        .size(TEXT_BODY)
                        .center()
                        .width(Length::Fill),
                )
                .class(standard_button_class())
                .on_press(Message::CancelGroupDialog)
                .padding([space_xxs, space_s])
                .width(DIALOG_ACTION_WIDTH),
            )
            .width(Length::Fixed(DIALOG_WIDTH))
            .into(),
        GroupDialog::Delete { .. } => dialog::dialog()
            .icon(icon::from_name("edit-delete-symbolic").size(ICON_LARGE))
            .title(fl!("delete-folder"))
            .body(fl!("delete-folder", "msg"))
            .primary_action(
                button::custom(
                    text::body(fl!("delete"))
                        .size(TEXT_BODY)
                        .center()
                        .width(Length::Fill),
                )
                .class(destructive_button_class())
                .on_press(Message::ConfirmGroupDialog)
                .padding([space_xxs, space_s])
                .width(DIALOG_ACTION_WIDTH),
            )
            .secondary_action(
                button::custom(
                    text::body(fl!("cancel"))
                        .size(TEXT_BODY)
                        .center()
                        .width(Length::Fill),
                )
                .class(standard_button_class())
                .on_press(Message::CancelGroupDialog)
                .padding([space_xxs, space_s])
                .width(DIALOG_ACTION_WIDTH),
            )
            .width(Length::Fixed(DIALOG_WIDTH))
            .into(),
    };

    Some(
        mouse_area(
            container(card)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .class(theme::Container::Custom(Box::new(menu_scrim))),
        )
        .on_press(Message::CancelGroupDialog)
        .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::{Effect, GroupDialog};

    /// A new group starts unnamed, and what is typed becomes the group the
    /// confirm button creates.
    #[test]
    fn a_create_dialog_creates_the_group_it_was_named() {
        let mut dialog = GroupDialog::default();
        dialog.open_create();
        assert_eq!(dialog.name(), "", "a new group starts unnamed");
        assert!(dialog.is_open());

        dialog.set_name("Speedruns".to_owned());

        assert_eq!(
            dialog.confirm(),
            Some(Effect::Create("Speedruns".to_owned()))
        );
        assert!(
            !dialog.is_open(),
            "and the dialog stands down once it decides"
        );
    }

    /// A folder with no name is not a folder anywhere: confirming the empty
    /// field creates nothing and keeps the dialog up, asking.
    #[test]
    fn an_unnamed_group_is_not_created() {
        let mut dialog = GroupDialog::default();
        dialog.open_create();

        assert_eq!(dialog.confirm(), None);
        assert!(dialog.is_open(), "the dialog is still asking for a name");

        dialog.set_name("   ".to_owned());
        assert_eq!(dialog.confirm(), None, "whitespace is not a name either");
    }

    /// The delete confirmation acts on the group it was opened for.
    #[test]
    fn a_delete_dialog_remembers_the_group_it_was_asked_about() {
        let mut dialog = GroupDialog::default();
        dialog.open_delete(2);

        assert_eq!(dialog.confirm(), Some(Effect::Delete(2)));
        assert!(!dialog.is_open());
    }

    /// Cancel is the way out that decides nothing, from either dialog.
    #[test]
    fn canceling_a_dialog_decides_nothing() {
        let mut dialog = GroupDialog::default();
        dialog.open_create();
        dialog.set_name("Speedruns".to_owned());
        assert!(dialog.is_open());

        dialog.cancel();

        assert!(!dialog.is_open(), "the dialog stood down");
        assert_eq!(dialog.confirm(), None, "and decided nothing on its way out");

        dialog.open_delete(1);
        dialog.cancel();
        assert!(!dialog.is_open());
        assert_eq!(dialog.confirm(), None);
    }
}
