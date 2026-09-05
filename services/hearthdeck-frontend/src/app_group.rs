use crate::config::APP_ID;
use crate::fl;
use cosmic::cosmic_config::cosmic_config_derive::CosmicConfigEntry;
use cosmic::cosmic_config::{
    CosmicConfigEntry, {self},
};
use cosmic::desktop::DesktopEntryData;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

static HOME: LazyLock<AppGroup> = LazyLock::new(|| AppGroup {
    name: "cosmic-library-home".to_string(),
    icon: "user-home-symbolic".to_string(),
    filter: FilterType::None,
});

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FilterType {
    /// A list of application IDs to include in the group.
    AppIds(Vec<String>),
    Categories {
        categories: Vec<String>,
        /// The ID of applications which should be excluded from the results.
        exclude: Vec<String>,
        /// The ID of applications which may not match the categories, but should be included anyway.
        include: Vec<String>,
    },
    /// No filter is applied.
    /// This is intended for use with Home.
    None,
}

impl Default for FilterType {
    fn default() -> Self {
        FilterType::AppIds(Vec::new())
    }
}

impl Ord for FilterType {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (FilterType::AppIds(_), FilterType::AppIds(_)) => std::cmp::Ordering::Equal,
            (FilterType::None, FilterType::None) => std::cmp::Ordering::Equal,
            (FilterType::Categories { .. }, FilterType::Categories { .. }) => {
                std::cmp::Ordering::Equal
            }
            (FilterType::Categories { .. } | FilterType::None, FilterType::AppIds(_)) => {
                std::cmp::Ordering::Less
            }
            (FilterType::AppIds(_), FilterType::Categories { .. } | FilterType::None) => {
                std::cmp::Ordering::Greater
            }
            (FilterType::Categories { .. }, FilterType::None) => std::cmp::Ordering::Greater,
            (FilterType::None, FilterType::Categories { .. }) => std::cmp::Ordering::Less,
        }
    }
}

impl PartialOrd for FilterType {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// Object holding the state
#[derive(Default, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppGroup {
    pub name: String,
    pub icon: String,
    pub filter: FilterType,
    // pub popup: bool,
}

impl PartialOrd for AppGroup {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AppGroup {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (&self.filter, &other.filter) {
            (FilterType::AppIds(_), FilterType::AppIds(_)) => {
                self.name.to_lowercase().cmp(&other.name.to_lowercase())
            }
            (FilterType::Categories { categories, .. }, FilterType::AppIds(_)) => {
                if let Some(cat_name) = categories.first() {
                    cat_name.to_lowercase().cmp(&other.name.to_lowercase())
                } else {
                    self.name.to_lowercase().cmp(&other.name.to_lowercase())
                }
            }
            (FilterType::AppIds(_), FilterType::Categories { categories, .. }) => {
                if let Some(other_name) = categories.first() {
                    self.name.to_lowercase().cmp(&other_name.to_lowercase())
                } else {
                    self.name.to_lowercase().cmp(&other.name.to_lowercase())
                }
            }
            (a, b) => a.cmp(b),
        }
    }
}

impl AppGroup {
    fn matches(&self, entry: &DesktopEntryData) -> bool {
        match &self.filter {
            FilterType::AppIds(names) => names.iter().any(|id| id == &entry.id),
            FilterType::Categories {
                categories,
                include,
                exclude,
                ..
            } => {
                categories.iter().any(|cat| {
                    entry
                        .categories
                        .iter()
                        .any(|acat| acat.to_lowercase() == cat.to_lowercase())
                }) && exclude.iter().all(|id| id != &entry.id)
                    || include.iter().any(|id| id == &entry.id)
            }
            FilterType::None => true,
        }
    }

    pub fn name(&self) -> String {
        if &self.name == "cosmic-library-home" {
            fl!("cosmic-library-home")
        } else if &self.name == "cosmic-office" {
            fl!("cosmic-office")
        } else if &self.name == "cosmic-system" {
            fl!("cosmic-system")
        } else if &self.name == "cosmic-utilities" {
            fl!("cosmic-utilities")
        } else {
            self.name.clone()
        }
    }
}

