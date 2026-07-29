import 'package:flutter/material.dart';

enum TvThemeMode { system, aurora, ember, indigo }

extension TvThemeModeLabel on TvThemeMode {
  String get label => switch (this) {
    TvThemeMode.system => 'System colors',
    TvThemeMode.aurora => 'Aurora',
    TvThemeMode.ember => 'Ember',
    TvThemeMode.indigo => 'Indigo',
  };
}

abstract final class TvTheme {
  // Legacy fallbacks keep non-migrated artwork and fixture surfaces stable.
  static const Color canvas = Color(0xFF071017);
  static const Color surface = Color(0xFF101C25);
  static const Color primaryAction = Color(0xFF347C28);
  static const Color focus = Color(0xFF7BE443);
  static const Color primaryText = Color(0xFFF4F7F9);
  static const Color secondaryText = Color(0xFFB5C1C9);
  static const Duration focusDuration = Duration(milliseconds: 140);
  static const Curve focusCurve = Curves.easeOutCubic;

  static ThemeData data(ColorScheme colors) {
    final palette = TvPalette(
      canvas: colors.surface,
      surface: colors.surfaceContainerHigh,
      surfaceMuted: colors.surfaceContainer,
      focus: colors.primary,
      primaryAction: colors.primary,
      primaryText: colors.onSurface,
      secondaryText: colors.onSurfaceVariant,
      warning: colors.tertiary,
      info: colors.secondary,
      backdropGlow: Color.lerp(colors.primary, colors.surface, 0.62)!,
    );
    return ThemeData(
      useMaterial3: true,
      brightness: Brightness.dark,
      colorScheme: colors,
      scaffoldBackgroundColor: palette.canvas,
      extensions: <ThemeExtension<dynamic>>[palette],
      textTheme: TextTheme(
        bodyMedium: TextStyle(color: palette.primaryText),
        bodySmall: TextStyle(color: palette.secondaryText),
        titleMedium: TextStyle(
          color: palette.primaryText,
          fontWeight: FontWeight.w600,
        ),
        titleLarge: TextStyle(
          color: palette.primaryText,
          fontWeight: FontWeight.w700,
        ),
      ),
      snackBarTheme: SnackBarThemeData(
        backgroundColor: palette.surface,
        behavior: SnackBarBehavior.floating,
        contentTextStyle: TextStyle(color: palette.primaryText),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
      ),
    );
  }

  static ColorScheme colorsFor(TvThemeMode mode, ColorScheme? dynamicColors) {
    if (mode == TvThemeMode.system && dynamicColors != null) {
      return dynamicColors;
    }
    final seed = switch (mode) {
      TvThemeMode.system || TvThemeMode.aurora => const Color(0xFF46C5A1),
      TvThemeMode.ember => const Color(0xFFE47A4D),
      TvThemeMode.indigo => const Color(0xFF9A8CFF),
    };
    return ColorScheme.fromSeed(seedColor: seed, brightness: Brightness.dark);
  }
}

class TvPalette extends ThemeExtension<TvPalette> {
  const TvPalette({
    required this.canvas,
    required this.surface,
    required this.surfaceMuted,
    required this.focus,
    required this.primaryAction,
    required this.primaryText,
    required this.secondaryText,
    required this.warning,
    required this.info,
    required this.backdropGlow,
  });

  final Color canvas;
  final Color surface;
  final Color surfaceMuted;
  final Color focus;
  final Color primaryAction;
  final Color primaryText;
  final Color secondaryText;
  final Color warning;
  final Color info;
  final Color backdropGlow;

  static const fallback = TvPalette(
    canvas: TvTheme.canvas,
    surface: TvTheme.surface,
    surfaceMuted: Color(0xFF172832),
    focus: TvTheme.focus,
    primaryAction: TvTheme.primaryAction,
    primaryText: TvTheme.primaryText,
    secondaryText: TvTheme.secondaryText,
    warning: Color(0xFFFFB36B),
    info: Color(0xFF7AC8FF),
    backdropGlow: Color(0xFF1B3A48),
  );

  static TvPalette of(BuildContext context) =>
      Theme.of(context).extension<TvPalette>() ?? fallback;

  @override
  TvPalette copyWith({
    Color? canvas,
    Color? surface,
    Color? surfaceMuted,
    Color? focus,
    Color? primaryAction,
    Color? primaryText,
    Color? secondaryText,
    Color? warning,
    Color? info,
    Color? backdropGlow,
  }) => TvPalette(
    canvas: canvas ?? this.canvas,
    surface: surface ?? this.surface,
    surfaceMuted: surfaceMuted ?? this.surfaceMuted,
    focus: focus ?? this.focus,
    primaryAction: primaryAction ?? this.primaryAction,
    primaryText: primaryText ?? this.primaryText,
    secondaryText: secondaryText ?? this.secondaryText,
    warning: warning ?? this.warning,
    info: info ?? this.info,
    backdropGlow: backdropGlow ?? this.backdropGlow,
  );

  @override
  TvPalette lerp(TvPalette? other, double t) {
    if (other is! TvPalette) {
      return this;
    }
    return TvPalette(
      canvas: Color.lerp(canvas, other.canvas, t)!,
      surface: Color.lerp(surface, other.surface, t)!,
      surfaceMuted: Color.lerp(surfaceMuted, other.surfaceMuted, t)!,
      focus: Color.lerp(focus, other.focus, t)!,
      primaryAction: Color.lerp(primaryAction, other.primaryAction, t)!,
      primaryText: Color.lerp(primaryText, other.primaryText, t)!,
      secondaryText: Color.lerp(secondaryText, other.secondaryText, t)!,
      warning: Color.lerp(warning, other.warning, t)!,
      info: Color.lerp(info, other.info, t)!,
      backdropGlow: Color.lerp(backdropGlow, other.backdropGlow, t)!,
    );
  }
}

class TvThemeScope extends InheritedWidget {
  const TvThemeScope({
    required this.mode,
    required this.onModeChanged,
    required super.child,
    super.key,
  });

  final TvThemeMode mode;
  final ValueChanged<TvThemeMode> onModeChanged;

  static TvThemeScope of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<TvThemeScope>()!;

  @override
  bool updateShouldNotify(TvThemeScope oldWidget) => mode != oldWidget.mode;
}
