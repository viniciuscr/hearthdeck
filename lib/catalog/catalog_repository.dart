import '../backend/hearthdeck_api_client.dart';
import '../dashboard_models.dart';

abstract interface class CatalogRepository {
  Future<HearthdeckHealth> health();

  Future<CatalogData> load();

  Future<void> launch(DashboardItem item);

  Future<void> requestRescan();

  Stream<CatalogEvent> watch();
}

class CatalogData {
  const CatalogData({required this.gameSources, required this.appSources});

  final List<CatalogSource> gameSources;
  final List<CatalogSource> appSources;
}

class CatalogSource {
  const CatalogSource({
    required this.id,
    required this.label,
    required this.items,
  });

  final String id;
  final String label;
  final List<DashboardItem> items;
}

sealed class CatalogEvent {
  const CatalogEvent();
}

class CatalogChanged extends CatalogEvent {
  const CatalogChanged({required this.sourceId, required this.recordCount});

  final String sourceId;
  final int recordCount;
}
