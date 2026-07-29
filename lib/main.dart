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

  @override
  Widget build(BuildContext context) {
    return GamepadControl(
      shortcuts: TvGamepadBindings.shortcuts,
      repeatIntents: TvGamepadBindings.repeatIntents,
      child: MaterialApp(
        debugShowCheckedModeBanner: false,
        title: 'Hearthdeck',
        theme: TvTheme.data,
        home: const TvDashboard(),
      ),
    );
  }
}
