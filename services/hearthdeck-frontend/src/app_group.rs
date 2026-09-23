use crate::config::APP_ID;
use crate::fl;
use cosmic::cosmic_config::cosmic_config_derive::CosmicConfigEntry;
use cosmic::cosmic_config::{
    CosmicConfigEntry, {self},
};
use cosmic::desktop::DesktopEntryData;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock};

static HOME: LazyLock<AppGroup> = LazyLock::new(|| AppGroup {
    name: "cosmic-library-home".to_string(),
    icon: "user-home-symbolic".to_string(),
    filter: FilterType::None,
    source: GroupSource::Local,
});

const CONSOLE_CATEGORY_PREFIX: &str = "hearthdeck-console:";

pub fn romm_console_category(platform_id: i64) -> String {
    format!("{CONSOLE_CATEGORY_PREFIX}{platform_id}")
}

/// RomM metadata that the console grid can filter on rides on each record as a
/// namespaced category, the same way stores ([`AppLibraryConfig::sync_category_groups`])
/// and consoles (above) do. `DesktopEntryData` has no metadata field, so
/// categories are the only channel a record's RomM metadata survives on; a
/// facet is then a prefix plus a value, and matching is a category lookup.
const GENRE_FACET_PREFIX: &str = "hearthdeck-genre:";
const DECADE_FACET_PREFIX: &str = "hearthdeck-decade:";
const REGION_FACET_PREFIX: &str = "hearthdeck-region:";

pub fn romm_genre_category(genre: &str) -> String {
    format!("{GENRE_FACET_PREFIX}{genre}")
}

pub fn romm_region_category(region: &str) -> String {
    format!("{REGION_FACET_PREFIX}{region}")
}

/// Tags a game with the decade its release year falls in: 1994 -> `1990`.
/// Decades rather than years because a console library spans few enough decades
/// for the filter row to stay a single short strip.
pub fn romm_decade_category(release_year: i32) -> String {
    let decade = release_year - release_year.rem_euclid(10);
    format!("{DECADE_FACET_PREFIX}{decade}")
}

/// A RomM metadata field the Console Games grid can be filtered by. Each
/// variant owns the record-category prefix carrying its values, so adding a
/// filter option is one variant plus one tag on the record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RommFacet {
    Genre,
    Decade,
    Region,
}

impl RommFacet {
    /// Every facet, in the order the filter bar shows them.
    pub const ALL: [RommFacet; 3] = [Self::Genre, Self::Decade, Self::Region];

