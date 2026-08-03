import 'dart:collection';

import 'package:flutter/material.dart';

import '../backend/hearthdeck_api_client.dart';
import '../dashboard_models.dart';
import 'catalog_repository.dart';

class ApiCatalogRepository implements CatalogRepository {
  ApiCatalogRepository(this._apiClient, {this.eventStream});

  final HearthdeckApiClient _apiClient;
  final Stream<HearthdeckServerEvent>? eventStream;

  @override
  Future<HearthdeckHealth> health() => _apiClient.health();

  @override
  Future<HearthdeckDiagnostics> diagnostics() => _apiClient.diagnostics();

  @override
  Future<CatalogData> load() async {
    final items = await _apiClient.library();
    final games = <HearthdeckLibraryItem>[];
    final appsByCategory = <String, List<HearthdeckLibraryItem>>{};
    for (final item in items) {
      if (item.kind == 'game') {
        games.add(item);
      } else {
        appsByCategory
            .putIfAbsent(
              _primaryCategoryFor(item),
              () => <HearthdeckLibraryItem>[],
            )
            .add(item);
      }
    }
    final gameSources = games.isEmpty
        ? const <CatalogSource>[]
        : <CatalogSource>[
            CatalogSource(
              id: 'all-games',
              label: 'All games',
              items: games.map(_toDashboardItem).toList(growable: false),
            ),
          ];
    final appSources =
        SplayTreeMap<String, List<HearthdeckLibraryItem>>.from(appsByCategory)
            .entries
            .map((MapEntry<String, List<HearthdeckLibraryItem>> entry) {
              return CatalogSource(
                id: 'category-${entry.key.toLowerCase().replaceAll(' ', '-')}',
                label: entry.key,
                items: entry.value
                    .map(_toDashboardItem)
                    .toList(growable: false),
              );
            })
            .toList(growable: false);
    return CatalogData(gameSources: gameSources, appSources: appSources);
  }

  @override
  Future<void> launch(DashboardItem item) => _apiClient.launch(item.id);

  @override
  Future<void> requestRescan() => _apiClient.requestRescan();

  @override
  Future<void> requestProviderRefresh(HearthdeckProviderHealth provider) {
    return provider.kind == 'metadata'
        ? _apiClient.requestMetadataRefresh(provider.id)
        : _apiClient.requestDiscoveryRefresh(provider.id);
  }

  @override
  Future<void> restartRommService() => _apiClient.restartRommService();

  @override
  Stream<CatalogEvent> watch() async* {
    await for (final event in eventStream ?? _apiClient.watchEvents()) {
      if (event is HearthdeckLibraryChanged) {
        yield CatalogChanged(
          sourceId: event.sourceId,
          recordCount: event.recordCount,
        );
      } else if (event is HearthdeckMetadataChanged) {
        yield CatalogChanged(
          sourceId: event.providerId,
          recordCount: event.recordCount,
        );
      }
    }
  }

  DashboardItem _toDashboardItem(HearthdeckLibraryItem item) {
    final kind = _kindFor(item.kind);
    final metadata = _metadata(item.metadata);
    return DashboardItem(
      id: item.id,
      title: item.title,
      description: metadata['summary'] as String? ?? item.title,
      icon: _iconFor(kind),
      colors: _colorsFor(item.id),
      artworkUrl: _artworkUrl(item.icon),
      kind: kind,
      details: _detailsFor(item, metadata),
    );
  }

