//! The group-management dialog: confirming a group's deletion.
//!
//! The inherited shell stood the dialog up as a layer surface, with
//! `DELETE_GROUP_WINDOW_ID` and its autosize holding the card together. This
//! app has no layer surfaces: a dialog is a card over the page, the way the
//! tile menu is, and its state is plain data the shell holds beside the menu's.
//! What a confirmed dialog asks of the config is returned as an [`Effect`] and
//! applied in the shell, so the decision itself can be tested with no app
//! around it.
//!
//! [`view`] draws the card up over the same dimming scrim the tile menu uses.
//!
//! Naming a new folder is deliberately absent here: folder names come from the
//! entries themselves, never from a name the user typed.

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::Length;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::theme;
use cosmic::widget::{button, container, dialog, icon, mouse_area, text};

use crate::app::Message;
use crate::fl;
use crate::style::{
    DIALOG_ACTION_WIDTH, DIALOG_WIDTH, ICON_LARGE, TEXT_BODY, destructive_button_class, menu_scrim,
    standard_button_class,
};

/// The dialog standing over the library, if one is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GroupDialog {
    /// Nothing stands between the pad and the grid.
    #[default]
    Closed,
    /// Confirming the deletion of one group of the current section.
    Delete {
        /// Which group, by its place in the section's tab strip.
        group: usize,
    },
}

/// What a confirmed dialog asks of the config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Take the group at this index out of the current section.
    Delete(usize),
}

impl GroupDialog {
    /// Opens the dialog that confirms taking the group at `group` away.
    pub fn open_delete(&mut self, group: usize) {
        *self = Self::Delete { group };
    }

    /// Whether a dialog stands over the page.
    pub fn is_open(&self) -> bool {
        !matches!(self, Self::Closed)
    }

    /// The confirm button: what to do about it, and the dialog stands down
    /// once it decides.
    pub fn confirm(&mut self) -> Option<Effect> {
        match self.clone() {
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

    /// The delete confirmation acts on the group it was opened for.
    #[test]
    fn a_delete_dialog_remembers_the_group_it_was_asked_about() {
        let mut dialog = GroupDialog::default();
        dialog.open_delete(2);

        assert_eq!(dialog.confirm(), Some(Effect::Delete(2)));
        assert!(!dialog.is_open());
    }

    /// Cancel is the way out that decides nothing.
    #[test]
    fn canceling_a_dialog_decides_nothing() {
        let mut dialog = GroupDialog::default();
        dialog.open_delete(1);
        assert!(dialog.is_open());

        dialog.cancel();

        assert!(!dialog.is_open(), "the dialog stood down");
        assert_eq!(dialog.confirm(), None, "and decided nothing on its way out");
    }
}
