import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:hearthdeck/backend/hearthdeck_api_client.dart';
import 'package:hearthdeck/backend/hearthdeck_endpoint.dart';
import 'package:hearthdeck/catalog/api_catalog_repository.dart';
import 'package:hearthdeck/catalog/catalog_repository.dart';
import 'package:hearthdeck/catalog/mock_catalog_repository.dart';
import 'package:hearthdeck/dashboard_models.dart';

void main() {
  test(
    'mock catalog preserves separate game and application sources',
    () async {
      final CatalogData catalog = await const MockCatalogRepository().load();

      expect(catalog.gameSources, isNotEmpty);
      expect(catalog.appSources, isNotEmpty);
      expect(catalog.gameSources.first.items, isNotEmpty);
      expect(catalog.appSources.first.items, isNotEmpty);
    },
  );

  test('API catalog groups records by backend provider source', () async {
    final client = HearthdeckApiClient(
      endpoint: HearthdeckEndpoint.local(),
      token: 'test-token',
      client: _CatalogHttpClient(),
    );
    final catalog = await ApiCatalogRepository(client).load();

    expect(catalog.gameSources.single.label, 'Steam');
    expect(catalog.appSources.single.label, 'Desktop Apps');
    expect(catalog.gameSources.single.items.single.title, 'Orbit');
    expect(catalog.appSources.single.items.single.title, 'Gallery');
    expect(
      catalog.appSources.single.items.single.details?.facts.any(
        (ContentFact fact) => fact.label == 'Rich metadata',
      ),
      isTrue,
    );
  });

  test('a provider kind the client does not recognize falls back to '
      'application instead of throwing', () async {
    final client = HearthdeckApiClient(
      endpoint: HearthdeckEndpoint.local(),
      token: 'test-token',
      client: _UnrecognizedKindHttpClient(),
    );

    final catalog = await ApiCatalogRepository(client).load();

    final item = catalog.appSources.single.items.single;
    expect(item.kind, TvContentKind.application);
  });

  test('API catalog translates source-aware library events', () async {
    final client = HearthdeckApiClient(
      endpoint: HearthdeckEndpoint.local(),
      token: 'test-token',
      client: _CatalogHttpClient(),
    );
    final repository = ApiCatalogRepository(
      client,
      eventStream: Stream<HearthdeckServerEvent>.value(
        const HearthdeckLibraryChanged(sourceId: 'macos-apps', recordCount: 49),
      ),
    );
    final event = await repository.watch().first;

    expect(event, isA<CatalogChanged>());
    expect((event as CatalogChanged).sourceId, 'macos-apps');
    expect(event.recordCount, 49);
  });

  test('API catalog reloads on metadata provider events', () async {
    final client = HearthdeckApiClient(
      endpoint: HearthdeckEndpoint.local(),
      token: 'test-token',
      client: _CatalogHttpClient(),
    );
    final repository = ApiCatalogRepository(
      client,
      eventStream: Stream<HearthdeckServerEvent>.value(
        const HearthdeckMetadataChanged(
          providerId: 'appstream-local',
          recordCount: 120,
        ),
      ),
    );
    final event = await repository.watch().first as CatalogChanged;

    expect(event.sourceId, 'appstream-local');
    expect(event.recordCount, 120);
  });
}

class _CatalogHttpClient extends http.BaseClient {
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    expect(request.headers['authorization'], 'Bearer test-token');
    const body = '''[
      {"id":"steam:orbit","source_id":"steam","title":"Orbit","kind":"game","launch_id":null,"icon":null,"metadata":{}},
      {"id":"desktop:gallery","source_id":"desktop-apps","title":"Gallery","kind":"application","launch_id":"gallery.desktop","icon":null,"metadata":{}}
    ]''';
    return http.StreamedResponse(
      Stream<List<int>>.value(body.codeUnits),
      200,
      headers: const <String, String>{'content-type': 'application/json'},
    );
  }
}

class _UnrecognizedKindHttpClient extends http.BaseClient {
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    const body = '''[
      {"id":"jellyfin:movie","source_id":"jellyfin","title":"A Movie","kind":"movie","desktop_id":null,"icon":null,"metadata":{}}
    ]''';
    return http.StreamedResponse(
      Stream<List<int>>.value(body.codeUnits),
      200,
      headers: const <String, String>{'content-type': 'application/json'},
    );
  }
}
