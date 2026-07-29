import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import 'dashboard_models.dart';
import 'tv_components.dart';
import 'tv_theme.dart';

class ContentDetailsPage extends StatefulWidget {
  const ContentDetailsPage({
    required this.item,
    required this.sourceShape,
    super.key,
    this.onPrimaryAction,
  });

  final DashboardItem item;
  final TvTileShape sourceShape;
  final Future<void> Function(DashboardItem item)? onPrimaryAction;

  @override
  State<ContentDetailsPage> createState() => _ContentDetailsPageState();
}

class _ContentDetailsPageState extends State<ContentDetailsPage> {
  late final FocusNode _routeFocusNode = FocusNode(
    debugLabel: '${widget.item.title} details',
    canRequestFocus: false,
  );

  @override
  void dispose() {
    _routeFocusNode.dispose();
    super.dispose();
  }

  KeyEventResult _handleKeyEvent(FocusNode node, KeyEvent event) {
    if (event is KeyDownEvent &&
        event.logicalKey == LogicalKeyboardKey.escape) {
      Navigator.of(context).maybePop();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    final details = contentDetailsFor(widget.item);
    return LayoutBuilder(
      builder: (BuildContext context, BoxConstraints constraints) {
        final layout = _ContentDetailsLayout.fromConstraints(constraints);
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
              focusNode: _routeFocusNode,
              onKeyEvent: _handleKeyEvent,
              child: Scaffold(
                body: SafeArea(
                  child: Stack(
                    children: <Widget>[
                      _DetailsBackdrop(item: widget.item),
                      CustomScrollView(
                        scrollCacheExtent: ScrollCacheExtent.viewport(2),
                        slivers: <Widget>[
                          SliverPadding(
                            padding: EdgeInsets.only(
                              left: layout.pagePadding,
                              right: layout.pagePadding,
                              top: layout.pagePadding * 0.55,
                              bottom: layout.sectionGap,
                            ),
                            sliver: SliverMainAxisGroup(
                              slivers: <Widget>[
                                SliverToBoxAdapter(
                                  child: _DetailsHeader(
                                    item: widget.item,
                                    details: details,
                                    sourceShape: widget.sourceShape,
                                    layout: layout,
                                    onPrimaryAction: widget.onPrimaryAction,
                                  ),
                                ),
                                SliverToBoxAdapter(
                                  child: SizedBox(height: layout.sectionGap),
                                ),
                                SliverToBoxAdapter(
                                  child: _DetailsSectionTitle(
                                    title: _factsTitle(widget.item.kind),
                                  ),
                                ),
                                SliverToBoxAdapter(
                                  child: SizedBox(height: layout.itemGap),
                                ),
                                SliverToBoxAdapter(
                                  child: _FactGrid(
                                    facts: details.facts,
                                    progress: details.progress,
                                    layout: layout,
                                  ),
                                ),
                                SliverToBoxAdapter(
                                  child: SizedBox(height: layout.sectionGap),
                                ),
                                SliverToBoxAdapter(
                                  child: _DetailsSectionTitle(
                                    title: details.galleryTitle,
                                  ),
                                ),
                                SliverToBoxAdapter(
                                  child: SizedBox(height: layout.itemGap),
                                ),
                                if (details.gallery.isNotEmpty)
                                  SliverGrid.builder(
                                    itemCount: details.gallery.length,
                                    gridDelegate:
                                        SliverGridDelegateWithMaxCrossAxisExtent(
                                          maxCrossAxisExtent:
                                              layout.galleryTileExtent,
                                          mainAxisSpacing: layout.itemGap,
                                          crossAxisSpacing: layout.itemGap,
                                          childAspectRatio:
                                              layout.galleryAspectRatio,
                                        ),
                                    itemBuilder:
                                        (BuildContext context, int index) {
                                          return _GalleryCard(
                                            item: details.gallery[index],
                                            onActivate: () =>
                                                _showGalleryFeedback(
                                                  context,
                                                  details.gallery[index],
                                                ),
                                          );
                                        },
                                  )
                                else
                                  const SliverToBoxAdapter(
                                    child: _NoDetailGallery(),
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
            ),
          ),
        );
      },
    );
  }
}

class _ContentDetailsLayout {
  const _ContentDetailsLayout._({
    required this.pagePadding,
    required this.headerGap,
    required this.itemGap,
    required this.sectionGap,
    required this.artworkExtent,
    required this.galleryTileExtent,
    required this.galleryAspectRatio,
    required this.isHorizontalHeader,
  });

  factory _ContentDetailsLayout.fromConstraints(BoxConstraints constraints) {
    final width = constraints.maxWidth;
    final height = constraints.maxHeight;
    final scale = (math.min(width, height) / 720).clamp(0.72, 1.3).toDouble();
    final pagePadding = (width * 0.05).clamp(24 * scale, 96 * scale).toDouble();
    final contentWidth = math.max(width - (pagePadding * 2), 1);

    return _ContentDetailsLayout._(
      pagePadding: pagePadding,
      headerGap: 24 * scale,
      itemGap: 14 * scale,
      sectionGap: 40 * scale,
      artworkExtent: (contentWidth * 0.235)
          .clamp(180 * scale, 350 * scale)
          .toDouble(),
      galleryTileExtent: (contentWidth * 0.3)
          .clamp(240 * scale, 480 * scale)
          .toDouble(),
      galleryAspectRatio: 16 / 9,
      isHorizontalHeader: contentWidth >= 840 * scale,
    );
  }

  final double pagePadding;
  final double headerGap;
  final double itemGap;
  final double sectionGap;
  final double artworkExtent;
  final double galleryTileExtent;
  final double galleryAspectRatio;
  final bool isHorizontalHeader;
}

class _DetailsHeader extends StatelessWidget {
  const _DetailsHeader({
    required this.item,
    required this.details,
    required this.sourceShape,
    required this.layout,
    this.onPrimaryAction,
  });

  final DashboardItem item;
  final ContentDetails details;
  final TvTileShape sourceShape;
  final _ContentDetailsLayout layout;
  final Future<void> Function(DashboardItem item)? onPrimaryAction;

  @override
  Widget build(BuildContext context) {
    final artwork = SizedBox(
      width: layout.artworkExtent,
      height: layout.artworkExtent,
      child: Hero(
        tag: contentArtworkHeroTag(item),
        child: TvArtworkVisual(item: item, shape: sourceShape),
      ),
    );
    final information = _DetailsInformation(
      item: item,
      details: details,
      layout: layout,
      onPrimaryAction: onPrimaryAction,
    );

    if (layout.isHorizontalHeader) {
      return Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          artwork,
          SizedBox(width: layout.headerGap),
          Expanded(child: information),
        ],
      );
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        Center(child: artwork),
        SizedBox(height: layout.headerGap),
        information,
      ],
    );
  }
}

class _DetailsInformation extends StatelessWidget {
  const _DetailsInformation({
    required this.item,
    required this.details,
    required this.layout,
    this.onPrimaryAction,
  });

  final DashboardItem item;
  final ContentDetails details;
  final _ContentDetailsLayout layout;
  final Future<void> Function(DashboardItem item)? onPrimaryAction;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tv = TvPalette.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        _ContentKindLabel(kind: item.kind),
        SizedBox(height: layout.itemGap * 0.5),
        Text(item.title, style: theme.textTheme.displaySmall),
        SizedBox(height: layout.itemGap * 0.75),
        ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 720),
          child: Text(
            details.summary,
            style: theme.textTheme.titleMedium?.copyWith(
              color: tv.secondaryText,
              height: 1.35,
            ),
          ),
        ),
        SizedBox(height: layout.headerGap),
        Wrap(
          spacing: layout.itemGap,
          runSpacing: layout.itemGap,
          children: details.actions
              .map(
                (ContentAction action) => TvDetailAction(
                  action: action,
                  autofocus: action.isPrimary,
                  onActivate: () async {
                    if (action.isPrimary && onPrimaryAction != null) {
                      await onPrimaryAction!(item);
                    } else if (context.mounted) {
                      _showDetailFeedback(context, item, action);
                    }
                  },
                ),
              )
              .toList(growable: false),
        ),
      ],
    );
  }
}