  ContentDetails? _detailsFor(
    HearthdeckLibraryItem item,
    Map<String, dynamic> metadata,
  ) {
    final summary = metadata['summary'] as String?;
    final description = metadata['description'] as String?;
    final developer = metadata['developer'] as String?;
    final license = metadata['project_license'] as String?;
    final store = metadata['store'] as String?;
    final runner = metadata['runner'] as String?;
    final version = metadata['version'] as String?;
    final platform = metadata['platform'] as String?;
    final installSize = metadata['install_size_bytes'];
    final cloudSaves = metadata['cloud_saves'] as bool?;
    final requirements = _requirements(metadata['requirements']);
    final memoryCompatibility =
        metadata['memory_compatibility'] as Map<String, dynamic>?;
    final categories =
        (metadata['categories'] as List<dynamic>? ?? const <dynamic>[])
            .whereType<String>()
            .toList(growable: false);
    final urls =
        (metadata['urls'] as Map<String, dynamic>? ?? const <String, dynamic>{})
            .entries
            .where(
              (MapEntry<String, dynamic> entry) =>
                  entry.value is String && _isHttpUrl(entry.value as String),
            )
            .map(
              (MapEntry<String, dynamic> entry) =>
                  MapEntry(entry.key, entry.value as String),
            )
            .toList(growable: false);

    return ContentDetails(
      summary: description ?? summary ?? item.title,
      actions: <ContentAction>[
        if (item.launchId != null)
          const ContentAction(
            id: 'launch',
            label: 'Launch',
            icon: Icons.open_in_new_rounded,
            isPrimary: true,
          ),
        ...urls.map(
          (MapEntry<String, String> entry) => ContentAction(
            id: 'open-${entry.key}',
            label: _urlLabel(entry.key),
            icon: _urlIcon(entry.key),
            url: entry.value,
          ),
        ),
      ],
      facts: const <ContentFact>[],
      highlights: <ContentFact>[
        if (memoryCompatibility != null)
          _memoryCompatibilityFact(memoryCompatibility),
      ],
      factSections: <ContentFactSection>[
        ContentFactSection(
          title: 'Installed',
          facts: <ContentFact>[
            if (store != null)
              ContentFact(
                label: 'Store',
                value: store,
                icon: Icons.storefront_outlined,
              ),
            if (runner != null)
              ContentFact(
                label: 'Launcher',
                value: runner == 'legendary' ? 'Heroic / Epic' : 'Heroic / GOG',
                icon: Icons.rocket_launch_outlined,
              ),
            if (version != null)
              ContentFact(
                label: 'Version',
                value: version,
                icon: Icons.new_releases_outlined,
              ),
            if (platform != null)
              ContentFact(
                label: 'Platform',
                value: platform,
                icon: Icons.computer_rounded,
              ),
            if (installSize is int)
              ContentFact(
                label: 'Size',
                value: _formatBytes(installSize),
                icon: Icons.storage_rounded,
              ),
            if (cloudSaves != null)
              ContentFact(
                label: 'Cloud saves',
                value: cloudSaves ? 'Supported' : 'Not reported',
                icon: cloudSaves
                    ? Icons.cloud_done_outlined
                    : Icons.cloud_off_outlined,
              ),
          ],
        ),
        if (developer != null || categories.isNotEmpty || license != null)
          ContentFactSection(
            title: 'About',
            facts: <ContentFact>[
              if (developer != null)
                ContentFact(
                  label: 'Developer',
                  value: developer,
                  icon: Icons.business_outlined,
                ),
              if (categories.isNotEmpty)
                ContentFact(
                  label: 'Genres',
                  value: categories.join(', '),
                  icon: Icons.category_outlined,
                ),
              if (license != null)
                ContentFact(
                  label: 'License',
                  value: license,
                  icon: Icons.gavel_outlined,
                ),
            ],
          ),
        if (requirements.isNotEmpty)
          ContentFactSection(
            title: 'Publisher requirements',
            facts: requirements,
          ),
      ],
      galleryTitle: 'Media',
      gallery: const <ContentGalleryItem>[],
    );
  }

  Map<String, dynamic> _metadata(Map<String, dynamic> metadata) {
    return metadata;
  }

  String _primaryCategoryFor(HearthdeckLibraryItem item) {
    final categories =
        (item.metadata['categories'] as List<dynamic>? ?? const <dynamic>[])
            .whereType<String>();
    for (final category in categories) {
      final mapped = _libraryCategory(category);
      if (mapped != null) {
        return mapped;
      }
    }
    return 'Other';
  }

  String? _libraryCategory(String category) => switch (category) {
    'AudioVideo' || 'Audio' || 'Video' => 'Media',
    'Development' || 'IDE' || 'Building' => 'Development',
    'Education' => 'Education',
    'Graphics' || 'Photography' || 'Viewer' => 'Graphics',
    'Network' || 'WebBrowser' || 'Email' || 'Chat' => 'Network',
    'Office' || 'Spreadsheet' || 'WordProcessor' => 'Office',
    'Science' || 'Math' => 'Science',
    'Settings' || 'System' || 'Security' => 'System',
    'Utility' || 'FileTools' || 'TextEditor' => 'Utility',
    _ => null,
  };

  String _categoryLabel(String category) => category
      .replaceAll('-', ' ')
      .split(' ')
      .where((String word) => word.isNotEmpty)
      .map(
        (String word) =>
            '${word[0].toUpperCase()}${word.substring(1).toLowerCase()}',
      )
      .join(' ');

  String _urlLabel(String type) => switch (type) {
    'homepage' => 'Website',
    'bugtracker' => 'Report issue',
    'vcs-browser' => 'Source code',
    'donation' => 'Support project',
    _ => 'Open ${_categoryLabel(type)}',
  };

