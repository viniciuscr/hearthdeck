use crate::config::APP_ID;
use crate::fl;
use crate::form_factor::{self, FormFactor};
use crate::library_item::{
    CONSOLE_FILTER_PREFIX, LibraryItem, SOURCE_FILTER_PREFIX, STORE_FILTER_PREFIX,
};
use crate::providers::filter::{Facet, FacetOption};
use cosmic::cosmic_config::cosmic_config_derive::CosmicConfigEntry;
use cosmic::cosmic_config::{
    CosmicConfigEntry, {self},
};
pub use hearthdeck_protocol::Section;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The name the Applications section's Desktop tab is stored under, which
/// [`AppGroup::name`] shows in the user's language. Its sibling, the Couch view,
/// is the section's "all" tab rather than a stored one.
const DESKTOP_TAB_NAME: &str = "hearthdeck-desktop";

pub fn romm_console_category(platform_id: i64) -> String {
    format!("{CONSOLE_FILTER_PREFIX}{platform_id}")
}

/// A RomM metadata field the Console Games grid can be filtered by. Each
/// variant maps to a daemon filter parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RommFacet {
    Genre,
    Region,
}

impl RommFacet {
    /// Every facet, in the order the filter bar shows them.
    pub const ALL: [RommFacet; 2] = [Self::Genre, Self::Region];

    /// Stable identifier for the provider-agnostic filter model
    /// ([`crate::providers::filter`]). The daemon supplies these ids itself
    /// when it reports the facets.
    pub fn id(self) -> &'static str {
        match self {
            Self::Genre => "genre",
            Self::Region => "region",
        }
    }

    /// The facet with this id, if any.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|facet| facet.id() == id)
    }

    /// The facet's name, shown on its chip in the filter bar.
    pub fn name(self) -> String {
        match self {
            Self::Genre => fl!("filter-genre"),
            Self::Region => fl!("filter-region"),
        }
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
        // Two filters of the same shape compare by their contents, not as equal.
        // Returning `Equal` for distinct `AppIds` broke the `Ord` contract against
        // the derived `PartialEq` — `Ord` requires `cmp == Equal` exactly when the
        // values are equal — which would make any sorted or `BTree`-backed use
        // silently reorder or collapse entries.
        match (self, other) {
            (FilterType::AppIds(left), FilterType::AppIds(right)) => left.cmp(right),
            (
                FilterType::Categories {
                    categories: left_categories,
                    exclude: left_exclude,
                    include: left_include,
                },
                FilterType::Categories {
                    categories: right_categories,
                    exclude: right_exclude,
                    include: right_include,
                },
            ) => left_categories
                .cmp(right_categories)
                .then_with(|| left_exclude.cmp(right_exclude))
                .then_with(|| left_include.cmp(right_include)),
            (FilterType::None, FilterType::None) => std::cmp::Ordering::Equal,
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
            (
                FilterType::Categories { categories, .. },
                FilterType::Categories {
                    categories: others, ..
                },
            ) => {
                // A category tab is shown by its first category, so that is what
                // it sorts on — matching the name comparison the other arms use.
                let left = categories.first().unwrap_or(&self.name);
                let right = others.first().unwrap_or(&other.name);
                left.to_lowercase().cmp(&right.to_lowercase())
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
    fn matches(&self, entry: &LibraryItem) -> bool {
        match &self.filter {
            FilterType::AppIds(names) => names.iter().any(|id| id == &entry.id),
            FilterType::Categories {
                categories,
                include,
                exclude,
                ..
            } => {
                categories.iter().any(|cat| entry.matches_category(cat))
                    && exclude.iter().all(|id| id != &entry.id)
                    || include.iter().any(|id| id == &entry.id)
            }
            FilterType::None => true,
        }
    }

    pub fn name(&self) -> String {
        if self.name == DESKTOP_TAB_NAME {
            fl!("desktop-apps")
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
                .strip_prefix(CONSOLE_FILTER_PREFIX)
                .and_then(|id| id.parse().ok())
        })
    }
}

pub(crate) trait SectionUi {
    /// Icon shown beside the section's name in the sidebar. Named rather than
    /// inlined at the view so the three sections cannot drift apart in style.
    fn icon_name(self) -> &'static str;
    fn name(self) -> String;
    fn index(self) -> usize;
    fn all_name(self) -> String;
    fn matches(self, entry: &LibraryItem) -> bool;
}

impl SectionUi for Section {
    fn icon_name(self) -> &'static str {
        match self {
            Self::PcGames => "applications-games-symbolic",
            Self::ConsoleGames => "input-gaming-symbolic",
            Self::Applications => "view-app-grid-symbolic",
        }
    }

    fn name(self) -> String {
        match self {
            Section::PcGames => fl!("pc-games"),
            Section::ConsoleGames => fl!("console-games"),
            Section::Applications => fl!("applications"),
        }
    }

    fn index(self) -> usize {
        match self {
            Section::PcGames => 0,
            Section::ConsoleGames => 1,
            Section::Applications => 2,
        }
    }

    /// Label of the "all" tab for this section.
    fn all_name(self) -> String {
        match self {
            Section::PcGames | Section::ConsoleGames => fl!("all-games"),
            Section::Applications => fl!("couch-apps"),
        }
    }

    /// Placement is resolved by the daemon, not inferred from display fields.
    fn matches(self, entry: &LibraryItem) -> bool {
        self == entry.section
    }
}

