import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import 'dashboard_models.dart';
import 'tv_theme.dart';

class TvLayoutMetrics {
  const TvLayoutMetrics._({
    required this.pageInsets,
    required this.topBarHeight,
    required this.heroHeight,
    required this.squareExtent,
    required this.gap,
    required this.sectionTitleGap,
    required this.sectionGap,
    required this.isCompact,
  });

  factory TvLayoutMetrics.fromConstraints(BoxConstraints constraints) {
    final width = constraints.maxWidth;
    final height = constraints.maxHeight;
    final visualScale = (math.min(width, height) / 720)
        .clamp(0.72, 1.35)
        .toDouble();
    final pagePadding = (width * 0.05)
        .clamp(24 * visualScale, 96 * visualScale)
        .toDouble();
    final pageInsets = EdgeInsets.symmetric(horizontal: pagePadding);
    final availableWidth = math.max(width - pageInsets.horizontal, 1);

    return TvLayoutMetrics._(
      pageInsets: pageInsets,
      topBarHeight: 64 * visualScale,
      heroHeight: (height * 0.42)
          .clamp(220 * visualScale, 500 * visualScale)
          .toDouble(),
      squareExtent: (availableWidth * 0.112)
          .clamp(104 * visualScale, 204 * visualScale)
          .toDouble(),
      gap: 14 * visualScale,
      sectionTitleGap: 8 * visualScale,
      sectionGap: 34 * visualScale,
      isCompact: width < 720,
    );
  }

  final EdgeInsets pageInsets;
  final double topBarHeight;
  final double heroHeight;
  final double squareExtent;
  final double gap;
  final double sectionTitleGap;
  final double sectionGap;
  final bool isCompact;
}

class TvFocusable extends StatefulWidget {
  const TvFocusable({
    required this.semanticLabel,
    required this.builder,
    super.key,
    this.autofocus = false,
    required this.onActivate,
  });

  final String semanticLabel;
  final Widget Function(BuildContext context, bool isFocused) builder;
  final bool autofocus;
  final VoidCallback? onActivate;

  @override
  State<TvFocusable> createState() => _TvFocusableState();
}

class TvDirectionalFocusNavigation extends StatelessWidget {
  const TvDirectionalFocusNavigation({required this.child, super.key});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Shortcuts(
      shortcuts: const <ShortcutActivator, Intent>{
        SingleActivator(LogicalKeyboardKey.arrowUp): DirectionalFocusIntent(
          TraversalDirection.up,
        ),
        SingleActivator(LogicalKeyboardKey.arrowDown): DirectionalFocusIntent(
          TraversalDirection.down,
        ),
        SingleActivator(LogicalKeyboardKey.arrowLeft): DirectionalFocusIntent(
          TraversalDirection.left,
        ),
        SingleActivator(LogicalKeyboardKey.arrowRight): DirectionalFocusIntent(
          TraversalDirection.right,
        ),
      },
      child: Actions(
        actions: <Type, Action<Intent>>{
          DirectionalFocusIntent: CallbackAction<DirectionalFocusIntent>(
            onInvoke: (DirectionalFocusIntent intent) {
              FocusManager.instance.primaryFocus?.focusInDirection(
                intent.direction,
              );
              return null;
            },
          ),
        },
        child: FocusTraversalGroup(
          policy: ReadingOrderTraversalPolicy(),
          child: child,
        ),
      ),
    );
  }
}

class _TvFocusableState extends State<TvFocusable> {
  late final FocusNode _focusNode = FocusNode(debugLabel: widget.semanticLabel);
  bool _isFocused = false;

  @override
  void dispose() {
    _focusNode.dispose();
    super.dispose();
  }

