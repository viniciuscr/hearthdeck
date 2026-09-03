import 'dart:async';

import 'package:flutter/material.dart';

bool _launchInFlight = false;

/// Runs [action] behind a full-screen, un-dismissable loader overlay.
///
/// The overlay covers the entire screen (blocking controller/gamepad input to
/// any other tile or surface), disables back navigation, and guards against
/// re-entry so a rapid double press can never launch the same item twice.
/// The overlay is removed once [action] completes.
///
/// Returns the result of [action], or `null` if a launch is already in flight
/// (the duplicate press is silently ignored).
Future<T?> runWithLaunchLoader<T>(
  BuildContext context, {
  required Future<T> Function() action,
  required String itemTitle,
}) async {
  if (_launchInFlight) {
    return null;
  }
  _launchInFlight = true;

  final navigator = Navigator.of(context);

  final route = PageRouteBuilder<T>(
    opaque: false,
    barrierDismissible: false,
    barrierColor: Colors.black.withValues(alpha: 0.72),
    transitionDuration: const Duration(milliseconds: 150),
    reverseTransitionDuration: const Duration(milliseconds: 150),
    pageBuilder:
        (
          BuildContext context,
          Animation<double> animation,
          Animation<double> secondaryAnimation,
        ) {
          final fade = CurvedAnimation(
            parent: animation,
            curve: Curves.easeOut,
            reverseCurve: Curves.easeIn,
          );
          return PopScope(
            canPop: false,
            child: FadeTransition(
              opacity: fade,
              child: Scaffold(
                backgroundColor: Colors.transparent,
                body: Center(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: <Widget>[
                      const CircularProgressIndicator(),
                      const SizedBox(height: 24),
                      const Text(
                        'Launching…',
                        style: TextStyle(
                          fontSize: 24,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                      const SizedBox(height: 8),
                      Text(itemTitle, textAlign: TextAlign.center),
                    ],
                  ),
                ),
              ),
            ),
          );
        },
  );

  unawaited(navigator.push(route));
  try {
    return await action();
  } finally {
    if (navigator.canPop()) {
      navigator.pop();
    }
    _launchInFlight = false;
  }
}
