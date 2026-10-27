//! Which text field owns the caret, if any.
//!
//! The shell tracks this itself because iced's focus is spread over widgets
//! that may not even be in the tree yet, and because the shell has to know the
//! answer for one reason: while a field has the caret the on-screen keyboard is
//! up, and the pad belongs to the keyboard instead of to the screen behind it.
//!
//! A boolean is not enough. There is more than one field - the library's search
//! box, the settings URL and the two credential fields in the RomM dialog - and
//! the messages that report focus carry no identity. A blur left over from a
//! field the user already left would clear the field that has the caret now,
//! and a field that disappears with the caret still in it would leave the pad
//! listening to a keyboard that is gone. Naming the field makes both of those
//! impossible to express.

/// A field the pad can put the caret in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// The library header's search box.
    Search,
    /// The RomM connection URL in settings.
    RommUrl,
    /// The RomM sign-in dialog's username.
    RommUsername,
    /// The RomM sign-in dialog's password.
    RommPassword,
}

/// The field that owns the caret, if one does.
#[derive(Debug, Default)]
pub struct TextEntry {
    active: Option<Field>,
}

impl TextEntry {
    /// Records that `field` took the caret, replacing whichever field had it.
    pub fn focus(&mut self, field: Field) {
        self.active = Some(field);
    }

    /// Records that `field` gave the caret up.
    ///
    /// A blur only clears the field it names: by the time it arrives the user
    /// may already be in another field - moving from username to password is
    /// exactly that - and clearing that field would drop the pad back to the
    /// screen under the keyboard.
    pub fn blur(&mut self, field: Field) {
        if self.active == Some(field) {
            self.active = None;
        }
    }

    /// The field that owns the caret.
    pub fn active(&self) -> Option<Field> {
        self.active
    }

    /// Whether any field owns the caret.
    pub fn is_active(&self) -> bool {
        self.active().is_some()
    }

    /// Gives the caret up, and returns whether there was a caret to give up.
    ///
    /// The answer is what tells the caller whether the renderer has to be told
    /// too: a field that is not on screen any more cannot be unfocused, and an
    /// unfocus that was never needed is a wasted widget walk.
    pub fn end(&mut self) -> bool {
        self.active.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{Field, TextEntry};

    #[test]
    fn focusing_a_field_makes_it_the_one_with_the_caret() {
        let mut entry = TextEntry::default();
        assert!(!entry.is_active());

        entry.focus(Field::RommUsername);

        assert_eq!(entry.active(), Some(Field::RommUsername));
        assert!(entry.is_active());
    }

    #[test]
    fn a_blur_from_another_field_leaves_the_caret_where_it_is() {
        let mut entry = TextEntry::default();
        entry.focus(Field::RommPassword);

        entry.blur(Field::Search);

        assert_eq!(
            entry.active(),
            Some(Field::RommPassword),
            "a stale blur took the caret away from the field that had it"
        );
    }

    #[test]
    fn a_blur_from_the_active_field_gives_the_caret_up() {
        let mut entry = TextEntry::default();
        entry.focus(Field::Search);

        entry.blur(Field::Search);

        assert!(!entry.is_active());
    }

    #[test]
    fn moving_between_fields_hands_the_caret_over() {
        let mut entry = TextEntry::default();
        entry.focus(Field::RommUsername);

        entry.focus(Field::RommPassword);
        entry.blur(Field::RommUsername);

        assert_eq!(entry.active(), Some(Field::RommPassword));
    }

    #[test]
    fn ending_reports_the_caret_it_took_away() {
        let mut entry = TextEntry::default();
        assert!(!entry.end(), "there was no caret to end");

        entry.focus(Field::RommUrl);

        assert!(entry.end());
        assert!(!entry.end(), "the caret was ended twice");
    }
}
