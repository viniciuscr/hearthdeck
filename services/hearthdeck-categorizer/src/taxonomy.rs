//! The category universe the scan chooses from.
//!
//! Laya is an encoder, not a generator: it answers *constrained* questions, so
//! it can pick a category only from a list we hand it. "Deciding which
//! categories to create" therefore has two halves — this module defines the
//! candidates (the project's opinion about what a TV library should look like),
//! and [`crate::ScanReport`] aggregates the model's picks into the handful of
//! tabs that are actually worth showing.

use serde::{Deserialize, Serialize};

use crate::error::{CategorizerError, Result};

/// The fixed top-level navigation tabs. Mirrors the frontend's `Section`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    PcGames,
    ConsoleGames,
    Applications,
}

impl Section {
    /// Every section, in sidebar order.
    pub const ALL: [Section; 3] = [
        Section::PcGames,
        Section::ConsoleGames,
        Section::Applications,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::PcGames => "PC games",
            Self::ConsoleGames => "Console games",
            Self::Applications => "Applications",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::PcGames => "pc_games",
            Self::ConsoleGames => "console_games",
            Self::Applications => "applications",
        }
    }

    /// Place a record from its two boolean traits, the same way the current
    /// frontend splits games from emulators from everything else.
    ///
    /// The emulator test comes first on purpose: emulator software is
    /// game-related even when its record forgot to declare the `Game` category,
    /// and filing it under Applications (as the frontend does) is a worse
    /// outcome than filing a game as an emulator.
    ///
    /// Returns the section plus the probability that best supports it, so a
    /// caller can reject a low-confidence placement outright.
    pub fn from_traits(game: Option<f64>, emulator: Option<f64>, threshold: f64) -> (Section, f64) {
        let game = game.unwrap_or(0.0);
        let emulator = emulator.unwrap_or(0.0);
        if emulator >= threshold {
            (Section::ConsoleGames, emulator)
        } else if game >= threshold {
            (Section::PcGames, game)
        } else {
            (Section::Applications, (1.0 - game).clamp(0.0, 1.0))
        }
    }
}

/// One candidate category, plus the text the model reads when choosing it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryDef {
    /// Stable key written back to the record, e.g. `video_streaming`.
    pub slug: String,
    /// Short human label, e.g. `Video & Streaming`.
    pub name: String,
    /// One sentence telling the model what belongs here. Also the tab tooltip.
    pub summary: String,
    /// Section this category can appear in.
    pub section: Section,
}

impl CategoryDef {
    pub fn new(
        slug: impl Into<String>,
        name: impl Into<String>,
        summary: impl Into<String>,
        section: Section,
    ) -> Self {
        Self {
            slug: slug.into(),
            name: name.into(),
            summary: summary.into(),
            section,
        }
    }

    /// The exact string used as a `choice` criterion. Keeping the description in
    /// the label is what lets the criteria stay an ordered JSON array (whose
    /// order is preserved) instead of a map (whose keys would be re-sorted).
    pub fn label(&self) -> String {
        format!("{} - {}", self.name, self.summary)
    }
}

/// A validated, de-duplicated set of candidate categories.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Taxonomy {
    categories: Vec<CategoryDef>,
}

impl Taxonomy {
    /// Validate a candidate set.
    ///
    /// Laya rejects a `choice` whose labels repeat, so uniqueness is enforced
    /// here rather than at inference time, where the failure would only surface
    /// once per application.
    pub fn new(categories: Vec<CategoryDef>) -> Result<Self> {
        let mut names = std::collections::HashSet::new();
        let mut slugs = std::collections::HashSet::new();
        for category in &categories {
            if category.name.trim().is_empty() {
                return Err(CategorizerError::Invalid("category name is empty".into()));
            }
            if category.slug.trim().is_empty() {
                return Err(CategorizerError::Invalid("category slug is empty".into()));
            }
            if !names.insert(category.name.to_lowercase()) {
                return Err(CategorizerError::Invalid(format!(
                    "duplicate category name {:?}",
                    category.name
                )));
            }
            if !slugs.insert(category.slug.to_lowercase()) {
                return Err(CategorizerError::Invalid(format!(
                    "duplicate category slug {:?}",
                    category.slug
                )));
            }
        }
        Ok(Self { categories })
    }