class _ContentKindLabel extends StatelessWidget {
  const _ContentKindLabel({required this.kind});

  final TvContentKind kind;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final label = switch (kind) {
      TvContentKind.game => 'Game',
      TvContentKind.media => 'Media',
      TvContentKind.application => 'Application',
      TvContentKind.system => 'System',
    };
    return Text(
      label.toUpperCase(),
      style: TextStyle(
        color: tv.accent,
        fontSize: 12,
        fontWeight: FontWeight.w700,
        letterSpacing: 1.2,
      ),
    );
  }
}

class TvDetailAction extends StatelessWidget {
  const TvDetailAction({
    required this.action,
    required this.onActivate,
    super.key,
    this.autofocus = false,
  });

  final ContentAction action;
  final VoidCallback onActivate;
  final bool autofocus;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return TvFocusable(
      semanticLabel: action.label,
      autofocus: autofocus,
      onActivate: onActivate,
      builder: (BuildContext context, bool isFocused) {
        final style = TvControlStyle.resolve(
          tv,
          variant: action.isPrimary
              ? TvControlVariant.primary
              : TvControlVariant.secondary,
          isFocused: isFocused,
        );
        return AnimatedContainer(
          duration: TvTheme.focusDuration,
          curve: TvTheme.focusCurve,
          constraints: const BoxConstraints(minHeight: 52, minWidth: 148),
          padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 12),
          decoration: BoxDecoration(
            color: style.background,
            borderRadius: BorderRadius.circular(8),
            border: Border.all(color: style.border, width: isFocused ? 2 : 1),
            boxShadow: isFocused
                ? const <BoxShadow>[
                    BoxShadow(
                      color: Color(0x80000000),
                      blurRadius: 14,
                      spreadRadius: 1,
                    ),
                  ]
                : null,
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              Icon(action.icon, color: style.foreground, size: 21),
              const SizedBox(width: 10),
              Flexible(
                child: Text(
                  action.label,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    color: style.foreground,
                    fontWeight: FontWeight.w700,
                  ),
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}

class _DetailsSectionTitle extends StatelessWidget {
  const _DetailsSectionTitle({required this.title});

  final String title;

  @override
  Widget build(BuildContext context) {
    return Text(title, style: Theme.of(context).textTheme.titleLarge);
  }
}

class _FactGrid extends StatelessWidget {
  const _FactGrid({
    required this.facts,
    required this.progress,
    required this.layout,
  });

  final List<ContentFact> facts;
  final ContentProgress? progress;
  final _ContentDetailsLayout layout;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (BuildContext context, BoxConstraints constraints) {
        final minPanelWidth = 210.0;
        final columnCount = math
            .max(1, (constraints.maxWidth / minPanelWidth).floor())
            .clamp(1, 4);
        final panels = <Widget>[
          ...facts.map((ContentFact fact) => _FactPanel(fact: fact)),
          if (progress case final ContentProgress progress)
            _ProgressPanel(progress: progress),
        ];

        return GridView.builder(
          shrinkWrap: true,
          primary: false,
          itemCount: panels.length,
          gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
            crossAxisCount: columnCount,
            mainAxisSpacing: layout.itemGap,
            crossAxisSpacing: layout.itemGap,
            childAspectRatio: 2.1,
          ),
          itemBuilder: (BuildContext context, int index) => panels[index],
        );
      },
    );
  }
}

class _FactPanel extends StatelessWidget {
  const _FactPanel({required this.fact});

