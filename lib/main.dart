import 'dart:async';

import 'package:dynamic_color/dynamic_color.dart';
import 'package:flutter/material.dart';
import 'package:flutter_gamepads/flutter_gamepads.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'settings/user_settings_repository.dart';
import 'tv_dashboard.dart';
import 'tv_gamepad.dart';
import 'tv_theme.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final preferences = await SharedPreferences.getInstance();
  final settingsRepository = await createUserSettingsRepository(preferences);
  runApp(HearthdeckApp(settingsRepository: settingsRepository));
}

class HearthdeckApp extends StatefulWidget {
  const HearthdeckApp({
    super.key,
    this.settingsRepository,
    this.initialThemeMode = TvThemeMode.system,
  });

  final UserSettingsRepository? settingsRepository;
  final TvThemeMode initialThemeMode;

  @override
  State<HearthdeckApp> createState() => _HearthdeckAppState();
}

class _HearthdeckAppState extends State<HearthdeckApp> {
  late final UserSettingsRepository _settings =
      widget.settingsRepository ??
      InMemoryUserSettingsRepository(widget.initialThemeMode);
  late var _themeMode = _settings.settings.themeMode;
  late var _backdropMode = _settings.settings.backdropMode;

  static final navigatorKey = GlobalKey<NavigatorState>();
  static final scaffoldMessengerKey = GlobalKey<ScaffoldMessengerState>();

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback(
      (_) => _settings.retryPending(),
    );
  }

  static Future<void> _handleBackIntent() async {
    final navigator = navigatorKey.currentState;
    final didPop = navigator != null && await navigator.maybePop();
    if (!didPop) {
      scaffoldMessengerKey.currentState?.hideCurrentSnackBar();
    }
  }

  void _setThemeMode(TvThemeMode mode) => unawaited(_saveThemeMode(mode));

  void _setBackdropMode(TvBackdropMode mode) =>
      unawaited(_saveBackdropMode(mode));

  Future<void> _saveThemeMode(TvThemeMode mode) async {
    try {
      final settings = await _settings.setThemeMode(mode);
      if (mounted) {
        setState(() {
          _themeMode = settings.themeMode;
          _backdropMode = settings.backdropMode;
        });
      }
    } on Object {
      if (mounted) {
        scaffoldMessengerKey.currentState?.showSnackBar(
          const SnackBar(content: Text('Could not save the color preference.')),
        );
      }
    }
  }

  Future<void> _saveBackdropMode(TvBackdropMode mode) async {
    try {
      final settings = await _settings.setBackdropMode(mode);
      if (mounted) {
        setState(() {
          _themeMode = settings.themeMode;
          _backdropMode = settings.backdropMode;
        });
      }
    } on Object {
      if (mounted) {
        scaffoldMessengerKey.currentState?.showSnackBar(
          const SnackBar(
            content: Text('Could not save the backdrop preference.'),
          ),
        );
      }
    }
  }

  @override
  Widget build(BuildContext context) => DynamicColorBuilder(
    builder: (ColorScheme? lightDynamic, ColorScheme? darkDynamic) {
      final colors = TvTheme.colorsFor(_themeMode, darkDynamic);
      final palette = TvTheme.paletteFor(_themeMode, darkDynamic);
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
          child: TvThemeScope(
            mode: _themeMode,
            onModeChanged: _setThemeMode,
            backdropMode: _backdropMode,
            onBackdropModeChanged: _setBackdropMode,
            child: MaterialApp(
              navigatorKey: navigatorKey,
              scaffoldMessengerKey: scaffoldMessengerKey,
              debugShowCheckedModeBanner: false,
              title: 'Hearthdeck',
              theme: TvTheme.data(colors, palette),
              themeAnimationDuration: TvTheme.focusDuration,
              themeAnimationCurve: TvTheme.focusCurve,
              home: const TvDashboard(),
            ),
          ),
        ),
      );
    },
  );
}
