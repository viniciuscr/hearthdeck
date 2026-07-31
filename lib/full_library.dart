import 'dart:math' as math;
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import 'catalog/catalog_repository.dart';
import 'catalog/catalog_repository_factory.dart';
import 'content_details.dart';
import 'dashboard_models.dart';
import 'library_filters.dart';
import 'library_models.dart';
import 'search.dart';
import 'tv_components.dart';
import 'tv_theme.dart';
import 'tv_two_pane.dart';
import 'virtual_keyboard.dart';

class FullLibraryPage extends StatefulWidget {
  const FullLibraryPage({super.key, this.catalogRepository});

  final CatalogRepository? catalogRepository;

  @override
  State<FullLibraryPage> createState() => _FullLibraryPageState();
}

class _FullLibraryPageState extends State<FullLibraryPage> {
  final GlobalKey<ScaffoldState> _scaffoldKey = GlobalKey<ScaffoldState>();
  late final CatalogRepository _catalogRepository =
      widget.catalogRepository ?? createCatalogRepository();
  LibraryCategory _category = LibraryCategory.games;
  var _sourceIndex = 0;
  var _isAscending = true;
  var _filters = const LibraryFilterState();
  CatalogData? _catalog;
  Object? _catalogError;
  StreamSubscription<CatalogEvent>? _catalogEvents;
  Timer? _eventRetryTimer;

  List<CatalogSource> get _sources => switch (_category) {
    LibraryCategory.games => _catalog?.gameSources ?? const <CatalogSource>[],
    LibraryCategory.apps => _catalog?.appSources ?? const <CatalogSource>[],
    LibraryCategory.groups => const <CatalogSource>[],
    LibraryCategory.history => const <CatalogSource>[],
  };

  List<LibraryFeature> get _features => _category == LibraryCategory.games
      ? gameLibraryFeatures
      : appLibraryFeatures;

  CatalogSource? get _selectedSource =>
      _sources.isEmpty ? null : _sources[_sourceIndex];

  List<DashboardItem> get _items {
    final items = (_selectedSource?.items ?? const <DashboardItem>[])
        .where(_filters.matches)
        .toList(growable: false);
    items.sort(
      (DashboardItem left, DashboardItem right) => _isAscending
          ? left.title.compareTo(right.title)
          : right.title.compareTo(left.title),
    );
    return items;
  }

  void _selectCategory(LibraryCategory category) {
    setState(() {
      _category = category;
      _sourceIndex = 0;
      _filters = const LibraryFilterState();
    });
  }

  void _selectSource(int index) {
    setState(() => _sourceIndex = index);
  }

  @override
  void initState() {
    super.initState();
    _loadCatalog();
    _subscribeToCatalogEvents();
  }

  @override
  void dispose() {
    _catalogEvents?.cancel();
    _eventRetryTimer?.cancel();
    super.dispose();
  }

  void _subscribeToCatalogEvents() {
    _catalogEvents?.cancel();
    _catalogEvents = _catalogRepository.watch().listen(
      (CatalogEvent event) {
        if (event is CatalogChanged) {
          _loadCatalog();
        }
      },
      onError: (Object error, StackTrace stackTrace) {
        // The initial HTTP load remains useful when event delivery is briefly
        // unavailable. Retry without surfacing a background stream failure.
        _scheduleCatalogEventRetry();
      },
      onDone: _scheduleCatalogEventRetry,
      cancelOnError: true,
    );
  }

  void _scheduleCatalogEventRetry() {
    if (!mounted || _eventRetryTimer?.isActive == true) {
      return;
    }
    _eventRetryTimer = Timer(
      const Duration(seconds: 3),
      _subscribeToCatalogEvents,
    );
  }

  Future<void> _loadCatalog() async {
    try {
      final catalog = await _catalogRepository.load();
      if (mounted) {
        setState(() {
          _catalog = catalog;
          _catalogError = null;
          _sourceIndex = 0;
        });
      }
    } catch (error) {
      if (mounted) {
        setState(() => _catalogError = error);
      }
    }
  }

