import 'dart:io';

import 'package:flutter/services.dart';

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
