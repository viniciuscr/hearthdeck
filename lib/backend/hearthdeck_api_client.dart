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

  Future<void> launch(String itemId) async {
    final response = await _client.post(
      endpoint.api('apps/$itemId/launch'),
      headers: _authorizationHeaders(),
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
  });

  factory HearthdeckHealth.fromJson(Map<String, dynamic> json) =>
      HearthdeckHealth(
        version: json['version'] as String,
        lanEnabled: json['lan_enabled'] as bool,
        transport: json['transport'] as String,
        providers: (json['providers'] as List<dynamic>)
            .cast<Map<String, dynamic>>()
            .map(HearthdeckProviderHealth.fromJson)
            .toList(growable: false),
      );

  final String version;
  final bool lanEnabled;
  final String transport;
  final List<HearthdeckProviderHealth> providers;
}

class HearthdeckProviderHealth {
  const HearthdeckProviderHealth({
    required this.id,
    required this.kind,
    required this.status,
    required this.recordCount,
    required this.lastSuccessAt,
    required this.lastError,
  });

  factory HearthdeckProviderHealth.fromJson(Map<String, dynamic> json) =>
      HearthdeckProviderHealth(
        id: json['id'] as String,
        kind: json['kind'] as String,
        status: json['status'] as String,
        recordCount: json['record_count'] as int?,
        lastSuccessAt: json['last_success_at'] as String?,
        lastError: json['last_error'] as String?,
      );

  final String id;
  final String kind;
  final String status;
  final int? recordCount;
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
      {'type': 'action_completed', 'item_id': String itemId} =>
        HearthdeckActionCompleted(itemId: itemId),
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

class HearthdeckActionCompleted extends HearthdeckServerEvent {
  const HearthdeckActionCompleted({required this.itemId});

  final String itemId;
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