  Future<void> _launchItem(DashboardItem item) async {
    try {
      await _catalogRepository.launch(item);
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('${item.title} launch requested.')),
        );
      }
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Could not launch ${item.title}: $error')),
        );
      }
    }
  }

  Future<void> _requestRescan() async {
    try {
      await _catalogRepository.requestRescan();
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('Library refresh requested.')),
        );
      }
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Could not refresh library: $error')),
        );
      }
    }
  }

  void _openItemDetails(DashboardItem item) {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        settings: RouteSettings(name: '/details/${item.id}'),
        builder: (BuildContext context) => ContentDetailsPage(
          item: item,
          sourceShape: TvTileShape.square,
          onPrimaryAction: _launchItem,
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (BuildContext context, BoxConstraints constraints) {
        final layout = _LibraryLayout.fromConstraints(constraints);
        return Actions(
          actions: <Type, Action<Intent>>{
            DismissIntent: CallbackAction<DismissIntent>(
              onInvoke: (DismissIntent intent) {
                dismissTextInputOrPop(context);
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
                  dismissTextInputOrPop(context);
                  return KeyEventResult.handled;
                }
                return KeyEventResult.ignored;
              },
              child: Scaffold(
                key: _scaffoldKey,
                endDrawerEnableOpenDragGesture: false,
                endDrawer: LibraryFilterSheet(
                  initialState: _filters,
                  onApply: (LibraryFilterState state) {
                    setState(() => _filters = state);
                  },
                ),
                body: SafeArea(
                  child: Stack(
                    children: <Widget>[
                      const Positioned.fill(child: _LibraryBackdrop()),
                      TvTwoPaneLayout(
                        rail: _LibraryRail(
                          width: layout.railWidth,
                          selected: _category,
                          compact: layout.isRailCompact,
                          onSelect: _selectCategory,
                        ),
                        content: _LibraryContent(
                          category: _category,
                          isLoading: _catalog == null && _catalogError == null,
                          loadError: _catalogError,
                          sources: _sources,
                          selectedSourceIndex: _sourceIndex,
                          items: _items,
                          features: _features,
                          layout: layout,
                          isAscending: _isAscending,
                          filters: _filters,
                          onSourceSelected: _selectSource,
                          onSortChanged: () =>
                              setState(() => _isAscending = !_isAscending),
                          onFilterRequested: () {
                            _scaffoldKey.currentState?.openEndDrawer();
                          },
                          onRefreshRequested: _loadCatalog,
                          onRescanRequested: _requestRescan,
                          onOpen: _openItemDetails,
                        ),
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

class _LibraryLayout {
  const _LibraryLayout._({
    required this.pagePadding,
    required this.railWidth,
    required this.gap,
    required this.sectionGap,
    required this.tileExtent,
    required this.isRailCompact,
  });

  factory _LibraryLayout.fromConstraints(BoxConstraints constraints) {
    final width = constraints.maxWidth;
    final height = constraints.maxHeight;
    final scale = (math.min(width, height) / 720).clamp(0.72, 1.3).toDouble();
    final pagePadding = (width * 0.04).clamp(24 * scale, 76 * scale).toDouble();
    final compact = width < 980 * scale;
    return _LibraryLayout._(
      pagePadding: pagePadding,
      railWidth: compact ? 72 * scale : 254 * scale,
      gap: 14 * scale,
      sectionGap: 34 * scale,
      tileExtent: (width * 0.14).clamp(152 * scale, 250 * scale).toDouble(),
      isRailCompact: compact,
    );
  }

  final double pagePadding;
  final double railWidth;
  final double gap;
  final double sectionGap;
  final double tileExtent;
  final bool isRailCompact;
}

class _LibraryRail extends StatelessWidget {
  const _LibraryRail({
    required this.width,
    required this.selected,
    required this.compact,
    required this.onSelect,
  });

  final double width;
  final LibraryCategory selected;
  final bool compact;
  final ValueChanged<LibraryCategory> onSelect;

  @override
  Widget build(BuildContext context) {
    return TvNavigationRail(
      width: width,
      compact: compact,
      headerBuilder: (BuildContext context, bool compact) =>
          TvProfileRailHeader(name: 'Alex', compact: compact),
      footerBuilder: compact
          ? null
          : (BuildContext context, bool compact) => const _StorageStatus(),
      items: libraryCategories
          .map(
            (LibraryCategoryDefinition category) => TvNavigationRailItem(
              id: category.category.name,
              label: category.label,
              icon: category.icon,
              isSelected: category.category == selected,
              onActivate: () => onSelect(category.category),
            ),
          )
          .toList(growable: false),
    );
  }
}

class _StorageStatus extends StatelessWidget {
  const _StorageStatus();

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tv.surfaceMuted,
        borderRadius: BorderRadius.circular(10),
      ),
      child: Padding(
        padding: const EdgeInsets.all(14),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            const Text(
              'Storage',
              style: TextStyle(fontWeight: FontWeight.w700),
            ),
            const SizedBox(height: 5),
            Text('156.6 GB free', style: TextStyle(color: tv.secondaryText)),
            const SizedBox(height: 9),
            LinearProgressIndicator(
              value: 0.46,
              color: tv.focus,
              backgroundColor: tv.canvas,
            ),
          ],
        ),
      ),
    );
  }
}

class _LibraryContent extends StatelessWidget {
  const _LibraryContent({
    required this.category,
    required this.isLoading,
    required this.loadError,
    required this.sources,
    required this.selectedSourceIndex,
    required this.items,
    required this.features,
    required this.layout,
    required this.isAscending,
    required this.filters,
    required this.onSourceSelected,
    required this.onSortChanged,
    required this.onFilterRequested,
    required this.onRefreshRequested,
    required this.onRescanRequested,
    required this.onOpen,
  });

  final LibraryCategory category;
  final bool isLoading;
  final Object? loadError;
  final List<CatalogSource> sources;
  final int selectedSourceIndex;
  final List<DashboardItem> items;
  final List<LibraryFeature> features;
  final _LibraryLayout layout;
  final bool isAscending;
  final LibraryFilterState filters;
  final ValueChanged<int> onSourceSelected;
  final VoidCallback onSortChanged;
  final VoidCallback onFilterRequested;
  final VoidCallback onRefreshRequested;
  final VoidCallback onRescanRequested;
  final ValueChanged<DashboardItem> onOpen;

  @override
  Widget build(BuildContext context) {
    final title = switch (category) {
      LibraryCategory.games => 'PC games',
      LibraryCategory.apps => 'Apps library',
      LibraryCategory.groups => 'Custom groups',
      LibraryCategory.history => 'Recently used',
    };
    return CustomScrollView(
      scrollCacheExtent: ScrollCacheExtent.viewport(2),
      slivers: <Widget>[
        SliverPadding(
          padding: EdgeInsets.fromLTRB(
            layout.pagePadding,
            layout.pagePadding,
            layout.pagePadding,
            layout.sectionGap,
          ),
          sliver: SliverMainAxisGroup(
            slivers: <Widget>[
              SliverToBoxAdapter(
                child: Row(
                  children: <Widget>[
                    Expanded(
                      child: Text(
                        title,
                        style: Theme.of(context).textTheme.displaySmall,
                      ),
                    ),
                    TvLibraryControl(
                      label: 'Refresh library',
                      icon: Icons.refresh_rounded,
                      onActivate: onRescanRequested,
                    ),
                    SizedBox(width: layout.gap),
                    TvLibraryControl(
                      label: 'Search library',
                      icon: Icons.search_rounded,
                      onActivate: () => Navigator.of(context).push(
                        MaterialPageRoute<void>(
                          settings: const RouteSettings(name: '/search'),
                          builder: (BuildContext context) =>
                              const TvSearchPage(),
                        ),
                      ),
                    ),
                  ],
                ),
              ),
              SliverToBoxAdapter(child: SizedBox(height: layout.sectionGap)),
              if (isLoading)
                const SliverToBoxAdapter(child: _LibraryLoadingState())
              else if (loadError case final Object error)
                SliverToBoxAdapter(
                  child: _LibraryErrorState(
                    error: error,
                    onRetry: onRefreshRequested,
                  ),
                )
              else if (sources.isNotEmpty) ...<Widget>[
                SliverToBoxAdapter(
                  child: _SourceTabs(
                    sources: sources,
                    selectedIndex: selectedSourceIndex,
                    onSelected: onSourceSelected,
                    gap: layout.gap,
                  ),
                ),
                SliverToBoxAdapter(child: SizedBox(height: layout.sectionGap)),
                SliverToBoxAdapter(
                  child: _LibraryControls(
                    count: items.length,
                    isAscending: isAscending,
                    onSortChanged: onSortChanged,
                    activeFilterCount: filters.selected.length,
                    onFilterRequested: onFilterRequested,
                  ),
                ),
                SliverToBoxAdapter(child: SizedBox(height: layout.gap)),
                SliverToBoxAdapter(
                  child: _FeatureShelf(features: features, gap: layout.gap),
                ),
                SliverToBoxAdapter(child: SizedBox(height: layout.sectionGap)),
                if (items.isEmpty)
                  const SliverToBoxAdapter(child: _EmptyLibraryState())
                else
                  SliverGrid.builder(
                    itemCount: items.length,
                    gridDelegate: SliverGridDelegateWithMaxCrossAxisExtent(
                      maxCrossAxisExtent: layout.tileExtent,
                      mainAxisSpacing: layout.gap,
                      crossAxisSpacing: layout.gap,
                      childAspectRatio: 1,
                    ),
                    itemBuilder: (BuildContext context, int index) {
                      final item = items[index];
                      return TvContentTile(
                        key: ValueKey<String>('library-tile-${item.id}'),
                        item: item,
                        shape: TvTileShape.square,
                        onActivate: () => onOpen(item),
                      );
                    },
                  ),
              ] else
                const SliverToBoxAdapter(child: _EmptyLibraryState()),
            ],
          ),
        ),
      ],
    );
  }
}

class _LibraryLoadingState extends StatelessWidget {
  const _LibraryLoadingState();

  @override
  Widget build(BuildContext context) {
    return const Padding(
      padding: EdgeInsets.symmetric(vertical: 80),
      child: Center(
        child: Column(
          children: <Widget>[
            SizedBox(width: 32, height: 32, child: CircularProgressIndicator()),
            SizedBox(height: 16),
            Text('Loading your library'),
          ],
        ),
      ),
    );
  }
}

class _LibraryErrorState extends StatelessWidget {
  const _LibraryErrorState({required this.error, required this.onRetry});

  final Object error;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 80),
      child: Center(
        child: Column(
          children: <Widget>[
            Icon(Icons.cloud_off_rounded, size: 46, color: tv.secondaryText),
            const SizedBox(height: 14),
            Text(
              'Library unavailable',
              style: Theme.of(context).textTheme.titleLarge,
            ),
            const SizedBox(height: 6),
            Text(
              'Check the Hearthdeck service connection and try again.',
              style: TextStyle(color: tv.secondaryText),
            ),
            const SizedBox(height: 18),
            TvLibraryControl(
              label: 'Try again',
              icon: Icons.refresh_rounded,
              onActivate: onRetry,
            ),
          ],
        ),
      ),
    );
  }
}

class _SourceTabs extends StatelessWidget {
  const _SourceTabs({
    required this.sources,
    required this.selectedIndex,
    required this.onSelected,
    required this.gap,
  });

  final List<CatalogSource> sources;
  final int selectedIndex;
  final ValueChanged<int> onSelected;
  final double gap;

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      child: Row(
        children: <Widget>[
          for (var index = 0; index < sources.length; index++) ...<Widget>[
            if (index > 0) SizedBox(width: gap),
            _SourceTab(
              source: sources[index],
              isSelected: index == selectedIndex,
              onActivate: () => onSelected(index),
            ),
          ],
        ],
      ),
    );
  }
}

