import 'package:flutter_test/flutter_test.dart';
import 'package:hearthdeck/backend/hearthdeck_api_client.dart';

void main() {
  test('health exposes provider lifecycle state', () {
    final health = HearthdeckHealth.fromJson(<String, dynamic>{
      'version': '0.1.0',
      'lan_enabled': false,
      'transport': 'http',
      'providers': <Map<String, dynamic>>[
        <String, dynamic>{
          'id': 'desktop-apps',
          'kind': 'discovery',
          'status': 'degraded',
          'record_count': null,
          'last_attempt_at': '2026-01-01T00:00:00Z',
          'last_success_at': null,
          'last_error': 'bridge unavailable',
        },
      ],
    });

    expect(health.providers.single.id, 'desktop-apps');
    expect(health.providers.single.status, 'degraded');
    expect(health.providers.single.lastAttemptAt, '2026-01-01T00:00:00Z');
    expect(health.providers.single.lastError, 'bridge unavailable');
  });

  test('diagnostics expose services, RomM, and bounded service logs', () {
    final diagnostics = HearthdeckDiagnostics.fromJson(<String, dynamic>{
      'generated_at': '2026-01-01T00:00:00Z',
      'services': <Map<String, dynamic>>[
        <String, dynamic>{
          'id': 'daemon',
          'unit': 'hearthdeck-daemon.service',
          'state': 'active',
          'detail': 'active (running)',
        },
      ],
      'romm': <String, dynamic>{
        'configured': true,
        'status': 'ready',
        'base_url': 'http://127.0.0.1:8080',
        'console_count': 12,
        'checked_at': '2026-01-01T00:00:00Z',
        'error': null,
      },
      'logs': <String, dynamic>{
        'available': true,
        'error': null,
        'entries': <Map<String, dynamic>>[
          <String, dynamic>{
            'timestamp': '2026-01-01T00:00:00Z',
            'service': 'Daemon',
            'level': 'info',
            'message': 'RomM console check completed (console_count=12)',
          },
        ],
      },
    });

    expect(diagnostics.services.single.state, 'active');
    expect(diagnostics.romm.consoleCount, 12);
    expect(diagnostics.logs.entries.single.service, 'Daemon');
  });
}
