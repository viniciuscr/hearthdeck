import 'package:flutter/material.dart';

import '../backend/hearthdeck_api_client.dart';
import '../dashboard_models.dart';
import '../library_models.dart';
import 'catalog_repository.dart';

class MockCatalogRepository implements CatalogRepository {
  const MockCatalogRepository();

  @override
  Future<HearthdeckHealth> health() async => const HearthdeckHealth(
    version: 'Mock catalog',
    lanEnabled: false,
    transport: 'mock',
    providers: <HearthdeckProviderHealth>[],
  );

  @override
  Future<HearthdeckDiagnostics> diagnostics() async =>
      const HearthdeckDiagnostics(
        generatedAt: '2026-01-01T00:00:00Z',
        services: <HearthdeckServiceStatus>[],
        romm: HearthdeckRommDiagnostic(
          configured: false,
          status: 'not_configured',
          baseUrl: null,
          consoleCount: null,
          checkedAt: '2026-01-01T00:00:00Z',
          error: null,
        ),
        logs: HearthdeckLogTail(
          available: false,
          error: 'Diagnostics are available when connected to Hearthdeck.',
          entries: <HearthdeckLogEntry>[],
        ),
      );

  @override
  Future<CatalogData> load() async {
    return CatalogData(
      gameSources: gameLibrarySources
          .where((LibrarySource source) => source.id == 'all-games')
          .map(
            (LibrarySource source) => CatalogSource(
              id: 'pc-games',
              label: 'PC games',
              items: source.items,
            ),
          )
          .toList(growable: false),
      appSources: appLibrarySources
          .map(
            (LibrarySource source) => CatalogSource(
              id: source.id,
              label: source.label,
              items: source.items,
            ),
          )
          .toList(growable: false),
      consoleSources: const <CatalogSource>[
        CatalogSource(
          id: 'romm-consoles',
          label: 'Consoles',
          isConsoleCollection: true,
          items: <DashboardItem>[
            DashboardItem(
              id: 'romm-console-nes',
              title: 'Nintendo Entertainment System',
              description: '341 games in RomM',
              badge: '341 games',
              icon: Icons.videogame_asset_rounded,
              colors: <Color>[Color(0xFF57307E), Color(0xFF201033)],
              kind: TvContentKind.game,
            ),
            DashboardItem(
              id: 'romm-console-snes',
              title: 'Super Nintendo',
              description: '187 games in RomM',
              badge: '187 games',
              icon: Icons.videogame_asset_rounded,
              colors: <Color>[Color(0xFF315E91), Color(0xFF142944)],
              kind: TvContentKind.game,
            ),
          ],
        ),
      ],
    );
  }

  @override
  Future<List<HearthdeckLibraryItem>> libraryItems() async =>
      const <HearthdeckLibraryItem>[];

  @override
  Future<void> updateLibraryClassification({
    required String itemId,
    required String? kind,
  }) async {}

  @override
  Future<void> launch(DashboardItem item) async {}

  @override
  Future<void> requestRescan() async {}

  @override
  Future<void> requestProviderRefresh(
    HearthdeckProviderHealth provider,
  ) async {}

  @override
  Stream<CatalogEvent> watch() => const Stream<CatalogEvent>.empty();
}