class _SourceTab extends StatelessWidget {
  const _SourceTab({
    required this.source,
    required this.isSelected,
    required this.onActivate,
  });

  final CatalogSource source;
  final bool isSelected;
  final VoidCallback onActivate;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return TvFocusable(
      semanticLabel: source.label,
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
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 11),
          decoration: BoxDecoration(
            color: style.background,
            borderRadius: BorderRadius.circular(22),
            border: Border.all(color: style.border, width: 2),
          ),
          child: Text(
            source.label,
            style: TextStyle(
              color: style.foreground,
              fontWeight: FontWeight.w700,
            ),
          ),
        );
      },
    );
  }
}

class _LibraryControls extends StatelessWidget {
  const _LibraryControls({
    required this.count,
    required this.isAscending,
    required this.onSortChanged,
    required this.activeFilterCount,
    required this.onFilterRequested,
  });

  final int count;
  final bool isAscending;
  final VoidCallback onSortChanged;
  final int activeFilterCount;
  final VoidCallback onFilterRequested;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Row(
      children: <Widget>[
        TvLibraryControl(
          label: isAscending ? 'Sort A-Z' : 'Sort Z-A',
          icon: isAscending ? Icons.sort_by_alpha_rounded : Icons.sort_rounded,
          onActivate: onSortChanged,
        ),
        const Spacer(),
        Text('$count items', style: TextStyle(color: tv.secondaryText)),
        const SizedBox(width: 16),
        TvLibraryControl(
          label: activeFilterCount == 0
              ? 'Filter'
              : 'Filter ($activeFilterCount)',
          icon: Icons.filter_alt_outlined,
          onActivate: onFilterRequested,
        ),
      ],
    );
  }
}

