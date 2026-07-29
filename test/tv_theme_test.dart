import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hearthdeck/tv_theme.dart';

void main() {
  test(
    'system mode falls back to the Aurora palette without platform colors',
    () {
      expect(
        TvTheme.colorsFor(TvThemeMode.system, null).primary,
        TvTheme.colorsFor(TvThemeMode.aurora, null).primary,
      );
    },
  );

  test('curated theme modes produce distinct primary colors', () {
    final ember = TvTheme.colorsFor(TvThemeMode.ember, null);
    final indigo = TvTheme.colorsFor(TvThemeMode.indigo, null);

    expect(ember.primary, isNot(indigo.primary));
  });

  test('system mode honors a dynamic dark scheme', () {
    const dynamicScheme = ColorScheme.dark(primary: Color(0xFF00FF99));

    expect(TvTheme.colorsFor(TvThemeMode.system, dynamicScheme), dynamicScheme);
  });
}