    fn prefix(self) -> &'static str {
        match self {
            Self::Genre => GENRE_FACET_PREFIX,
            Self::Decade => DECADE_FACET_PREFIX,
            Self::Region => REGION_FACET_PREFIX,
        }
    }

    /// The facet's name, shown on its chip in the filter bar.
    pub fn name(self) -> String {
        match self {
            Self::Genre => fl!("filter-genre"),
            Self::Decade => fl!("filter-decade"),
            Self::Region => fl!("filter-region"),
        }
    }

    /// How one of this facet's values reads on a chip: a decade is a range, so
    /// it is shown as one ("1990s") rather than as its first year.
    pub fn value_label(self, value: &str) -> String {
        match self {
            Self::Decade => format!("{value}s"),
            Self::Genre | Self::Region => value.to_owned(),
        }
    }

    /// Every value of this facet present in `entries`, deduplicated and sorted.
    /// Decades sort as text just like years would, because they are all four
    /// digits long.
    pub fn values(self, entries: &[Arc<DesktopEntryData>]) -> Vec<String> {
        entries
            .iter()
            .flat_map(|entry| entry.categories.iter())
            .filter_map(|category| category.strip_prefix(self.prefix()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// Whether `entry` carries `value` for this facet.
    fn matches(self, value: &str, entry: &DesktopEntryData) -> bool {
        let tag = format!("{}{value}", self.prefix());
        entry.categories.iter().any(|category| category == &tag)
    }
}

/// The Console Games filter selection: at most one value per facet, with an
/// absent facet meaning "All". Transient view state rather than configuration,
/// so it is never written to disk; it is also deliberately *not* multi-select,
/// which keeps the panel a single flat strip of options per facet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RommFilters {
    selected: BTreeMap<RommFacet, String>,
}

impl RommFilters {
    pub fn selected(&self, facet: RommFacet) -> Option<&str> {
        self.selected.get(&facet).map(String::as_str)
    }

    /// Points `facet` at `value`, or clears it when `value` is `None`.
    pub fn set(&mut self, facet: RommFacet, value: Option<String>) {
        match value {
            Some(value) => {
                self.selected.insert(facet, value);
            }
            None => {
                self.selected.remove(&facet);
            }
        }
    }

    pub fn clear(&mut self) {
        self.selected.clear();
    }

    pub fn is_active(&self) -> bool {
        !self.selected.is_empty()
    }

    /// How many facets are narrowed, for the filter bar's badge.
    pub fn len(&self) -> usize {
        self.selected.len()
    }

    /// Whether `entry` satisfies every selected facet. An empty selection
    /// matches everything, so an unfiltered grid is the same code path.
    pub fn matches(&self, entry: &DesktopEntryData) -> bool {
        self.selected
            .iter()
            .all(|(facet, value)| facet.matches(value, entry))
    }
}

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

/// Who owns a tab, which decides who is allowed to replace it.
///
/// The distinction exists so a library scan can swap out exactly what a previous
/// scan put there, instead of guessing from tab names or wiping the user's own
/// folders along with it.
#[derive(Default, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GroupSource {
    /// Made here: by the user, or derived from the entries by
    /// [`AppLibraryConfig::sync_category_groups`] and
    /// [`AppLibraryConfig::sync_console_groups`].
    #[default]
    Local,
    /// Proposed by a library scan, and replaced by the next report.
    Categorization,
}

/// One tab a scan proposed, already resolved to the entry ids it holds.
///
/// The scan speaks in catalog item ids and [`AppGroup`] filters in library entry
/// ids, so the translation happens where the report is read rather than here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategorizationGroup {
    pub name: String,
    pub entry_ids: Vec<String>,
}

// Object holding the state
#[derive(Default, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppGroup {
    pub name: String,
    pub icon: String,
    pub filter: FilterType,
    /// Defaulted, so a config written before scans existed loads every tab as
    /// [`GroupSource::Local`] and nothing is mistaken for a scan's work.
    #[serde(default)]
    pub source: GroupSource,
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

    pub fn romm_platform_id(&self) -> Option<i64> {
        let FilterType::Categories { categories, .. } = &self.filter else {
            return None;
        };
        categories.iter().find_map(|category| {
            category
                .strip_prefix(CONSOLE_CATEGORY_PREFIX)
                .and_then(|id| id.parse().ok())
        })
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

    /// Icon shown beside the section's name in the sidebar. Named rather than
    /// inlined at the view so the three sections cannot drift apart in style.
    pub fn icon_name(self) -> &'static str {
        match self {
            Self::PcGames => "applications-games-symbolic",
            Self::ConsoleGames => "input-gaming-symbolic",
            Self::Applications => "view-app-grid-symbolic",
        }
    }

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