  KeyEventResult _onKeyEvent(FocusNode node, KeyEvent event) {
    final isActivationKey =
        event.logicalKey == LogicalKeyboardKey.enter ||
        event.logicalKey == LogicalKeyboardKey.select ||
        event.logicalKey == LogicalKeyboardKey.space;
    if (widget.onActivate != null && event is KeyDownEvent && isActivationKey) {
      widget.onActivate?.call();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  void _handleFocusChange(bool hasFocus) {
    if (_isFocused != hasFocus) {
      setState(() => _isFocused = hasFocus);
    }
    if (hasFocus) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && _focusNode.hasFocus) {
          Scrollable.ensureVisible(
            context,
            alignmentPolicy: ScrollPositionAlignmentPolicy.keepVisibleAtEnd,
            duration: TvTheme.focusDuration,
            curve: TvTheme.focusCurve,
          );
        }
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return Actions(
      actions: <Type, Action<Intent>>{
        ActivateIntent: CallbackAction<ActivateIntent>(
          onInvoke: (ActivateIntent intent) {
            widget.onActivate?.call();
            return null;
          },
        ),
      },
      child: Focus(
        canRequestFocus: widget.onActivate != null,
        autofocus: widget.onActivate != null && widget.autofocus,
        focusNode: _focusNode,
        onFocusChange: _handleFocusChange,
        onKeyEvent: _onKeyEvent,
        child: Semantics(
          button: widget.onActivate != null,
          enabled: widget.onActivate != null,
          focused: _isFocused,
          label: widget.semanticLabel,
          child: MouseRegion(
            cursor: widget.onActivate == null
                ? MouseCursor.defer
                : SystemMouseCursors.click,
            onEnter: widget.onActivate == null
                ? null
                : (_) => _focusNode.requestFocus(),
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTap: widget.onActivate == null
                  ? null
                  : () {
                      _focusNode.requestFocus();
                      widget.onActivate?.call();
                    },
              child: widget.builder(context, _isFocused),
            ),
          ),
        ),
      ),
    );
  }
}

enum TvControlVariant { secondary, primary, selectable, icon }

class TvControlStyle {
  const TvControlStyle({
    required this.background,
    required this.foreground,
    required this.border,
  });

  final Color background;
  final Color foreground;
  final Color border;

  static TvControlStyle resolve(
    TvPalette tv, {
    required TvControlVariant variant,
    required bool isFocused,
    bool isSelected = false,
  }) {
    if (isFocused) {
      return TvControlStyle(
        background: tv.focusFill,
        foreground: tv.onFocus,
        border: tv.focus,
      );
    }
    if (variant == TvControlVariant.primary) {
      return TvControlStyle(
        background: tv.action,
        foreground: tv.onAction,
        border: tv.action,
      );
    }
    if (isSelected) {
      return TvControlStyle(
        background: tv.selected,
        foreground: tv.onSelected,
        border: tv.accent,
      );
    }
    return TvControlStyle(
      background: variant == TvControlVariant.icon
          ? Colors.transparent
          : tv.surfaceMuted,
      foreground: variant == TvControlVariant.selectable
          ? tv.accent
          : tv.primaryText,
      border: Colors.transparent,
    );
  }
}

class TvTopBar extends StatelessWidget {
  const TvTopBar({
    required this.metrics,
    required this.onLibraryActivate,
    required this.onSearchActivate,
    required this.onSettingsActivate,
    super.key,
  });

  final TvLayoutMetrics metrics;
  final VoidCallback onLibraryActivate;
  final VoidCallback onSearchActivate;
  final VoidCallback onSettingsActivate;

  @override
  Widget build(BuildContext context) {
    final iconSize = 22 * (metrics.topBarHeight / 64);
    return SizedBox(
      height: metrics.topBarHeight,
      child: Row(
        children: <Widget>[
          _ProfileSummary(compact: metrics.isCompact),
          SizedBox(width: metrics.gap * 2),
          Expanded(
            child: Center(
              child: Wrap(
                spacing: metrics.gap,
                children: <Widget>[
                  TvIconAction(
                    label: 'Home',
                    icon: Icons.dashboard_outlined,
                    iconSize: iconSize,
                  ),
                  TvIconAction(
                    label: 'Calendar',
                    icon: Icons.calendar_month_outlined,
                    iconSize: iconSize,
                  ),
                  if (!metrics.isCompact)
                    TvIconAction(
                      label: 'Full library',
                      icon: Icons.library_books_outlined,
                      iconSize: iconSize,
                      onActivate: onLibraryActivate,
                    ),
                  TvIconAction(
                    label: 'Search',
                    icon: Icons.search_rounded,
                    iconSize: iconSize,
                    onActivate: onSearchActivate,
                  ),
                  if (!metrics.isCompact)
                    TvIconAction(
                      label: 'Settings',
                      icon: Icons.settings_outlined,
                      iconSize: iconSize,
                      hasNotification: true,
                      onActivate: onSettingsActivate,
                    ),
                ],
              ),
            ),
          ),
          SizedBox(width: metrics.gap * 2),
          _SystemStatus(compact: metrics.isCompact),
        ],
      ),
    );
  }
}

class _ProfileSummary extends StatelessWidget {
  const _ProfileSummary({required this.compact});

