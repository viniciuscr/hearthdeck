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

  test(
    'API catalog groups records by content category and exposes metadata',
    () async {
      final client = HearthdeckApiClient(
        endpoint: HearthdeckEndpoint.local(),
        token: 'test-token',
        client: _CatalogHttpClient(),
      );
      final catalog = await ApiCatalogRepository(client).load();

      expect(catalog.gameSources.single.label, 'All games');
      expect(catalog.appSources.single.label, 'Graphics');
      expect(catalog.gameSources.single.items.single.title, 'Orbit');
      expect(catalog.appSources.single.items.single.title, 'Gallery');
      expect(
        catalog.appSources.single.items.single.details?.factSections.any(
          (ContentFactSection section) =>
              section.title == 'About' &&
              section.facts.any(
                (ContentFact fact) =>
                    fact.label == 'Genres' && fact.value == 'Graphics',
              ),
        ),
        isTrue,
      );
      expect(
        catalog.appSources.single.items.single.details?.actions.any(
          (ContentAction action) => action.label == 'Website',
        ),
        isTrue,
      );
    },
  );

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

  test(
    'API catalog exposes Heroic installed-game details and artwork',
    () async {
      final client = HearthdeckApiClient(
        endpoint: HearthdeckEndpoint.local(),
        token: 'test-token',
        client: _HeroicCatalogHttpClient(),
      );

      final item = (await ApiCatalogRepository(
        client,
      ).load()).gameSources.single.items.single;

      expect(item.artworkUrl, 'https://example.org/game.jpg');
      expect(
        item.details?.factSections.any(
          (ContentFactSection section) =>
              section.title == 'Installed' &&
              section.facts.any(
                (ContentFact fact) =>
                    fact.label == 'Store' && fact.value == 'GOG',
              ),
        ),
        isTrue,
      );
      expect(
        item.details?.factSections.any(
          (ContentFactSection section) => section.facts.any(
            (ContentFact fact) =>
                fact.label == 'Size' && fact.value == '1.5 GB',
          ),
        ),
        isTrue,
      );
    },
  );

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

  test(
    'API client creates local pairing codes without a bearer token',
    () async {
      final client = HearthdeckApiClient(
        endpoint: HearthdeckEndpoint.localAdmin(),
        token: null,
        client: _PairingHttpClient(),
      );

      final pairingCode = await client.createPairingCode();

      expect(pairingCode.code, 'ABC123');
    },
  );

  test(
    'API client lists authenticated RomM consoles through Hearthdeck',
    () async {
      final client = HearthdeckApiClient(
        endpoint: HearthdeckEndpoint.local(),
        token: 'test-token',
        client: _RetroHttpClient(),
      );

      final consoles = await client.retroConsoles();

      expect(consoles, hasLength(2));
      expect(consoles.first.displayName, 'Nintendo Entertainment System');
      expect(consoles.first.romCount, 341);
      expect(consoles.last.filesystemSlug, 'snes');
    },
  );

  test(
    'API client saves RomM credentials without receiving them back',
    () async {
      final client = HearthdeckApiClient(
        endpoint: HearthdeckEndpoint.local(),
        token: 'test-token',
        client: _RommSettingsHttpClient(),
      );

      final settings = await client.updateRommSettings(
        baseUrl: 'http://127.0.0.1:8080',
        token: 'rmm_private_token',
      );

      expect(settings.baseUrl, 'http://127.0.0.1:8080');
      expect(settings.configured, isTrue);
    },
  );

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

  test('API client saves settings with an authenticated revision', () async {
    final client = HearthdeckApiClient(
      endpoint: HearthdeckEndpoint.local(),
      token: 'test-token',
      client: _SettingsHttpClient(),
    );

    final settings = await client.updateUserSettings(
      themeMode: 'ember',
      backdropMode: 'solid',
      revision: 4,
    );

    expect(settings.themeMode, 'ember');
    expect(settings.backdropMode, 'solid');
    expect(settings.revision, 5);
  });

  test(
    'API client exposes the current settings on a revision conflict',
    () async {
      final client = HearthdeckApiClient(
        endpoint: HearthdeckEndpoint.local(),
        token: 'test-token',
        client: _SettingsConflictHttpClient(),
      );

      expect(
        () => client.updateUserSettings(
          themeMode: 'ember',
          backdropMode: 'solid',
          revision: 2,
        ),
        throwsA(
          isA<HearthdeckSettingsConflict>().having(
            (HearthdeckSettingsConflict error) => error.settings.revision,
            'revision',
            3,
          ),
        ),
      );
    },
  );
}

