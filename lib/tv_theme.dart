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

  static ThemeData data(ColorScheme colors, TvPalette palette) {
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
    final palette = paletteFor(mode, dynamicColors);
    return ColorScheme.dark(
      primary: palette.action,
      onPrimary: palette.onAction,
      primaryContainer: palette.actionMuted,
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
        canvas: Color(0xFF0B1416),
        surface: Color(0xFF122023),
        surfaceMuted: Color(0xFF1A2B2E),
        surfaceRaised: Color(0xFF263B40),
        borderSubtle: Color(0xFF344F55),
        borderStrong: Color(0xFF66858D),
        focus: Color(0xFFEAF7F4),
        action: Color(0xFF28756A),
        actionMuted: Color(0xFF1A4B44),
        onAction: Color(0xFFFFFFFF),
        primaryText: Color(0xFFF1F7F5),
        secondaryText: Color(0xFFB5C6C1),
        success: Color(0xFF65C58B),
        warning: Color(0xFFF0B678),
        info: Color(0xFF8CC5E7),
        backdropGlow: Color(0xFF173B39),
      ),
      TvThemeMode.ember => const TvPalette(
        canvas: Color(0xFF17110F),
        surface: Color(0xFF241916),
        surfaceMuted: Color(0xFF33231E),
        surfaceRaised: Color(0xFF443028),
        borderSubtle: Color(0xFF5B4035),
        borderStrong: Color(0xFF7A5949),
        focus: Color(0xFFFFF1E9),
        action: Color(0xFF9B4B32),
        actionMuted: Color(0xFF65301F),
        onAction: Color(0xFFFFFFFF),
        primaryText: Color(0xFFFFF6F1),
        secondaryText: Color(0xFFD1BFB4),
        success: Color(0xFF6BC58D),
        warning: Color(0xFFF0C56B),
        info: Color(0xFF92C7E6),
        backdropGlow: Color(0xFF42251B),
      ),
      TvThemeMode.indigo => const TvPalette(
        canvas: Color(0xFF10121D),
        surface: Color(0xFF191C2B),
        surfaceMuted: Color(0xFF252940),
        surfaceRaised: Color(0xFF303653),
        borderSubtle: Color(0xFF444B70),
        borderStrong: Color(0xFF626C9C),
        focus: Color(0xFFF0F0FF),
        action: Color(0xFF5E67C9),
        actionMuted: Color(0xFF383F78),
        onAction: Color(0xFFFFFFFF),
        primaryText: Color(0xFFF4F3FF),
        secondaryText: Color(0xFFC5C5D9),
        success: Color(0xFF6CC691),
        warning: Color(0xFFEFC477),
        info: Color(0xFF8AC4EB),
        backdropGlow: Color(0xFF262A50),
      ),
      TvThemeMode.noir => const TvPalette(
        canvas: Color(0xFF08080A),
        surface: Color(0xFF111116),
        surfaceMuted: Color(0xFF19191F),
        surfaceRaised: Color(0xFF24242D),
        borderSubtle: Color(0xFF343440),
        borderStrong: Color(0xFF7D7D8C),
        focus: Color(0xFFE4E4ED),
        action: Color(0xFF6F5BCE),
        actionMuted: Color(0xFF2E2853),
        onAction: Color(0xFFFFFFFF),
        primaryText: Color(0xFFF5F5F7),
        secondaryText: Color(0xFFB9B9C5),
        success: Color(0xFF5DBD87),
        warning: Color(0xFFEAB664),
        info: Color(0xFF72B8EF),
        backdropGlow: Color(0xFF1A1827),
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
    required this.action,
    required this.actionMuted,
    required this.onAction,
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
  final Color action;
  final Color actionMuted;
  final Color onAction;
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
    action: TvTheme.primaryAction,
    actionMuted: Color(0xFF1F4E1C),
    onAction: TvTheme.canvas,
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
    action: colors.primary,
    actionMuted: colors.primaryContainer,
    onAction: colors.onPrimary,
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
    Color? action,
    Color? actionMuted,
    Color? onAction,
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
    action: action ?? this.action,
    actionMuted: actionMuted ?? this.actionMuted,
    onAction: onAction ?? this.onAction,
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
      action: Color.lerp(action, other.action, t)!,
      actionMuted: Color.lerp(actionMuted, other.actionMuted, t)!,
      onAction: Color.lerp(onAction, other.onAction, t)!,
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
