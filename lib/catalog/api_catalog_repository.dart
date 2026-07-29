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
  Future<CatalogData> load() async {
    final items = await _apiClient.library();
    final sourcesById = <String, List<HearthdeckLibraryItem>>{};
    for (final item in items) {
      sourcesById
          .putIfAbsent(item.sourceId, () => <HearthdeckLibraryItem>[])
          .add(item);
    }
    final gameSources = <CatalogSource>[];
    final appSources = <CatalogSource>[];
    for (final entry in sourcesById.entries) {
      final source = CatalogSource(
        id: entry.key,
        label: _sourceLabel(entry.key),
        items: entry.value.map(_toDashboardItem).toList(growable: false),
      );
      if (entry.value.every(
        (HearthdeckLibraryItem item) => item.kind == 'game',
      )) {
        gameSources.add(source);
      } else {
        appSources.add(source);
      }
    }
    return CatalogData(gameSources: gameSources, appSources: appSources);
  }

  @override
  Future<void> launch(DashboardItem item) => _apiClient.launch(item.id);

  @override
  Future<void> requestRescan() => _apiClient.requestRescan();

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
    final enrichment = _enrichment(item.metadata);
    return DashboardItem(
      id: item.id,
      title: item.title,
      description: item.launchId == null
          ? 'Available from Hearthdeck'
          : 'Installed application',
      icon: _iconFor(kind),
      colors: _colorsFor(item.id),
      kind: kind,
      details: _detailsFor(item, enrichment),
    );
  }

  ContentDetails? _detailsFor(
    HearthdeckLibraryItem item,
    Map<String, dynamic>? enrichment,
  ) {
    if (enrichment == null) {
      return _discoveryOnlyDetails(item);
    }
    final summary = enrichment['summary'] as String?;
    final description = enrichment['description'] as String?;
    final developer = enrichment['developer'] as String?;
    final license = enrichment['project_license'] as String?;
    final categories =
        (enrichment['categories'] as List<dynamic>? ?? const <dynamic>[])
            .whereType<String>()
            .toList(growable: false);
    final urls =
        (enrichment['urls'] as Map<String, dynamic>? ??
                const <String, dynamic>{})
            .values
            .whereType<String>()
            .toList(growable: false);

    return ContentDetails(
      summary: description ?? summary ?? item.title,
      actions: <ContentAction>[
        const ContentAction(
          id: 'launch',
          label: 'Launch',
          icon: Icons.open_in_new_rounded,
          isPrimary: true,
        ),
        if (urls.isNotEmpty)
          const ContentAction(
            id: 'official-page',
            label: 'Official page',
            icon: Icons.language_rounded,
          ),
      ],
      facts: <ContentFact>[
        if (developer != null)
          ContentFact(
            label: 'Developer',
            value: developer,
            icon: Icons.business_outlined,
          ),
        if (license != null)
          ContentFact(
            label: 'License',
            value: license,
            icon: Icons.gavel_outlined,
          ),
        if (categories.isNotEmpty)
          ContentFact(
            label: 'Categories',
            value: categories.join(', '),
            icon: Icons.category_outlined,
          ),
        if (urls.isNotEmpty)
          ContentFact(
            label: 'Project links',
            value: '${urls.length} available',
            icon: Icons.link_rounded,
          ),
      ],
      galleryTitle: 'Application details',
      gallery: const <ContentGalleryItem>[],
    );
  }

  ContentDetails _discoveryOnlyDetails(HearthdeckLibraryItem item) {
    return ContentDetails(
      summary: item.launchId == null
          ? 'Discovered by the local Hearthdeck catalog service.'
          : 'Installed application discovered by the local Hearthdeck catalog service.',
      actions: const <ContentAction>[
        ContentAction(
          id: 'launch',
          label: 'Launch',
          icon: Icons.open_in_new_rounded,
          isPrimary: true,
        ),
      ],
      facts: <ContentFact>[
        ContentFact(
          label: 'Source',
          value: _sourceLabel(item.sourceId),
          icon: Icons.inventory_2_outlined,
        ),
        if (item.launchId != null)
          ContentFact(
            label: 'Launch ID',
            value: item.launchId!,
            icon: Icons.terminal_rounded,
          ),
        const ContentFact(
          label: 'Rich metadata',
          value: 'Not available from this source',
          icon: Icons.info_outline_rounded,
        ),
      ],
      galleryTitle: 'Application details',
      gallery: const <ContentGalleryItem>[],
    );
  }

  Map<String, dynamic>? _enrichment(Map<String, dynamic> metadata) {
    final enrichment = metadata['enrichment'];
    return enrichment is Map<String, dynamic> ? enrichment : null;
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

  String _sourceLabel(String sourceId) => sourceId
      .split('-')
      .map(
        (String word) => word.isEmpty
            ? word
            : '${word[0].toUpperCase()}${word.substring(1)}',
      )
      .join(' ');
}