  final bool compact;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: <Widget>[
        Container(
          width: compact ? 32 : 42,
          height: compact ? 32 : 42,
          decoration: const BoxDecoration(
            color: Color(0xFF6FC4CF),
            shape: BoxShape.circle,
            gradient: LinearGradient(
              begin: Alignment.topLeft,
              end: Alignment.bottomRight,
              colors: <Color>[Color(0xFFEC8B54), Color(0xFF277F9B)],
            ),
          ),
          child: const Icon(Icons.person_rounded, color: Colors.white),
        ),
        if (!compact) ...<Widget>[
          const SizedBox(width: 10),
          Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              Text(
                'Alex Morgan',
                style: TextStyle(fontWeight: FontWeight.w600),
              ),
              SizedBox(height: 2),
              Text(
                '12,480 points',
                style: TextStyle(fontSize: 12, color: tv.secondaryText),
              ),
            ],
          ),
        ],
      ],
    );
  }
}

class _SystemStatus extends StatelessWidget {
  const _SystemStatus({required this.compact});

  final bool compact;

  @override
  Widget build(BuildContext context) {
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: <Widget>[
        if (!compact) ...<Widget>[
          const Icon(Icons.mic_off_outlined, size: 18),
          const SizedBox(width: 14),
          const Icon(Icons.wifi_rounded, size: 18),
          const SizedBox(width: 14),
        ],
        const Icon(Icons.battery_full_rounded, size: 20),
        const SizedBox(width: 8),
        Text(
          compact ? '4:55' : '4:55 PM',
          style: const TextStyle(fontSize: 16, fontWeight: FontWeight.w600),
        ),
      ],
    );
  }
}

class TvIconAction extends StatelessWidget {
  const TvIconAction({
    required this.label,
    required this.icon,
    required this.iconSize,
    this.hasNotification = false,
    this.onActivate,
    super.key,
  });

  final String label;
  final IconData icon;
  final double iconSize;
  final bool hasNotification;
  final VoidCallback? onActivate;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return TvFocusable(
      semanticLabel: label,
      onActivate: onActivate ?? () => _showUnavailableMessage(context, label),
      builder: (BuildContext context, bool isFocused) {
        final style = TvControlStyle.resolve(
          tv,
          variant: TvControlVariant.icon,
          isFocused: isFocused,
        );
        return AnimatedContainer(
          duration: TvTheme.focusDuration,
          curve: TvTheme.focusCurve,
          width: iconSize + 18,
          height: iconSize + 18,
          decoration: BoxDecoration(
            color: style.background,
            border: Border.all(color: style.border, width: 2),
            borderRadius: BorderRadius.circular(8),
          ),
          child: Stack(
            alignment: Alignment.center,
            children: <Widget>[
              Icon(icon, size: iconSize, color: style.foreground),
              if (hasNotification)
                Positioned(
                  top: 5,
                  right: 5,
                  child: DecoratedBox(
                    decoration: BoxDecoration(
                      color: tv.accent,
                      shape: BoxShape.circle,
                    ),
                    child: SizedBox(width: 7, height: 7),
                  ),
                ),
            ],
          ),
        );
      },
    );
  }
}

class TvShelf extends StatelessWidget {
  const TvShelf({
    required this.section,
    required this.metrics,
    required this.onActivate,
    super.key,
  });

  final DashboardSection section;
  final TvLayoutMetrics metrics;
  final void Function(DashboardItem item, TvTileShape sourceShape) onActivate;

  @override
  Widget build(BuildContext context) {
    final tileWidth = section.shape == TvTileShape.square
        ? metrics.squareExtent
        : metrics.squareExtent * 1.92;
    final tileHeight = section.shape == TvTileShape.square
        ? metrics.squareExtent
        : metrics.squareExtent / 1.1;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        if (section.title case final String title)
          Padding(
            padding: metrics.pageInsets.copyWith(
              bottom: metrics.sectionTitleGap,
            ),
            child: Text(
              key: ValueKey<String>('shelf-title-${section.id}'),
              title,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: Theme.of(context).textTheme.titleMedium,
            ),
          ),
        SizedBox(
          height: tileHeight,
          child: ListView.separated(
            key: PageStorageKey<String>('shelf-${section.id}'),
            scrollDirection: Axis.horizontal,
            padding: metrics.pageInsets,
            itemCount: section.items.length,
            scrollCacheExtent: ScrollCacheExtent.pixels(tileWidth * 2),
            separatorBuilder: (BuildContext context, int index) =>
                SizedBox(width: metrics.gap),
            itemBuilder: (BuildContext context, int index) {
              final item = section.items[index];
              return SizedBox(
                width: tileWidth,
                child: TvContentTile(
                  key: ValueKey<String>('tile-${item.id}'),
                  item: item,
                  shape: section.shape,
                  autofocus: section.id == 'pinned' && index == 0,
                  onActivate: () => onActivate(item, section.shape),
                ),
              );
            },
          ),
        ),
      ],
    );
  }
}

class TvContentTile extends StatelessWidget {
  const TvContentTile({
    required this.item,
    required this.shape,
    required this.onActivate,
    super.key,
    this.autofocus = false,
  });