/// The fixed top-level navigation tabs shown in the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Section {
    PcGames,
    ConsoleGames,
    Applications,
}

impl Section {
    pub const ALL: [Section; 3] = [
        Section::PcGames,
        Section::ConsoleGames,
        Section::Applications,
    ];

    pub fn name(&self) -> String {
        match self {
            Section::PcGames => fl!("pc-games"),
            Section::ConsoleGames => fl!("console-games"),
            Section::Applications => fl!("applications"),
        }
    }

    pub fn index(&self) -> usize {
        match self {
            Section::PcGames => 0,
            Section::ConsoleGames => 1,
            Section::Applications => 2,
        }
    }

    /// Label of the "all" tab for this section.
    pub fn all_name(&self) -> String {
        match self {
            Section::PcGames | Section::ConsoleGames => fl!("all-games"),
            Section::Applications => fl!("all-apps"),
        }
    }

    /// Whether an entry belongs to this section. Games are split between the
    /// PC Games and Console Games sections (console games being emulator
    /// titles), everything else lands in Applications.
    pub fn matches(&self, entry: &DesktopEntryData) -> bool {
        let is_game = entry
            .categories
            .iter()
            .any(|cat| cat.eq_ignore_ascii_case("game"));
        let is_emulator = is_emulator_entry(entry);
        match self {
            Section::PcGames => is_game && !is_emulator,
            Section::ConsoleGames => is_game && is_emulator,
            Section::Applications => !is_game,
        }
    }
}

/// Sub-tabs (filter chips) shown in the top bar, one set per section.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Sections {
    #[serde(default)]
    pub pc_games: Vec<AppGroup>,
    #[serde(default)]
    pub console_games: Vec<AppGroup>,
    #[serde(default)]
    pub applications: Vec<AppGroup>,
}

impl Sections {
    pub fn get(&self, section: Section) -> &Vec<AppGroup> {
        match section {
            Section::PcGames => &self.pc_games,
            Section::ConsoleGames => &self.console_games,
            Section::Applications => &self.applications,
        }
    }

    pub fn get_mut(&mut self, section: Section) -> &mut Vec<AppGroup> {
        match section {
            Section::PcGames => &mut self.pc_games,
            Section::ConsoleGames => &mut self.console_games,
            Section::Applications => &mut self.applications,
        }
    }
}

/// Returns true when the entry looks like a console emulator, identified by
/// its ID or Exec string.
fn is_emulator_entry(entry: &DesktopEntryData) -> bool {
    const EMULATORS: &[&str] = &[
        "retroarch",
        "dolphin",
        "pcsx2",
        "duckstation",
        "rpcs3",
        "citra",
        "yuzu",
        "ryujinx",
        "mame",
        "mupen",
        "xemu",
        "melon",
        "ppsspp",
        "snes9x",
        "mgba",
        "mednafen",
        "desmume",
        "fsuae",
        "scummvm",
        "cemu",
        "ares",
        "bsnes",
        "limbo",
        "flycast",
        "redream",
        "standalone",
        "emulator",
    ];
    let haystack =
        format!("{} {}", entry.id, entry.exec.as_deref().unwrap_or_default()).to_lowercase();
    EMULATORS.iter().any(|emulator| haystack.contains(emulator))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, CosmicConfigEntry)]
pub struct AppLibraryConfig {
    #[serde(default)]
    pub sections: Sections,
}

impl AppLibraryConfig {
    pub fn version() -> u64 {
        2
    }

    pub fn helper() -> Option<cosmic_config::Config> {
        cosmic_config::Config::new(APP_ID, Self::version()).ok()
    }

