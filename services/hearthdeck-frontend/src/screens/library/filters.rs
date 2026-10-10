//! The library filter sidebar: a source's [`Facet`]s rendered generically.
//!
//! One row per facet: its label, then a `◀ value ▶` control that steps through
//! "All" and every value, wrapping. The value itself clears the facet back to
//! "All". It knows nothing about RomM or sections; it draws whatever facets the
//! source reported and reports the selection back. The header's Filter button
//! toggles it open.

use std::collections::BTreeMap;

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::Alignment;
use cosmic::iced::Length;
use cosmic::theme;
use cosmic::widget::{button, column, container, row, text};

use crate::app::Message;
use crate::fl;
use crate::providers::filter::{Facet, cycle};
use crate::style::{
    FILTER_VALUE_WIDTH, ICON_BODY, TEXT_BODY, destructive_button_class, filter_row,
    icon_button_class, passthrough, text_button_class,
};
use crate::ui;

/// The facets to offer and the current selection (facet id -> value).
pub struct Filters<'a> {
    pub facets: &'a [Facet],
    pub selected: &'a BTreeMap<String, String>,
    /// The row the controller's cursor is on, when it is in this drawer.
    ///
    /// A row is a container, and a container cannot hold focus, so the drawer is
    /// told where the cursor is rather than deriving it: this is what rings the
    /// row the confirm button would act on.
    pub cursor: Option<usize>,
}

#[must_use]
pub fn view(filters: Filters<'_>) -> Element<'_, Message> {
    let Spacing {
        space_s, space_xs, ..
    } = theme::spacing();

    let mut rows: Vec<Element<'_, Message>> = Vec::new();
    for (row, facet) in filters.facets.iter().enumerate() {
        let current = filters.selected.get(&facet.id).map(String::as_str);
        let value = current
            .and_then(|value| facet.options.iter().find(|option| option.value == value))
            .map_or_else(|| fl!("filter-all"), |option| option.label.clone());
        let step = |icon_name: &'static str, delta: i32| {
            button::custom(ui::icon(icon_name, ICON_BODY))
                .class(icon_button_class())
                .on_press(Message::SelectFacet {
                    facet: facet.id.clone(),
                    value: cycle(facet, current, delta),
                })
        };
        // The value is the row's control: pressing it clears the facet to "All".
        let clear = button::custom(
            container(text::body(value).size(TEXT_BODY))
                .width(Length::Fixed(FILTER_VALUE_WIDTH))
                .align_x(Alignment::Center)
                .class(theme::Container::Custom(Box::new(passthrough))),
        )
        .class(text_button_class())
        .on_press(Message::SelectFacet {
            facet: facet.id.clone(),
            value: None,
        });

        rows.push(
            container(
                row![
                    text::body(&facet.label).size(TEXT_BODY).width(Length::Fill),
                    step("go-previous-symbolic", -1),
                    clear,
                    step("go-next-symbolic", 1),
                ]
                .spacing(space_s)
                .align_y(Alignment::Center)
                .width(Length::Fill),
            )
            .width(Length::Fill)
            .padding([space_xs, space_s])
            .class(theme::Container::Custom(Box::new(filter_row(
                current.is_some(),
                filters.cursor == Some(row),
            ))))
            .into(),
        );
    }

    if !filters.selected.is_empty() {
        rows.push(
            button::custom(text::body(fl!("filter-clear")))
                .padding([space_xs, space_s])
                .class(destructive_button_class())
                .on_press(Message::ClearFilters)
                .into(),
        );
    }

    column(rows).spacing(space_xs).into()
}
