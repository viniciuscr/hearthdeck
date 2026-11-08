//! Semantic navigation shared by broker input and keyboard input.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GamepadEvent {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    Confirm,
    Back,
    Search,
    ContextMenu,
    PrevGroup,
    NextGroup,
    PrevTab,
    NextTab,
    FilterPanel,
    Keyboard,
}

/// The word the settings window understands for this action.
///
/// It is a name, not a second vocabulary: the window turns each word back into
/// the gpui action its own keys are bound to, so the pad and the keyboard reach
/// one handler. `None` for the actions only the library page has a use for.
#[must_use]
pub fn name(event: GamepadEvent) -> Option<&'static str> {
    Some(match event {
        GamepadEvent::MoveUp => "up",
        GamepadEvent::MoveDown => "down",
        GamepadEvent::MoveLeft => "left",
        GamepadEvent::MoveRight => "right",
        GamepadEvent::Confirm => "confirm",
        GamepadEvent::Back => "back",
        GamepadEvent::Search
        | GamepadEvent::ContextMenu
        | GamepadEvent::PrevGroup
        | GamepadEvent::NextGroup
        | GamepadEvent::PrevTab
        | GamepadEvent::NextTab
        | GamepadEvent::FilterPanel
        | GamepadEvent::Keyboard => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{GamepadEvent, name};

    #[test]
    fn the_actions_the_settings_window_shares_are_named() {
        assert_eq!(name(GamepadEvent::MoveUp), Some("up"));
        assert_eq!(name(GamepadEvent::MoveDown), Some("down"));
        assert_eq!(name(GamepadEvent::MoveLeft), Some("left"));
        assert_eq!(name(GamepadEvent::MoveRight), Some("right"));
        assert_eq!(name(GamepadEvent::Confirm), Some("confirm"));
        assert_eq!(name(GamepadEvent::Back), Some("back"));
    }

    #[test]
    fn the_actions_only_the_library_uses_are_not_forwarded() {
        for event in [
            GamepadEvent::Search,
            GamepadEvent::ContextMenu,
            GamepadEvent::PrevGroup,
            GamepadEvent::NextGroup,
            GamepadEvent::PrevTab,
            GamepadEvent::NextTab,
            GamepadEvent::FilterPanel,
            GamepadEvent::Keyboard,
        ] {
            assert_eq!(name(event), None, "{event:?} has no settings meaning");
        }
    }
}