    /// The categories this project ships with.
    ///
    /// Only [`Section::Applications`] is populated: games are already grouped by
    /// store and by console in the UI, so there is nothing for the model to
    /// invent there. The list is the "purpose of this project" made explicit —
    /// what a living-room library of installed Linux apps should look like.
    pub fn baseline() -> Self {
        Self::new(vec![
            CategoryDef::new(
                "video_streaming",
                "Video & Streaming",
                "Apps for watching streaming services such as Netflix, Disney+, Prime Video, Max, Crunchyroll or Hulu.",
                Section::Applications,
            ),
            CategoryDef::new(
                "media_center",
                "Media Center",
                "Local or self-hosted media libraries and players such as Plex, Jellyfin, Emby, Kodi, VLC or mpv.",
                Section::Applications,
            ),
            CategoryDef::new(
                "music_audio",
                "Music & Audio",
                "Music players, podcast clients, audio editors, synthesisers and digital audio workstations.",
                Section::Applications,
            ),
            CategoryDef::new(
                "photo_video",
                "Photo & Video",
                "Image viewers and editors, drawing and design tools, 3D and CAD, video editors and screen capture.",
                Section::Applications,
            ),
            CategoryDef::new(
                "development",
                "Development",
                "Code editors and IDEs, compilers, terminals, version control, container and database tools.",
                Section::Applications,
            ),
            CategoryDef::new(
                "productivity",
                "Productivity",
                "Office suites, notes, calendars, PDF and document tools, and task managers.",
                Section::Applications,
            ),
            CategoryDef::new(
                "internet",
                "Internet",
                "Web browsers, email, chat and video-call clients, torrents and remote desktop clients.",
                Section::Applications,
            ),
            CategoryDef::new(
                "system",
                "System",
                "Desktop settings, file managers, disk and system monitors, package managers and backup tools.",
                Section::Applications,
            ),
            CategoryDef::new(
                "education",
                "Education & Reference",
                "Learning apps, dictionaries, e-book readers, calculators, mathematics and science tools.",
                Section::Applications,
            ),
            CategoryDef::new(
                "desktop",
                "Desktop & Customization",
                "Themes, wallpapers, fonts, launchers, panels, widgets and other desktop appearance tools.",
                Section::Applications,
            ),
        ])
        .expect("baseline taxonomy is valid")
    }

    pub fn categories(&self) -> &[CategoryDef] {
        &self.categories
    }

    pub fn is_empty(&self) -> bool {
        self.categories.is_empty()
    }

    pub fn in_section(&self, section: Section) -> impl Iterator<Item = &CategoryDef> {
        self.categories
            .iter()
            .filter(move |category| category.section == section)
    }

    pub fn by_slug(&self, slug: &str) -> Option<&CategoryDef> {
        self.categories
            .iter()
            .find(|category| category.slug == slug)
    }

    /// Resolve the exact criterion string a `choice` answer comes back as.
    pub fn by_label(&self, label: &str) -> Option<&CategoryDef> {
        self.categories
            .iter()
            .find(|category| category.label() == label)
    }
}

#[cfg(test)]
mod tests {
    use super::{CategoryDef, Section, Taxonomy};

    #[test]
    fn baseline_is_valid_and_only_covers_applications() {
        let taxonomy = Taxonomy::baseline();
        assert!(!taxonomy.is_empty());
        assert!(
            taxonomy.in_section(Section::PcGames).next().is_none(),
            "games are grouped by store and platform, not by category"
        );
        assert_eq!(
            taxonomy.in_section(Section::Applications).count(),
            taxonomy.categories().len()
        );
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let duplicate = Taxonomy::new(vec![
            CategoryDef::new("a", "Same", "first", Section::Applications),
            CategoryDef::new("b", "same", "second", Section::Applications),
        ]);
        assert!(duplicate.is_err());
    }

    #[test]
    fn traits_split_games_from_emulators_from_apps() {
        assert_eq!(
            Section::from_traits(Some(0.9), Some(0.1), 0.5).0,
            Section::PcGames
        );
        assert_eq!(
            Section::from_traits(Some(0.9), Some(0.8), 0.5).0,
            Section::ConsoleGames
        );
        // An emulator is a console-game app even when the record forgot to say
        // it is a game.
        assert_eq!(
            Section::from_traits(Some(0.0), Some(0.9), 0.5).0,
            Section::ConsoleGames
        );
        assert_eq!(
            Section::from_traits(Some(0.05), Some(0.0), 0.5).0,
            Section::Applications
        );
    }

    #[test]
    fn labels_round_trip_through_the_taxonomy() {
        let taxonomy = Taxonomy::baseline();
        let category = &taxonomy.categories()[0];
        assert_eq!(taxonomy.by_label(&category.label()).unwrap(), category);
    }
}