  final DashboardItem item;
  final TvTileShape shape;
  final bool autofocus;
  final VoidCallback onActivate;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return RepaintBoundary(
      child: TvFocusable(
        semanticLabel: item.title,
        autofocus: autofocus,
        onActivate: onActivate,
        builder: (BuildContext context, bool isFocused) {
          return AnimatedScale(
            scale: isFocused ? 1.045 : 1,
            duration: TvTheme.focusDuration,
            curve: TvTheme.focusCurve,
            child: AnimatedContainer(
              duration: TvTheme.focusDuration,
              curve: TvTheme.focusCurve,
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(8),
                border: Border.all(
                  color: isFocused ? tv.focus : Colors.transparent,
                  width: isFocused ? 3 : 0,
                ),
                boxShadow: isFocused
                    ? const <BoxShadow>[
                        BoxShadow(
                          color: Color(0xA6000000),
                          blurRadius: 18,
                          spreadRadius: 2,
                        ),
                      ]
                    : null,
              ),
              child: TvArtwork(item: item, shape: shape, isFocused: isFocused),
            ),
          );
        },
      ),
    );
  }
}

String contentArtworkHeroTag(DashboardItem item) =>
    'content-artwork-${item.id}';

class TvArtwork extends StatelessWidget {
  const TvArtwork({
    required this.item,
    required this.shape,
    required this.isFocused,
    super.key,
  });

  final DashboardItem item;
  final TvTileShape shape;
  final bool isFocused;

  @override
  Widget build(BuildContext context) {
    return Stack(
      children: <Widget>[
        Positioned.fill(
          child: Hero(
            tag: contentArtworkHeroTag(item),
            child: TvArtworkVisual(item: item, shape: shape),
          ),
        ),
        Align(
          alignment: Alignment.bottomCenter,
          child: _ArtworkCaption(
            item: item,
            shape: shape,
            isFocused: isFocused,
          ),
        ),
      ],
    );
  }
}

class TvArtworkVisual extends StatelessWidget {
  const TvArtworkVisual({required this.item, required this.shape, super.key});

  final DashboardItem item;
  final TvTileShape shape;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final iconSize = shape == TvTileShape.square ? 42.0 : 48.0;
    return DecoratedBox(
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(6),
        gradient: LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: item.colors,
        ),
      ),
      child: Stack(
        children: <Widget>[
          Align(
            alignment: shape == TvTileShape.square
                ? const Alignment(0, -0.15)
                : const Alignment(0.44, -0.08),
            child: Icon(
              item.icon,
              size: iconSize,
              color: Colors.white.withValues(alpha: 0.92),
            ),
          ),
          if (item.badge case final String badge)
            Positioned(
              top: 10,
              left: 10,
              child: DecoratedBox(
                decoration: BoxDecoration(
                  color: tv.canvas.withValues(alpha: 0.72),
                  borderRadius: BorderRadius.circular(4),
                ),
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 7,
                    vertical: 4,
                  ),
                  child: Text(
                    badge.toUpperCase(),
                    style: const TextStyle(
                      fontSize: 10,
                      fontWeight: FontWeight.w700,
                      letterSpacing: 0.5,
                    ),
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _ArtworkCaption extends StatelessWidget {
  const _ArtworkCaption({
    required this.item,
    required this.shape,
    required this.isFocused,
  });

  final DashboardItem item;
  final TvTileShape shape;
  final bool isFocused;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tv.canvas.withValues(
          alpha: shape == TvTileShape.square ? 0.78 : 0.66,
        ),
        borderRadius: const BorderRadius.vertical(bottom: Radius.circular(6)),
      ),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(12, 9, 12, 10),
        child: Row(
          children: <Widget>[
            Expanded(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    item.title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(fontWeight: FontWeight.w600),
                  ),
                  if (shape == TvTileShape.landscape &&
                      item.description != null) ...<Widget>[
                    const SizedBox(height: 2),
                    Text(
                      item.description!,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(fontSize: 12, color: tv.secondaryText),
                    ),
                  ],
                ],
              ),
            ),
            if (isFocused && shape == TvTileShape.landscape)
              const Padding(
                padding: EdgeInsets.only(left: 8),
                child: Icon(Icons.arrow_forward_rounded, size: 18),
              ),
          ],
        ),
      ),
    );
  }
}

void _showUnavailableMessage(BuildContext context, String itemName) {
  ScaffoldMessenger.of(context).hideCurrentSnackBar();
  ScaffoldMessenger.of(
    context,
  ).showSnackBar(SnackBar(content: Text('$itemName is ready to launch.')));
}
