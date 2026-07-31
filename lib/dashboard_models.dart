import 'package:flutter/material.dart';

enum TvTileShape { square, landscape }

enum TvContentKind { game, media, application, system }

class ContentAction {
  const ContentAction({
    required this.id,
    required this.label,
    required this.icon,
    this.isPrimary = false,
    this.url,
  });

  final String id;
  final String label;
  final IconData icon;
  final bool isPrimary;
  final String? url;
}

class ContentFact {
  const ContentFact({
    required this.label,
    required this.value,
    required this.icon,
  });

  final String label;
  final String value;
  final IconData icon;
}

class ContentFactSection {
  const ContentFactSection({required this.title, required this.facts});

  final String title;
  final List<ContentFact> facts;
}

class ContentProgress {
  const ContentProgress({
    required this.label,
    required this.value,
    required this.summary,
  });

  final String label;
  final double value;
  final String summary;
}

class ContentGalleryItem {
  const ContentGalleryItem({
    required this.label,
    required this.icon,
    required this.colors,
  });

  final String label;
  final IconData icon;
  final List<Color> colors;
}

class ContentDetails {
  const ContentDetails({
    required this.summary,
    required this.actions,
    required this.facts,
    required this.gallery,
    required this.galleryTitle,
    this.factSections = const <ContentFactSection>[],
    this.highlights = const <ContentFact>[],
    this.progress,
  });

  final String summary;
  final List<ContentAction> actions;
  final List<ContentFact> facts;
  final List<ContentFactSection> factSections;
  final List<ContentFact> highlights;
  final ContentProgress? progress;
  final String galleryTitle;
  final List<ContentGalleryItem> gallery;
}

class DashboardItem {
  const DashboardItem({
    required this.id,
    required this.title,
    required this.icon,
    required this.colors,
    this.badge,
    this.description,
    this.artworkUrl,
    this.artworkHeaders,
    this.kind = TvContentKind.application,
    this.details,
  });

  final String id;
  final String title;
  final IconData icon;
  final List<Color> colors;
  final String? badge;
  final String? description;
  final String? artworkUrl;
  final Map<String, String>? artworkHeaders;
  final TvContentKind kind;
  final ContentDetails? details;
}

class DashboardSection {
  const DashboardSection({
    required this.id,
    required this.items,
    this.title,
    this.shape = TvTileShape.landscape,
  });

  final String id;
  final String? title;
  final TvTileShape shape;
  final List<DashboardItem> items;
}

