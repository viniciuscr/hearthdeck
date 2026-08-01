import 'package:flutter/material.dart';

import 'tv_components.dart';
import 'tv_theme.dart';

class ThemeSettingsPage extends StatelessWidget {
  const ThemeSettingsPage({super.key});

  @override
  Widget build(BuildContext context) {
    final scope = TvThemeScope.of(context);
    final tv = TvPalette.of(context);
    // Escape/back is handled globally (see main.dart's HardwareKeyboard
    // listener), regardless of what has focus on this screen.
    return TvDirectionalFocusNavigation(
      child: Scaffold(
        body: SafeArea(
          child: Stack(
            children: <Widget>[
              const Positioned.fill(
                child: TvBackdrop(center: Alignment(-0.5, -0.6)),
              ),
              CustomScrollView(
                slivers: <Widget>[
                  SliverPadding(
                    padding: const EdgeInsets.fromLTRB(48, 40, 48, 56),
                    sliver: SliverMainAxisGroup(
                      slivers: <Widget>[
                        SliverToBoxAdapter(
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
                                    style: Theme.of(
                                      context,
                                    ).textTheme.displaySmall,
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
                              const SizedBox(height: 18),
                              const _AppearanceRolePreview(),
                              const SizedBox(height: 34),
                            ],
                          ),
                        ),
                        SliverGrid.builder(
                          itemCount: TvThemeMode.values.length,
                          gridDelegate:
                              const SliverGridDelegateWithMaxCrossAxisExtent(
                                maxCrossAxisExtent: 430,
                                mainAxisSpacing: 18,
                                crossAxisSpacing: 18,
                                childAspectRatio: 1.42,
                              ),
                          itemBuilder: (BuildContext context, int index) {
                            final mode = TvThemeMode.values[index];
                            return _ThemeChoiceCard(
                              mode: mode,
                              isSelected: mode == scope.mode,
                              onActivate: () => scope.onModeChanged(mode),
                            );
                          },
                        ),
                      ],
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _AppearanceRolePreview extends StatelessWidget {
  const _AppearanceRolePreview();

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final theme = Theme.of(context);
    return Semantics(
      label: 'Appearance role preview',
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: tv.surface,
          borderRadius: BorderRadius.circular(16),
          border: Border.all(color: tv.borderSubtle),
        ),
        child: Padding(
          padding: const EdgeInsets.all(20),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              Text('Live role preview', style: theme.textTheme.titleLarge),
              const SizedBox(height: 5),
              Text(
                'Check hierarchy, focus, action, and status before choosing a palette.',
                style: TextStyle(color: tv.secondaryText),
              ),
              const SizedBox(height: 18),
              Wrap(
                spacing: 12,
                runSpacing: 12,
                children: <Widget>[
                  _PreviewSurface(
                    label: 'Canvas',
                    color: tv.canvas,
                    foreground: tv.primaryText,
                    border: tv.borderSubtle,
                  ),
                  _PreviewSurface(
                    label: 'Base surface',
                    color: tv.surfaceMuted,
                    foreground: tv.primaryText,
                    border: tv.borderSubtle,
                  ),
                  _PreviewSurface(
                    label: 'Raised surface',
                    color: tv.surfaceRaised,
                    foreground: tv.primaryText,
                    border: tv.borderStrong,
                  ),
                  _PreviewSurface(
                    label: 'Selected',
                    color: tv.selected,
                    foreground: tv.onSelected,
                    border: tv.action,
                  ),
                  _PreviewSurface(
                    label: 'Primary action',
                    color: tv.action,
                    foreground: tv.onAction,
                    border: tv.action,
                  ),
                  _PreviewSurface(
                    label: 'Focused',
                    color: tv.focusFill,
                    foreground: tv.onFocus,
                    border: tv.focus,
                    borderWidth: 3,
                  ),
                ],
              ),
              const SizedBox(height: 18),
              Wrap(
                spacing: 12,
                runSpacing: 8,
                children: <Widget>[
                  _StatusChip(
                    label: 'Ready',
                    color: tv.success,
                    icon: Icons.check_circle_outline_rounded,
                  ),
                  _StatusChip(
                    label: 'Attention',
                    color: tv.warning,
                    icon: Icons.error_outline_rounded,
                  ),
                  _StatusChip(
                    label: 'Starting',
                    color: tv.info,
                    icon: Icons.hourglass_top_rounded,
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _PreviewSurface extends StatelessWidget {
  const _PreviewSurface({
    required this.label,
    required this.color,
    required this.foreground,
    required this.border,
    this.borderWidth = 1,
  });

  final String label;
  final Color color;
  final Color foreground;
  final Color border;
  final double borderWidth;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 144,
      height: 74,
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: color,
          borderRadius: BorderRadius.circular(10),
          border: Border.all(color: border, width: borderWidth),
        ),
        child: Padding(
          padding: const EdgeInsets.all(12),
          child: Align(
            alignment: Alignment.bottomLeft,
            child: Text(
              label,
              style: TextStyle(color: foreground, fontWeight: FontWeight.w700),
            ),
          ),
        ),
      ),
    );
  }
}

class _StatusChip extends StatelessWidget {
  const _StatusChip({
    required this.label,
    required this.color,
    required this.icon,
  });

  final String label;
  final Color color;
  final IconData icon;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.14),
        borderRadius: BorderRadius.circular(20),
        border: Border.all(color: color.withValues(alpha: 0.5)),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 7),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            Icon(icon, size: 20, color: color),
            const SizedBox(width: 6),
            Text(
              label,
              style: TextStyle(color: color, fontWeight: FontWeight.w700),
            ),
          ],
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
      height: 148,
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
      height: 148,
      child: TvFocusable(
        semanticLabel: mode.label,
        autofocus: isSelected,
        onActivate: onActivate,
        builder: (BuildContext context, bool isFocused) {
          final style = TvControlStyle.resolve(
            tv,
            variant: TvControlVariant.selectable,
            isFocused: isFocused,
            isSelected: isSelected,
          );
          return AnimatedContainer(
            duration: TvTheme.focusDuration,
            curve: TvTheme.focusCurve,
            padding: const EdgeInsets.all(14),
            decoration: BoxDecoration(
              color: isSelected || isFocused ? style.background : tv.surface,
              borderRadius: BorderRadius.circular(12),
              border: Border.all(
                color: isSelected || isFocused ? style.border : tv.borderSubtle,
                width: isFocused ? 3 : 1,
              ),
            ),
            child: Row(
              children: <Widget>[
                _BackdropSwatch(mode: mode),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    mainAxisAlignment: MainAxisAlignment.center,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: <Widget>[
                      Text(
                        mode.label,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          color: isSelected || isFocused
                              ? style.foreground
                              : tv.primaryText,
                          fontWeight: FontWeight.w700,
                        ),
                      ),
                      const SizedBox(height: 4),
                      Text(
                        mode.description,
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: TvTheme.labelSmallSize,
                          color: isSelected || isFocused
                              ? style.foreground.withValues(alpha: 0.78)
                              : tv.secondaryText,
                        ),
                      ),
                    ],
                  ),
                ),
                if (isSelected)
                  Icon(Icons.check_rounded, color: style.foreground),
              ],
            ),
          );
        },
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
    required this.onActivate,
  });

  final TvThemeMode mode;
  final bool isSelected;
  final VoidCallback onActivate;

  @override
  Widget build(BuildContext context) {
    final preview = TvTheme.paletteFor(mode, null);
    return TvFocusable(
      semanticLabel: mode.label,
      onActivate: onActivate,
      builder: (BuildContext context, bool isFocused) => AnimatedContainer(
        duration: TvTheme.focusDuration,
        curve: TvTheme.focusCurve,
        padding: const EdgeInsets.all(20),
        decoration: BoxDecoration(
          color: preview.surface,
          borderRadius: BorderRadius.circular(14),
          border: Border.all(
            color: isFocused || isSelected
                ? preview.focus
                : preview.borderStrong,
            width: isFocused ? 3 : 1,
          ),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            Expanded(
              child: ClipRRect(
                borderRadius: BorderRadius.circular(9),
                child: Column(
                  children: <Widget>[
                    Expanded(flex: 3, child: ColoredBox(color: preview.canvas)),
                    Expanded(
                      flex: 2,
                      child: Row(
                        children: <Widget>[
                          Expanded(
                            child: ColoredBox(color: preview.surfaceMuted),
                          ),
                          Expanded(
                            child: ColoredBox(color: preview.surfaceRaised),
                          ),
                        ],
                      ),
                    ),
                    Expanded(
                      child: Row(
                        children: <Widget>[
                          Expanded(child: ColoredBox(color: preview.focusFill)),
                          Expanded(child: ColoredBox(color: preview.action)),
                          Expanded(child: ColoredBox(color: preview.selected)),
                        ],
                      ),
                    ),
                  ],
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
                  Icon(Icons.check_circle_rounded, color: preview.focus),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
