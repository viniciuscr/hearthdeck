import 'dart:io';

import '../backend/hearthdeck_api_client.dart';
import '../backend/hearthdeck_endpoint.dart';
import '../backend/local_hearthdeck_client.dart';
import 'api_catalog_repository.dart';
import 'catalog_repository.dart';
import 'local_catalog_repository.dart';
import 'mock_catalog_repository.dart';

/// Selects an explicit catalog source without making macOS development depend
/// on a Linux daemon. Use `--dart-define=HEARTHDECK_BACKEND_URL=...` with a
/// `HEARTHDECK_PAIRING_TOKEN` to enable the live API repository.
CatalogRepository createCatalogRepository() {
  const backendUrl = String.fromEnvironment('HEARTHDECK_BACKEND_URL');
  const token = String.fromEnvironment('HEARTHDECK_PAIRING_TOKEN');
  if (backendUrl.isNotEmpty && token.isNotEmpty) {
    return ApiCatalogRepository(
      HearthdeckApiClient(
        endpoint: HearthdeckEndpoint.parse(backendUrl),
        token: token,
      ),
    );
  }
  const useLocalCatalog = bool.fromEnvironment('HEARTHDECK_USE_LOCAL_CATALOG');
  if (useLocalCatalog && Platform.isLinux) {
    return LocalCatalogRepository();
  }
  return const MockCatalogRepository();
}

Future<HearthdeckApiClient?> createRetroApiClient() async {
  const backendUrl = String.fromEnvironment('HEARTHDECK_BACKEND_URL');
  const token = String.fromEnvironment('HEARTHDECK_PAIRING_TOKEN');
  if (backendUrl.isNotEmpty && token.isNotEmpty) {
    return HearthdeckApiClient(
      endpoint: HearthdeckEndpoint.parse(backendUrl),
      token: token,
    );
  }
  const useLocalCatalog = bool.fromEnvironment('HEARTHDECK_USE_LOCAL_CATALOG');
  return useLocalCatalog && Platform.isLinux
      ? createLocalHearthdeckClient()
      : null;
}
