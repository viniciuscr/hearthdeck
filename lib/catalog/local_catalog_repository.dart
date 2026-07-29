import '../backend/hearthdeck_api_client.dart';
import '../backend/local_hearthdeck_client.dart';
import '../dashboard_models.dart';
import 'api_catalog_repository.dart';
import 'catalog_repository.dart';

/// Pairs the packaged Linux client with its loopback daemon for this app run.
class LocalCatalogRepository implements CatalogRepository {
  LocalCatalogRepository({Future<ApiCatalogRepository> Function()? create})
    : _create = create ?? _pairWithLocalDaemon;

  final Future<ApiCatalogRepository> Function() _create;
  Future<ApiCatalogRepository>? _repository;

  Future<ApiCatalogRepository> _getRepository() => _repository ??= _create();

  @override
  Future<HearthdeckHealth> health() =>
      _request((ApiCatalogRepository repository) => repository.health());

  @override
  Future<CatalogData> load() =>
      _request((ApiCatalogRepository repository) => repository.load());

  @override
  Future<void> launch(DashboardItem item) =>
      _request((ApiCatalogRepository repository) => repository.launch(item));

  @override
  Future<void> requestRescan() =>
      _request((ApiCatalogRepository repository) => repository.requestRescan());

  @override
  Stream<CatalogEvent> watch() async* {
    try {
      yield* (await _getRepository()).watch();
    } on HearthdeckApiException catch (error) {
      if (error.statusCode != 401) {
        rethrow;
      }
      _repository = null;
      yield* (await _getRepository()).watch();
    }
  }

  Future<T> _request<T>(
    Future<T> Function(ApiCatalogRepository repository) request,
  ) async {
    try {
      return await request(await _getRepository());
    } on HearthdeckApiException catch (error) {
      if (error.statusCode != 401) {
        rethrow;
      }
      _repository = null;
      return request(await _getRepository());
    }
  }

  static Future<ApiCatalogRepository> _pairWithLocalDaemon() async {
    return ApiCatalogRepository(await createLocalHearthdeckClient());
  }
}