class _CatalogHttpClient extends http.BaseClient {
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    expect(request.headers['authorization'], 'Bearer test-token');
    const body = '''[
      {"id":"steam:orbit","source_id":"steam","title":"Orbit","kind":"game","launch_id":null,"icon":null,"metadata":{"summary":"Space exploration","categories":["Adventure"],"urls":{},"provenance":"steam"}},
      {"id":"desktop:gallery","source_id":"desktop-apps","title":"Gallery","kind":"application","launch_id":"gallery.desktop","icon":null,"metadata":{"summary":"Browse photos","categories":["Graphics"],"urls":{"homepage":"https://example.org/gallery"},"provenance":"appstream-local"}}
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
      {"id":"jellyfin:movie","source_id":"jellyfin","title":"A Movie","kind":"movie","launch_id":null,"icon":null,"metadata":{}}
    ]''';
    return http.StreamedResponse(
      Stream<List<int>>.value(body.codeUnits),
      200,
      headers: const <String, String>{'content-type': 'application/json'},
    );
  }
}

class _HeroicCatalogHttpClient extends http.BaseClient {
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    const body = '''[
      {"id":"heroic:gog:1091500","source_id":"heroic","title":"Cyberpunk 2077","kind":"game","launch_id":"gog:1091500","icon":"https://example.org/game.jpg","metadata":{"summary":"Night City","description":"Night City","categories":["RPG"],"store":"GOG","runner":"gog","version":"2.2","platform":"windows","cloud_saves":true,"install_size_bytes":1610612736,"requirements":[],"memory_compatibility":null,"urls":{},"provenance":"heroic"}}
    ]''';
    return http.StreamedResponse(Stream<List<int>>.value(body.codeUnits), 200);
  }
}

class _PairingHttpClient extends http.BaseClient {
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    expect(request.method, 'POST');
    expect(request.url.toString(), 'http://127.0.0.1:38401/v1/pairing');
    expect(request.headers, isNot(contains('authorization')));
    const body = '{"code":"ABC123","expires_at":"2026-01-01T00:00:00Z"}';
    return http.StreamedResponse(
      Stream<List<int>>.value(body.codeUnits),
      200,
      headers: const <String, String>{'content-type': 'application/json'},
    );
  }
}

class _RetroHttpClient extends http.BaseClient {
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    expect(request.method, 'GET');
    expect(request.url.path, '/v1/retro/consoles');
    expect(request.headers['authorization'], 'Bearer test-token');
    const body = '''[
      {"id":1,"name":"Nintendo Entertainment System","display_name":null,"rom_count":341,"slug":"nes","fs_slug":"nes"},
      {"id":2,"name":"Super Nintendo Entertainment System","display_name":"Super Nintendo","rom_count":187,"slug":"snes","fs_slug":"snes"}
    ]''';
    return http.StreamedResponse(Stream<List<int>>.value(body.codeUnits), 200);
  }
}

class _RommSettingsHttpClient extends http.BaseClient {
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    expect(request.method, 'PUT');
    expect(request.url.path, '/v1/retro/settings');
    expect(request.headers['authorization'], 'Bearer test-token');
    expect(
      (request as http.Request).body,
      '{"base_url":"http://127.0.0.1:8080","token":"rmm_private_token"}',
    );
    const body =
        '{"base_url":"http://127.0.0.1:8080","configured":true,"updated_at":"2026-01-01T00:00:00Z"}';
    return http.StreamedResponse(Stream<List<int>>.value(body.codeUnits), 200);
  }
}

class _SettingsHttpClient extends http.BaseClient {
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    expect(request.method, 'PUT');
    expect(request.url.path, '/v1/settings');
    expect(request.headers['authorization'], 'Bearer test-token');
    expect(request.headers['content-type'], 'application/json');
    expect(
      (request as http.Request).body,
      '{"theme_mode":"ember","backdrop_mode":"solid","revision":4}',
    );
    const body =
        '{"theme_mode":"ember","backdrop_mode":"solid","revision":5,"updated_at":"2026-01-01T00:00:00Z"}';
    return http.StreamedResponse(Stream<List<int>>.value(body.codeUnits), 200);
  }
}

class _SettingsConflictHttpClient extends http.BaseClient {
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    const body =
        '{"error":"settings version conflict","settings":{"theme_mode":"indigo","backdrop_mode":"edge_wash","revision":3,"updated_at":"2026-01-01T00:00:00Z"}}';
    return http.StreamedResponse(Stream<List<int>>.value(body.codeUnits), 409);
  }
}
