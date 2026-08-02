import 'dart:collection';

import 'package:flutter/foundation.dart';

/// A single line captured for the health page's "Frontend" log tab.
///
/// Unlike the daemon/bridge/api/romm sources (which come from the host via
/// `/v1/diagnostics`), frontend entries live only in this running app's
/// memory: they exist so the terminal has something real to show for the
/// client itself, not to be a durable log store.
class FrontendLogEntry {
  const FrontendLogEntry({
    required this.timestamp,
    required this.level,
    required this.message,
  });

  final DateTime timestamp;
  final String level;
  final String message;
}

/// In-memory ring buffer of frontend events, exposed as a [ChangeNotifier]
/// so the health page can update its "Frontend" tab live instead of waiting
/// on the 5-second diagnostics poll used for the other sources.
class FrontendLog extends ChangeNotifier {
  FrontendLog._();

  static final FrontendLog instance = FrontendLog._();

  static const int _limit = 200;

  final Queue<FrontendLogEntry> _entries = Queue<FrontendLogEntry>();

  List<FrontendLogEntry> get entries => List.unmodifiable(_entries);

  void info(String message) => _add('info', message);

  void warning(String message) => _add('warning', message);

  void error(String message) => _add('error', message);

  void _add(String level, String message) {
    _entries.addLast(
      FrontendLogEntry(timestamp: DateTime.now(), level: level, message: message),
    );
    while (_entries.length > _limit) {
      _entries.removeFirst();
    }
    notifyListeners();
  }
}

/// Installs global Flutter/Dart error hooks that forward into [FrontendLog].
/// Call once during app startup (see main.dart).
void installFrontendLogErrorHooks() {
  final previousOnError = FlutterError.onError;
  FlutterError.onError = (FlutterErrorDetails details) {
    FrontendLog.instance.error(details.exceptionAsString());
    previousOnError?.call(details);
  };
  final previousDispatcherOnError = PlatformDispatcher.instance.onError;
  PlatformDispatcher.instance.onError = (Object error, StackTrace stack) {
    FrontendLog.instance.error(error.toString());
    return previousDispatcherOnError?.call(error, stack) ?? true;
  };
}
