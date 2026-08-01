import 'package:flutter/material.dart';

import 'dashboard_models.dart';

enum LibraryCategory { games, consoleGames, apps, groups, history }

class LibraryCategoryDefinition {
  const LibraryCategoryDefinition({
    required this.category,
    required this.label,
    required this.icon,
  });

  final LibraryCategory category;
  final String label;
  final IconData icon;
}

class LibrarySource {
  const LibrarySource({
    required this.id,
    required this.label,
    required this.items,
  });

  final String id;
  final String label;
  final List<DashboardItem> items;
}

class LibraryFeature {
  const LibraryFeature({
    required this.id,
    required this.title,
    required this.subtitle,
    required this.icon,
    required this.colors,
  });

  final String id;
  final String title;
  final String subtitle;
  final IconData icon;
  final List<Color> colors;
}

const List<LibraryCategoryDefinition> libraryCategories =
    <LibraryCategoryDefinition>[
      LibraryCategoryDefinition(
        category: LibraryCategory.games,
        label: 'PC games',
        icon: Icons.computer_rounded,
      ),
      LibraryCategoryDefinition(
        category: LibraryCategory.consoleGames,
        label: 'Console games',
        icon: Icons.videogame_asset_rounded,
      ),
      LibraryCategoryDefinition(
        category: LibraryCategory.apps,
        label: 'Apps',
        icon: Icons.apps_rounded,
      ),
      LibraryCategoryDefinition(
        category: LibraryCategory.groups,
        label: 'Groups',
        icon: Icons.bookmarks_outlined,
      ),
      LibraryCategoryDefinition(
        category: LibraryCategory.history,
        label: 'History',
        icon: Icons.history_rounded,
      ),
    ];

const List<LibraryFeature> gameLibraryFeatures = <LibraryFeature>[
  LibraryFeature(
    id: 'recently-added',
    title: 'Recently added',
    subtitle: 'New to your collection',
    icon: Icons.new_releases_outlined,
    colors: <Color>[Color(0xFF164A70), Color(0xFF102436)],
  ),
  LibraryFeature(
    id: 'continue-playing',
    title: 'Continue playing',
    subtitle: 'Pick up where you left off',
    icon: Icons.play_circle_outline_rounded,
    colors: <Color>[Color(0xFF6C3E1D), Color(0xFF25170C)],
  ),
  LibraryFeature(
    id: 'cloud-gaming',
    title: 'Cloud gaming',
    subtitle: 'No download required',
    icon: Icons.cloud_queue_rounded,
    colors: <Color>[Color(0xFF354D8A), Color(0xFF151B42)],
  ),
];

const List<LibraryFeature> appLibraryFeatures = <LibraryFeature>[
  LibraryFeature(
    id: 'streaming',
    title: 'Streams',
    subtitle: 'Netflix, Prime, and more',
    icon: Icons.settings_input_component_rounded,
    colors: <Color>[Color(0xFF6B376B), Color(0xFF2B1532)],
  ),
  LibraryFeature(
    id: 'media',
    title: 'Media essentials',
    subtitle: 'Watch, listen, and browse',
    icon: Icons.playlist_play_rounded,
    colors: <Color>[Color(0xFF17695D), Color(0xFF0C302B)],
  ),
  LibraryFeature(
    id: 'utilities',
    title: 'Utilities',
    subtitle: 'Everyday controls',
    icon: Icons.handyman_outlined,
    colors: <Color>[Color(0xFF4C5364), Color(0xFF212630)],
  ),
];

