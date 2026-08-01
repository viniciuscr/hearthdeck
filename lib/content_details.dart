import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';

import 'dashboard_models.dart';
import 'external_link.dart';
import 'tv_components.dart';
import 'tv_theme.dart';

class ContentDetailsPage extends StatefulWidget {
  const ContentDetailsPage({
    required this.item,
    required this.sourceShape,
    super.key,
    this.onPrimaryAction,
    this.externalLink,
  });

  final DashboardItem item;
  final TvTileShape sourceShape;
  final Future<void> Function(DashboardItem item)? onPrimaryAction;
  final ExternalLink? externalLink;

  @override
  State<ContentDetailsPage> createState() => _ContentDetailsPageState();
}

class _ContentDetailsPageState extends State<ContentDetailsPage> {
  @override
  Widget build(BuildContext context) {
    final details = contentDetailsFor(widget.item);
    final factSections = details.factSections.isEmpty
        ? <ContentFactSection>[
            ContentFactSection(
              title: _factsTitle(widget.item.kind),
              facts: details.facts,
            ),
          ]
        : details.factSections;
    return LayoutBuilder(
      builder: (BuildContext context, BoxConstraints constraints) {
        final layout = _ContentDetailsLayout.fromConstraints(constraints);
        // Escape/back is handled globally (see main.dart's HardwareKeyboard
        // listener), regardless of what has focus on this screen.
        return TvDirectionalFocusNavigation(
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
                                factSections: factSections,
                                sourceShape: widget.sourceShape,
                                layout: layout,
                                onPrimaryAction: widget.onPrimaryAction,
                                externalLink:
                                    widget.externalLink ??
                                    const NativeExternalLink(),
                              ),
                            ),
                            SliverToBoxAdapter(
                              child: SizedBox(height: layout.sectionGap * 0.75),
                            ),
                            SliverToBoxAdapter(
                              child: _DescriptionPanel(
                                description: details.summary,
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
                                itemBuilder: (BuildContext context, int index) {
                                  return _GalleryCard(
                                    item: details.gallery[index],
                                    onActivate: () => _showGalleryFeedback(
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
    required this.factSections,
    required this.sourceShape,
    required this.layout,
    this.onPrimaryAction,
    required this.externalLink,
  });

  final DashboardItem item;
  final ContentDetails details;
  final List<ContentFactSection> factSections;
  final TvTileShape sourceShape;
  final _ContentDetailsLayout layout;
  final Future<void> Function(DashboardItem item)? onPrimaryAction;
  final ExternalLink externalLink;

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
      factSections: factSections,
      layout: layout,
      onPrimaryAction: onPrimaryAction,
      externalLink: externalLink,
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
    required this.factSections,
    required this.layout,
    this.onPrimaryAction,
    required this.externalLink,
  });

  final DashboardItem item;
  final ContentDetails details;
  final List<ContentFactSection> factSections;
  final _ContentDetailsLayout layout;
  final Future<void> Function(DashboardItem item)? onPrimaryAction;
  final ExternalLink externalLink;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        _ContentKindLabel(kind: item.kind),
        SizedBox(height: layout.itemGap * 0.5),
        Text(item.title, style: theme.textTheme.displaySmall),
        if (details.highlights.isNotEmpty) ...<Widget>[
          SizedBox(height: layout.itemGap * 0.75),
          Wrap(
            spacing: layout.itemGap * 0.7,
            runSpacing: layout.itemGap * 0.7,
            children: details.highlights
                .map((ContentFact fact) => _DetailHighlight(fact: fact))
                .toList(growable: false),
          ),
        ],
        SizedBox(height: layout.itemGap),
        _MetadataPanel(
          sections: factSections,
          progress: details.progress,
          layout: layout,
        ),
        SizedBox(height: layout.headerGap),
        Wrap(
          spacing: layout.itemGap,
          runSpacing: layout.itemGap,
          children: details.actions
              .asMap()
              .entries
              .map(
                (MapEntry<int, ContentAction> entry) => TvDetailAction(
                  action: entry.value,
                  autofocus:
                      entry.value.isPrimary ||
                      (entry.key == 0 &&
                          !details.actions.any(
                            (ContentAction action) => action.isPrimary,
                          )),
                  onActivate: () async {
                    final action = entry.value;
                    if (action.id == 'launch' && onPrimaryAction != null) {
                      await onPrimaryAction!(item);
                    } else if (action.url case final String url) {
                      try {
                        await externalLink.open(url);
                      } on Object catch (error) {
                        if (context.mounted) {
                          ScaffoldMessenger.of(context).showSnackBar(
                            SnackBar(
                              content: Text('Could not open link: $error'),
                            ),
                          );
                        }
                      }
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

class _DescriptionPanel extends StatelessWidget {
  const _DescriptionPanel({required this.description, required this.layout});

  final String description;
  final _ContentDetailsLayout layout;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tv.surface.withValues(alpha: 0.9),
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: tv.surfaceMuted),
      ),
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            Text(
              'DESCRIPTION',
              style: TextStyle(
                color: tv.secondaryText,
                fontSize: 12,
                fontWeight: FontWeight.w700,
                letterSpacing: 0.8,
              ),
            ),
            SizedBox(height: layout.itemGap * 0.75),
            Text(
              description,
              style: Theme.of(context).textTheme.titleMedium?.copyWith(
                color: tv.primaryText,
                height: 1.5,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _DetailHighlight extends StatelessWidget {
  const _DetailHighlight({required this.fact});

  final ContentFact fact;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tv.surface.withValues(alpha: 0.92),
        borderRadius: BorderRadius.circular(999),
        border: Border.all(color: tv.accent.withValues(alpha: 0.5)),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            Icon(fact.icon, size: 18, color: tv.accent),
            const SizedBox(width: 7),
            Text(
              fact.label,
              style: const TextStyle(fontWeight: FontWeight.w700),
            ),
            const SizedBox(width: 6),
            Text(fact.value, style: TextStyle(color: tv.secondaryText)),
          ],
        ),
      ),
    );
  }
}

class _MetadataPanel extends StatelessWidget {
  const _MetadataPanel({
    required this.sections,
    required this.progress,
    required this.layout,
  });

  final List<ContentFactSection> sections;
  final ContentProgress? progress;
  final _ContentDetailsLayout layout;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final visibleSections = sections
        .where((ContentFactSection section) => section.facts.isNotEmpty)
        .toList(growable: false);
    if (visibleSections.isEmpty && progress == null) {
      return const SizedBox.shrink();
    }
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tv.surface.withValues(alpha: 0.9),
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: tv.surfaceMuted),
      ),
      child: Padding(
        padding: const EdgeInsets.all(18),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            for (
              var index = 0;
              index < visibleSections.length;
              index++
            ) ...<Widget>[
              if (index > 0) ...<Widget>[
                SizedBox(height: layout.itemGap),
                Divider(color: tv.surfaceMuted, height: 1),
                SizedBox(height: layout.itemGap),
              ],
              Text(
                visibleSections[index].title.toUpperCase(),
                style: TextStyle(
                  color: tv.secondaryText,
                  fontSize: 12,
                  fontWeight: FontWeight.w700,
                  letterSpacing: 0.8,
                ),
              ),
              SizedBox(height: layout.itemGap * 0.65),
              LayoutBuilder(
                builder: (BuildContext context, BoxConstraints constraints) {
                  final wide = constraints.maxWidth >= 640;
                  final width = wide
                      ? (constraints.maxWidth - layout.itemGap) / 2
                      : constraints.maxWidth;
                  return Wrap(
                    spacing: layout.itemGap,
                    runSpacing: layout.itemGap * 0.7,
                    children: visibleSections[index].facts
                        .map(
                          (ContentFact fact) => SizedBox(
                            width: width,
                            child: _CompactFact(fact: fact),
                          ),
                        )
                        .toList(growable: false),
                  );
                },
              ),
            ],
            if (progress case final ContentProgress progress) ...<Widget>[
              if (visibleSections.isNotEmpty) ...<Widget>[
                SizedBox(height: layout.itemGap),
                Divider(color: tv.surfaceMuted, height: 1),
                SizedBox(height: layout.itemGap),
              ],
              _CompactProgress(progress: progress),
            ],
          ],
        ),
      ),
    );
  }
}

class _CompactFact extends StatelessWidget {
  const _CompactFact({required this.fact});

  final ContentFact fact;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        Icon(fact.icon, size: 18, color: tv.accent),
        const SizedBox(width: 9),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              Text(fact.label, style: TextStyle(color: tv.secondaryText)),
              const SizedBox(height: 2),
              Text(
                fact.value,
                maxLines: 4,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(
                  fontWeight: FontWeight.w600,
                  height: 1.25,
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _CompactProgress extends StatelessWidget {
  const _CompactProgress({required this.progress});

  final ContentProgress progress;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Column(
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
                if (item.artworkUrl case final String artworkUrl)
                  Positioned.fill(
                    child: ClipRRect(
                      borderRadius: BorderRadius.circular(8),
                      child: Image.network(
                        artworkUrl,
                        fit: BoxFit.cover,
                        headers: item.artworkHeaders,
                        errorBuilder:
                            (
                              BuildContext context,
                              Object error,
                              StackTrace? stackTrace,
                            ) => const SizedBox.shrink(),
                      ),
                    ),
                  ),
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
