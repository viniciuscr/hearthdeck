import 'hearthdeck_api_client.dart';
import 'hearthdeck_endpoint.dart';

/// Creates an authenticated client for the packaged loopback daemon.
Future<HearthdeckApiClient> createLocalHearthdeckClient() async {
  final adminClient = HearthdeckApiClient(
    endpoint: HearthdeckEndpoint.localAdmin(),
    token: null,
  );
  final pairingCode = await adminClient.createPairingCode();
  adminClient.close();

  final pairingClient = HearthdeckApiClient(
    endpoint: HearthdeckEndpoint.local(),
    token: null,
  );
  final pairing = await pairingClient.completePairing(
    code: pairingCode.code,
    clientName: 'hearthdeck-local-client',
  );
  pairingClient.close();

  return HearthdeckApiClient(
    endpoint: HearthdeckEndpoint.local(),
    token: pairing.token,
  );
}