const List<DashboardSection> dashboardSections = <DashboardSection>[
  DashboardSection(
    id: 'pinned',
    shape: TvTileShape.square,
    items: <DashboardItem>[
      DashboardItem(
        id: 'orbit',
        title: 'Orbit',
        icon: Icons.play_circle_outline_rounded,
        colors: <Color>[Color(0xFF143C5B), Color(0xFF071A2B)],
        badge: 'Continue',
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'stream',
        title: 'Stream',
        icon: Icons.live_tv_rounded,
        colors: <Color>[Color(0xFF632F71), Color(0xFF25133D)],
        kind: TvContentKind.media,
      ),
      DashboardItem(
        id: 'arcade',
        title: 'Arcade',
        icon: Icons.sports_esports_rounded,
        colors: <Color>[Color(0xFF0E5D51), Color(0xFF07342F)],
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'cinema',
        title: 'Cinema',
        icon: Icons.movie_filter_outlined,
        colors: <Color>[Color(0xFF7B3E23), Color(0xFF35190E)],
        kind: TvContentKind.media,
      ),
      DashboardItem(
        id: 'music',
        title: 'Music',
        icon: Icons.graphic_eq_rounded,
        colors: <Color>[Color(0xFF3557A0), Color(0xFF17234E)],
        kind: TvContentKind.media,
      ),
      DashboardItem(
        id: 'retro',
        title: 'Console games',
        icon: Icons.videogame_asset_rounded,
        colors: <Color>[Color(0xFF883A55), Color(0xFF3E1425)],
        kind: TvContentKind.game,
      ),
      DashboardItem(
        id: 'gallery',
        title: 'Gallery',
        icon: Icons.photo_library_outlined,
        colors: <Color>[Color(0xFF397971), Color(0xFF123D3D)],
      ),
      DashboardItem(
        id: 'browser',
        title: 'Browser',
        icon: Icons.language_rounded,
        colors: <Color>[Color(0xFF5850A4), Color(0xFF282454)],
      ),
      DashboardItem(
        id: 'settings',
        title: 'Settings',
        icon: Icons.tune_rounded,
        colors: <Color>[Color(0xFF4A5560), Color(0xFF1F272F)],
        kind: TvContentKind.system,
      ),
    ],
  ),
  DashboardSection(
    id: 'discover',
    title: 'Discover something new',
    items: <DashboardItem>[
      DashboardItem(
        id: 'library',
        title: 'Explore your library',
        description: 'All apps and games',
        icon: Icons.grid_view_rounded,
        colors: <Color>[Color(0xFF225F69), Color(0xFF102B38)],
      ),
      DashboardItem(
        id: 'ambient',
        title: 'Ambient mode',
        description: 'Make your TV your own',
        icon: Icons.wallpaper_rounded,
        colors: <Color>[Color(0xFF204975), Color(0xFF111D42)],
      ),
      DashboardItem(
        id: 'weekend',
        title: 'Weekend picks',
        description: 'Games for the whole room',
        icon: Icons.auto_awesome_rounded,
        colors: <Color>[Color(0xFF866328), Color(0xFF3E2911)],
        badge: 'Featured',
      ),
      DashboardItem(
        id: 'controller',
        title: 'Pair a controller',
        description: 'Play your way',
        icon: Icons.gamepad_rounded,
        colors: <Color>[Color(0xFF404C5A), Color(0xFF151B22)],
      ),
    ],
  ),
  DashboardSection(
    id: 'popular',
    title: 'Popular tonight',
    items: <DashboardItem>[
      DashboardItem(
        id: 'watchlist',
        title: 'Your watchlist',
        description: '12 unwatched episodes',
        icon: Icons.bookmark_added_outlined,
        colors: <Color>[Color(0xFF7D345A), Color(0xFF3B1930)],
      ),
      DashboardItem(
        id: 'cloud',
        title: 'Cloud play',
        description: 'No download required',
        icon: Icons.cloud_queue_rounded,
        colors: <Color>[Color(0xFF336C95), Color(0xFF193149)],
      ),
      DashboardItem(
        id: 'family',
        title: 'Family room',
        description: 'Four local players',
        icon: Icons.groups_rounded,
        colors: <Color>[Color(0xFF597342), Color(0xFF26381C)],
      ),
      DashboardItem(
        id: 'store',
        title: 'Store',
        description: 'New releases and offers',
        icon: Icons.storefront_outlined,
        colors: <Color>[Color(0xFF824D26), Color(0xFF3D210E)],
      ),
    ],
  ),
];

