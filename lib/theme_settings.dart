import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'tv_components.dart';
import 'tv_theme.dart';

class ThemeSettingsPage extends StatelessWidget {
  const ThemeSettingsPage({super.key});

  @override
  Widget build(BuildContext context) {
    final scope = TvThemeScope.of(context);
    final tv = TvPalette.of(context);
    return Actions(
      actions: <Type, Action<Intent>>{
        DismissIntent: CallbackAction<DismissIntent>(
          onInvoke: (DismissIntent intent) {
            Navigator.of(context).maybePop();
            return null;
          },
        ),
      },
      child: TvDirectionalFocusNavigation(
        child: Focus(
          canRequestFocus: false,
          onKeyEvent: (FocusNode node, KeyEvent event) {
            if (event is KeyDownEvent &&
                event.logicalKey == LogicalKeyboardKey.escape) {
              Navigator.of(context).maybePop();
              return KeyEventResult.handled;
            }
            return KeyEventResult.ignored;
          },
          child: Scaffold(
            body: SafeArea(
              child: Stack(
                children: <Widget>[
                  Positioned.fill(
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        gradient: RadialGradient(
                          center: const Alignment(-0.5, -0.6),
                          radius: 1.25,
                          colors: <Color>[tv.backdropGlow, tv.canvas],
                          stops: const <double>[0, 0.76],
                        ),
                      ),
                    ),
                  ),
                  Padding(
                    padding: const EdgeInsets.fromLTRB(48, 40, 48, 56),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: <Widget>[
                        Row(
                          children: <Widget>[
                            Icon(
                              Icons.palette_outlined,
                              color: tv.focus,
                              size: 34,
                            ),
                            const SizedBox(width: 14),
                            Text(
                              'Appearance & color',
                              style: Theme.of(context).textTheme.displaySmall,
                            ),
                          ],
                        ),
                        const SizedBox(height: 10),
                        Text(
                          'System Colors follows your GTK theme accent on Linux and wallpaper colors on Android.',
                          style: TextStyle(color: tv.secondaryText),
                        ),
                        const SizedBox(height: 34),
                        Expanded(
                          child: GridView.builder(
                            gridDelegate:
                                const SliverGridDelegateWithMaxCrossAxisExtent(
                                  maxCrossAxisExtent: 430,
                                  mainAxisSpacing: 18,
                                  crossAxisSpacing: 18,
                                  childAspectRatio: 1.42,
                                ),
                            itemCount: TvThemeMode.values.length,
                            itemBuilder: (BuildContext context, int index) {
                              final mode = TvThemeMode.values[index];
                              return _ThemeChoiceCard(
                                mode: mode,
                                isSelected: mode == scope.mode,
                                autofocus: mode == scope.mode,
                                onActivate: () => scope.onModeChanged(mode),
                              );
                            },
                          ),
                        ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _ThemeChoiceCard extends StatelessWidget {
  const _ThemeChoiceCard({
    required this.mode,
    required this.isSelected,
    required this.autofocus,
    required this.onActivate,
  });

  final TvThemeMode mode;
  final bool isSelected;
  final bool autofocus;
  final VoidCallback onActivate;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final colors = TvTheme.colorsFor(mode, null);
    return TvFocusable(
      semanticLabel: mode.label,
      autofocus: autofocus,
      onActivate: onActivate,
      builder: (BuildContext context, bool isFocused) => AnimatedContainer(
        duration: TvTheme.focusDuration,
        curve: TvTheme.focusCurve,
        padding: const EdgeInsets.all(20),
        decoration: BoxDecoration(
          color: tv.surface,
          borderRadius: BorderRadius.circular(14),
          border: Border.all(
            color: isFocused || isSelected
                ? tv.focus
                : colors.primary.withValues(alpha: 0.45),
            width: isFocused ? 3 : 1,
          ),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            Expanded(
              child: DecoratedBox(
                decoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(9),
                  gradient: LinearGradient(
                    begin: Alignment.topLeft,
                    end: Alignment.bottomRight,
                    colors: <Color>[
                      colors.primary,
                      colors.secondary,
                      colors.surface,
                    ],
                  ),
                ),
              ),
            ),
            const SizedBox(height: 14),
            Row(
              children: <Widget>[
                Expanded(
                  child: Text(
                    mode.label,
                    style: Theme.of(context).textTheme.titleLarge,
                  ),
                ),
                if (isSelected)
                  Icon(Icons.check_circle_rounded, color: tv.focus),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
