import 'dart:convert';
import 'dart:io';

import 'package:http/http.dart' as http;

import 'hearthdeck_endpoint.dart';

class HearthdeckApiClient {
  HearthdeckApiClient({
    required this.endpoint,
    required this.token,
    http.Client? client,
  }) : _client = client ?? http.Client();

  final HearthdeckEndpoint endpoint;
  final String? token;
  final http.Client _client;

  Future<HearthdeckPairingCode> createPairingCode() async {
    final response = await _client.post(endpoint.api('pairing'));
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
    return HearthdeckPairingCode.fromJson(
      jsonDecode(response.body) as Map<String, dynamic>,
    );
  }

  Future<HearthdeckPairing> completePairing({
    required String code,
    required String clientName,
  }) async {
    final response = await _client.post(
      endpoint.api('pairing/complete'),
      headers: const <String, String>{'Content-Type': 'application/json'},
      body: jsonEncode(<String, String>{
        'code': code,
        'client_name': clientName,
      }),
    );
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
    return HearthdeckPairing.fromJson(
      jsonDecode(response.body) as Map<String, dynamic>,
    );
  }

  Future<HearthdeckHealth> health() async {
    final response = await _client.get(endpoint.api('health'));
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
    return HearthdeckHealth.fromJson(
      jsonDecode(response.body) as Map<String, dynamic>,
    );
  }

