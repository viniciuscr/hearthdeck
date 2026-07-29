import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_gamepads/flutter_gamepads.dart';

import 'tv_dashboard.dart';
import 'tv_gamepad.dart';
import 'tv_theme.dart';

void main() {
  runApp(const HearthdeckApp());
}

class HearthdeckApp extends StatelessWidget {
  const HearthdeckApp({super.key});

  static final navigatorKey = GlobalKey<NavigatorState>();
  static final scaffoldMessengerKey = GlobalKey<ScaffoldMessengerState>();

  static Future<void> _handleBackIntent() async {
    final navigator = navigatorKey.currentState;
    final didPop = navigator != null && await navigator.maybePop();
    if (!didPop) {
      scaffoldMessengerKey.currentState?.hideCurrentSnackBar();
    }
  }

  @override
  Widget build(BuildContext context) {
    return GamepadControl(
      shortcuts: TvGamepadBindings.shortcuts,
      repeatIntents: TvGamepadBindings.repeatIntents,
      child: Actions(
        actions: <Type, Action<Intent>>{
          TvBackIntent: CallbackAction<TvBackIntent>(
            onInvoke: (TvBackIntent intent) {
              unawaited(_handleBackIntent());
              return null;
            },
          ),
        },
        child: MaterialApp(
          navigatorKey: navigatorKey,
          scaffoldMessengerKey: scaffoldMessengerKey,
          debugShowCheckedModeBanner: false,
          title: 'Hearthdeck',
          theme: TvTheme.data,
          home: const TvDashboard(),
        ),
      ),
    );
  }
}
