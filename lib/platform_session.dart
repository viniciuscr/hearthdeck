import 'dart:io';

import 'package:flutter/services.dart';

/// The logged-in OS username, for display (e.g. the dashboard profile
/// summary). Falls back to 'Player' if the environment doesn't expose one.
String currentUsername() {
  final env = Platform.environment;
  final raw = env['USER'] ?? env['USERNAME'] ?? env['LOGNAME'];
  if (raw == null || raw.isEmpty) {
    return 'Player';
  }
  return raw[0].toUpperCase() + raw.substring(1);
}

abstract interface class PlatformSession {
  bool get supportsExitToDesktop;

  Future<void> exitToDesktop();
}

class NativePlatformSession implements PlatformSession {
  const NativePlatformSession();

  static const MethodChannel _channel = MethodChannel(
    'io.github.viniciuscr.hearthdeck/session',
  );

  @override
  bool get supportsExitToDesktop => Platform.isLinux;

  @override
  Future<void> exitToDesktop() async {
    if (!supportsExitToDesktop) {
      throw UnsupportedError('Exit to desktop is available only on Linux.');
    }
    await _channel.invokeMethod<void>('exitToDesktop');
  }
}
