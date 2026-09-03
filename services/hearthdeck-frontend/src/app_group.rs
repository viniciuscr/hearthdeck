use crate::config::APP_ID;
use crate::fl;
use crate::providers::GameRecord;
use cosmic::cosmic_config::cosmic_config_derive::CosmicConfigEntry;
use cosmic::cosmic_config::{
    CosmicConfigEntry, {self},
};
use cosmic::desktop::DesktopEntryData;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, LazyLock};
use std::vec;

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

#[derive(Debug, Clone, Serialize, Deserialize, CosmicConfigEntry)]
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

    /// Ensure category-based groups exist for each store found in provider records.
    /// Groups are created dynamically from the `metadata.store` value that each
    /// provider sets (e.g. "Epic Games", "GOG"). No store names are hardcoded.
    pub fn ensure_provider_groups(&mut self, records: &[GameRecord]) {
        for record in records {
            let Some(store_name) = record.metadata.get("store").and_then(|v| v.as_str()) else {
                continue;
            };
            if store_name.is_empty() {
                continue;
            }
            // Determine which section this record belongs to based on its
            // categories (same logic as Section::matches but without
            // DesktopEntryData).
            let is_game = record
                .categories
                .iter()
                .any(|cat| cat.eq_ignore_ascii_case("game"));
            if !is_game {
                continue;
            }
            let section = Section::PcGames;

            let already_has_group = self.sections.get(section).iter().any(|g| {
                matches!(
                    &g.filter,
                    FilterType::Categories { categories, .. }
                    if categories.iter().any(|c| c == store_name)
                )
            });
            if already_has_group {
                continue;
            }
            self.sections.get_mut(section).insert(
                0,
                AppGroup {
                    name: store_name.to_string(),
                    icon: "folder-symbolic".to_string(),
                    filter: FilterType::Categories {
                        categories: vec![store_name.to_string()],
                        include: Vec::new(),
                        exclude: Vec::new(),
                    },
                },
            );
        }
    }
}

impl Default for AppLibraryConfig {
    fn default() -> Self {
        AppLibraryConfig {
            sections: Sections {
                pc_games: vec![AppGroup {
                    name: "Epic Games".to_string(),
                    icon: "folder-symbolic".to_string(),
                    filter: FilterType::Categories {
                        categories: vec!["Epic Games".to_string()],
                        include: Vec::new(),
                        exclude: Vec::new(),
                    },
                }],
                console_games: Vec::new(),
                applications: vec![
                    AppGroup {
                        name: "cosmic-office".to_string(),
                        icon: "folder-symbolic".to_string(),
                        filter: FilterType::Categories {
                            categories: vec!["Office".to_string()],
                            include: vec![
                                "org.gnome.Totem".to_string(),
                                "org.gnome.eog".to_string(),
                                "simple-scan".to_string(),
                                "thunderbird".to_string(),
                            ],
                            exclude: Vec::new(),
                        },
                    },
                    AppGroup {
                        name: "cosmic-system".to_string(),
                        icon: "folder-symbolic".to_string(),
                        filter: FilterType::Categories {
                            categories: vec!["System".to_string()],
                            include: vec![
                                "gnome-language-selector".to_string(),
                                "im-config".to_string(),
                                "org.freedesktop.IBus.Setup".to_string(),
                                "system76-driver".to_string(),
                            ],
                            exclude: vec![
                                "com.system76.CosmicStore".to_string(),
                                "com.system76.CosmicTerm".to_string(),
                            ],
                        },
                    },
                    AppGroup {
                        name: "cosmic-utilities".to_string(),
                        icon: "folder-symbolic".to_string(),
                        filter: FilterType::Categories {
                            categories: vec!["Utility".to_string()],
                            include: vec!["nm-connection-editor".to_string()],
                            exclude: vec![
                                "com.system76.CosmicEdit".to_string(),
                                "com.system76.CosmicFiles".to_string(),
                            ],
                        },
                    },
                ],
            },
        }
    }
}
