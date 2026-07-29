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
                  const Positioned.fill(
                    child: TvBackdrop(center: Alignment(-0.5, -0.6)),
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
                        Text(
                          'Backdrop treatment',
                          style: Theme.of(context).textTheme.titleLarge,
                        ),
                        const SizedBox(height: 14),
                        _BackdropChoices(
                          selected: scope.backdropMode,
                          onActivate: scope.onBackdropModeChanged,
                        ),
                        const SizedBox(height: 34),
                        Text(
                          'Theme palette',
                          style: Theme.of(context).textTheme.titleLarge,
                        ),
                        const SizedBox(height: 14),
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

class _BackdropChoices extends StatelessWidget {
  const _BackdropChoices({required this.selected, required this.onActivate});

  final TvBackdropMode selected;
  final ValueChanged<TvBackdropMode> onActivate;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: 116,
      child: ListView.separated(
        scrollDirection: Axis.horizontal,
        itemCount: TvBackdropMode.values.length,
        separatorBuilder: (BuildContext context, int index) =>
            const SizedBox(width: 14),
        itemBuilder: (BuildContext context, int index) {
          final mode = TvBackdropMode.values[index];
          return _BackdropChoiceCard(
            mode: mode,
            isSelected: mode == selected,
            onActivate: () => onActivate(mode),
          );
        },
      ),
    );
  }
}

class _BackdropChoiceCard extends StatelessWidget {
  const _BackdropChoiceCard({
    required this.mode,
    required this.isSelected,
    required this.onActivate,
  });

  final TvBackdropMode mode;
  final bool isSelected;
  final VoidCallback onActivate;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return SizedBox(
      width: 220,
      child: TvFocusable(
        semanticLabel: mode.label,
        onActivate: onActivate,
        builder: (BuildContext context, bool isFocused) => AnimatedContainer(
          duration: TvTheme.focusDuration,
          curve: TvTheme.focusCurve,
          padding: const EdgeInsets.all(14),
          decoration: BoxDecoration(
            color: tv.surface,
            borderRadius: BorderRadius.circular(12),
            border: Border.all(
              color: isFocused || isSelected ? tv.focus : tv.surfaceMuted,
              width: isFocused ? 3 : 1,
            ),
          ),
          child: Row(
            children: <Widget>[
              _BackdropSwatch(mode: mode),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  mainAxisAlignment: MainAxisAlignment.center,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: <Widget>[
                    Text(
                      mode.label,
                      style: const TextStyle(fontWeight: FontWeight.w700),
                    ),
                    const SizedBox(height: 4),
                    Text(
                      mode.description,
                      style: TextStyle(fontSize: 12, color: tv.secondaryText),
                    ),
                  ],
                ),
              ),
              if (isSelected) Icon(Icons.check_rounded, color: tv.focus),
            ],
          ),
        ),
      ),
    );
  }
}

class _BackdropSwatch extends StatelessWidget {
  const _BackdropSwatch({required this.mode});

  final TvBackdropMode mode;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return SizedBox(
      width: 54,
      height: 68,
      child: ClipRRect(
        borderRadius: BorderRadius.circular(7),
        child: switch (mode) {
          TvBackdropMode.solid => ColoredBox(color: tv.canvas),
          TvBackdropMode.edgeWash => DecoratedBox(
            decoration: BoxDecoration(
              gradient: RadialGradient(
                center: const Alignment(-0.4, -0.6),
                colors: <Color>[tv.backdropGlow, tv.canvas],
              ),
            ),
          ),
          TvBackdropMode.quietGrid => CustomPaint(
            painter: _ThemeGridPainter(canvas: tv.canvas, line: tv.primaryText),
          ),
        },
      ),
    );
  }
}

class _ThemeGridPainter extends CustomPainter {
  const _ThemeGridPainter({required this.canvas, required this.line});

  final Color canvas;
  final Color line;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawColor(this.canvas, BlendMode.src);
    final paint = Paint()
      ..color = line.withValues(alpha: 0.07)
      ..strokeWidth = 1;
    for (var x = 0.0; x < size.width; x += 12) {
      canvas.drawLine(Offset(x, 0), Offset(x, size.height), paint);
    }
    for (var y = 0.0; y < size.height; y += 12) {
      canvas.drawLine(Offset(0, y), Offset(size.width, y), paint);
    }
  }

  @override
  bool shouldRepaint(_ThemeGridPainter oldDelegate) =>
      canvas != oldDelegate.canvas || line != oldDelegate.line;
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
    final preview = TvTheme.paletteFor(mode, null);
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
                : preview.focus.withValues(alpha: 0.45),
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
                      preview.focus,
                      preview.info,
                      preview.canvas,
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
