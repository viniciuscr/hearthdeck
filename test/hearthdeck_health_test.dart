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
          'last_success_at': null,
          'last_error': 'bridge unavailable',
        },
      ],
    });

    expect(health.providers.single.id, 'desktop-apps');
    expect(health.providers.single.status, 'degraded');
    expect(health.providers.single.lastError, 'bridge unavailable');
  });
}
