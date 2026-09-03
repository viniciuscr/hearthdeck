# Game Provider Architecture Design

## Reference project analysis

The `../hearthdeck` daemon project uses a clean provider pattern with two
trait layers:

### DiscoveryProvider trait (discovers what's installed)
```rust
#[async_trait]
trait DiscoveryProvider: Send + Sync {
    fn source_id(&self) -> &'static str;
    fn refresh_interval(&self) -> Option<Duration>;
    async fn discover(&self) -> Result<Vec<CatalogRecord>>;
}
```

### CatalogRecord (unified data model)
```rust
struct CatalogRecord {
    id: String,           // "heroic:epic:Fortnite"
    title: String,
    kind: String,         // "game" | "application"
    launch_id: Option<String>,  // "legendary:Fortnite"
    icon: Option<String>,
    metadata: serde_json::Value,
    updated_at: String,
}
```

### DiscoveryService (manages provider lifecycle)
- Spawns a tokio task per provider
- Coalesces duplicate refresh requests
- Tracks provider health (starting/refreshing/ready/degraded)
- Writes results to a CatalogStore (SQLite)
- Broadcasts `LibraryChanged` events via tokio broadcast channel

### Heroic provider
- Reads `~/.config/heroic/legendaryConfig/legendary/installed.json` for Epic
- Reads `~/.config/heroic/gog_store/installed.json` for GOG
- Reads metadata from `store_cache/` and `metadata/` dirs
- Launches via URI handler: `legendary:appid` or `gog:appid`
- 5-minute refresh interval

### DesktopApps provider
- Discovers XDG .desktop files via a bridge process
- Filters out game launchers (Steam, Lutris, Heroic) from game list
- 15-minute refresh interval

---

## Adaptation for cosmic-app-library

Our app is an Iced/COSMIC app (not a daemon+HTTP API), so communication
is in-process. But the provider pattern maps directly.

### Step 1: Define the provider trait and record type

File: `src/providers/mod.rs`

```rust
pub mod heroic;
pub mod desktop;

use async_trait::async_trait;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameRecord {
    pub id: String,              // "heroic:epic:Fortnite"
    pub title: String,
    pub kind: RecordKind,        // Game | Application
    pub launch_id: Option<String>,  // "legendary:Fortnite"
    pub icon: Option<String>,    // URL or path
    pub categories: Vec<String>,
    pub store: Option<String>,   // "Epic Games", "GOG", "Flatpak"
    pub source: String,          // provider source_id
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecordKind {
    Game,
    Application,
}

#[async_trait]
pub trait GameProvider: Send + Sync {
    /// Stable provider ID. Becomes the record's `source` field.
    fn source_id(&self) -> &'static str;

    /// `None` = only runs on manual refresh.
    fn refresh_interval(&self) -> Option<Duration>;

    /// Discover all items from this provider.
    async fn discover(&self) -> anyhow::Result<Vec<GameRecord>>;
}
```

### Step 2: Implement the Heroic provider

File: `src/providers/heroic.rs`

Reads Heroic's config files to discover installed Epic and GOG games.
Same logic as the reference project's `heroic.rs`, adapted to return
`GameRecord` instead of `CatalogRecord`.

Key files to read:
- `~/.config/heroic/legendaryConfig/legendary/installed.json` (Epic)
- `~/.config/heroic/gog_store/installed.json` (GOG)
- `~/.config/heroic/store_cache/legendary_gameinfo.json`
- `~/.config/heroic/store_cache/gog_library.json`
- `~/.config/heroic/legendaryConfig/legendary/metadata/{id}.json`

Launch: via Heroic URI handler (`heroic://launch/{runner}/{appid}`)

### Step 3: Implement the Desktop apps provider

File: `src/providers/desktop_apps.rs`

Wraps the existing `cosmic::desktop::load_applications()` call.
Filters out game launchers (Steam, Lutris, Heroic) from the game list.

### Step 4: Provider service (manages lifecycle)

File: `src/providers/service.rs`

```rust
pub struct ProviderService {
    providers: Vec<Arc<dyn GameProvider>>,
    records: Arc<Mutex<Vec<GameRecord>>>,
    health: Arc<Mutex<Vec<ProviderHealth>>>,
    events: broadcast::Sender<ProviderEvent>,
}

pub enum ProviderEvent {
    RecordsChanged { source_id: String, count: usize },
    ProviderFailed { source_id: String, error: String },
}

impl ProviderService {
    pub fn start(providers: Vec<Arc<dyn GameProvider>>) -> Self;
    pub async fn refresh_all(&self);
    pub async fn refresh(&self, source_id: &str);
    pub fn records(&self) -> Vec<GameRecord>;
    pub fn subscribe(&self) -> broadcast::Receiver<ProviderEvent>;
}
```

### Step 5: Integrate with HearthDeck app

The app calls `ProviderService::start()` in `init()`.
Records are merged with the existing `all_entries` / `entry_path_input`.
The UI displays games from providers alongside XDG desktop entries.
Provider health is shown in the sidebar or settings.

### Step 6: Launch integration

When a user activates a game:
1. Look up the `GameRecord` by ID
2. Use the `launch_id` to determine how to launch:
   - `"legendary:{appid}"` → `heroic://launch/legendary/{appid}`
   - `"gog:{appid}"` → `heroic://launch/gog/{appid}`
   - Desktop entry ID → existing `spawn_desktop_exec` path

---

## Implementation order

1. Create `src/providers/mod.rs` with `GameProvider` trait + `GameRecord`
2. Create `src/providers/heroic.rs` — read Heroic config, return records
3. Create `src/providers/desktop_apps.rs` — wrap existing desktop entry loading
4. Create `src/providers/service.rs` — lifecycle management
5. Wire into `HearthDeck::init()` — start providers, merge records
6. Update `load_apps()` — include provider records alongside XDG entries
7. Update `activate_app()` — handle provider-specific launch paths
8. Add provider health display in sidebar/settings
