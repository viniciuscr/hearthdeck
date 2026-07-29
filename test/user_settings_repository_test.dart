import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:hearthdeck/backend/hearthdeck_api_client.dart';
import 'package:hearthdeck/backend/hearthdeck_endpoint.dart';
import 'package:hearthdeck/settings/user_settings_repository.dart';
import 'package:hearthdeck/tv_theme.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  test('loads the versioned cache without opening a daemon client', () async {
    SharedPreferences.setMockInitialValues(<String, Object>{
      'user-settings-v1': jsonEncode(<String, Object?>{
        'theme_mode': 'indigo',
        'backdrop_mode': 'quiet_grid',
        'revision': 7,
      }),
    });
    var clientCreated = false;

    final repository = await CachedUserSettingsRepository.load(
      preferences: await SharedPreferences.getInstance(),
      createClient: () async {
        clientCreated = true;
        throw StateError('the cache read must not connect');
      },
    );

    expect(repository.settings.themeMode, TvThemeMode.indigo);
    expect(repository.settings.backdropMode, TvBackdropMode.quietGrid);
    expect(repository.settings.revision, 7);
    expect(clientCreated, isFalse);
  });

  test('migrates the legacy theme cache without a daemon read', () async {
    SharedPreferences.setMockInitialValues(<String, Object>{
      'theme-mode': 'ember',
    });

    final repository = await CachedUserSettingsRepository.load(
      preferences: await SharedPreferences.getInstance(),
    );

    expect(repository.settings.themeMode, TvThemeMode.ember);
    expect(
      (await SharedPreferences.getInstance()).getString('theme-mode'),
      isNull,
    );
  });

  test(
    'queues an offline change and retries it once at the next launch',
    () async {
      SharedPreferences.setMockInitialValues(<String, Object>{});
      final preferences = await SharedPreferences.getInstance();
      final offline = await CachedUserSettingsRepository.load(
        preferences: preferences,
        createClient: () async => throw const SocketException('offline'),
      );

      await offline.setThemeMode(TvThemeMode.ember);
      await Future<void>.delayed(Duration.zero);
      expect(offline.settings.isPending, isTrue);

      final retryClient = _SettingsClient();
      final restored = await CachedUserSettingsRepository.load(
        preferences: preferences,
        createClient: () async => HearthdeckApiClient(
          endpoint: HearthdeckEndpoint.local(),
          token: 'token',
          client: retryClient,
        ),
      );
      restored.retryPending();
      await retryClient.written;
      await _waitFor(() => !restored.settings.isPending);

      expect(retryClient.requests, 1);
      expect(restored.settings.themeMode, TvThemeMode.ember);
      expect(restored.settings.isPending, isFalse);
      expect(restored.settings.revision, 1);
    },
  );

  test(
    'retries a stale write using the revision supplied by the daemon',
    () async {
      SharedPreferences.setMockInitialValues(<String, Object>{
        'user-settings-v1': jsonEncode(<String, Object?>{
          'theme_mode': 'aurora',
          'backdrop_mode': 'edge_wash',
          'revision': 1,
        }),
      });
      final client = _SettingsClient(conflictFirst: true);
      final repository = await CachedUserSettingsRepository.load(
        preferences: await SharedPreferences.getInstance(),
        createClient: () async => HearthdeckApiClient(
          endpoint: HearthdeckEndpoint.local(),
          token: 'token',
          client: client,
        ),
      );

      await repository.setThemeMode(TvThemeMode.indigo);
      await client.written;
      await _waitFor(() => !repository.settings.isPending);

      expect(client.requests, 2);
      expect(repository.settings.themeMode, TvThemeMode.indigo);
      expect(repository.settings.revision, 3);
      expect(repository.settings.isPending, isFalse);
    },
  );

  test('queues backdrop changes with the same cache-first behavior', () async {
    SharedPreferences.setMockInitialValues(<String, Object>{});
    final client = _SettingsClient();
    final repository = await CachedUserSettingsRepository.load(
      preferences: await SharedPreferences.getInstance(),
      createClient: () async => HearthdeckApiClient(
        endpoint: HearthdeckEndpoint.local(),
        token: 'token',
        client: client,
      ),
    );

    await repository.setBackdropMode(TvBackdropMode.quietGrid);
    await client.written;
    await _waitFor(() => !repository.settings.isPending);

    expect(repository.settings.backdropMode, TvBackdropMode.quietGrid);
    expect(repository.settings.revision, 1);
  });
}

Future<void> _waitFor(bool Function() condition) async {
  for (var attempt = 0; attempt < 20; attempt++) {
    if (condition()) {
      return;
    }
    await Future<void>.delayed(Duration.zero);
  }
  expect(condition(), isTrue, reason: 'timed out waiting for cached settings');
}

class _SettingsClient extends http.BaseClient {
  _SettingsClient({this.conflictFirst = false});

  final bool conflictFirst;
  final Completer<void> _written = Completer<void>();
  int requests = 0;

  Future<void> get written => _written.future;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    requests += 1;
    if (conflictFirst && requests == 1) {
      const body =
          '{"error":"settings version conflict","settings":{"theme_mode":"ember","backdrop_mode":"edge_wash","revision":2,"updated_at":"2026-01-01T00:00:00Z"}}';
      return http.StreamedResponse(
        Stream<List<int>>.value(body.codeUnits),
        409,
      );
    }
    final requestBody =
        jsonDecode((request as http.Request).body) as Map<String, dynamic>;
    final mode = requestBody['theme_mode'];
    final backdropMode = requestBody['backdrop_mode'];
    final revision = conflictFirst ? 3 : 1;
    final body = jsonEncode(<String, Object?>{
      'theme_mode': mode,
      'backdrop_mode': backdropMode,
      'revision': revision,
      'updated_at': '2026-01-01T00:00:00Z',
    });
    if (!_written.isCompleted) {
      _written.complete();
    }
    return http.StreamedResponse(Stream<List<int>>.value(body.codeUnits), 200);
  }
}