  Future<HearthdeckDiagnostics> diagnostics() async {
    final response = await _client.get(
      endpoint.api('diagnostics'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
    return HearthdeckDiagnostics.fromJson(
      jsonDecode(response.body) as Map<String, dynamic>,
    );
  }

  Future<List<HearthdeckLibraryItem>> library() async {
    final response = await _client.get(
      endpoint.api('library'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
    return (jsonDecode(response.body) as List<dynamic>)
        .cast<Map<String, dynamic>>()
        .map(HearthdeckLibraryItem.fromJson)
        .toList(growable: false);
  }

  Future<List<HearthdeckRetroConsole>> retroConsoles() async {
    final response = await _client.get(
      endpoint.api('retro/consoles'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
    return (jsonDecode(response.body) as List<dynamic>)
        .cast<Map<String, dynamic>>()
        .map(HearthdeckRetroConsole.fromJson)
        .toList(growable: false);
  }

  Future<HearthdeckRetroGamePage> retroGames({
    int? platformId,
    String? search,
    int limit = 48,
    int offset = 0,
  }) async {
    final trimmedSearch = search?.trim();
    final response = await _client.get(
      endpoint
          .api('retro/roms')
          .replace(
            queryParameters: <String, String>{
              if (platformId != null) 'platform_id': '$platformId',
              if (trimmedSearch != null && trimmedSearch.isNotEmpty)
                'q': trimmedSearch,
              'limit': '$limit',
              'offset': '$offset',
            },
          ),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
    return HearthdeckRetroGamePage.fromJson(
      jsonDecode(response.body) as Map<String, dynamic>,
    );
  }

  Uri retroAssetUri(String path) => endpoint
      .api('retro/assets')
      .replace(queryParameters: <String, String>{'path': path});

  Map<String, String> get authorizationHeaders => _authorizationHeaders();

  Future<HearthdeckRommSettings?> rommSettings() async {
    final response = await _client.get(
      endpoint.api('retro/settings'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
    if (response.body.trim() == 'null') {
      return null;
    }
    return HearthdeckRommSettings.fromJson(
      jsonDecode(response.body) as Map<String, dynamic>,
    );
  }

  Future<HearthdeckRommSettings> updateRommSettings({
    required String baseUrl,
    required String token,
  }) async {
    final response = await _client.put(
      endpoint.api('retro/settings'),
      headers: <String, String>{
        ..._authorizationHeaders(),
        'Content-Type': 'application/json',
      },
      body: jsonEncode(<String, String>{'base_url': baseUrl, 'token': token}),
    );
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
    return HearthdeckRommSettings.fromJson(
      jsonDecode(response.body) as Map<String, dynamic>,
    );
  }

  Future<void> clearRommSettings() async {
    final response = await _client.delete(
      endpoint.api('retro/settings'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 204) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
  }

  Future<void> launch(String itemId) async {
    final response = await _client.post(
      endpoint.api('apps/$itemId/launch'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
  }

  /// Launches a RomM rom through RetroArch. Not the generic [launch]: RomM
  /// roms are not catalog items, so this takes RomM's own rom ID directly
  /// (see docs/retroarch-integration.md decision 6).
  Future<void> launchRetroRom(int romId) async {
    final response = await _client.post(
      endpoint.api('retro/roms/$romId/launch'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
  }

  /// Restarts the fixed `romm.service` systemd unit (see
  /// deploy/systemd/romm.service), if configured. Not a generic
  /// "restart any unit" call: there is no unit name parameter.
  Future<void> restartRommService() async {
    final response = await _client.post(
      endpoint.api('retro/service/restart'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 202) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
  }

  Future<HearthdeckApplicationSession?> activeApplicationSession() async {
    final response = await _client.get(
      endpoint.api('sessions/active'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
    if (response.body.trim() == 'null') {
      return null;
    }
    return HearthdeckApplicationSession.fromJson(
      jsonDecode(response.body) as Map<String, dynamic>,
    );
  }

  Future<void> stopApplicationSession(String sessionId) async {
    final response = await _client.post(
      endpoint.api('sessions/$sessionId/stop'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 202) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
  }

  Future<void> requestInstall(String itemId) async {
    final response = await _client.post(
      endpoint.api('install-requests'),
      headers: <String, String>{
        ..._authorizationHeaders(),
        'Content-Type': 'application/json',
      },
      body: jsonEncode(<String, String>{'item_id': itemId}),
    );
    if (response.statusCode != 202) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
  }

  Future<void> requestRescan() async {
    final response = await _client.post(
      endpoint.api('library/rescan'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 202) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
  }

  Future<void> requestDiscoveryRefresh(String sourceId) async {
    final response = await _client.post(
      endpoint.api('discovery/$sourceId/refresh'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 202) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
  }

  Future<void> requestMetadataRefresh(String providerId) async {
    final response = await _client.post(
      endpoint.api('metadata/$providerId/refresh'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 202) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
  }

  Future<HearthdeckUserSettings> userSettings() async {
    final response = await _client.get(
      endpoint.api('settings'),
      headers: _authorizationHeaders(),
    );
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
    return HearthdeckUserSettings.fromJson(
      jsonDecode(response.body) as Map<String, dynamic>,
    );
  }

  Future<HearthdeckUserSettings> updateUserSettings({
    required String themeMode,
    required String backdropMode,
    required int? revision,
  }) async {
    final response = await _client.put(
      endpoint.api('settings'),
      headers: <String, String>{
        ..._authorizationHeaders(),
        'Content-Type': 'application/json',
      },
      body: jsonEncode(<String, Object?>{
        'theme_mode': themeMode,
        'backdrop_mode': backdropMode,
        'revision': ?revision,
      }),
    );
    if (response.statusCode == 409) {
      final body = jsonDecode(response.body) as Map<String, dynamic>;
      if (body['settings'] case final Map<String, dynamic> settings) {
        throw HearthdeckSettingsConflict(
          HearthdeckUserSettings.fromJson(settings),
        );
      }
    }
    if (response.statusCode != 200) {
      throw HearthdeckApiException(response.statusCode, response.body);
    }
    return HearthdeckUserSettings.fromJson(
      jsonDecode(response.body) as Map<String, dynamic>,
    );
  }

  Stream<HearthdeckServerEvent> watchEvents() async* {
    final socket = await WebSocket.connect(
      _webSocketUri().toString(),
      headers: _authorizationHeaders(),
    );
    try {
      await for (final message in socket) {
        if (message case String payload) {
          final event = HearthdeckServerEvent.fromJson(
            jsonDecode(payload) as Map<String, dynamic>,
          );
          yield event;
        }
      }
    } finally {
      await socket.close();
    }
  }

  void close() => _client.close();

  Map<String, String> _authorizationHeaders() {
    final token = this.token;
    if (token == null || token.isEmpty) {
      throw StateError(
        'An Hearthdeck pairing token is required for this request.',
      );
    }
    return <String, String>{'Authorization': 'Bearer $token'};
  }

  Uri _webSocketUri() {
    final uri = endpoint.api('events');
    return uri.replace(scheme: uri.scheme == 'https' ? 'wss' : 'ws');
  }
}

class HearthdeckHealth {
  const HearthdeckHealth({
    required this.version,
    required this.lanEnabled,
    required this.transport,
    required this.providers,
    this.capabilities = const HearthdeckHostCapabilities(
      launch: false,
      applicationSessions: false,
      installRequests: false,
      retroLaunch: false,
    ),
  });

  factory HearthdeckHealth.fromJson(Map<String, dynamic> json) =>
      HearthdeckHealth(
        version: json['version'] as String,
        lanEnabled: json['lan_enabled'] as bool,
        transport: json['transport'] as String,
        providers: (json['providers'] as List<dynamic>? ?? const <dynamic>[])
            .cast<Map<String, dynamic>>()
            .map(HearthdeckProviderHealth.fromJson)
            .toList(growable: false),
        capabilities: HearthdeckHostCapabilities.fromJson(
          json['capabilities'] as Map<String, dynamic>? ??
              const <String, dynamic>{},
        ),
      );

  final String version;
  final bool lanEnabled;
  final String transport;
  final List<HearthdeckProviderHealth> providers;
  final HearthdeckHostCapabilities capabilities;
}

class HearthdeckHostCapabilities {
  const HearthdeckHostCapabilities({
    required this.launch,
    required this.applicationSessions,
    required this.installRequests,
    required this.retroLaunch,
  });

  factory HearthdeckHostCapabilities.fromJson(Map<String, dynamic> json) =>
      HearthdeckHostCapabilities(
        launch: json['launch'] as bool? ?? false,
        applicationSessions: json['application_sessions'] as bool? ?? false,
        installRequests: json['install_requests'] as bool? ?? false,
        retroLaunch: json['retro_launch'] as bool? ?? false,
      );

  final bool launch;
  final bool applicationSessions;
  final bool installRequests;
  final bool retroLaunch;
}

class HearthdeckDiagnostics {
  const HearthdeckDiagnostics({
    required this.generatedAt,
    required this.services,
    required this.romm,
    required this.logs,
  });

  factory HearthdeckDiagnostics.fromJson(Map<String, dynamic> json) =>
      HearthdeckDiagnostics(
        generatedAt: json['generated_at'] as String,
        services: (json['services'] as List<dynamic>)
            .cast<Map<String, dynamic>>()
            .map(HearthdeckServiceStatus.fromJson)
            .toList(growable: false),
        romm: HearthdeckRommDiagnostic.fromJson(
          json['romm'] as Map<String, dynamic>,
        ),
        logs: HearthdeckLogTail.fromJson(json['logs'] as Map<String, dynamic>),
      );

  final String generatedAt;
  final List<HearthdeckServiceStatus> services;
  final HearthdeckRommDiagnostic romm;
  final HearthdeckLogTail logs;
}

class HearthdeckServiceStatus {
  const HearthdeckServiceStatus({
    required this.id,
    required this.unit,
    required this.state,
    required this.detail,
  });

  factory HearthdeckServiceStatus.fromJson(Map<String, dynamic> json) =>
      HearthdeckServiceStatus(
        id: json['id'] as String,
        unit: json['unit'] as String,
        state: json['state'] as String,
        detail: json['detail'] as String,
      );

  final String id;
  final String unit;
  final String state;
  final String detail;
}

class HearthdeckRommDiagnostic {
  const HearthdeckRommDiagnostic({
    required this.configured,
    required this.status,
    required this.baseUrl,
    required this.consoleCount,
    required this.checkedAt,
    required this.error,
  });

  factory HearthdeckRommDiagnostic.fromJson(Map<String, dynamic> json) =>
      HearthdeckRommDiagnostic(
        configured: json['configured'] as bool,
        status: json['status'] as String,
        baseUrl: json['base_url'] as String?,
        consoleCount: json['console_count'] as int?,
        checkedAt: json['checked_at'] as String,
        error: json['error'] as String?,
      );

  final bool configured;
  final String status;
  final String? baseUrl;
  final int? consoleCount;
  final String checkedAt;
  final String? error;
}

class HearthdeckLogTail {
  const HearthdeckLogTail({
    required this.available,
    required this.error,
    required this.entries,
  });

  factory HearthdeckLogTail.fromJson(Map<String, dynamic> json) =>
      HearthdeckLogTail(
        available: json['available'] as bool,
        error: json['error'] as String?,
        entries: (json['entries'] as List<dynamic>)
            .cast<Map<String, dynamic>>()
            .map(HearthdeckLogEntry.fromJson)
            .toList(growable: false),
      );

  final bool available;
  final String? error;
  final List<HearthdeckLogEntry> entries;
}

/// [source] is a stable id (`daemon`, `api`, `bridge`, `romm`) used to group
/// entries into the health page's log source tabs; it is not meant for
/// direct display (see `_logSourceLabel` in system_health.dart).
class HearthdeckLogEntry {
  const HearthdeckLogEntry({
    required this.timestamp,
    required this.source,
    required this.level,
    required this.message,
  });

  factory HearthdeckLogEntry.fromJson(Map<String, dynamic> json) =>
      HearthdeckLogEntry(
        timestamp: json['timestamp'] as String?,
        source: json['source'] as String,
        level: json['level'] as String,
        message: json['message'] as String,
      );

  final String? timestamp;
  final String source;
  final String level;
  final String message;
}

class HearthdeckApplicationSession {
  const HearthdeckApplicationSession({
    required this.id,
    required this.sourceId,
    required this.applicationId,
    required this.state,
  });

  factory HearthdeckApplicationSession.fromJson(Map<String, dynamic> json) =>
      HearthdeckApplicationSession(
        id: json['id'] as String,
        sourceId: json['source_id'] as String,
        applicationId: json['application_id'] as String,
        state: json['state'] as String,
      );

  final String id;
  final String sourceId;
  final String applicationId;
  final String state;
}

class HearthdeckProviderHealth {
  const HearthdeckProviderHealth({
    required this.id,
    required this.kind,
    required this.status,
    required this.recordCount,
    required this.lastAttemptAt,
    required this.lastSuccessAt,
    required this.lastError,
  });

  factory HearthdeckProviderHealth.fromJson(Map<String, dynamic> json) =>
      HearthdeckProviderHealth(
        id: json['id'] as String,
        kind: json['kind'] as String,
        status: json['status'] as String,
        recordCount: json['record_count'] as int?,
        lastAttemptAt: json['last_attempt_at'] as String?,
        lastSuccessAt: json['last_success_at'] as String?,
        lastError: json['last_error'] as String?,
      );

  final String id;
  final String kind;
  final String status;
  final int? recordCount;
  final String? lastAttemptAt;
  final String? lastSuccessAt;
  final String? lastError;
}

class HearthdeckPairing {
  const HearthdeckPairing({required this.clientId, required this.token});

  factory HearthdeckPairing.fromJson(Map<String, dynamic> json) =>
      HearthdeckPairing(
        clientId: json['client_id'] as String,
        token: json['token'] as String,
      );

  final String clientId;
  final String token;
}

class HearthdeckPairingCode {
  const HearthdeckPairingCode({required this.code});

  factory HearthdeckPairingCode.fromJson(Map<String, dynamic> json) =>
      HearthdeckPairingCode(code: json['code'] as String);

  final String code;
}

class HearthdeckUserSettings {
  const HearthdeckUserSettings({
    required this.themeMode,
    required this.backdropMode,
    required this.revision,
    required this.updatedAt,
  });

  factory HearthdeckUserSettings.fromJson(Map<String, dynamic> json) =>
      HearthdeckUserSettings(
        themeMode: json['theme_mode'] as String,
        backdropMode: json['backdrop_mode'] as String,
        revision: json['revision'] as int,
        updatedAt: json['updated_at'] as String,
      );

  final String themeMode;
  final String backdropMode;
  final int revision;
  final String updatedAt;
}

class HearthdeckSettingsConflict implements Exception {
  const HearthdeckSettingsConflict(this.settings);

  final HearthdeckUserSettings settings;
}

class HearthdeckLibraryItem {
  const HearthdeckLibraryItem({
    required this.id,
    required this.sourceId,
    required this.title,
    required this.kind,
    required this.metadata,
    this.launchId,
    this.icon,
  });

  factory HearthdeckLibraryItem.fromJson(Map<String, dynamic> json) =>
      HearthdeckLibraryItem(
        id: json['id'] as String,
        sourceId: json['source_id'] as String,
        title: json['title'] as String,
        kind: json['kind'] as String,
        launchId: json['launch_id'] as String?,
        icon: json['icon'] as String?,
        metadata: json['metadata'] as Map<String, dynamic>,
      );

  final String id;
  final String sourceId;
  final String title;
  final String kind;
  final String? launchId;
  final String? icon;
  final Map<String, dynamic> metadata;
}

class HearthdeckRetroConsole {
  const HearthdeckRetroConsole({
    required this.id,
    required this.name,
    required this.displayName,
    required this.romCount,
    this.slug,
    this.filesystemSlug,
  });

  factory HearthdeckRetroConsole.fromJson(Map<String, dynamic> json) =>
      HearthdeckRetroConsole(
        id: json['id'] as int,
        name: json['name'] as String,
        displayName: json['display_name'] as String? ?? json['name'] as String,
        romCount: json['rom_count'] as int,
        slug: json['slug'] as String?,
        filesystemSlug: json['fs_slug'] as String?,
      );

  final int id;
  final String name;
  final String displayName;
  final int romCount;
  final String? slug;
  final String? filesystemSlug;
}

class HearthdeckRetroGamePage {
  const HearthdeckRetroGamePage({
    required this.items,
    required this.total,
    required this.limit,
    required this.offset,
  });

  factory HearthdeckRetroGamePage.fromJson(Map<String, dynamic> json) =>
      HearthdeckRetroGamePage(
        items: (json['items'] as List<dynamic>? ?? const <dynamic>[])
            .cast<Map<String, dynamic>>()
            .map(HearthdeckRetroGame.fromJson)
            .toList(growable: false),
        total: json['total'] as int,
        limit: json['limit'] as int,
        offset: json['offset'] as int,
      );

  final List<HearthdeckRetroGame> items;
  final int total;
  final int limit;
  final int offset;
}

class HearthdeckRetroGame {
  const HearthdeckRetroGame({
    required this.id,
    required this.platformId,
    required this.title,
    required this.screenshotPaths,
    required this.hasManual,
    required this.genres,
    required this.regions,
    this.summary,
    this.coverPath,
    this.coverUrl,
    this.playerCount,
    this.releaseYear,
  });

  factory HearthdeckRetroGame.fromJson(Map<String, dynamic> json) =>
      HearthdeckRetroGame(
        id: json['id'] as int,
        platformId: json['platform_id'] as int,
        title: json['title'] as String,
        summary: json['summary'] as String?,
        coverPath: json['cover_path'] as String?,
        coverUrl: json['cover_url'] as String?,
        screenshotPaths:
            (json['screenshot_paths'] as List<dynamic>? ?? const <dynamic>[])
                .whereType<String>()
                .toList(growable: false),
        hasManual: json['has_manual'] as bool? ?? false,
        genres: (json['genres'] as List<dynamic>? ?? const <dynamic>[])
            .whereType<String>()
            .toList(growable: false),
        playerCount: json['player_count'] as String?,
        releaseYear: json['release_year'] as int?,
        regions: (json['regions'] as List<dynamic>? ?? const <dynamic>[])
            .whereType<String>()
            .toList(growable: false),
      );

  final int id;
  final int platformId;
  final String title;
  final String? summary;
  final String? coverPath;
  final String? coverUrl;
  final List<String> screenshotPaths;
  final bool hasManual;
  final List<String> genres;
  final String? playerCount;
  final int? releaseYear;
  final List<String> regions;
}

class HearthdeckRommSettings {
  const HearthdeckRommSettings({
    required this.baseUrl,
    required this.configured,
    required this.updatedAt,
  });

  factory HearthdeckRommSettings.fromJson(Map<String, dynamic> json) =>
      HearthdeckRommSettings(
        baseUrl: json['base_url'] as String,
        configured: json['configured'] as bool,
        updatedAt: json['updated_at'] as String,
      );

  final String baseUrl;
  final bool configured;
  final String updatedAt;
}

sealed class HearthdeckServerEvent {
  const HearthdeckServerEvent();

  factory HearthdeckServerEvent.fromJson(Map<String, dynamic> json) {
    return switch (json) {
      {
        'type': 'library_changed',
        'source_id': String sourceId,
        'record_count': int recordCount,
      } =>
        HearthdeckLibraryChanged(sourceId: sourceId, recordCount: recordCount),
      {
        'type': 'metadata_changed',
        'provider_id': String providerId,
        'record_count': int recordCount,
      } =>
        HearthdeckMetadataChanged(
          providerId: providerId,
          recordCount: recordCount,
        ),
      {'type': 'install_requested', 'item_id': String itemId} =>
        HearthdeckInstallRequested(itemId: itemId),
      {
        'type': 'application_session_changed',
        'session': final Map<String, dynamic>? session,
      } =>
        HearthdeckApplicationSessionChanged(
          session: session == null
              ? null
              : HearthdeckApplicationSession.fromJson(session),
        ),
      _ => const HearthdeckUnknownEvent(),
    };
  }
}

class HearthdeckLibraryChanged extends HearthdeckServerEvent {
  const HearthdeckLibraryChanged({
    required this.sourceId,
    required this.recordCount,
  });

  final String sourceId;
  final int recordCount;
}

class HearthdeckInstallRequested extends HearthdeckServerEvent {
  const HearthdeckInstallRequested({required this.itemId});

  final String itemId;
}

class HearthdeckApplicationSessionChanged extends HearthdeckServerEvent {
  const HearthdeckApplicationSessionChanged({required this.session});

  final HearthdeckApplicationSession? session;
}

class HearthdeckMetadataChanged extends HearthdeckServerEvent {
  const HearthdeckMetadataChanged({
    required this.providerId,
    required this.recordCount,
  });

  final String providerId;
  final int recordCount;
}

class HearthdeckUnknownEvent extends HearthdeckServerEvent {
  const HearthdeckUnknownEvent();
}

class HearthdeckApiException implements Exception {
  const HearthdeckApiException(this.statusCode, this.body);

  final int statusCode;
  final String body;

  @override
  String toString() => 'Hearthdeck API request failed ($statusCode): $body';
}
