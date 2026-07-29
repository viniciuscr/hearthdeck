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

  test('Noir is a black-forward palette with a separate violet action', () {
    final noir = TvTheme.paletteFor(TvThemeMode.noir, null);

    expect(noir.canvas.computeLuminance(), lessThan(0.004));
    expect(noir.action, isNot(noir.focus));
    expect(noir.focus, isNot(noir.success));
  });

  test('curated palettes meet minimum contrast for text and focus', () {
    for (final mode in TvThemeMode.values.where(
      (TvThemeMode mode) => mode != TvThemeMode.system,
    )) {
      final palette = TvTheme.paletteFor(mode, null);

      expect(
        _contrastRatio(palette.primaryText, palette.canvas),
        greaterThanOrEqualTo(4.5),
        reason: '${mode.name} primary text on canvas',
      );
      expect(
        _contrastRatio(palette.secondaryText, palette.canvas),
        greaterThanOrEqualTo(4.5),
        reason: '${mode.name} secondary text on canvas',
      );
      expect(
        _contrastRatio(palette.focus, palette.surface),
        greaterThanOrEqualTo(3),
        reason: '${mode.name} focus ring on surface',
      );
      expect(
        _contrastRatio(palette.action, palette.canvas),
        greaterThanOrEqualTo(3),
        reason: '${mode.name} action on canvas',
      );
      expect(
        _contrastRatio(palette.onAction, palette.action),
        greaterThanOrEqualTo(4.5),
        reason: '${mode.name} action text',
      );
    }
  });

  test('system mode honors a dynamic dark scheme', () {
    const dynamicScheme = ColorScheme.dark(primary: Color(0xFF00FF99));

    expect(TvTheme.colorsFor(TvThemeMode.system, dynamicScheme), dynamicScheme);
  });
}

double _contrastRatio(Color left, Color right) {
  final lighter = left.computeLuminance() > right.computeLuminance()
      ? left
      : right;
  final darker = identical(lighter, left) ? right : left;
  return (lighter.computeLuminance() + 0.05) /
      (darker.computeLuminance() + 0.05);
}