/// Returns true when the entry is a streaming service client — the kind the
/// dashboard's Watch shelf is for — rather than a game, a local media player,
/// or a general utility.
///
/// Matching is by name only, on purpose. Freedesktop categories are too coarse
/// to identify a service: `AudioVideo` is also declared by volume mixers and
/// video-capture tools, and `Player` by local players, so a category match
/// pulled in apps that have nothing to watch. The services themselves rarely
/// declare anything useful either — they ship as a browser `--app=<url>` window
/// (a hand-written desktop entry) or a bundled Electron client — so the id and
/// exec line are what is left to match on.
pub(crate) fn is_watch_entry(entry: &DesktopEntryData) -> bool {
    if entry
        .categories
        .iter()
        .any(|category| category.eq_ignore_ascii_case("game"))
    {
        return false;
    }

    // Deliberately specific strings: these are matched against the whole exec
    // line, which can contain a path with the user's name in it, so a bare
    // service name like "max" or "prime" would match far too much.
    const SERVICES: &[&str] = &[
        "stremio",
        "netflix",
        "primevideo",
        "prime video",
        "prime-video",
        "disneyplus",
        "disney+",
        "hulu",
        "hbomax",
        "hbo max",
        "max.com",
        "crunchyroll",
        "paramountplus",
        "paramount+",
        "peacock",
        "tubitv",
        "tubi.tv",
        "plutotv",
        "pluto.tv",
        "appletv",
        "apple tv",
        "tv.apple",
        "plex",
        "jellyfin",
        "emby",
        "kodi",
    ];
    let haystack =
        format!("{} {}", entry.id, entry.exec.as_deref().unwrap_or_default()).to_lowercase();
    SERVICES.iter().any(|service| haystack.contains(service))
}

/// Returns true when the entry looks like a console emulator, identified by
/// its ID or Exec string.
fn is_emulator_entry(entry: &DesktopEntryData) -> bool {
    if entry
        .categories
        .iter()
        .any(|category| category.starts_with(CONSOLE_CATEGORY_PREFIX))
    {
        return true;
    }
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
    #[serde(default)]
    pub desktop_input_apps: Vec<String>,
    #[serde(default)]
    pub favorite_ids: Vec<String>,
}

impl AppLibraryConfig {
    pub fn version() -> u64 {
        2
    }

    pub fn helper() -> Option<cosmic_config::Config> {
        cosmic_config::Config::new(APP_ID, Self::version()).ok()
    }

    pub fn desktop_input_enabled(&self, id: &str) -> bool {
        self.desktop_input_apps
            .iter()
            .any(|configured| configured == id)
    }

    pub fn toggle_desktop_input(&mut self, id: &str) {
        if self.desktop_input_enabled(id) {
            self.desktop_input_apps
                .retain(|configured| configured != id);
        } else {
            self.desktop_input_apps.push(id.to_owned());
        }
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.favorite_ids.iter().any(|favorite| favorite == id)
    }

    pub fn toggle_favorite(&mut self, id: &str) {
        if self.is_favorite(id) {
            self.favorite_ids.retain(|favorite| favorite != id);
        } else {
            self.favorite_ids.push(id.to_owned());
        }
    }

