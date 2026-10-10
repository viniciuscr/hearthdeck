//! Provider-agnostic filter vocabulary.
//!
//! A screen filters whatever it is showing through one shape: a set of
//! [`Facet`]s the source can be narrowed by, each with the values it offers.
//! The screen renders them generically and sets a selection; the source — the
//! daemon today, any provider tomorrow — maps that to its own query.
//!
//! Nothing here knows about RomM, sections or categories; the RomM-specific
//! translation lives where the facets are produced.

use serde::Deserialize;

/// A dimension a source can be filtered by, with the values it offers.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Facet {
    /// Stable identifier the selection is keyed by.
    pub id: String,
    /// Human label, e.g. "Genre".
    pub label: String,
    /// The values this facet offers. Empty until the source reports them.
    pub options: Vec<FacetOption>,
}

/// One selectable value of a [`Facet`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FacetOption {
    /// The value passed back in a selection.
    pub value: String,
    /// Human label, which may differ from the value (e.g. `1990` -> "1990s").
    pub label: String,
}

/// The value a facet's arrow steps to: `None` ("All") first, then each option
/// in order, wrapping at both ends. `delta` is -1 for the left arrow and +1 for
/// the right.
pub fn cycle(facet: &Facet, current: Option<&str>, delta: i32) -> Option<String> {
    let len = facet.options.len() as i32 + 1;
    let index = current
        .and_then(|value| {
            facet
                .options
                .iter()
                .position(|option| option.value == value)
        })
        .map_or(0, |position| position as i32 + 1);
    let next = (index + delta).rem_euclid(len);
    if next == 0 {
        None
    } else {
        Some(facet.options[(next - 1) as usize].value.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{Facet, FacetOption, cycle};

    fn genre() -> Facet {
        Facet {
            id: "genre".to_owned(),
            label: "Genre".to_owned(),
            options: ["Action", "RPG"]
                .into_iter()
                .map(|value| FacetOption {
                    value: value.to_owned(),
                    label: value.to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn arrows_step_through_all_then_each_option_and_wrap() {
        let facet = genre();
        assert_eq!(cycle(&facet, None, 1).as_deref(), Some("Action"));
        assert_eq!(cycle(&facet, Some("Action"), 1).as_deref(), Some("RPG"));
        assert_eq!(cycle(&facet, Some("RPG"), 1), None);
        // Wrapping: back from "All" lands on the last option, and back from the
        // first option returns to "All".
        assert_eq!(cycle(&facet, None, -1).as_deref(), Some("RPG"));
        assert_eq!(cycle(&facet, Some("Action"), -1), None);
    }
}
