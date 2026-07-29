import 'package:flutter/material.dart';

import 'tv_components.dart';
import 'tv_theme.dart';

class TvTwoPaneLayout extends StatelessWidget {
  const TvTwoPaneLayout({required this.rail, required this.content, super.key});

  final Widget rail;
  final Widget content;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: <Widget>[
        rail,
        Expanded(child: content),
      ],
    );
  }
}

class TvNavigationRailItem {
  const TvNavigationRailItem({
    required this.id,
    required this.label,
    required this.icon,
    required this.isSelected,
    required this.onActivate,
  });

  final String id;
  final String label;
  final IconData icon;
  final bool isSelected;
  final VoidCallback onActivate;
}

class TvNavigationRail extends StatelessWidget {
  const TvNavigationRail({
    required this.width,
    required this.compact,
    required this.items,
    super.key,
    this.headerBuilder,
    this.footerBuilder,
  });

  final double width;
  final bool compact;
  final List<TvNavigationRailItem> items;
  final Widget Function(BuildContext context, bool compact)? headerBuilder;
  final Widget Function(BuildContext context, bool compact)? footerBuilder;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final headerBuilder = this.headerBuilder;
    final footerBuilder = this.footerBuilder;
    final resolvedWidth = compact ? width.clamp(64.0, double.infinity) : width;
    return SizedBox(
      width: resolvedWidth,
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: tv.canvas.withValues(alpha: 0.72),
          border: Border(right: BorderSide(color: tv.borderSubtle)),
        ),
        child: Padding(
          padding: EdgeInsets.symmetric(
            horizontal: compact ? 8 : 20,
            vertical: 24,
          ),
          child: Column(
            crossAxisAlignment: compact
                ? CrossAxisAlignment.center
                : CrossAxisAlignment.start,
            children: <Widget>[
              if (headerBuilder != null) ...<Widget>[
                headerBuilder(context, compact),
                const SizedBox(height: 34),
              ],
              Expanded(
                child: ListView.separated(
                  primary: false,
                  itemCount: items.length,
                  separatorBuilder: (BuildContext context, int index) =>
                      const SizedBox(height: 7),
                  itemBuilder: (BuildContext context, int index) {
                    final item = items[index];
                    return _TvNavigationRailButton(
                      item: item,
                      compact: compact,
                    );
                  },
                ),
              ),
              if (footerBuilder != null) ...<Widget>[
                const SizedBox(height: 16),
                footerBuilder(context, compact),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class TvProfileRailHeader extends StatelessWidget {
  const TvProfileRailHeader({
    required this.name,
    required this.compact,
    super.key,
    this.icon = Icons.person_rounded,
  });

  final String name;
  final bool compact;
  final IconData icon;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: <Widget>[
        Container(
          width: compact ? 36 : 42,
          height: compact ? 36 : 42,
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            gradient: LinearGradient(colors: <Color>[tv.accent, tv.info]),
          ),
          child: Icon(icon),
        ),
        if (!compact) ...<Widget>[
          const SizedBox(width: 11),
          Text(name, style: const TextStyle(fontWeight: FontWeight.w700)),
        ],
      ],
    );
  }
}

class TvOptionCard extends StatelessWidget {
  const TvOptionCard({
    required this.label,
    required this.icon,
    required this.onActivate,
    super.key,
    this.description,
    this.autofocus = false,
  });

  final String label;
  final String? description;
  final IconData icon;
  final VoidCallback onActivate;
  final bool autofocus;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return RepaintBoundary(
      child: TvFocusable(
        semanticLabel: label,
        autofocus: autofocus,
        onActivate: onActivate,
        builder: (BuildContext context, bool isFocused) {
          final style = TvControlStyle.resolve(
            tv,
            variant: TvControlVariant.secondary,
            isFocused: isFocused,
          );
          return AnimatedContainer(
            duration: TvTheme.focusDuration,
            curve: TvTheme.focusCurve,
            padding: const EdgeInsets.all(14),
            decoration: BoxDecoration(
              color: isFocused ? style.background : tv.surface,
              borderRadius: BorderRadius.circular(10),
              border: Border.all(color: style.border, width: 2),
              boxShadow: isFocused
                  ? const <BoxShadow>[
                      BoxShadow(
                        color: Color(0x70000000),
                        blurRadius: 14,
                        spreadRadius: 1,
                      ),
                    ]
                  : null,
            ),
            child: Row(
              children: <Widget>[
                Icon(
                  icon,
                  size: 28,
                  color: isFocused ? style.foreground : tv.accent,
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: <Widget>[
                      Text(
                        label,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          color: isFocused ? style.foreground : tv.primaryText,
                          fontWeight: FontWeight.w700,
                        ),
                      ),
                      if (description
                          case final String description) ...<Widget>[
                        const SizedBox(height: 5),
                        Text(
                          description,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            color: isFocused
                                ? style.foreground.withValues(alpha: 0.76)
                                : tv.secondaryText,
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
                if (isFocused)
                  Icon(Icons.arrow_forward_rounded, color: style.foreground),
              ],
            ),
          );
        },
      ),
    );
  }
}

class _TvNavigationRailButton extends StatelessWidget {
  const _TvNavigationRailButton({required this.item, required this.compact});

  final TvNavigationRailItem item;
  final bool compact;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return TvFocusable(
      semanticLabel: item.label,
      autofocus: item.isSelected,
      onActivate: item.onActivate,
      builder: (BuildContext context, bool isFocused) {
        final style = TvControlStyle.resolve(
          tv,
          variant: TvControlVariant.selectable,
          isFocused: isFocused,
          isSelected: item.isSelected,
        );
        return AnimatedContainer(
          duration: TvTheme.focusDuration,
          curve: TvTheme.focusCurve,
          height: 52,
          padding: EdgeInsets.symmetric(horizontal: compact ? 0 : 14),
          decoration: BoxDecoration(
            color: style.background,
            borderRadius: BorderRadius.circular(8),
            border: Border.all(color: style.border, width: 2),
          ),
          child: Row(
            mainAxisAlignment: compact
                ? MainAxisAlignment.center
                : MainAxisAlignment.start,
            children: <Widget>[
              Icon(item.icon, color: style.foreground),
              if (!compact) ...<Widget>[
                const SizedBox(width: 14),
                Expanded(
                  child: Text(
                    item.label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(fontWeight: FontWeight.w600),
                  ),
                ),
              ],
            ],
          ),
        );
      },
    );
  }
}