    pub fn favorite_entries<'a>(
        &self,
        entries: &'a [Arc<DesktopEntryData>],
    ) -> Vec<&'a Arc<DesktopEntryData>> {
        self.favorite_ids
            .iter()
            .filter_map(|id| entries.iter().find(|entry| entry.id == *id))
            .collect()
    }

    pub fn home() -> &'static AppGroup {
        &HOME
    }

    pub fn add(&mut self, section: Section, name: String) {
        self.sections.get_mut(section).push(AppGroup {
            name,
            icon: "folder-symbolic".to_string(),
            filter: FilterType::AppIds(Vec::new()),
            source: GroupSource::Local,
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

    /// The entries one tab of one section shows: the section's records, the
    /// tab's group, the search box and the console filters, all applied.
    pub fn filtered(
        &self,
        section: Section,
        tab: Option<usize>,
        input_value: &str,
        entries: &[Arc<DesktopEntryData>],
        filters: &RommFilters,
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
                // The filters are the console grid's own: only that section
                // renders their controls, and only its records carry the tags,
                // so leaving them set must not empty another section.
                if section == Section::ConsoleGames && !filters.matches(de) {
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
    ///
    /// A scan owns the Applications section once it has a report: when the tabs
    /// it made are present, the derived ones are not rebuilt at all. They are
    /// exactly what the scan replaced, so rebuilding them would quietly put the
    /// freedesktop tabs back beside the scan's.
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
        const STORE_CATEGORY_PREFIX: &str = "hearthdeck-store:";

        let mut changed = false;
        for section in [Section::PcGames, Section::Applications] {
            let mut categories = BTreeMap::new();
            for entry in entries.iter().filter(|entry| section.matches(entry)) {
                for category in &entry.categories {
                    let visible = match section {
                        Section::Applications => APPLICATION_CATEGORIES
                            .iter()
                            .any(|known| category.eq_ignore_ascii_case(known)),
                        Section::PcGames => category.starts_with(STORE_CATEGORY_PREFIX),
                        Section::ConsoleGames => false,
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
            // The scan's own tabs are kept here too: they use explicit app ids,
            // which is also what makes a user's folder survive a rebuild.
            let kept_groups = existing
                .iter()
                .filter(|group| matches!(group.filter, FilterType::AppIds(_)))
                .cloned();
            let scanned = section == Section::Applications
                && existing
                    .iter()
                    .any(|group| group.source == GroupSource::Categorization);
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
                    source: GroupSource::Local,
                })
                .collect();
            if scanned {
                groups.clear();
            }
            groups.extend(kept_groups);

            if existing != &groups {
                *self.sections.get_mut(section) = groups;
                changed = true;
            }
        }
        changed
    }

    pub fn sync_console_groups(&mut self, platforms: &[(i64, String)]) -> bool {
        let existing = &self.sections.console_games;
        let custom_groups = existing
            .iter()
            .filter(|group| matches!(group.filter, FilterType::AppIds(_)))
            .cloned();
        let mut groups = platforms
            .iter()
            .map(|(id, name)| AppGroup {
                name: name.clone(),
                icon: "folder-symbolic".to_string(),
                filter: FilterType::Categories {
                    categories: vec![romm_console_category(*id)],
                    include: Vec::new(),
                    exclude: Vec::new(),
                },
                source: GroupSource::Local,
            })
            .collect::<Vec<_>>();
        groups.extend(custom_groups);

        if existing == &groups {
            return false;
        }
        self.sections.console_games = groups;
        true
    }

    /// Replaces the Applications tabs with the ones a scan proposed, and nothing
    /// else.
    ///
    /// This is a swap, not a merge: the tabs a previous scan made are dropped,
    /// the user's own folders are kept, and an empty list removes the scan's tabs
    /// entirely — which hands the section back to the tabs derived from the
    /// entries. Tabs the user made are told apart from a scan's by
    /// [`is_user_group`], never by name.
    pub fn sync_categorization(&mut self, groups: &[CategorizationGroup]) -> bool {
        let existing = &self.sections.applications;
        let mut replacements: Vec<AppGroup> = groups
            .iter()
            .map(|group| AppGroup {
                name: group.name.clone(),
                icon: "folder-symbolic".to_string(),
                filter: FilterType::AppIds(group.entry_ids.clone()),
                source: GroupSource::Categorization,
            })
            .collect();
        replacements.extend(
            existing
                .iter()
                .filter(|group| is_user_group(group))
                .cloned(),
        );

        if existing == &replacements {
            return false;
        }
        self.sections.applications = replacements;
        true
    }
}

/// True for a tab the user made by hand, which nothing may replace.
///
/// A `Categories` filter can never be a user's: [`AppLibraryConfig::add`] and
/// `add_entry` only ever produce `AppIds`, so a stored `Categories` group was
/// derived by this module and is free to drop. Testing the filter as well as the
/// source is what keeps a config written before [`GroupSource`] existed — where
/// every tab loads as [`GroupSource::Local`] — from having its derived tabs
/// mistaken for the user's own.
fn is_user_group(group: &AppGroup) -> bool {
    group.source == GroupSource::Local && matches!(group.filter, FilterType::AppIds(_))
}

#[cfg(test)]
mod tests {
    use super::{
        AppGroup, AppLibraryConfig, CategorizationGroup, FilterType, GroupSource, RommFacet,
        RommFilters, Section, romm_console_category, romm_decade_category, romm_genre_category,
        romm_region_category,
    };
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
    fn pc_game_tabs_only_show_stores_with_installed_games() {
        let mut config = AppLibraryConfig::default();
        config.sync_category_groups(&[
            entry(
                "epic-game",
                &[
                    "Game",
                    "PackageManager",
                    "RPG",
                    "hearthdeck-store:Epic Games",
                ],
            ),
            entry(
                "gog-game",
                &["Game", "FileTransfer", "Shooter", "hearthdeck-store:GOG"],
            ),
        ]);

        assert_eq!(
            config
                .sections
                .get(Section::PcGames)
                .iter()
                .map(|group| group.name())
                .collect::<Vec<_>>(),
            vec!["Epic Games", "GOG"]
        );

        config.sync_category_groups(&[entry(
            "gog-game",
            &["Game", "FileTransfer", "Shooter", "hearthdeck-store:GOG"],
        )]);

        assert_eq!(
            config
                .sections
                .get(Section::PcGames)
                .iter()
                .map(|group| group.name())
                .collect::<Vec<_>>(),
            vec!["GOG"]
        );
    }

    #[test]
    fn console_tabs_follow_live_romm_platforms_not_game_genres() {
        let mut config = AppLibraryConfig::default();
        config.sync_category_groups(&[entry("romm:42", &["Game", "RPG", "hearthdeck-console:7"])]);
        assert!(config.sections.console_games.is_empty());

        config.sync_console_groups(&[(7, "SNES".into()), (9, "PlayStation".into())]);
        assert_eq!(
            config
                .sections
                .get(Section::ConsoleGames)
                .iter()
                .map(|group| group.name())
                .collect::<Vec<_>>(),
            vec!["SNES", "PlayStation"]
        );
        assert_eq!(config.sections.console_games[0].romm_platform_id(), Some(7));

        config.sync_console_groups(&[(9, "PlayStation".into())]);
        assert_eq!(
            config
                .sections
                .get(Section::ConsoleGames)
                .iter()
                .map(|group| group.name())
                .collect::<Vec<_>>(),
            vec!["PlayStation"]
        );
    }

    #[test]
    fn watch_entries_are_classified_by_name_only() {
        // A packaged streaming client is recognized by its id...
        let stremio = entry(
            "com.stremio.Stremio.desktop",
            &["AudioVideo", "Video", "Player"],
        );
        // ...and a hand-written browser web app by its `--app=` URL, because it
        // declares nothing useful.
        let netflix = Arc::new(DesktopEntryData {
            exec: Some("google-chrome --app=https://www.netflix.com".into()),
            ..entry("chrome-netflix.desktop", &["Network"])
                .as_ref()
                .clone()
        });

        assert!(super::is_watch_entry(&stremio));
        assert!(super::is_watch_entry(&netflix));
        // A game never counts, whatever it plays.
        assert!(!super::is_watch_entry(&entry(
            "hearthdeck:video-game",
            &["Game", "AudioVideo"]
        )));
    }

    #[test]
    fn media_categories_alone_do_not_make_a_watch_app() {
        // Volume mixers and capture tools declare AudioVideo/Video/Player but
        // have nothing to watch; matching on the category used to pull them in
        // alongside the players a local install already has.
        let mixer = entry(
            "org.pulseaudio.pavucontrol.desktop",
            &["AudioVideo", "Utility"],
        );
        let capture = entry("qvidcap.desktop", &["AudioVideo", "Video", "Player"]);
        let player = entry(
            "com.system76.CosmicPlayer.desktop",
            &["AudioVideo", "Player", "Video"],
        );

        assert!(!super::is_watch_entry(&mixer));
        assert!(!super::is_watch_entry(&capture));
        assert!(!super::is_watch_entry(&player));
    }

    #[test]
    fn watch_apps_stay_in_the_application_section() {
        // Watch apps have no library section of their own; the dashboard shelf
        // is the grouping, so the catalog keeps them as applications.
        let stremio = entry("stremio.desktop", &["AudioVideo", "Video"]);
        assert!(Section::Applications.matches(&stremio));
        assert_eq!(Section::ALL.len(), 3);
    }

    #[test]
    fn category_sync_preserves_custom_groups() {
        let mut config = AppLibraryConfig::default();
        config.sections.applications.push(AppGroup {
            name: "Favorites".into(),
            icon: "folder-symbolic".into(),
            filter: FilterType::AppIds(vec!["writer".into()]),
            source: GroupSource::Local,
        });

        config.sync_category_groups(&[entry("writer", &["Office"])]);

        assert!(config.sections.applications.iter().any(|group| {
            group.name == "Favorites"
                && matches!(&group.filter, FilterType::AppIds(ids) if ids == &["writer"])
        }));
    }

    fn scan_group(name: &str, ids: &[&str]) -> CategorizationGroup {
        CategorizationGroup {
            name: name.into(),
            entry_ids: ids.iter().map(|id| (*id).into()).collect(),
        }
    }

    fn tab_names(config: &AppLibraryConfig) -> Vec<String> {
        config
            .sections
            .applications
            .iter()
            .map(|group| group.name.clone())
            .collect()
    }

    #[test]
    fn a_scan_replaces_the_derived_tabs() {
        let mut config = AppLibraryConfig::default();
        let entries = [entry("writer", &["Office"]), entry("vlc", &["Video"])];
        config.sync_category_groups(&entries);
        assert_eq!(tab_names(&config).len(), 2);

        assert!(config.sync_categorization(&[scan_group("Media Center", &["vlc"])]));

        assert_eq!(tab_names(&config), vec!["Media Center"]);
        // A later rebuild must not put the freedesktop tabs back beside it.
        assert!(!config.sync_category_groups(&entries));
        assert_eq!(tab_names(&config), vec!["Media Center"]);
        assert!(
            config.sections.applications[0].matches(&entries[1]),
            "the scan's tab must still narrow to the app it named"
        );
    }

    #[test]
    fn a_second_scan_swaps_only_the_tabs_a_scan_made() {
        let mut config = AppLibraryConfig::default();
        config.sync_categorization(&[scan_group("Development", &["editor"])]);
        // The user's own folder, made between the two scans.
        config.add(Section::Applications, "Mine".into());

        assert!(config.sync_categorization(&[scan_group("Media Center", &["vlc"])]));

        assert_eq!(tab_names(&config), vec!["Media Center", "Mine"]);
        assert_eq!(
            config.sections.applications[0].source,
            GroupSource::Categorization
        );
    }

    #[test]
    fn an_empty_report_hands_the_section_back_to_the_derived_tabs() {
        let mut config = AppLibraryConfig::default();
        let entries = [entry("writer", &["Office"])];
        config.sync_categorization(&[scan_group("Development", &["editor"])]);

        config.sync_categorization(&[]);
        assert!(config.sections.applications.is_empty());

        assert!(config.sync_category_groups(&entries));
        assert_eq!(tab_names(&config), vec!["cosmic-office"]);
    }

    #[test]
    fn a_scan_keeps_the_folders_the_user_made() {
        let mut config = AppLibraryConfig::default();
        config.add(Section::Applications, "Mine".into());
        config.sections.applications[0].filter = FilterType::AppIds(vec!["writer".into()]);

        config.sync_categorization(&[scan_group("Development", &["editor"])]);

        assert_eq!(tab_names(&config), vec!["Development", "Mine"]);
    }

    #[test]
    fn the_scan_is_idempotent_so_a_poll_does_not_churn_the_config() {
        let mut config = AppLibraryConfig::default();
        let groups = [scan_group("Development", &["editor"])];

        assert!(config.sync_categorization(&groups));
        assert!(
            !config.sync_categorization(&groups),
            "re-applying the same report must not report a change"
        );
    }

    #[test]
    fn desktop_input_can_be_toggled_per_app() {
        let mut config = AppLibraryConfig::default();

        config.toggle_desktop_input("writer");
        assert!(config.desktop_input_enabled("writer"));
        assert!(!config.desktop_input_enabled("terminal"));

        config.toggle_desktop_input("writer");
        assert!(!config.desktop_input_enabled("writer"));
    }

    /// A console game as the RomM provider records it: the tags that put it in
    /// the Console Games section, plus the facet tags the filters read.
    fn console_game(
        id: &str,
        genres: &[&str],
        release_years: &[i32],
        regions: &[&str],
    ) -> Arc<DesktopEntryData> {
        let mut categories = vec!["Game".to_string(), romm_console_category(7)];
        categories.extend(genres.iter().map(|genre| romm_genre_category(genre)));
        categories.extend(release_years.iter().map(|year| romm_decade_category(*year)));
        categories.extend(regions.iter().map(|region| romm_region_category(region)));
        let categories: Vec<&str> = categories.iter().map(String::as_str).collect();
        entry(id, &categories)
    }

    #[test]
    fn console_facets_read_their_values_from_the_romm_tags() {
        let entries = [
            console_game("mario", &["Action", "Platformer"], &[1991], &["USA"]),
            console_game("chrono", &["RPG"], &[1995], &["USA", "Japan"]),
        ];

        assert_eq!(
            RommFacet::Genre.values(&entries),
            ["Action", "Platformer", "RPG"]
        );
        assert_eq!(RommFacet::Decade.values(&entries), ["1990"]);
        assert_eq!(RommFacet::Region.values(&entries), ["Japan", "USA"]);
        // A decade widens to the span it stands for; other values read as-is.
        assert_eq!(RommFacet::Decade.value_label("1990"), "1990s");
        assert_eq!(RommFacet::Genre.value_label("RPG"), "RPG");
    }

    #[test]
    fn filters_narrow_console_games_and_leave_other_sections_alone() {
        let entries = [
            console_game("mario", &["Action"], &[1991], &["USA"]),
            console_game("chrono", &["RPG"], &[1995], &["Japan"]),
            entry("writer", &["Office"]),
        ];
        let config = AppLibraryConfig::default();
        let mut filters = RommFilters::default();
        let visible = |section, filters: &RommFilters| {
            config
                .filtered(section, None, "", &entries, filters)
                .iter()
                .map(|entry| entry.id.clone())
                .collect::<Vec<_>>()
        };

        // No selection matches everything, so browsing is the same code path.
        assert!(!filters.is_active());
        assert_eq!(
            visible(Section::ConsoleGames, &filters),
            ["mario", "chrono"]
        );

        filters.set(RommFacet::Genre, Some("RPG".into()));
        assert_eq!(visible(Section::ConsoleGames, &filters), ["chrono"]);
        // The selection is the console grid's: applications keep their entry.
        assert_eq!(visible(Section::Applications, &filters), ["writer"]);

        // Facets combine, and setting one back to "All" drops only that one.
        filters.set(RommFacet::Region, Some("USA".into()));
        assert!(visible(Section::ConsoleGames, &filters).is_empty());
        filters.set(RommFacet::Region, None);
        assert_eq!(visible(Section::ConsoleGames, &filters), ["chrono"]);

        filters.clear();
        assert!(filters.selected(RommFacet::Genre).is_none());
        assert_eq!(
            visible(Section::ConsoleGames, &filters),
            ["mario", "chrono"]
        );
    }

    #[test]
    fn favorites_follow_saved_order_and_ignore_missing_entries() {
        let mut config = AppLibraryConfig::default();
        config.toggle_favorite("terminal");
        config.toggle_favorite("missing");
        config.toggle_favorite("writer");
        let entries = [entry("writer", &[]), entry("terminal", &[])];

        assert_eq!(
            config
                .favorite_entries(&entries)
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["terminal", "writer"]
        );

        config.toggle_favorite("terminal");
        assert!(!config.is_favorite("terminal"));
    }
}
