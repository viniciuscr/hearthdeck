import '../backend/hearthdeck_api_client.dart';
import '../dashboard_models.dart';

abstract interface class CatalogRepository {
  Future<HearthdeckHealth> health();

  Future<HearthdeckDiagnostics> diagnostics();

  Future<CatalogData> load();

  Future<void> launch(DashboardItem item);

  Future<void> requestRescan();

  Future<void> requestProviderRefresh(HearthdeckProviderHealth provider);

  /// Restarts the fixed `romm.service` systemd unit, if the user has
  /// installed `deploy/systemd/romm.service.example`. No-op on repositories
  /// that don't back onto a real Hearthdeck daemon.
  Future<void> restartRommService();

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