class TvLibraryControl extends StatelessWidget {
  const TvLibraryControl({
    required this.label,
    required this.icon,
    required this.onActivate,
    super.key,
  });

  final String label;
  final IconData icon;
  final VoidCallback onActivate;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return TvFocusable(
      semanticLabel: label,
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
          constraints: const BoxConstraints(minHeight: 46),
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
          decoration: BoxDecoration(
            color: style.background,
            borderRadius: BorderRadius.circular(8),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              Icon(icon, color: style.foreground),
              const SizedBox(width: 9),
              Text(
                label,
                style: TextStyle(
                  color: style.foreground,
                  fontWeight: FontWeight.w700,
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}

class _FeatureShelf extends StatelessWidget {
  const _FeatureShelf({required this.features, required this.gap});

  final List<LibraryFeature> features;
  final double gap;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return SizedBox(
      height: 126,
      child: ListView.separated(
        scrollDirection: Axis.horizontal,
        itemCount: features.length,
        separatorBuilder: (BuildContext context, int index) =>
            SizedBox(width: gap),
        itemBuilder: (BuildContext context, int index) {
          final feature = features[index];
          return SizedBox(
            width: 258,
            child: TvFocusable(
              semanticLabel: feature.title,
              onActivate: () => _showLibraryMessage(context, feature.title),
              builder: (BuildContext context, bool isFocused) {
                return AnimatedContainer(
                  duration: TvTheme.focusDuration,
                  curve: TvTheme.focusCurve,
                  padding: const EdgeInsets.all(16),
                  decoration: BoxDecoration(
                    gradient: LinearGradient(colors: feature.colors),
                    borderRadius: BorderRadius.circular(10),
                    border: Border.all(
                      color: isFocused ? tv.focus : Colors.transparent,
                      width: 3,
                    ),
                  ),
                  child: Row(
                    children: <Widget>[
                      Icon(feature.icon, size: 36),
                      const SizedBox(width: 13),
                      Expanded(
                        child: Column(
                          mainAxisAlignment: MainAxisAlignment.center,
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: <Widget>[
                            Text(
                              feature.title,
                              style: const TextStyle(
                                fontWeight: FontWeight.w700,
                              ),
                            ),
                            const SizedBox(height: 4),
                            Text(
                              feature.subtitle,
                              maxLines: 2,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(
                                fontSize: 12,
                                color: tv.secondaryText,
                              ),
                            ),
                          ],
                        ),
                      ),
                    ],
                  ),
                );
              },
            ),
          );
        },
      ),
    );
  }
}

class _EmptyLibraryState extends StatelessWidget {
  const _EmptyLibraryState();

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Center(
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 80),
        child: Column(
          children: <Widget>[
            Icon(Icons.library_add_outlined, size: 46, color: tv.secondaryText),
            const SizedBox(height: 16),
            Text(
              'Nothing here yet',
              style: Theme.of(context).textTheme.titleLarge,
            ),
            const SizedBox(height: 6),
            Text(
              'This collection will appear as content is added.',
              style: TextStyle(color: tv.secondaryText),
            ),
          ],
        ),
      ),
    );
  }
}

class _LibraryBackdrop extends StatelessWidget {
  const _LibraryBackdrop();

  @override
  Widget build(BuildContext context) {
    return const TvBackdrop(center: Alignment(0.88, -0.6));
  }
}

void _showLibraryMessage(BuildContext context, String feature) {
  ScaffoldMessenger.of(context).hideCurrentSnackBar();
  ScaffoldMessenger.of(context).showSnackBar(
    SnackBar(content: Text('$feature is ready for configuration.')),
  );
}