  final ContentFact fact;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tv.surface.withValues(alpha: 0.9),
        borderRadius: BorderRadius.circular(10),
      ),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Row(
          children: <Widget>[
            Icon(fact.icon, color: tv.accent),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(fact.label, style: TextStyle(color: tv.secondaryText)),
                  const SizedBox(height: 4),
                  Text(
                    fact.value,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(fontWeight: FontWeight.w700),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ProgressPanel extends StatelessWidget {
  const _ProgressPanel({required this.progress});

  final ContentProgress progress;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tv.surface.withValues(alpha: 0.9),
        borderRadius: BorderRadius.circular(10),
      ),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            Text(progress.label, style: TextStyle(color: tv.secondaryText)),
            const SizedBox(height: 7),
            Row(
              children: <Widget>[
                Expanded(
                  child: LinearProgressIndicator(
                    value: progress.value,
                    minHeight: 7,
                    borderRadius: BorderRadius.circular(8),
                    backgroundColor: tv.surfaceMuted,
                    color: tv.accent,
                  ),
                ),
                const SizedBox(width: 10),
                Text(
                  progress.summary,
                  style: const TextStyle(fontWeight: FontWeight.w700),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

class _GalleryCard extends StatelessWidget {
  const _GalleryCard({required this.item, required this.onActivate});

  final ContentGalleryItem item;
  final VoidCallback onActivate;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return RepaintBoundary(
      child: TvFocusable(
        semanticLabel: item.label,
        onActivate: onActivate,
        builder: (BuildContext context, bool isFocused) {
          return AnimatedContainer(
            duration: TvTheme.focusDuration,
            curve: TvTheme.focusCurve,
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(10),
              border: Border.all(
                color: isFocused ? tv.focus : Colors.transparent,
                width: isFocused ? 3 : 0,
              ),
              gradient: LinearGradient(
                begin: Alignment.topLeft,
                end: Alignment.bottomRight,
                colors: item.colors,
              ),
            ),
            child: Stack(
              children: <Widget>[
                Center(
                  child: Icon(
                    item.icon,
                    size: 54,
                    color: Colors.white.withValues(alpha: 0.86),
                  ),
                ),
                Positioned(
                  left: 12,
                  right: 12,
                  bottom: 10,
                  child: Text(
                    item.label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(fontWeight: FontWeight.w600),
                  ),
                ),
              ],
            ),
          );
        },
      ),
    );
  }
}

class _NoDetailGallery extends StatelessWidget {
  const _NoDetailGallery();

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Padding(
      padding: EdgeInsets.symmetric(vertical: 28),
      child: Text(
        'No screenshots or media supplied by this metadata source.',
        style: TextStyle(color: tv.secondaryText),
      ),
    );
  }
}

class _DetailsBackdrop extends StatelessWidget {
  const _DetailsBackdrop({required this.item});

  final DashboardItem item;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Positioned.fill(
      child: DecoratedBox(
        decoration: BoxDecoration(
          gradient: RadialGradient(
            center: const Alignment(0.78, -0.55),
            radius: 1.25,
            colors: <Color>[
              item.colors.first.withValues(alpha: 0.82),
              tv.canvas,
            ],
            stops: const <double>[0, 0.7],
          ),
        ),
      ),
    );
  }
}

String _factsTitle(TvContentKind kind) => switch (kind) {
  TvContentKind.game => 'Your game stats',
  TvContentKind.media => 'Playback details',
  TvContentKind.application => 'Application information',
  TvContentKind.system => 'At a glance',
};

void _showDetailFeedback(
  BuildContext context,
  DashboardItem item,
  ContentAction action,
) {
  ScaffoldMessenger.of(context).hideCurrentSnackBar();
  ScaffoldMessenger.of(context).showSnackBar(
    SnackBar(content: Text('${action.label} selected for ${item.title}.')),
  );
}

void _showGalleryFeedback(BuildContext context, ContentGalleryItem item) {
  ScaffoldMessenger.of(context).hideCurrentSnackBar();
  ScaffoldMessenger.of(
    context,
  ).showSnackBar(SnackBar(content: Text('${item.label} selected.')));
}
