/// Validated connection details for an Hearthdeck daemon.
///
/// Loopback HTTP is allowed for the local Linux client. Any remote endpoint
/// must use HTTPS so an Android client cannot be configured for plaintext LAN
/// control traffic by accident.
class HearthdeckEndpoint {
  const HearthdeckEndpoint._(this.baseUri);

  factory HearthdeckEndpoint.parse(String value) {
    final uri = Uri.parse(value);
    if (!uri.hasAuthority || (uri.scheme != 'http' && uri.scheme != 'https')) {
      throw const FormatException(
        'Hearthdeck server URL must be an absolute HTTP(S) URL.',
      );
    }
    if (uri.scheme == 'http' && !_isLoopback(uri.host)) {
      throw const FormatException('Remote Hearthdeck servers must use HTTPS.');
    }
    return HearthdeckEndpoint._(uri.replace(path: _normalizedPath(uri.path)));
  }

  factory HearthdeckEndpoint.local() =>
      HearthdeckEndpoint.parse('http://127.0.0.1:38400');

  final Uri baseUri;

  Uri api(String path) {
    final normalizedPath = path.startsWith('/') ? path.substring(1) : path;
    return baseUri.resolve('v1/$normalizedPath');
  }

  static bool _isLoopback(String host) =>
      host == 'localhost' || host == '127.0.0.1' || host == '::1';

  static String _normalizedPath(String path) {
    if (path.isEmpty || path == '/') {
      return '/';
    }
    return path.endsWith('/') ? path : '$path/';
  }
}
