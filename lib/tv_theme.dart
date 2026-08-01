import 'package:flutter/material.dart';

enum TvThemeMode { system, aurora, ember, indigo, noir }

enum TvBackdropMode { solid, edgeWash, quietGrid }

extension TvThemeModeLabel on TvThemeMode {
  String get label => switch (this) {
    TvThemeMode.system => 'System colors',
    TvThemeMode.aurora => 'Aurora',
    TvThemeMode.ember => 'Ember',
    TvThemeMode.indigo => 'Indigo',
    TvThemeMode.noir => 'Noir',
  };
}

extension TvBackdropModeLabel on TvBackdropMode {
  String get label => switch (this) {
    TvBackdropMode.solid => 'Solid',
    TvBackdropMode.edgeWash => 'Edge wash',
    TvBackdropMode.quietGrid => 'Quiet grid',
  };

  String get description => switch (this) {
    TvBackdropMode.solid => 'Pure neutral canvas',
    TvBackdropMode.edgeWash => 'A restrained color field',
    TvBackdropMode.quietGrid => 'A subtle architectural grid',
  };

  String get wireName => switch (this) {
    TvBackdropMode.solid => 'solid',
    TvBackdropMode.edgeWash => 'edge_wash',
    TvBackdropMode.quietGrid => 'quiet_grid',
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

  // Comfortable-reading-distance type scale for a 10-foot / TV interface.
  // Roughly 1.4-1.5x the stock Material 3 sizes so text stays legible from
  // a couch without looking oversized up close.
  static const double displayLargeSize = 64;
  static const double displayMediumSize = 52;
  static const double displaySmallSize = 44;
  static const double headlineLargeSize = 40;
  static const double headlineMediumSize = 36;
  static const double headlineSmallSize = 32;
  static const double titleLargeSize = 30;
  static const double titleMediumSize = 24;
  static const double titleSmallSize = 20;
  static const double labelLargeSize = 20;
  static const double labelMediumSize = 18;
  static const double labelSmallSize = 16;
  static const double bodyLargeSize = 24;
  static const double bodyMediumSize = 20;
  static const double bodySmallSize = 18;

  static ThemeData data(ColorScheme colors, TvPalette palette) {
    return ThemeData(
      useMaterial3: true,
      brightness: Brightness.dark,
      colorScheme: colors,
      scaffoldBackgroundColor: palette.canvas,
      extensions: <ThemeExtension<dynamic>>[palette],
      textTheme: TextTheme(
        displayLarge: const TextStyle(fontSize: displayLargeSize),
        displayMedium: const TextStyle(fontSize: displayMediumSize),
        displaySmall: const TextStyle(fontSize: displaySmallSize),
        headlineLarge: const TextStyle(fontSize: headlineLargeSize),
        headlineMedium: const TextStyle(fontSize: headlineMediumSize),
        headlineSmall: const TextStyle(fontSize: headlineSmallSize),
        titleSmall: const TextStyle(fontSize: titleSmallSize),
        labelLarge: const TextStyle(fontSize: labelLargeSize),
        labelMedium: const TextStyle(fontSize: labelMediumSize),
        labelSmall: const TextStyle(fontSize: labelSmallSize),
        bodyLarge: const TextStyle(fontSize: bodyLargeSize),
        bodyMedium: TextStyle(
          fontSize: bodyMediumSize,
          color: palette.primaryText,
        ),
        bodySmall: TextStyle(
          fontSize: bodySmallSize,
          color: palette.secondaryText,
        ),
        titleMedium: TextStyle(
          fontSize: titleMediumSize,
          color: palette.primaryText,
          fontWeight: FontWeight.w600,
        ),
        titleLarge: TextStyle(
          fontSize: titleLargeSize,
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
    final palette = paletteFor(mode, dynamicColors);
    return ColorScheme.dark(
      primary: palette.action,
      onPrimary: palette.onAction,
      primaryContainer: palette.selected,
      onPrimaryContainer: palette.primaryText,
      secondary: palette.info,
      onSecondary: palette.canvas,
      tertiary: palette.warning,
      onTertiary: palette.canvas,
      surface: palette.surface,
      onSurface: palette.primaryText,
      surfaceContainer: palette.surfaceMuted,
      onSurfaceVariant: palette.secondaryText,
    );
  }

  static TvPalette paletteFor(TvThemeMode mode, ColorScheme? dynamicColors) {
    if (mode == TvThemeMode.system && dynamicColors != null) {
      return TvPalette.fromDynamicColors(dynamicColors);
    }
    return switch (mode) {
      TvThemeMode.system || TvThemeMode.aurora => const TvPalette(
        canvas: Color(0xFF061816),
        surface: Color(0xFF0C2925),
        surfaceMuted: Color(0xFF113832),
        surfaceRaised: Color(0xFF194A42),
        borderSubtle: Color(0xFF1E5A50),
        borderStrong: Color(0xFF368476),
        focus: Color(0xFF6AF0D0),
        focusFill: Color(0xFF075D50),
        onFocus: Color(0xFFFFFFFF),
        action: Color(0xFF087E6D),
        onAction: Color(0xFFFFFFFF),
        selected: Color(0xFF0D4B42),
        onSelected: Color(0xFFFFFFFF),
        accent: Color(0xFF34DAB9),
        primaryText: Color(0xFFF1F7F5),
        secondaryText: Color(0xFFB5C6C1),
        success: Color(0xFF65C58B),
        warning: Color(0xFFF0B678),
        info: Color(0xFF8CC5E7),
        backdropGlow: Color(0xFF0B4B40),
      ),
      TvThemeMode.ember => const TvPalette(
        canvas: Color(0xFF1B0A05),
        surface: Color(0xFF321309),
        surfaceMuted: Color(0xFF471C0C),
        surfaceRaised: Color(0xFF5A2915),
        borderSubtle: Color(0xFF74391F),
        borderStrong: Color(0xFFA0542F),
        focus: Color(0xFFFFA978),
        focusFill: Color(0xFF7D2C12),
        onFocus: Color(0xFFFFFFFF),
        action: Color(0xFFAB3E16),
        onAction: Color(0xFFFFFFFF),
        selected: Color(0xFF65220E),
        onSelected: Color(0xFFFFFFFF),
        accent: Color(0xFFFF7645),
        primaryText: Color(0xFFFFF6F1),
        secondaryText: Color(0xFFD1BFB4),
        success: Color(0xFF6BC58D),
        warning: Color(0xFFF0C56B),
        info: Color(0xFF92C7E6),
        backdropGlow: Color(0xFF68270F),
      ),
      TvThemeMode.indigo => const TvPalette(
        canvas: Color(0xFF0E102A),
        surface: Color(0xFF1A1B46),
        surfaceMuted: Color(0xFF28275E),
        surfaceRaised: Color(0xFF393878),
        borderSubtle: Color(0xFF4A4990),
        borderStrong: Color(0xFF716FC6),
        focus: Color(0xFFAAB8FF),
        focusFill: Color(0xFF353F91),
        onFocus: Color(0xFFFFFFFF),
        action: Color(0xFF6C4BC1),
        onAction: Color(0xFFFFFFFF),
        selected: Color(0xFF3D286F),
        onSelected: Color(0xFFFFFFFF),
        accent: Color(0xFFA18FFF),
        primaryText: Color(0xFFF4F3FF),
        secondaryText: Color(0xFFC5C5D9),
        success: Color(0xFF6CC691),
        warning: Color(0xFFEFC477),
        info: Color(0xFF8AC4EB),
        backdropGlow: Color(0xFF29256D),
      ),
      TvThemeMode.noir => const TvPalette(
        canvas: Color(0xFF050507),
        surface: Color(0xFF101014),
        surfaceMuted: Color(0xFF1A1A20),
        surfaceRaised: Color(0xFF292930),
        borderSubtle: Color(0xFF3A3A45),
        borderStrong: Color(0xFF8B8998),
        focus: Color(0xFFE4E4ED),
        focusFill: Color(0xFF4A4756),
        onFocus: Color(0xFFFFFFFF),
        action: Color(0xFF7657CB),
        onAction: Color(0xFFFFFFFF),
        selected: Color(0xFF382A68),
        onSelected: Color(0xFFFFFFFF),
        accent: Color(0xFFA990FF),
        primaryText: Color(0xFFF5F5F7),
        secondaryText: Color(0xFFB9B9C5),
        success: Color(0xFF5DBD87),
        warning: Color(0xFFEAB664),
        info: Color(0xFF72B8EF),
        backdropGlow: Color(0xFF211C35),
      ),
    };
  }
}

class TvPalette extends ThemeExtension<TvPalette> {
  const TvPalette({
    required this.canvas,
    required this.surface,
    required this.surfaceMuted,
    required this.surfaceRaised,
    required this.borderSubtle,
    required this.borderStrong,
    required this.focus,
    required this.focusFill,
    required this.onFocus,
    required this.action,
    required this.onAction,
    required this.selected,
    required this.onSelected,
    required this.accent,
    required this.primaryText,
    required this.secondaryText,
    required this.success,
    required this.warning,
    required this.info,
    required this.backdropGlow,
  });

  final Color canvas;
  final Color surface;
  final Color surfaceMuted;
  final Color surfaceRaised;
  final Color borderSubtle;
  final Color borderStrong;
  final Color focus;
  final Color focusFill;
  final Color onFocus;
  final Color action;
  final Color onAction;
  final Color selected;
  final Color onSelected;
  final Color accent;
  final Color primaryText;
  final Color secondaryText;
  final Color success;
  final Color warning;
  final Color info;
  final Color backdropGlow;

  static const fallback = TvPalette(
    canvas: TvTheme.canvas,
    surface: TvTheme.surface,
    surfaceMuted: Color(0xFF172832),
    surfaceRaised: Color(0xFF243942),
    borderSubtle: Color(0xFF39555F),
    borderStrong: Color(0xFF6C8993),
    focus: TvTheme.focus,
    focusFill: Color(0xFF214B43),
    onFocus: TvTheme.primaryText,
    action: TvTheme.primaryAction,
    onAction: TvTheme.canvas,
    selected: Color(0xFF1F4E1C),
    onSelected: TvTheme.primaryText,
    accent: Color(0xFF46C5A1),
    primaryText: TvTheme.primaryText,
    secondaryText: TvTheme.secondaryText,
    success: Color(0xFF6BC58D),
    warning: Color(0xFFFFB36B),
    info: Color(0xFF7AC8FF),
    backdropGlow: Color(0xFF1B3A48),
  );

  factory TvPalette.fromDynamicColors(ColorScheme colors) => TvPalette(
    canvas: colors.surface,
    surface: colors.surfaceContainerHigh,
    surfaceMuted: colors.surfaceContainer,
    surfaceRaised: colors.surfaceContainerHighest,
    borderSubtle: colors.outlineVariant,
    borderStrong: colors.outline,
    focus: colors.onSurface,
    focusFill: colors.secondaryContainer,
    onFocus: colors.onSecondaryContainer,
    action: colors.primary,
    onAction: colors.onPrimary,
    selected: colors.primaryContainer,
    onSelected: colors.onPrimaryContainer,
    accent: colors.primary,
    primaryText: colors.onSurface,
    secondaryText: colors.onSurfaceVariant,
    success: colors.tertiary,
    warning: colors.tertiary,
    info: colors.secondary,
    backdropGlow: Color.lerp(colors.primary, colors.surface, 0.62)!,
  );

  static TvPalette of(BuildContext context) =>
      Theme.of(context).extension<TvPalette>() ?? fallback;

  @override
  TvPalette copyWith({
    Color? canvas,
    Color? surface,
    Color? surfaceMuted,
    Color? surfaceRaised,
    Color? borderSubtle,
    Color? borderStrong,
    Color? focus,
    Color? focusFill,
    Color? onFocus,
    Color? action,
    Color? onAction,
    Color? selected,
    Color? onSelected,
    Color? accent,
    Color? primaryText,
    Color? secondaryText,
    Color? success,
    Color? warning,
    Color? info,
    Color? backdropGlow,
  }) => TvPalette(
    canvas: canvas ?? this.canvas,
    surface: surface ?? this.surface,
    surfaceMuted: surfaceMuted ?? this.surfaceMuted,
    surfaceRaised: surfaceRaised ?? this.surfaceRaised,
    borderSubtle: borderSubtle ?? this.borderSubtle,
    borderStrong: borderStrong ?? this.borderStrong,
    focus: focus ?? this.focus,
    focusFill: focusFill ?? this.focusFill,
    onFocus: onFocus ?? this.onFocus,
    action: action ?? this.action,
    onAction: onAction ?? this.onAction,
    selected: selected ?? this.selected,
    onSelected: onSelected ?? this.onSelected,
    accent: accent ?? this.accent,
    primaryText: primaryText ?? this.primaryText,
    secondaryText: secondaryText ?? this.secondaryText,
    success: success ?? this.success,
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
      surfaceRaised: Color.lerp(surfaceRaised, other.surfaceRaised, t)!,
      borderSubtle: Color.lerp(borderSubtle, other.borderSubtle, t)!,
      borderStrong: Color.lerp(borderStrong, other.borderStrong, t)!,
      focus: Color.lerp(focus, other.focus, t)!,
      focusFill: Color.lerp(focusFill, other.focusFill, t)!,
      onFocus: Color.lerp(onFocus, other.onFocus, t)!,
      action: Color.lerp(action, other.action, t)!,
      onAction: Color.lerp(onAction, other.onAction, t)!,
      selected: Color.lerp(selected, other.selected, t)!,
      onSelected: Color.lerp(onSelected, other.onSelected, t)!,
      accent: Color.lerp(accent, other.accent, t)!,
      primaryText: Color.lerp(primaryText, other.primaryText, t)!,
      secondaryText: Color.lerp(secondaryText, other.secondaryText, t)!,
      success: Color.lerp(success, other.success, t)!,
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
    required this.backdropMode,
    required this.onBackdropModeChanged,
    required super.child,
    super.key,
  });

  final TvThemeMode mode;
  final ValueChanged<TvThemeMode> onModeChanged;
  final TvBackdropMode backdropMode;
  final ValueChanged<TvBackdropMode> onBackdropModeChanged;

  static TvThemeScope of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<TvThemeScope>()!;

  static TvThemeScope? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<TvThemeScope>();

  @override
  bool updateShouldNotify(TvThemeScope oldWidget) =>
      mode != oldWidget.mode || backdropMode != oldWidget.backdropMode;
}

class TvBackdrop extends StatelessWidget {
  const TvBackdrop({super.key, this.center = const Alignment(0.84, -0.58)});

  final Alignment center;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final backdropMode =
        TvThemeScope.maybeOf(context)?.backdropMode ?? TvBackdropMode.edgeWash;
    return switch (backdropMode) {
      TvBackdropMode.solid => ColoredBox(color: tv.canvas),
      TvBackdropMode.edgeWash => DecoratedBox(
        decoration: BoxDecoration(
          gradient: RadialGradient(
            center: center,
            radius: 1.32,
            colors: <Color>[tv.backdropGlow, tv.canvas],
            stops: const <double>[0, 0.74],
          ),
        ),
      ),
      TvBackdropMode.quietGrid => CustomPaint(
        painter: _QuietGridPainter(canvas: tv.canvas, line: tv.primaryText),
      ),
    };
  }
}

class _QuietGridPainter extends CustomPainter {
  const _QuietGridPainter({required this.canvas, required this.line});

  final Color canvas;
  final Color line;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawColor(this.canvas, BlendMode.src);
    final paint = Paint()
      ..color = line.withValues(alpha: 0.035)
      ..strokeWidth = 1;
    const spacing = 96.0;
    for (var x = 0.0; x < size.width; x += spacing) {
      canvas.drawLine(Offset(x, 0), Offset(x, size.height), paint);
    }
    for (var y = 0.0; y < size.height; y += spacing) {
      canvas.drawLine(Offset(0, y), Offset(size.width, y), paint);
    }
  }

  @override
  bool shouldRepaint(_QuietGridPainter oldDelegate) =>
      canvas != oldDelegate.canvas || line != oldDelegate.line;
}
