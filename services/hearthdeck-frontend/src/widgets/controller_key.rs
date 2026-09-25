//! The key-cap badge that names a gamepad button on a control.
//!
//! On a 10-foot UI a control that the controller can also trigger has to say
//! *which* button does it, or the user has to guess. `controller_key_badge`
//! renders that button as a key cap, and `controller_hint` is the one shape
//! every such control should use: its label followed by the badge.

use cosmic::iced::core::alignment::Horizontal;
use cosmic::iced::widget::{container, row};
use cosmic::iced::{Alignment, Length};
use cosmic::widget::text;
use cosmic::{Element, theme};

use crate::style::{CONTROLLER_KEY_SIZE, TEXT_BODY, TEXT_CAPTION, controller_key};

/// A gamepad face button, named by position the way SDL reports it: A south,
/// B east, X west, Y north.
///
/// The full vocabulary is defined here so a call site names the button it
/// actually binds; a build that wires only some of them leaves the rest unused.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceButton {
    A,
    B,
    X,
    Y,
}

impl FaceButton {
    pub fn letter(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::X => "X",
            Self::Y => "Y",
        }
    }
}

/// A round key-cap badge showing a face button's letter.
pub fn controller_key_badge<'a, Message: 'a>(button: FaceButton) -> Element<'a, Message> {
    container(
        text::body(button.letter())
            .size(TEXT_CAPTION)
            .align_x(Horizontal::Center),
    )
    .width(Length::Fixed(CONTROLLER_KEY_SIZE))
    .height(Length::Fixed(CONTROLLER_KEY_SIZE))
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .class(theme::Container::Custom(Box::new(controller_key)))
    .into()
}

/// A control's label followed by the button that triggers it, e.g.
/// `Clear filters Ⓧ`.
pub fn controller_hint<'a, Message: 'a>(
    label: impl Into<String>,
    button: FaceButton,
) -> Element<'a, Message> {
    row![
        text::body(label.into()).size(TEXT_BODY),
        controller_key_badge(button),
    ]
    .spacing(theme::spacing().space_xs)
    .align_y(Alignment::Center)
    .into()
}