const List<LibrarySource> gameLibrarySources = <LibrarySource>[
  LibrarySource(
    id: 'all-games',
    label: 'All games',
    items: <DashboardItem>[
      DashboardItem(
        id: 'orbit-library',
        title: 'Orbit',
        description: 'Continue your expedition',
        icon: Icons.play_circle_outline_rounded,
        colors: <Color>[Color(0xFF143C5B), Color(0xFF071A2B)],
        badge: 'Continue',
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'ember',
        title: 'Emberfall',
        description: 'Action RPG',
        icon: Icons.local_fire_department_outlined,
        colors: <Color>[Color(0xFF7B3022), Color(0xFF35100C)],
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'drift',
        title: 'Neon Drift',
        description: 'Arcade racing',
        icon: Icons.directions_car_filled_outlined,
        colors: <Color>[Color(0xFF205E8D), Color(0xFF102843)],
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'isle',
        title: 'Quiet Isle',
        description: 'Cozy adventure',
        icon: Icons.terrain_outlined,
        colors: <Color>[Color(0xFF547B43), Color(0xFF22361B)],
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'citadel',
        title: 'Citadel',
        description: 'Strategy',
        icon: Icons.castle_outlined,
        colors: <Color>[Color(0xFF5C4F89), Color(0xFF292349)],
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'signal',
        title: 'Signal Lost',
        description: 'Mystery',
        icon: Icons.radar_outlined,
        colors: <Color>[Color(0xFF486D71), Color(0xFF1B3035)],
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'forge',
        title: 'Sky Forge',
        description: 'Survival',
        icon: Icons.construction_outlined,
        colors: <Color>[Color(0xFF8A5B22), Color(0xFF3B220B)],
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'violet',
        title: 'Violet Protocol',
        description: 'Tactical action',
        icon: Icons.memory_rounded,
        colors: <Color>[Color(0xFF713C76), Color(0xFF301631)],
        kind: TvContentKind.game,
      ),
    ],
  ),
  LibrarySource(
    id: 'steam',
    label: 'Steam',
    items: <DashboardItem>[
      DashboardItem(
        id: 'ember-steam',
        title: 'Emberfall',
        description: 'Steam library',
        icon: Icons.local_fire_department_outlined,
        colors: <Color>[Color(0xFF7B3022), Color(0xFF35100C)],
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'drift-steam',
        title: 'Neon Drift',
        description: 'Steam library',
        icon: Icons.directions_car_filled_outlined,
        colors: <Color>[Color(0xFF205E8D), Color(0xFF102843)],
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'citadel-steam',
        title: 'Citadel',
        description: 'Steam library',
        icon: Icons.castle_outlined,
        colors: <Color>[Color(0xFF5C4F89), Color(0xFF292349)],
        kind: TvContentKind.game,
      ),
    ],
  ),
  LibrarySource(
    id: 'gog',
    label: 'GOG',
    items: <DashboardItem>[
      DashboardItem(
        id: 'isle-gog',
        title: 'Quiet Isle',
        description: 'GOG library',
        icon: Icons.terrain_outlined,
        colors: <Color>[Color(0xFF547B43), Color(0xFF22361B)],
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'signal-gog',
        title: 'Signal Lost',
        description: 'GOG library',
        icon: Icons.radar_outlined,
        colors: <Color>[Color(0xFF486D71), Color(0xFF1B3035)],
        kind: TvContentKind.game,
      ),
    ],
  ),
  LibrarySource(
    id: 'epic',
    label: 'Epic Games',
    items: <DashboardItem>[
      DashboardItem(
        id: 'forge-epic',
        title: 'Sky Forge',
        description: 'Epic Games library',
        icon: Icons.construction_outlined,
        colors: <Color>[Color(0xFF8A5B22), Color(0xFF3B220B)],
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'violet-epic',
        title: 'Violet Protocol',
        description: 'Epic Games library',
        icon: Icons.memory_rounded,
        colors: <Color>[Color(0xFF713C76), Color(0xFF301631)],
        kind: TvContentKind.game,
      ),
    ],
  ),
  LibrarySource(
    id: 'emulators',
    label: 'Emulators',
    items: <DashboardItem>[
      DashboardItem(
        id: 'retro-emulator',
        title: 'Retro',
        description: 'Emulator collection',
        icon: Icons.videogame_asset_rounded,
        colors: <Color>[Color(0xFF883A55), Color(0xFF3E1425)],
        kind: TvContentKind.game,
      ),
    ],
  ),
];

const List<LibrarySource> appLibrarySources = <LibrarySource>[
  LibrarySource(
    id: 'all-apps',
    label: 'All apps',
    items: <DashboardItem>[
      DashboardItem(
        id: 'stream-app',
        title: 'Stream',
        description: 'Watch live and on demand',
        icon: Icons.live_tv_rounded,
        colors: <Color>[Color(0xFF632F71), Color(0xFF25133D)],
        kind: TvContentKind.application,
      ),
      DashboardItem(
        id: 'cinema-app',
        title: 'Cinema',
        description: 'Your movie collection',
        icon: Icons.movie_filter_outlined,
        colors: <Color>[Color(0xFF7B3E23), Color(0xFF35190E)],
        kind: TvContentKind.application,
      ),
      DashboardItem(
        id: 'music-app',
        title: 'Music',
        description: 'Albums and stations',
        icon: Icons.graphic_eq_rounded,
        colors: <Color>[Color(0xFF3557A0), Color(0xFF17234E)],
        kind: TvContentKind.application,
      ),
      DashboardItem(
        id: 'gallery-app',
        title: 'Gallery',
        description: 'Photos and videos',
        icon: Icons.photo_library_outlined,
        colors: <Color>[Color(0xFF397971), Color(0xFF123D3D)],
        kind: TvContentKind.application,
      ),
      DashboardItem(
        id: 'browser-app',
        title: 'Browser',
        description: 'The open web',
        icon: Icons.language_rounded,
        colors: <Color>[Color(0xFF5850A4), Color(0xFF282454)],
        kind: TvContentKind.application,
      ),
      DashboardItem(
        id: 'settings-app',
        title: 'Settings',
        description: 'System preferences',
        icon: Icons.tune_rounded,
        colors: <Color>[Color(0xFF4A5560), Color(0xFF1F272F)],
        kind: TvContentKind.system,
      ),
    ],
  ),
  LibrarySource(id: 'streams', label: 'Streams', items: <DashboardItem>[]),
  LibrarySource(id: 'media-apps', label: 'Media', items: <DashboardItem>[]),
  LibrarySource(id: 'utilities', label: 'Utilities', items: <DashboardItem>[]),
];