ContentDetails contentDetailsFor(DashboardItem item) {
  if (item.details case final ContentDetails details) {
    return details;
  }

  final description = item.description ?? 'Ready when you are.';
  final primaryAction = switch (item.kind) {
    TvContentKind.game => const ContentAction(
      id: 'play',
      label: 'Play',
      icon: Icons.play_arrow_rounded,
      isPrimary: true,
    ),
    TvContentKind.media => const ContentAction(
      id: 'open',
      label: 'Open',
      icon: Icons.play_arrow_rounded,
      isPrimary: true,
    ),
    TvContentKind.application => const ContentAction(
      id: 'launch',
      label: 'Launch',
      icon: Icons.open_in_new_rounded,
      isPrimary: true,
    ),
    TvContentKind.system => const ContentAction(
      id: 'open',
      label: 'Open settings',
      icon: Icons.tune_rounded,
      isPrimary: true,
    ),
  };
  final secondaryAction = switch (item.kind) {
    TvContentKind.game => const ContentAction(
      id: 'manage',
      label: 'Manage game',
      icon: Icons.settings_outlined,
    ),
    TvContentKind.media => const ContentAction(
      id: 'watchlist',
      label: 'Add to list',
      icon: Icons.bookmark_add_outlined,
    ),
    TvContentKind.application => const ContentAction(
      id: 'options',
      label: 'App options',
      icon: Icons.more_horiz_rounded,
    ),
    TvContentKind.system => const ContentAction(
      id: 'shortcut',
      label: 'Quick access',
      icon: Icons.bolt_rounded,
    ),
  };

  return ContentDetails(
    summary: description,
    actions: <ContentAction>[primaryAction, secondaryAction],
    facts: switch (item.kind) {
      TvContentKind.game => const <ContentFact>[
        ContentFact(
          label: 'Time played',
          value: '24h 18m',
          icon: Icons.timer_outlined,
        ),
        ContentFact(
          label: 'Achievements',
          value: '31 / 48',
          icon: Icons.emoji_events_outlined,
        ),
        ContentFact(
          label: 'Cloud save',
          value: 'Synced',
          icon: Icons.cloud_done_outlined,
        ),
      ],
      TvContentKind.media => const <ContentFact>[
        ContentFact(
          label: 'Watching',
          value: 'Continue',
          icon: Icons.play_circle_outline_rounded,
        ),
        ContentFact(
          label: 'Quality',
          value: '4K ready',
          icon: Icons.high_quality_outlined,
        ),
        ContentFact(
          label: 'Audio',
          value: '5.1 surround',
          icon: Icons.surround_sound_outlined,
        ),
      ],
      TvContentKind.application => const <ContentFact>[
        ContentFact(
          label: 'Status',
          value: 'Installed',
          icon: Icons.download_done_outlined,
        ),
        ContentFact(
          label: 'Storage',
          value: '1.8 GB',
          icon: Icons.storage_outlined,
        ),
        ContentFact(
          label: 'Updated',
          value: 'Today',
          icon: Icons.update_rounded,
        ),
      ],
      TvContentKind.system => const <ContentFact>[
        ContentFact(
          label: 'Status',
          value: 'Ready',
          icon: Icons.check_circle_outline_rounded,
        ),
        ContentFact(
          label: 'Devices',
          value: '3 connected',
          icon: Icons.devices_other_outlined,
        ),
        ContentFact(
          label: 'Profile',
          value: 'Alex Morgan',
          icon: Icons.person_outline_rounded,
        ),
      ],
    },
    progress: item.kind == TvContentKind.game
        ? const ContentProgress(
            label: 'Campaign progress',
            value: 0.73,
            summary: '73% complete',
          )
        : null,
    galleryTitle: switch (item.kind) {
      TvContentKind.game => 'Recent captures',
      TvContentKind.media => 'More to explore',
      TvContentKind.application => 'App previews',
      TvContentKind.system => 'Quick tools',
    },
    gallery: <ContentGalleryItem>[
      ContentGalleryItem(
        label: '${item.title} preview one',
        icon: Icons.landscape_outlined,
        colors: item.colors,
      ),
      ContentGalleryItem(
        label: '${item.title} preview two',
        icon: Icons.auto_awesome_rounded,
        colors: <Color>[item.colors.last, item.colors.first],
      ),
      ContentGalleryItem(
        label: '${item.title} preview three',
        icon: Icons.photo_camera_outlined,
        colors: <Color>[
          item.colors.first.withValues(alpha: 0.8),
          item.colors.last,
        ],
      ),
      ContentGalleryItem(
        label: '${item.title} preview four',
        icon: Icons.collections_outlined,
        colors: <Color>[
          item.colors.last,
          item.colors.first.withValues(alpha: 0.8),
        ],
      ),
    ],
  );
}