    pub fn home() -> &'static AppGroup {
        &HOME
    }

    pub fn add(&mut self, section: Section, name: String) {
        self.sections.get_mut(section).push(AppGroup {
            name,
            icon: "folder-symbolic".to_string(),
            filter: FilterType::AppIds(Vec::new()),
        });
    }

    pub fn remove(&mut self, section: Section, i: usize) {
        if i < self.sections.get(section).len() {
            self.sections.get_mut(section).remove(i);
        }
    }

    pub fn set_name(&mut self, section: Section, i: usize, name: String) {
        if let Some(group) = self.sections.get_mut(section).get_mut(i) {
            group.name = name;
        }
    }

    pub fn remove_entry(&mut self, section: Section, group: Option<usize>, id: &str) {
        let Some(group) = group.and_then(|i| self.sections.get_mut(section).get_mut(i)) else {
            return;
        };
        match &mut group.filter {
            FilterType::AppIds(ids) => ids.retain(|conf_id| conf_id != id),
            FilterType::Categories {
                exclude, include, ..
            } => {
                include.retain(|conf_id| conf_id != id);
                exclude.retain(|conf_id| conf_id != id);
                exclude.push(id.to_string());
            }
            FilterType::None => {}
        }
    }

    pub fn add_entry(&mut self, section: Section, group: Option<usize>, id: &str) {
        if let Some(group) = group.and_then(|i| self.sections.get_mut(section).get_mut(i)) {
            match &mut group.filter {
                FilterType::AppIds(ids) => {
                    if ids.iter().all(|s| s != id) {
                        ids.push(id.to_string());
                    }
                }
                FilterType::Categories {
                    exclude, include, ..
                } => {
                    include.retain(|conf_id| conf_id != id);
                    exclude.retain(|conf_id| conf_id != id);
                    include.push(id.to_string());
                }
                FilterType::None => {}
            }
        }
    }

    pub fn filtered(
        &self,
        section: Section,
        tab: Option<usize>,
        input_value: &str,
        entries: &[Arc<DesktopEntryData>],
    ) -> Vec<Arc<DesktopEntryData>> {
        let tab_group = tab.and_then(|i| self.sections.get(section).get(i));
        entries
            .iter()
            .filter(|de| {
                if !section.matches(de) {
                    return false;
                }
                if let Some(group) = tab_group
                    && !group.matches(de)
                {
                    return false;
                }
                if !input_value.is_empty()
                    && !de.name.to_lowercase().contains(&input_value.to_lowercase())
                    && !de
                        .categories
                        .iter()
                        .any(|acat| acat.to_lowercase() == input_value.to_lowercase())
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect()
    }

    /// Rebuild category tabs from the entries currently present while keeping
    /// user-created groups, which use explicit application IDs.
    pub fn sync_category_groups(&mut self, entries: &[Arc<DesktopEntryData>]) -> bool {
        const APPLICATION_CATEGORIES: &[&str] = &[
            "Audio",
            "AudioVideo",
            "Development",
            "Education",
            "Graphics",
            "Network",
            "Office",
            "Science",
            "Settings",
            "System",
            "Utility",
            "Video",
        ];
        const GAME_CATEGORIES: &[&str] = &[
            "Action",
            "ActionGame",
            "Adventure",
            "AdventureGame",
            "Arcade",
            "ArcadeGame",
            "BoardGame",
            "CardGame",
            "Casual",
            "Fighting",
            "Horror",
            "Indie",
            "KidsGame",
            "LogicGame",
            "MMO",
            "MMORPG",
            "Platformer",
            "Puzzle",
            "Racing",
            "Roguelike",
            "RolePlaying",
            "RPG",
            "Shooter",
            "Simulation",
            "Sports",
            "SportsGame",
            "Strategy",
            "StrategyGame",
            "Survival",
        ];
        const STORE_CATEGORY_PREFIX: &str = "hearthdeck-store:";

        let mut changed = false;
        for section in Section::ALL {
            let mut categories = BTreeMap::new();
            for entry in entries.iter().filter(|entry| section.matches(entry)) {
                for category in &entry.categories {
                    let visible = match section {
                        Section::Applications => APPLICATION_CATEGORIES
                            .iter()
                            .any(|known| category.eq_ignore_ascii_case(known)),
                        Section::PcGames | Section::ConsoleGames => {
                            category.starts_with(STORE_CATEGORY_PREFIX)
                                || GAME_CATEGORIES
                                    .iter()
                                    .any(|known| category.eq_ignore_ascii_case(known))
                        }
                    };
                    if category.eq_ignore_ascii_case("game") || !visible {
                        continue;
                    }
                    categories
                        .entry(category.to_lowercase())
                        .or_insert_with(|| category.clone());
                }
            }

            let existing = self.sections.get(section);
            let custom_groups = existing
                .iter()
                .filter(|group| matches!(group.filter, FilterType::AppIds(_)))
                .cloned();
            let mut groups: Vec<_> = categories
                .into_values()
                .map(|category| AppGroup {
                    name: category
                        .strip_prefix(STORE_CATEGORY_PREFIX)
                        .map(str::to_owned)
                        .unwrap_or_else(|| match category.to_ascii_lowercase().as_str() {
                            "office" => "cosmic-office".to_string(),
                            "system" => "cosmic-system".to_string(),
                            "utility" => "cosmic-utilities".to_string(),
                            _ => category.clone(),
                        }),
                    icon: "folder-symbolic".to_string(),
                    filter: FilterType::Categories {
                        categories: vec![category],
                        include: Vec::new(),
                        exclude: Vec::new(),
                    },
                })
                .collect();
            groups.extend(custom_groups);

            if existing != &groups {
                *self.sections.get_mut(section) = groups;
                changed = true;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::{AppGroup, AppLibraryConfig, FilterType, Section};
    use cosmic::desktop::{DesktopEntryData, fde::IconSource};
    use std::sync::Arc;

    fn entry(id: &str, categories: &[&str]) -> Arc<DesktopEntryData> {
        Arc::new(DesktopEntryData {
            id: id.into(),
            name: id.into(),
            wm_class: None,
            exec: None,
            icon: IconSource::Name(String::new()),
            path: None,
            categories: categories
                .iter()
                .map(|category| (*category).into())
                .collect(),
            desktop_actions: Vec::new(),
            mime_types: Vec::new(),
            prefers_dgpu: false,
            terminal: false,
        })
    }

    #[test]
    fn category_tabs_follow_loaded_entries() {
        let mut config = AppLibraryConfig::default();
        config.sync_category_groups(&[
            entry("writer", &["Office"]),
            entry("terminal", &["Utility"]),
        ]);

        assert_eq!(
            config
                .sections
                .get(Section::Applications)
                .iter()
                .map(|group| group.name())
                .collect::<Vec<_>>(),
            vec!["Office", "Utilities"]
        );

        config.sync_category_groups(&[entry("terminal", &["Utility"])]);
        assert_eq!(
            config
                .sections
                .get(Section::Applications)
                .iter()
                .map(|group| group.name())
                .collect::<Vec<_>>(),
            vec!["Utilities"]
        );
    }

    #[test]
    fn game_tabs_only_show_stores_and_game_genres() {
        let mut config = AppLibraryConfig::default();
        config.sync_category_groups(&[entry(
            "game",
            &[
                "Game",
                "PackageManager",
                "FileTransfer",
                "RPG",
                "hearthdeck-store:Epic Games",
            ],
        )]);

        assert_eq!(
            config
                .sections
                .get(Section::PcGames)
                .iter()
                .map(|group| group.name())
                .collect::<Vec<_>>(),
            vec!["Epic Games", "RPG"]
        );
    }

    #[test]
    fn category_sync_preserves_custom_groups() {
        let mut config = AppLibraryConfig::default();
        config.sections.applications.push(AppGroup {
            name: "Favorites".into(),
            icon: "folder-symbolic".into(),
            filter: FilterType::AppIds(vec!["writer".into()]),
        });

        config.sync_category_groups(&[entry("writer", &["Office"])]);

        assert!(config.sections.applications.iter().any(|group| {
            group.name == "Favorites"
                && matches!(&group.filter, FilterType::AppIds(ids) if ids == &["writer"])
        }));
    }
}
