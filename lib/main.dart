import 'dart:async';

import 'package:dynamic_color/dynamic_color.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_gamepads/flutter_gamepads.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'frontend_log.dart';
import 'settings/user_settings_repository.dart';
import 'tv_dashboard.dart';
import 'tv_gamepad.dart';
import 'tv_theme.dart';
import 'virtual_keyboard.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  installFrontendLogErrorHooks();
  FrontendLog.instance.info('Hearthdeck UI started');
  final preferences = await SharedPreferences.getInstance();
  final settingsRepository = await createUserSettingsRepository(preferences);
  runApp(HearthdeckApp(settingsRepository: settingsRepository));
}

class HearthdeckApp extends StatefulWidget {
  const HearthdeckApp({
    super.key,
    this.settingsRepository,
    this.initialThemeMode = TvThemeMode.noir,
    this.virtualKeyboard,
  });

  final UserSettingsRepository? settingsRepository;
  final TvThemeMode initialThemeMode;
  final VirtualKeyboard? virtualKeyboard;

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
    // Registered once for the whole app's lifetime. Unlike Focus-tree based
    // key handling (which only sees a key event once it bubbles up from
    // whichever widget currently has focus), HardwareKeyboard handlers fire
    // for every key event regardless of what has focus. This makes "Escape
    // goes back" a true core-system behavior instead of something each screen
    // has to wire up (and can forget to make reachable, e.g. by omitting an
    // autofocus somewhere).
    HardwareKeyboard.instance.addHandler(_handleHardwareKeyEvent);
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_handleHardwareKeyEvent);
    super.dispose();
  }

  static bool _handleHardwareKeyEvent(KeyEvent event) {
    if (event is! KeyDownEvent ||
        event.logicalKey != LogicalKeyboardKey.escape) {
      return false;
    }
    unawaited(_handleBackIntent());
    return true;
  }

  static Future<void> _handleBackIntent() async {
    if (unfocusWritableEditableText()) {
      return;
    }
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
        child: TvThemeScope(
          mode: _themeMode,
          onModeChanged: _setThemeMode,
          backdropMode: _backdropMode,
          onBackdropModeChanged: _setBackdropMode,
          child: VirtualKeyboardFocusObserver(
            virtualKeyboard: widget.virtualKeyboard,
            child: MaterialApp(
              navigatorKey: navigatorKey,
              scaffoldMessengerKey: scaffoldMessengerKey,
              debugShowCheckedModeBanner: false,
              title: 'Hearthdeck',
              theme: TvTheme.data(colors, palette),
              themeAnimationDuration: TvTheme.focusDuration,
              themeAnimationCurve: TvTheme.focusCurve,
              // Escape is handled globally by the HardwareKeyboard listener
              // registered in initState, regardless of what has focus. Drop
              // it from the framework's default Shortcuts map so it can't
              // also reach DismissIntent through the focus tree and pop
              // twice for a single key press.
              shortcuts: <ShortcutActivator, Intent>{
                ...WidgetsApp.defaultShortcuts,
              }..remove(const SingleActivator(LogicalKeyboardKey.escape)),
              builder: (BuildContext context, Widget? child) => Actions(
                actions: <Type, Action<Intent>>{
                  DismissIntent: CallbackAction<DismissIntent>(
                    onInvoke: (DismissIntent intent) {
                      if (!VirtualKeyboardFocusScope.dismissFocusedEditableForBackOf(
                        context,
                      )) {
                        unawaited(_handleBackIntent());
                      }
                      return null;
                    },
                  ),
                  TvBackIntent: CallbackAction<TvBackIntent>(
                    onInvoke: (TvBackIntent intent) {
                      if (!VirtualKeyboardFocusScope.dismissFocusedEditableForBackOf(
                        context,
                      )) {
                        unawaited(_handleBackIntent());
                      }
                      return null;
                    },
                  ),
                  TvDirectionalFocusIntent:
                      CallbackAction<TvDirectionalFocusIntent>(
                        onInvoke: (TvDirectionalFocusIntent intent) {
                          final focusedNode =
                              FocusManager.instance.primaryFocus;
                          final dismissedTextInput =
                              VirtualKeyboardFocusScope.dismissFocusedEditableForNavigationOf(
                                context,
                              );
                          (dismissedTextInput
                                  ? focusedNode
                                  : FocusManager.instance.primaryFocus)
                              ?.focusInDirection(intent.direction);
                          return null;
                        },
                      ),
                },
                child: child ?? const SizedBox.shrink(),
              ),
              home: const TvDashboard(),
            ),
          ),
        ),
      );
    },
  );
}