/// The filter facets a locally-loaded section offers, derived from the entries
/// it shows.
///
/// This is the local counterpart to the console grid's daemon-supplied facets,
/// so every section filters the same way. Store values keep their saved filter
/// encoding but come from typed metadata. A dimension with fewer than two
/// options is dropped: a filter with one choice is not a choice.
pub fn section_facets(section: Section, entries: &[Arc<LibraryItem>]) -> Vec<Facet> {
    let (id, label) = match section {
        Section::PcGames => ("store", fl!("filter-store")),
        Section::Applications => ("source", fl!("filter-source")),
        Section::ConsoleGames => return Vec::new(),
    };
    let mut values = BTreeSet::new();
    for entry in entries.iter().filter(|entry| section.matches(entry)) {
        match section {
            Section::PcGames => values.extend(entry.store_filter_value()),
            Section::Applications => values.extend(entry.source_filter_values()),
            Section::ConsoleGames => {}
        }
    }
    let options = values
        .into_iter()
        .map(|value| FacetOption {
            label: facet_value_label(&value),
            value,
        })
        .collect::<Vec<_>>();
    if options.len() < 2 {
        return Vec::new();
    }
    vec![Facet {
        id: id.to_owned(),
        label,
        options,
    }]
}

/// How a local facet value reads on a chip: a store drops its tag prefix, and a
/// source drops its own and reads as the place it names.
fn facet_value_label(value: &str) -> String {
    if let Some(source) = value.strip_prefix(SOURCE_FILTER_PREFIX) {
        return source_label(source);
    }
    value
        .strip_prefix(STORE_FILTER_PREFIX)
        .unwrap_or(value)
        .to_owned()
}

/// How a source reads on a filter chip. Spelled out rather than title-cased, so
/// the vocabulary and the words a user sees for it stay together.
fn source_label(source: &str) -> String {
    match source {
        "system" => "System",
        "local" => "Local",
        "flatpak" => "Flatpak",
        "snap" => "Snap",
        "nix" => "Nix",
        "streaming" => "Streaming",
        other => other,
    }
    .to_owned()
}

