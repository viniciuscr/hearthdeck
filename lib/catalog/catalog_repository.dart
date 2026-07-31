import '../backend/hearthdeck_api_client.dart';
import '../dashboard_models.dart';

abstract interface class CatalogRepository {
  Future<HearthdeckHealth> health();

  Future<HearthdeckDiagnostics> diagnostics();

  Future<CatalogData> load();

  Future<List<HearthdeckLibraryItem>> libraryItems();

  Future<void> updateLibraryClassification({
    required String itemId,
    required String? kind,
  });

  Future<void> launch(DashboardItem item);

  Future<void> requestRescan();

  Future<void> requestProviderRefresh(HearthdeckProviderHealth provider);

  Stream<CatalogEvent> watch();
}

class CatalogData {
  const CatalogData({
    required this.gameSources,
    required this.appSources,
    this.consoleSources = const <CatalogSource>[],
  });

  final List<CatalogSource> gameSources;
  final List<CatalogSource> appSources;
  final List<CatalogSource> consoleSources;
}

class CatalogSource {
  const CatalogSource({
    required this.id,
    required this.label,
    required this.items,
    this.isConsoleCollection = false,
  });

  final String id;
  final String label;
  final List<DashboardItem> items;
  final bool isConsoleCollection;
}

sealed class CatalogEvent {
  const CatalogEvent();
}

class CatalogChanged extends CatalogEvent {
  const CatalogChanged({required this.sourceId, required this.recordCount});

  final String sourceId;
  final int recordCount;
}
