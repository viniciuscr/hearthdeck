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
          .map(
            (LibrarySource source) => CatalogSource(
              id: source.id,
              label: source.label,
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
    );
  }

  @override
  Future<void> launch(DashboardItem item) async {}

  @override
  Future<void> requestRescan() async {}

  @override
  Future<void> requestProviderRefresh(
    HearthdeckProviderHealth provider,
  ) async {}

  @override
  Future<void> restartRommService() async {}

  @override
  Stream<CatalogEvent> watch() => const Stream<CatalogEvent>.empty();
}
