import 'dart:io';

abstract interface class ExternalLink {
  Future<void> open(String url);
}

class NativeExternalLink implements ExternalLink {
  const NativeExternalLink();

  @override
  Future<void> open(String url) async {
    final uri = Uri.tryParse(url);
    if (uri == null ||
        !uri.hasAuthority ||
        (uri.scheme != 'http' && uri.scheme != 'https')) {
      throw ArgumentError.value(url, 'url', 'A valid HTTP(S) URL is required.');
    }
    if (Platform.isLinux) {
      await Process.start('xdg-open', <String>[uri.toString()]);
      return;
    }
    if (Platform.isMacOS) {
      await Process.start('open', <String>[uri.toString()]);
      return;
    }
    throw UnsupportedError('External links are unsupported on this platform.');
  }
}
