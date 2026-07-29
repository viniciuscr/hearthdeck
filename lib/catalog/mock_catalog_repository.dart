import '../dashboard_models.dart';
import '../library_models.dart';
import 'catalog_repository.dart';

class MockCatalogRepository implements CatalogRepository {
  const MockCatalogRepository();

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
  Stream<CatalogEvent> watch() => const Stream<CatalogEvent>.empty();
}
