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
