import 'package:flutter_test/flutter_test.dart';
import 'package:hearthdeck/backend/hearthdeck_endpoint.dart';

void main() {
  test('local endpoint uses loopback HTTP', () {
    final endpoint = HearthdeckEndpoint.local();

    expect(
      endpoint.api('health').toString(),
      'http://127.0.0.1:38400/v1/health',
    );
  });

  test('remote endpoint requires HTTPS', () {
    expect(
      () => HearthdeckEndpoint.parse('http://192.168.1.10:38400'),
      throwsFormatException,
    );
    expect(
      HearthdeckEndpoint.parse(
        'https://hearthdeck.local:38400',
      ).api('library').toString(),
      'https://hearthdeck.local:38400/v1/library',
    );
  });
}
