//! The library filter drawer: its source facets, selection and local decisions.
//!
//! One row per facet: its label, then a `◀ value ▶` control that steps through
//! "All" and every value, wrapping. The value itself clears the facet back to
//! "All". The view draws whatever facets the state supplies; local sections
//! derive them from the catalog and console labels follow the locale. Changes
//! ask the shell to refilter through the current source.

use std::collections::BTreeMap;
use std::sync::Arc;

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::Alignment;
use cosmic::iced::Length;
use cosmic::theme;
use cosmic::widget::{button, column, container, row, text};

use crate::app_group::{RommFacet, Section, section_facets};
use crate::fl;
use crate::library_item::LibraryItem;
use crate::providers::filter::{Facet, cycle};
use crate::style::{
    FILTER_VALUE_WIDTH, ICON_BODY, TEXT_BODY, destructive_button_class, filter_row,
    icon_button_class, passthrough, text_button_class,
};
use crate::ui;

#[derive(Debug, Clone)]
pub enum Message {
    Select {
        facet: String,
        value: Option<String>,
    },
    Cycle {
        row: usize,
        delta: i32,
    },
    Toggle,
    Close,
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    None,
    Refilter,
}

#[derive(Default)]
pub struct State {
    facets: Vec<Facet>,
    selected: BTreeMap<String, String>,
    open: bool,
}

impl State {
    #[must_use]
    pub fn facets(&self) -> &[Facet] {
        &self.facets
    }

    #[must_use]
    pub fn selected(&self) -> &BTreeMap<String, String> {
        &self.selected
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn update(&mut self, message: Message) -> Effect {
        match message {
            Message::Select { facet, value } => {
                match value {
                    Some(value) => {
                        self.selected.insert(facet, value);
                    }
                    None => {
                        self.selected.remove(&facet);
                    }
                }
                Effect::Refilter
            }
            Message::Cycle { row, delta } => {
                let Some(facet) = self.facets.get(row) else {
                    return Effect::None;
                };
                let current = self.selected.get(&facet.id).map(String::as_str);
                self.update(Message::Select {
                    facet: facet.id.clone(),
                    value: cycle(facet, current, delta),
                })
            }
            Message::Clear => {
                self.selected.clear();
                Effect::Refilter
            }
            Message::Toggle => {
                self.open = !self.open;
                Effect::None
            }
            Message::Close => {
                self.open = false;
                Effect::None
            }
        }
    }

    pub fn load_console_facets(&mut self, facets: Vec<Facet>) {
        self.facets = facets
            .into_iter()
            .map(|mut facet| {
                if let Some(known) = RommFacet::from_id(&facet.id) {
                    facet.label = known.name();
                }
                facet
            })
            .collect();
    }

    pub fn load_local_facets(&mut self, section: Section, catalog: &[Arc<LibraryItem>]) {
        if section != Section::ConsoleGames {
            self.facets = section_facets(section, catalog);
        }
    }

    pub fn clear_source(&mut self) {
        self.facets.clear();
        self.selected.clear();
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

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
                .on_press(Message::Select {
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
        .on_press(Message::Select {
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
                .on_press(Message::Clear)
                .into(),
        );
    }

    column(rows).spacing(space_xs).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::filter::FacetOption;

    fn genre() -> Facet {
        Facet {
            id: "genre".into(),
            label: "upstream genre".into(),
            options: ["Action", "RPG"]
                .into_iter()
                .map(|value| FacetOption {
                    value: value.into(),
                    label: value.into(),
                })
                .collect(),
        }
    }

    #[test]
    fn drawer_visibility_does_not_change_the_query() {
        let mut state = State::default();
        assert_eq!(
            state.update(Message::Select {
                facet: "genre".into(),
                value: Some("Action".into())
            }),
            Effect::Refilter
        );
        assert_eq!(state.update(Message::Toggle), Effect::None);
        assert!(state.open);
        assert_eq!(state.update(Message::Close), Effect::None);
        assert!(!state.open);
        assert_eq!(
            state.selected.get("genre").map(String::as_str),
            Some("Action")
        );
        assert_eq!(state.update(Message::Toggle), Effect::None);
        assert_eq!(state.update(Message::Toggle), Effect::None);
        assert!(!state.open);
    }

    #[test]
    fn controller_and_clicks_share_selection_and_clear_rules() {
        let mut state = State {
            facets: vec![genre()],
            ..State::default()
        };
        assert_eq!(
            state.update(Message::Cycle { row: 0, delta: -1 }),
            Effect::Refilter
        );
        assert_eq!(state.selected.get("genre").map(String::as_str), Some("RPG"));
        assert_eq!(
            state.update(Message::Cycle { row: 0, delta: 1 }),
            Effect::Refilter
        );
        assert!(state.selected.is_empty());
        state.update(Message::Select {
            facet: "genre".into(),
            value: Some("Action".into()),
        });
        state.update(Message::Select {
            facet: "region".into(),
            value: Some("Europe".into()),
        });
        state.update(Message::Select {
            facet: "genre".into(),
            value: None,
        });
        assert_eq!(state.selected.len(), 1);
        state.update(Message::Toggle);
        assert_eq!(state.update(Message::Clear), Effect::Refilter);
        assert!(state.selected.is_empty());
        assert!(state.open);
        assert_eq!(state.update(Message::Clear), Effect::Refilter);
        assert_eq!(
            state.update(Message::Cycle { row: 9, delta: 1 }),
            Effect::None
        );
    }

    #[test]
    fn loading_facets_localizes_known_ids_and_keeps_unknown_labels_and_selection() {
        let mut state = State::default();
        state.update(Message::Select {
            facet: "genre".into(),
            value: Some("Action".into()),
        });
        state.load_console_facets(vec![
            genre(),
            Facet {
                id: "custom".into(),
                label: "Custom label".into(),
                options: Vec::new(),
            },
        ]);
        assert_eq!(state.facets[0].label, fl!("filter-genre"));
        assert_eq!(state.facets[1].label, "Custom label");
        assert_eq!(
            state.selected.get("genre").map(String::as_str),
            Some("Action")
        );
    }

    #[test]
    fn source_removal_keeps_visibility_but_scope_reset_closes_the_drawer() {
        let mut state = State {
            facets: vec![genre()],
            ..State::default()
        };
        state.update(Message::Toggle);
        state.update(Message::Select {
            facet: "genre".into(),
            value: Some("Action".into()),
        });
        state.clear_source();
        assert!(state.facets.is_empty());
        assert!(state.selected.is_empty());
        assert!(state.open);

        state.facets = vec![genre()];
        state.update(Message::Select {
            facet: "genre".into(),
            value: Some("RPG".into()),
        });
        state.reset();
        assert!(state.facets.is_empty());
        assert!(state.selected.is_empty());
        assert!(!state.open);
    }
}
