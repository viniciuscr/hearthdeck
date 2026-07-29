import '../backend/hearthdeck_api_client.dart';
import '../backend/hearthdeck_endpoint.dart';
import 'api_catalog_repository.dart';
import 'catalog_repository.dart';
import 'mock_catalog_repository.dart';

/// Selects an explicit catalog source without making macOS development depend
/// on a Linux daemon. Use `--dart-define=HEARTHDECK_BACKEND_URL=...` with an optional
/// `HEARTHDECK_PAIRING_TOKEN` to enable the live API repository.
CatalogRepository createCatalogRepository() {
  const backendUrl = String.fromEnvironment('HEARTHDECK_BACKEND_URL');
  const token = String.fromEnvironment('HEARTHDECK_PAIRING_TOKEN');
  if (backendUrl.isEmpty || token.isEmpty) {
    return const MockCatalogRepository();
  }
  return ApiCatalogRepository(
    HearthdeckApiClient(
      endpoint: HearthdeckEndpoint.parse(backendUrl),
      token: token,
    ),
  );
}
