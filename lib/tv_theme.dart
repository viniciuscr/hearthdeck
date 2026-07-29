import 'package:flutter/material.dart';

abstract final class TvTheme {
  static const Color canvas = Color(0xFF071017);
  static const Color surface = Color(0xFF101C25);
  static const Color primaryAction = Color(0xFF347C28);
  static const Color focus = Color(0xFF7BE443);
  static const Color primaryText = Color(0xFFF4F7F9);
  static const Color secondaryText = Color(0xFFB5C1C9);
  static const Duration focusDuration = Duration(milliseconds: 140);
  static const Curve focusCurve = Curves.easeOutCubic;

  static final ThemeData data = ThemeData(
    brightness: Brightness.dark,
    scaffoldBackgroundColor: canvas,
    colorScheme: const ColorScheme.dark(
      primary: focus,
      surface: surface,
      onSurface: primaryText,
    ),
    textTheme: const TextTheme(
      bodyMedium: TextStyle(color: primaryText),
      bodySmall: TextStyle(color: secondaryText),
      titleMedium: TextStyle(color: primaryText, fontWeight: FontWeight.w600),
      titleLarge: TextStyle(color: primaryText, fontWeight: FontWeight.w700),
    ),
    snackBarTheme: SnackBarThemeData(
      backgroundColor: surface,
      behavior: SnackBarBehavior.floating,
      contentTextStyle: const TextStyle(color: primaryText),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
    ),
  );
}