/// Sub-tabs (filter chips) shown in the top bar, one set per section.
///
/// Derived from the catalog and the console platforms every time they load, so
/// never persisted: a saved copy could only go stale.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Sections {
    pub pc_games: Vec<AppGroup>,
    pub console_games: Vec<AppGroup>,
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

    /// Rebuilds the local sections' tabs from the entries currently present.
    ///
    /// PC Games gets a tab per store that installed something. Applications gets
    /// one Desktop tab when anything wants a desktop; the Couch view is the
    /// section's "all" tab.
    pub fn sync_category_groups(&mut self, entries: &[Arc<LibraryItem>]) {
        self.pc_games = derived_groups(Section::PcGames, entries);
        self.applications = derived_groups(Section::Applications, entries);
    }

    /// Rebuilds the console tabs from the platforms RomM reported, and says
    /// whether they changed.
    pub fn sync_console_groups(&mut self, platforms: &[(i64, String)]) -> bool {
        let groups: Vec<AppGroup> = platforms
            .iter()
            .map(|(id, name)| AppGroup {
                name: name.clone(),
                icon: "folder-symbolic".to_string(),
                filter: FilterType::Categories {
                    categories: vec![romm_console_category(*id)],
                    include: Vec::new(),
                    exclude: Vec::new(),
                },
            })
            .collect();
        if self.console_games == groups {
            return false;
        }
        self.console_games = groups;
        true
    }

    /// The entries one tab of one section shows: the section's records, the
    /// tab's group, the search box and the facet selection, all applied.
    ///
    /// Every selected value must match the item's metadata or categories using
    /// the compatible values offered by [`section_facets`].
    pub fn filtered_by_facets(
        &self,
        section: Section,
        tab: Option<usize>,
        input_value: &str,
        entries: &[Arc<LibraryItem>],
        facets: &BTreeMap<String, String>,
    ) -> Vec<Arc<LibraryItem>> {
        let tab_group = tab.and_then(|i| self.get(section).get(i));
        // The Applications section opens on what suits the couch; a search looks
        // through both, because the app being looked for may be the other kind.
        let couch_only =
            section == Section::Applications && tab.is_none() && input_value.is_empty();
        entries
            .iter()
            .filter(|de| {
                if !section.matches(de) {
                    return false;
                }
                if couch_only && form_factor::of(de) != FormFactor::Couch {
                    return false;
                }
                if let Some(group) = tab_group
                    && !group.matches(de)
                {
                    return false;
                }
                if !facets.values().all(|value| de.matches_facet(value)) {
                    return false;
                }
                if !input_value.is_empty()
                    && !de.name.to_lowercase().contains(&input_value.to_lowercase())
                    && !de.matches_category(input_value)
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, CosmicConfigEntry)]
pub struct AppLibraryConfig {
    #[serde(default)]
    pub desktop_input_apps: Vec<String>,
    /// Applications the user moved between Couch and Desktop. What they chose
    /// outranks whatever the rules or a model say.
    #[serde(default)]
    pub form_overrides: BTreeMap<String, FormFactor>,
}

impl AppLibraryConfig {
    pub fn version() -> u64 {
        2
    }

    pub fn helper() -> Option<cosmic_config::Config> {
        cosmic_config::Config::new(APP_ID, Self::version()).ok()
    }

    /// Reads the saved entry, falling back to the defaults where a key is
    /// missing or does not parse - a broken key must not take the shell down.
    pub fn read(store: Option<&cosmic_config::Config>) -> Self {
        store
            .map(|store| {
                Self::get_entry(store).unwrap_or_else(|(errors, config)| {
                    for error in errors {
                        log::error!("{error:?}");
                    }
                    config
                })
            })
            .unwrap_or_default()
    }

    pub fn desktop_input_enabled(&self, id: &str) -> bool {
        self.desktop_input_apps
            .iter()
            .any(|configured| configured == id)
    }

    /// Whether `entry` launches with the pointer-driving input profile.
    ///
    /// A Desktop application starts there by default, and the per-app toggle
    /// flips whichever default the application has. Games have no form of their
    /// own - a console game is always pad-driven, and a PC game keeps the
    /// explicit toggle alone - so the form default applies to applications only.
    #[must_use]
    pub fn desktop_input_for(&self, entry: &LibraryItem) -> bool {
        let desktop_by_default =
            Section::Applications.matches(entry) && form_factor::of(entry) == FormFactor::Desktop;
        self.desktop_input_enabled(&entry.id) ^ desktop_by_default
    }

    pub fn set_form_override(&mut self, id: &str, form: FormFactor) {
        self.form_overrides.insert(id.to_owned(), form);
    }

    /// Puts the user's choices onto freshly loaded records.
    pub fn apply_form_overrides(&self, entries: &mut [Arc<LibraryItem>]) {
        for entry in entries {
            if let Some(form) = self.form_overrides.get(&entry.id)
                && form_factor::of(entry) != *form
            {
                form_factor::set(Arc::make_mut(entry), *form);
            }
        }
    }

    pub fn toggle_desktop_input(&mut self, id: &str) {
        if self.desktop_input_enabled(id) {
            self.desktop_input_apps
                .retain(|configured| configured != id);
        } else {
            self.desktop_input_apps.push(id.to_owned());
        }
    }
}

/// The tabs `section` derives from `entries`.
fn derived_groups(section: Section, entries: &[Arc<LibraryItem>]) -> Vec<AppGroup> {
    let mut shown = entries.iter().filter(|entry| section.matches(entry));
    match section {
        Section::PcGames => {
            let stores: BTreeMap<String, String> = shown
                .filter_map(|entry| entry.store_filter_value())
                .map(|value| (value.to_lowercase(), value))
                .collect();
            stores
                .into_values()
                .map(|category| AppGroup {
                    name: category
                        .strip_prefix(STORE_FILTER_PREFIX)
                        .map_or_else(|| category.clone(), str::to_owned),
                    icon: "folder-symbolic".to_string(),
                    filter: FilterType::Categories {
                        categories: vec![category],
                        include: Vec::new(),
                        exclude: Vec::new(),
                    },
                })
                .collect()
        }
        Section::Applications => {
            if !shown.any(|entry| form_factor::of(entry) == FormFactor::Desktop) {
                return Vec::new();
            }
            vec![AppGroup {
                name: DESKTOP_TAB_NAME.to_owned(),
                icon: "folder-symbolic".to_string(),
                filter: FilterType::Categories {
                    categories: vec![form_factor::category(FormFactor::Desktop)],
                    include: Vec::new(),
                    exclude: Vec::new(),
                },
            }]
        }
        Section::ConsoleGames => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppGroup, AppLibraryConfig, FilterType, Section, SectionUi, Sections, section_facets,
    };
    use crate::form_factor::{self, FormFactor};
    use crate::library_item::LibraryItem;
    use cosmic::desktop::fde::IconSource;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    #[test]
    fn filter_type_ordering_is_consistent_with_equality() {
        // `Ord` must agree with the derived `PartialEq`: `cmp == Equal` exactly when
        // the values are equal. It used to return `Equal` for any two `AppIds`, which
        // would let a sorted or `BTree`-backed collection drop a distinct filter.
        let a = FilterType::AppIds(vec!["a".to_owned()]);
        let b = FilterType::AppIds(vec!["b".to_owned()]);
        assert_ne!(a, b);
        assert_ne!(a.cmp(&b), std::cmp::Ordering::Equal);
        assert_eq!(a.cmp(&a.clone()), std::cmp::Ordering::Equal);

        let left = FilterType::Categories {
            categories: vec!["Media".to_owned()],
            exclude: Vec::new(),
            include: Vec::new(),
        };
        let right = FilterType::Categories {
            categories: vec!["Games".to_owned()],
            exclude: Vec::new(),
            include: Vec::new(),
        };
        assert_ne!(left, right);
        assert_ne!(left.cmp(&right), std::cmp::Ordering::Equal);
    }

    /// Display fixtures with the section and form a daemon response supplies.
    fn entry(id: &str, categories: &[&str]) -> Arc<LibraryItem> {
        let mut entry = LibraryItem {
            id: id.into(),
            name: id.into(),
            wm_class: None,
            exec: None,
            icon: IconSource::Name(String::new()),
            form_factor: None,
            section: if categories.contains(&"Game") {
                if categories
                    .iter()
                    .any(|value| value.starts_with("hearthdeck-console:"))
                {
                    Section::ConsoleGames
                } else {
                    Section::PcGames
                }
            } else {
                Section::Applications
            },
            console: categories.iter().find_map(|value| {
                value
                    .strip_prefix("hearthdeck-console:")
                    .and_then(|id| id.parse().ok())
                    .map(|platform_id| crate::library_item::ConsoleMetadata {
                        platform_id,
                        genres: Vec::new(),
                        regions: Vec::new(),
                    })
            }),
            store: categories
                .iter()
                .find_map(|value| value.strip_prefix("hearthdeck-store:"))
                .map(str::to_owned),
            sources: categories
                .iter()
                .filter_map(|value| value.strip_prefix("hearthdeck-source:"))
                .map(str::to_owned)
                .collect(),
            categories: categories
                .iter()
                .filter(|value| !value.starts_with("hearthdeck-store:"))
                .filter(|value| !value.starts_with("hearthdeck-source:"))
                .filter(|value| !value.starts_with("hearthdeck-console:"))
                .map(|category| (*category).into())
                .collect(),
        };
        if Section::Applications.matches(&entry) {
            entry.form_factor = Some(match id {
                "com.stremio.Stremio.desktop" | "stremio.desktop" => FormFactor::Couch,
                _ => FormFactor::Desktop,
            });
        }
        Arc::new(entry)
    }

    #[test]
    fn the_desktop_tab_exists_only_while_something_wants_it() {
        let stremio = entry("com.stremio.Stremio.desktop", &["AudioVideo"]);
        let mut tabs = Sections::default();

        tabs.sync_category_groups(std::slice::from_ref(&stremio));
        assert!(tabs.applications.is_empty());

        tabs.sync_category_groups(&[stremio, entry("writer", &["Office"])]);
        assert_eq!(tab_names(&tabs), vec!["hearthdeck-desktop"]);
    }

    #[test]
    fn applications_open_on_the_couch_and_a_search_looks_through_both() {
        let stremio = entry("com.stremio.Stremio.desktop", &["AudioVideo"]);
        let writer = entry("writer", &["Office"]);
        let entries = [stremio, writer];
        let mut tabs = Sections::default();
        tabs.sync_category_groups(&entries);
        let names = |found: Vec<Arc<LibraryItem>>| {
            found
                .iter()
                .map(|found| found.id.clone())
                .collect::<Vec<_>>()
        };
        let show = |tab, search: &str| {
            names(tabs.filtered_by_facets(
                Section::Applications,
                tab,
                search,
                &entries,
                &BTreeMap::new(),
            ))
        };

        assert_eq!(show(None, ""), vec!["com.stremio.Stremio.desktop"]);
        assert_eq!(show(Some(0), ""), vec!["writer"]);
        assert_eq!(show(None, "writer"), vec!["writer"]);
    }

    #[test]
    fn what_the_user_chose_outranks_the_rules() {
        let mut config = AppLibraryConfig::default();
        let mut entries = [entry("writer", &["Office"])];
        assert_eq!(form_factor::of(&entries[0]), FormFactor::Desktop);

        config.set_form_override("writer", FormFactor::Couch);
        config.apply_form_overrides(&mut entries);

        assert_eq!(form_factor::of(&entries[0]), FormFactor::Couch);
    }

    #[test]
    fn saved_form_filters_match_typed_items_without_category_tags() {
        let group: AppGroup = serde_json::from_value(serde_json::json!({
            "name": "Desktop",
            "icon": "folder-symbolic",
            "filter": {"Categories": {
                "categories": ["hearthdeck-form:desktop"],
                "include": [],
                "exclude": []
            }}
        }))
        .expect("existing group format");
        let mut writer = entry("writer", &["Office"]);
        let game = entry("game", &["Game"]);
        assert_eq!(writer.categories, ["Office"]);
        assert!(group.matches(&writer));
        assert!(!group.matches(&game));
        form_factor::set(Arc::make_mut(&mut writer), FormFactor::Couch);
        assert!(!group.matches(&writer));

        let facets = BTreeMap::from([("form".to_owned(), "hearthdeck-form:couch".to_owned())]);
        assert_eq!(
            Sections::default()
                .filtered_by_facets(Section::Applications, None, "", &[writer], &facets)
                .len(),
            1
        );
    }

    #[test]
    fn a_desktop_application_launches_with_the_pointer_unless_told_otherwise() {
        let mut config = AppLibraryConfig::default();
        let writer = entry("writer", &["Office"]);
        let stremio = entry("com.stremio.Stremio.desktop", &["AudioVideo"]);

        assert!(config.desktop_input_for(&writer));
        assert!(!config.desktop_input_for(&stremio));

        config.toggle_desktop_input("writer");
        config.toggle_desktop_input("com.stremio.Stremio.desktop");

        assert!(
            !config.desktop_input_for(&writer),
            "the toggle flips the default"
        );
        assert!(config.desktop_input_for(&stremio));
    }

    #[test]
    fn games_keep_the_explicit_input_toggle_alone() {
        let mut config = AppLibraryConfig::default();
        let game = entry("epic-game", &["Game"]);

        assert!(
            !config.desktop_input_for(&game),
            "a PC game stays pad-driven until the switch says otherwise"
        );

        config.toggle_desktop_input("epic-game");

        assert!(config.desktop_input_for(&game));
    }

    #[test]
    fn pc_game_tabs_only_show_stores_with_installed_games() {
        let mut tabs = Sections::default();
        tabs.sync_category_groups(&[
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
            tabs.get(Section::PcGames)
                .iter()
                .map(|group| group.name())
                .collect::<Vec<_>>(),
            vec!["Epic Games", "GOG"]
        );

        tabs.sync_category_groups(&[entry(
            "gog-game",
            &["Game", "FileTransfer", "Shooter", "hearthdeck-store:GOG"],
        )]);

        assert_eq!(
            tabs.get(Section::PcGames)
                .iter()
                .map(|group| group.name())
                .collect::<Vec<_>>(),
            vec!["GOG"]
        );
    }

    #[test]
    fn console_tabs_follow_live_romm_platforms_not_game_genres() {
        let mut tabs = Sections::default();
        tabs.sync_category_groups(&[entry("romm:42", &["Game", "RPG", "hearthdeck-console:7"])]);
        assert!(tabs.console_games.is_empty());

        tabs.sync_console_groups(&[(7, "SNES".into()), (9, "PlayStation".into())]);
        assert_eq!(
            tabs.get(Section::ConsoleGames)
                .iter()
                .map(|group| group.name())
                .collect::<Vec<_>>(),
            vec!["SNES", "PlayStation"]
        );
        assert_eq!(tabs.console_games[0].romm_platform_id(), Some(7));

        tabs.sync_console_groups(&[(9, "PlayStation".into())]);
        assert_eq!(
            tabs.get(Section::ConsoleGames)
                .iter()
                .map(|group| group.name())
                .collect::<Vec<_>>(),
            vec!["PlayStation"]
        );
    }

    #[test]
    fn watch_apps_stay_in_the_application_section() {
        // Watch apps have no library section of their own; the dashboard shelf
        // is the grouping, so the catalog keeps them as applications.
        let stremio = entry("stremio.desktop", &["AudioVideo", "Video"]);
        assert!(Section::Applications.matches(&stremio));
        assert_eq!(Section::ALL.len(), 3);
    }

    fn tab_names(tabs: &Sections) -> Vec<String> {
        tabs.applications
            .iter()
            .map(|group| group.name.clone())
            .collect()
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

    #[test]
    fn pc_games_offer_a_store_facet_and_applications_a_source_facet() {
        let entries = [
            entry("steam-game", &["Game", "hearthdeck-store:Steam"]),
            entry("heroic-game", &["Game", "hearthdeck-store:Heroic"]),
            entry("editor", &["Development", "hearthdeck-source:flatpak"]),
            entry("browser", &["Network", "hearthdeck-source:system"]),
        ];

        let pc = section_facets(Section::PcGames, &entries);
        assert_eq!(entries[0].categories, ["Game"]);
        assert_eq!(entries[0].store.as_deref(), Some("Steam"));
        assert_eq!(pc.len(), 1, "pc games should offer one facet");
        assert_eq!(pc[0].id, "store");
        assert_eq!(
            pc[0]
                .options
                .iter()
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["hearthdeck-store:Heroic", "hearthdeck-store:Steam"]
        );
        assert_eq!(pc[0].options[0].label, "Heroic");

        // Applications filter by where they were installed rather than by
        // freedesktop category: "Utility" is a bucket an entry lands in, not
        // something anyone browses a launcher by.
        let apps = section_facets(Section::Applications, &entries);
        assert_eq!(entries[2].categories, ["Development"]);
        assert_eq!(entries[2].sources, ["flatpak"]);
        assert_eq!(apps.len(), 1, "applications should offer one facet");
        assert_eq!(apps[0].id, "source");
        assert_eq!(
            apps[0]
                .options
                .iter()
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["hearthdeck-source:flatpak", "hearthdeck-source:system"]
        );
        assert_eq!(apps[0].options[0].label, "Flatpak");
        assert_eq!(apps[0].options[1].label, "System");

        // The console grid's facets come from the daemon, not the local catalog.
        assert!(section_facets(Section::ConsoleGames, &entries).is_empty());
    }

    /// One source is not a choice, so it is not offered: a machine whose
    /// applications all came from the distribution gets no filter rather than
    /// one that shows the same list twice.
    #[test]
    fn a_single_source_offers_no_facet() {
        let entries = [
            entry("editor", &["Development", "hearthdeck-source:system"]),
            entry("browser", &["Network", "hearthdeck-source:system"]),
        ];

        assert!(section_facets(Section::Applications, &entries).is_empty());
    }

    #[test]
    fn facet_selection_narrows_local_sections_by_category() {
        let entries = [
            entry("a", &["Game", "hearthdeck-store:Steam"]),
            entry("b", &["Game", "hearthdeck-store:Heroic"]),
            entry("c", &["Network"]),
        ];
        let tabs = Sections::default();
        let facets = std::collections::BTreeMap::from([(
            "store".to_owned(),
            "hearthdeck-store:Steam".to_owned(),
        )]);

        let visible = tabs.filtered_by_facets(Section::PcGames, None, "", &entries, &facets);
        assert_eq!(
            visible
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["a"]
        );
    }

    #[test]
    fn saved_store_groups_and_search_match_items_without_store_tags() {
        let group: AppGroup = serde_json::from_value(serde_json::json!({
            "name": "Steam",
            "icon": "folder-symbolic",
            "filter": {"Categories": {
                "categories": ["HEARTHDECK-STORE:STEAM"],
                "include": [],
                "exclude": []
            }}
        }))
        .expect("existing group format");
        let game = entry("a", &["Game", "hearthdeck-store:Steam"]);
        assert_eq!(game.categories, ["Game"]);
        assert!(group.matches(&game));
        let tabs = Sections::default();
        let found = tabs.filtered_by_facets(
            Section::PcGames,
            None,
            "hearthdeck-store:steam",
            &[game],
            &BTreeMap::new(),
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "a");
    }

    #[test]
    fn saved_source_groups_search_and_facets_match_without_source_tags() {
        let group: AppGroup = serde_json::from_value(serde_json::json!({
            "name": "Custom origin",
            "icon": "folder-symbolic",
            "filter": {"Categories": {
                "categories": ["HEARTHDECK-SOURCE:CUSTOM"],
                "include": [],
                "exclude": []
            }}
        }))
        .expect("existing group format");
        let app = entry(
            "a",
            &[
                "Utility",
                "hearthdeck-source:custom",
                "hearthdeck-source:streaming",
            ],
        );
        assert_eq!(app.categories, ["Utility"]);
        assert!(group.matches(&app));
        let tabs = Sections::default();
        let facets = BTreeMap::from([(
            "source".to_owned(),
            "hearthdeck-source:streaming".to_owned(),
        )]);
        let found = tabs.filtered_by_facets(
            Section::Applications,
            None,
            "hearthdeck-source:CUSTOM",
            &[app],
            &facets,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "a");
    }

    #[test]
    fn saved_console_groups_search_and_facets_match_without_category_tags() {
        let group: AppGroup = serde_json::from_value(serde_json::json!({
            "name": "SNES",
            "icon": "folder-symbolic",
            "filter": {"Categories": {
                "categories": ["hearthdeck-console:7"],
                "include": [],
                "exclude": []
            }}
        }))
        .expect("existing group format");
        let game = Arc::new(LibraryItem {
            id: "romm:42".to_owned(),
            section: Section::ConsoleGames,
            categories: vec!["Game".to_owned()],
            console: Some(crate::library_item::ConsoleMetadata {
                platform_id: 7,
                genres: vec!["RPG".to_owned()],
                regions: vec!["Japan".to_owned()],
            }),
            ..LibraryItem::default()
        });
        assert!(Section::ConsoleGames.matches(&game));
        assert!(!Section::PcGames.matches(&game));
        assert!(group.matches(&game));
        assert_eq!(group.romm_platform_id(), Some(7));
        let tabs = Sections::default();
        let found = tabs.filtered_by_facets(
            Section::ConsoleGames,
            None,
            "HEARTHDECK-GENRE:rpg",
            &[game],
            &BTreeMap::from([("region".to_owned(), "hearthdeck-region:Japan".to_owned())]),
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "romm:42");
    }
}