  IconData _urlIcon(String type) => switch (type) {
    'homepage' => Icons.language_rounded,
    'bugtracker' => Icons.bug_report_outlined,
    'vcs-browser' => Icons.code_rounded,
    'donation' => Icons.volunteer_activism_outlined,
    _ => Icons.open_in_new_rounded,
  };

  bool _isHttpUrl(String value) {
    final uri = Uri.tryParse(value);
    return uri != null &&
        uri.hasAuthority &&
        (uri.scheme == 'http' || uri.scheme == 'https');
  }

  String? _artworkUrl(String? value) =>
      value != null && _isHttpUrl(value) ? value : null;

  String _formatBytes(int bytes) {
    const units = <String>['B', 'KB', 'MB', 'GB', 'TB'];
    var value = bytes.toDouble();
    var unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024;
      unit += 1;
    }
    final precision = value >= 10 || unit == 0 ? 0 : 1;
    return '${value.toStringAsFixed(precision)} ${units[unit]}';
  }

  List<ContentFact> _requirements(Object? value) {
    if (value is! List<dynamic>) {
      return const <ContentFact>[];
    }
    return value
        .whereType<Map<String, dynamic>>()
        .map((requirement) {
          final title = requirement['title'] as String? ?? 'Requirement';
          final minimum = requirement['minimum'] as String?;
          final recommended = requirement['recommended'] as String?;
          final detail = switch ((minimum, recommended)) {
            (final String minimum, final String recommended) =>
              'Min: $minimum\nRecommended: $recommended',
            (final String minimum, null) => 'Min: $minimum',
            (null, final String recommended) => 'Recommended: $recommended',
            _ => '',
          };
          return ContentFact(
            label: title,
            value: detail,
            icon: Icons.tune_rounded,
          );
        })
        .where((fact) => fact.value.isNotEmpty)
        .toList(growable: false);
  }

  ContentFact _memoryCompatibilityFact(Map<String, dynamic> value) {
    final status = value['status'] as String?;
    final memory = value['system_memory_bytes'];
    final label = switch (status) {
      'recommended' => 'Memory: recommended',
      'minimum' => 'Memory: minimum met',
      'below_minimum' => 'Memory: below minimum',
      'below_recommended' => 'Memory: below recommended',
      _ => 'Memory requirements',
    };
    return ContentFact(
      label: label,
      value: memory is int ? '${_formatBytes(memory)} detected' : 'Unavailable',
      icon: switch (status) {
        'recommended' => Icons.verified_rounded,
        'minimum' => Icons.info_outline_rounded,
        _ => Icons.warning_amber_rounded,
      },
    );
  }

  // Single choke point every backend `kind` string passes through. An
  // unrecognized value (a new provider shipping "movie" or "show", say)
  // must not silently masquerade as a known kind, so it's surfaced here
  // instead of only in a UI code review.
  TvContentKind _kindFor(String value) => switch (value) {
    'game' => TvContentKind.game,
    'media' => TvContentKind.media,
    'system' => TvContentKind.system,
    'application' => TvContentKind.application,
    _ => _unrecognizedKind(value),
  };

  TvContentKind _unrecognizedKind(String value) {
    // ponytail: defaults to `application` rather than crashing the catalog
    // load; if this fires for a real provider, give `kind` a shared enum in
    // contracts/openapi.yaml instead of a free string.
    assert(() {
      debugPrint('ApiCatalogRepository: unrecognized catalog kind "$value"');
      return true;
    }());
    return TvContentKind.application;
  }

  IconData _iconFor(TvContentKind kind) => switch (kind) {
    TvContentKind.game => Icons.sports_esports_rounded,
    TvContentKind.media => Icons.play_circle_outline_rounded,
    TvContentKind.system => Icons.tune_rounded,
    TvContentKind.application => Icons.apps_rounded,
  };

  List<Color> _colorsFor(String id) {
    const palette = <List<Color>>[
      <Color>[Color(0xFF225F69), Color(0xFF102B38)],
      <Color>[Color(0xFF3557A0), Color(0xFF17234E)],
      <Color>[Color(0xFF7B3E23), Color(0xFF35190E)],
      <Color>[Color(0xFF713C76), Color(0xFF301631)],
      <Color>[Color(0xFF547B43), Color(0xFF22361B)],
    ];
    final index =
        id.codeUnits.fold<int>(0, (int total, int unit) => total + unit) %
        palette.length;
    return palette[index];
  }
}
