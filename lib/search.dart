import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';

import 'backend/hearthdeck_api_client.dart';
import 'catalog/catalog_repository.dart';
import 'catalog/catalog_repository_factory.dart';
import 'content_details.dart';
import 'dashboard_models.dart';
import 'launch_loader.dart';
import 'library_models.dart';
import 'retro.dart';
import 'tv_components.dart';
import 'tv_theme.dart';

/// The major-category chips this screen offers. `groups` and `history`
/// are left out: they aren't backed by real content yet (see
/// `FullLibraryPage._sources`), so they'd always show zero results.
const List<LibraryCategory> _searchableCategories = <LibraryCategory>[
  LibraryCategory.games,
  LibraryCategory.apps,
  LibraryCategory.consoleGames,
];

class TvSearchPage extends StatefulWidget {
  const TvSearchPage({
    super.key,
    this.initialQuery = '',
    this.initialCategory,
    this.catalogRepository,
    this.apiClient,
  });

  /// Text the search field starts with.
  final String initialQuery;

  /// Pre-selects a category chip (e.g. opening search from the Console
  /// games screen starts scoped to [LibraryCategory.consoleGames]). This is
  /// only a default: the user can still pick a different chip, including
  /// "All", to widen the search.
  final LibraryCategory? initialCategory;

  /// Test/DI seam for the live PC-games/apps catalog. Defaults to
  /// [createCatalogRepository].
  final CatalogRepository? catalogRepository;

  /// Test/DI seam for the RomM client used to search console games.
  /// Defaults to [createRetroApiClient].
  final HearthdeckApiClient? apiClient;

  @override
  State<TvSearchPage> createState() => _TvSearchPageState();
}

class _TvSearchPageState extends State<TvSearchPage> {
  static const _retroSearchDebounce = Duration(milliseconds: 350);
  static const _retroSearchLimit = 24;

  late final TextEditingController _controller = TextEditingController(
    text: widget.initialQuery,
  );
  late final FocusNode _textFocusNode = FocusNode(debugLabel: 'Search input');
  late LibraryCategory? _category = widget.initialCategory;

  late final CatalogRepository _catalogRepository =
      widget.catalogRepository ?? createCatalogRepository();
  late final Future<HearthdeckApiClient?> _apiClient = widget.apiClient == null
      ? createRetroApiClient()
      : Future<HearthdeckApiClient?>.value(widget.apiClient);
  HearthdeckApiClient? _connectedApiClient;

  CatalogData? _catalog;
  Object? _catalogError;
  var _isLoadingCatalog = true;

  List<HearthdeckRetroGame>? _retroResults;
  Object? _retroError;
  var _isSearchingRetro = false;
  Timer? _retroDebounce;
  var _retroRequestId = 0;

  bool get _includesGames =>
      _category == null || _category == LibraryCategory.games;

  bool get _includesApps =>
      _category == null || _category == LibraryCategory.apps;

  bool get _includesConsoleGames =>
      _category == null || _category == LibraryCategory.consoleGames;

  /// True while a RomM search is queued (debouncing) or actually in
  /// flight. Distinct from [_isSearchingRetro], which only flips on once
  /// the request has actually been sent - this also covers the ~350ms
  /// debounce window right after a keystroke, so the empty-state logic
  /// below doesn't flash "no results" before a search has even started.
  bool get _isRetroSearchPending =>
      _isSearchingRetro || (_retroDebounce?.isActive ?? false);

  List<DashboardItem> get _catalogResults {
    final catalog = _catalog;
    if (catalog == null) {
      return const <DashboardItem>[];
    }
    final items = <DashboardItem>[
      if (_includesGames)
        ...catalog.gameSources.expand((CatalogSource source) => source.items),
      if (_includesApps)
        ...catalog.appSources.expand((CatalogSource source) => source.items),
    ];
    final query = _controller.text.trim().toLowerCase();
    if (query.isEmpty) {
      return items;
    }
    return items
        .where((DashboardItem item) => _matchesQuery(item, query))
        .toList(growable: false);
  }

  bool _matchesQuery(DashboardItem item, String query) {
    return item.title.toLowerCase().contains(query) ||
        (item.description?.toLowerCase().contains(query) ?? false);
  }

  List<DashboardItem> get _retroDashboardResults {
    final results = _retroResults;
    if (results == null) {
      return const <DashboardItem>[];
    }
    return results
        .map(
          (HearthdeckRetroGame game) =>
              retroGameToDashboardItem(game, apiClient: _connectedApiClient),
        )
        .toList(growable: false);
  }

  List<DashboardItem> get _results => <DashboardItem>[
    ..._catalogResults,
    ..._retroDashboardResults,
  ];

  @override
  void initState() {
    super.initState();
    _controller.addListener(_handleTextChanged);
    unawaited(_loadCatalog());
    // Not a keystroke, so there's nothing to debounce: if a caller ever
    // passes both a non-empty initialQuery and a console-games-inclusive
    // initialCategory, this should search right away instead of making the
    // first frame wait out an artificial 350ms UI debounce.
    _scheduleRetroSearch(immediate: true);
  }

  @override
  void dispose() {
    _retroDebounce?.cancel();
    _controller
      ..removeListener(_handleTextChanged)
      ..dispose();
    _textFocusNode.dispose();
    super.dispose();
  }

  Future<void> _loadCatalog() async {
    try {
      final catalog = await _catalogRepository.load();
      if (mounted) {
        setState(() {
          _catalog = catalog;
          _isLoadingCatalog = false;
        });
      }
    } catch (error) {
      if (mounted) {
        setState(() {
          _catalogError = error;
          _isLoadingCatalog = false;
        });
      }
    }
  }

  void _handleTextChanged() {
    setState(() {});
    _scheduleRetroSearch();
  }

  void _selectCategory(LibraryCategory? category) {
    if (_category == category) {
      return;
    }
    setState(() => _category = category);
    _scheduleRetroSearch(immediate: true);
  }

  void _scheduleRetroSearch({bool immediate = false}) {
    _retroDebounce?.cancel();
    final query = _controller.text.trim();
    if (!_includesConsoleGames || query.isEmpty) {
      _retroRequestId++;
      setState(() {
        _retroResults = null;
        _retroError = null;
        _isSearchingRetro = false;
      });
      return;
    }
    if (immediate) {
      unawaited(_searchRetro(query));
    } else {
      _retroDebounce = Timer(
        _retroSearchDebounce,
        () => unawaited(_searchRetro(query)),
      );
    }
  }

  Future<void> _searchRetro(String query) async {
    final requestId = ++_retroRequestId;
    setState(() => _isSearchingRetro = true);
    try {
      final apiClient = await _apiClient;
      if (apiClient == null) {
        throw StateError('No Hearthdeck backend is connected.');
      }
      _connectedApiClient = apiClient;
      final page = await apiClient.retroGames(
        search: query,
        limit: _retroSearchLimit,
      );
      if (mounted && requestId == _retroRequestId) {
        setState(() {
          _retroResults = page.items;
          _retroError = null;
          _isSearchingRetro = false;
        });
      }
    } catch (error) {
      if (mounted && requestId == _retroRequestId) {
        setState(() {
          _retroError = error;
          _isSearchingRetro = false;
        });
      }
    }
  }

  /// Results are either regular catalog items (PC games/apps, launched
  /// through [CatalogRepository]) or RomM console games (identified by the
  /// `romm:<id>` id format `retroGameToDashboardItem` uses, launched through
  /// the same dedicated RomM endpoint `retro.dart` uses) - this dispatches
  /// to whichever one actually produced the tapped result.
  Future<void> _launchItem(DashboardItem item) async {
    await runWithLaunchLoader<void>(
      context,
      itemTitle: item.title,
      action: () async {
        final rommId = item.id.startsWith('romm:')
            ? int.tryParse(item.id.substring('romm:'.length))
            : null;
        try {
          if (rommId != null) {
            final apiClient = _connectedApiClient ?? await _apiClient;
            if (apiClient == null) {
              throw StateError('No Hearthdeck backend is connected.');
            }
            await apiClient.launchRetroRom(rommId);
          } else {
            await _catalogRepository.launch(item);
          }
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
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    final results = _results;
    final query = _controller.text.trim();
    return LayoutBuilder(
      builder: (BuildContext context, BoxConstraints constraints) {
        final layout = _SearchLayout.fromConstraints(constraints);
        // Escape/back is handled globally (see main.dart's HardwareKeyboard
        // listener), regardless of what has focus on this screen.
        return TvDirectionalFocusNavigation(
          child: Scaffold(
            body: SafeArea(
              child: Stack(
                children: <Widget>[
                  const Positioned.fill(child: _SearchBackdrop()),
                  CustomScrollView(
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
                              child: _SearchHeader(
                                controller: _controller,
                                focusNode: _textFocusNode,
                                onClear: _controller.clear,
                              ),
                            ),
                            SliverToBoxAdapter(
                              child: SizedBox(height: layout.sectionGap),
                            ),
                            SliverToBoxAdapter(
                              child: _CategoryChips(
                                selected: _category,
                                onSelected: _selectCategory,
                              ),
                            ),
                            SliverToBoxAdapter(
                              child: SizedBox(height: layout.sectionGap),
                            ),
                            SliverToBoxAdapter(
                              child: _ResultsSummary(
                                query: query,
                                resultCount: results.length,
                                isLoadingCatalog: _isLoadingCatalog,
                                isSearchingRetro: _isRetroSearchPending,
                              ),
                            ),
                            SliverToBoxAdapter(
                              child: SizedBox(height: layout.gap),
                            ),
                            if (results.isEmpty &&
                                (((_includesGames || _includesApps) &&
                                        _isLoadingCatalog) ||
                                    (_includesConsoleGames &&
                                        _isRetroSearchPending)))
                              // Nothing to show yet, but a load this scope
                              // actually depends on is still running - don't
                              // claim "no results" (or a stale error) while
                              // that's still in flight.
                              const SliverToBoxAdapter(
                                child: _SearchLoadingState(),
                              )
                            else if (_catalogError != null &&
                                (_includesGames || _includesApps) &&
                                results.isEmpty)
                              const SliverToBoxAdapter(
                                child: _SearchProblem(
                                  message:
                                      'Could not load your library. Pull to refresh from Full library and try again.',
                                ),
                              )
                            else if (_retroError != null &&
                                _includesConsoleGames &&
                                query.isNotEmpty &&
                                _category == LibraryCategory.consoleGames &&
                                results.isEmpty)
                              SliverToBoxAdapter(
                                child: _SearchProblem(
                                  message:
                                      'Could not search your RomM library: $_retroError',
                                ),
                              )
                            else if (query.isEmpty &&
                                _category == LibraryCategory.consoleGames)
                              const SliverToBoxAdapter(
                                child: _SearchHint(
                                  message:
                                      'Type to search your RomM console library.',
                                ),
                              )
                            else if (results.isEmpty)
                              const SliverToBoxAdapter(
                                child: _NoSearchResults(),
                              )
                            else
                              SliverGrid.builder(
                                itemCount: results.length,
                                gridDelegate:
                                    SliverGridDelegateWithMaxCrossAxisExtent(
                                      maxCrossAxisExtent: layout.tileExtent,
                                      mainAxisSpacing: layout.gap,
                                      crossAxisSpacing: layout.gap,
                                      childAspectRatio: 1,
                                    ),
                                itemBuilder: (BuildContext context, int index) {
                                  final item = results[index];
                                  return TvContentTile(
                                    key: ValueKey<String>(
                                      'search-tile-${item.id}',
                                    ),
                                    item: item,
                                    shape: TvTileShape.square,
                                    onActivate: () =>
                                        Navigator.of(context).push(
                                          MaterialPageRoute<void>(
                                            settings: RouteSettings(
                                              name: '/details/${item.id}',
                                            ),
                                            builder: (BuildContext context) =>
                                                ContentDetailsPage(
                                                  item: item,
                                                  sourceShape:
                                                      TvTileShape.square,
                                                  onPrimaryAction: _launchItem,
                                                ),
                                          ),
                                        ),
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
      },
    );
  }
}

class _SearchLayout {
  const _SearchLayout._({
    required this.pagePadding,
    required this.gap,
    required this.sectionGap,
    required this.tileExtent,
  });

  factory _SearchLayout.fromConstraints(BoxConstraints constraints) {
    final width = constraints.maxWidth;
    final height = constraints.maxHeight;
    final scale = (width < height ? width : height).clamp(520, 1080) / 720;
    return _SearchLayout._(
      pagePadding: (width * 0.04).clamp(20 * scale, 64 * scale),
      gap: 10 * scale,
      sectionGap: 18 * scale,
      tileExtent: (width * 0.15).clamp(154 * scale, 254 * scale),
    );
  }

  final double pagePadding;
  final double gap;
  final double sectionGap;
  final double tileExtent;
}

class _SearchHeader extends StatelessWidget {
  const _SearchHeader({
    required this.controller,
    required this.focusNode,
    required this.onClear,
  });

  final TextEditingController controller;
  final FocusNode focusNode;
  final VoidCallback onClear;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        Text('Search', style: Theme.of(context).textTheme.displaySmall),
        const SizedBox(height: 10),
        DecoratedBox(
          decoration: BoxDecoration(
            color: tv.surface,
            borderRadius: BorderRadius.circular(10),
            border: Border.all(
              color: tv.focus.withValues(alpha: 0.7),
              width: 2,
            ),
          ),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: Row(
              children: <Widget>[
                Icon(Icons.search_rounded, color: tv.focus),
                const SizedBox(width: 12),
                Expanded(
                  child: TextField(
                    key: const ValueKey<String>('search-input'),
                    controller: controller,
                    focusNode: focusNode,
                    autofocus: true,
                    textInputAction: TextInputAction.search,
                    onSubmitted: (_) => FocusScope.of(context).nextFocus(),
                    style: Theme.of(context).textTheme.titleMedium,
                    cursorColor: tv.focus,
                    decoration: InputDecoration(
                      border: InputBorder.none,
                      hintText: 'Search games, apps, and console games',
                      hintStyle: TextStyle(color: tv.secondaryText),
                    ),
                  ),
                ),
                if (controller.text.isNotEmpty)
                  TvFocusable(
                    semanticLabel: 'Clear search',
                    onActivate: onClear,
                    builder: (BuildContext context, bool isFocused) {
                      final style = TvControlStyle.resolve(
                        tv,
                        variant: TvControlVariant.icon,
                        isFocused: isFocused,
                      );
                      return AnimatedContainer(
                        duration: TvTheme.focusDuration,
                        curve: TvTheme.focusCurve,
                        width: 36,
                        height: 36,
                        decoration: BoxDecoration(
                          color: style.background,
                          borderRadius: BorderRadius.circular(8),
                          border: Border.all(color: style.border, width: 2),
                        ),
                        child: Icon(
                          Icons.backspace_outlined,
                          color: isFocused
                              ? style.foreground
                              : tv.secondaryText,
                        ),
                      );
                    },
                  ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}

class _CategoryChips extends StatelessWidget {
  const _CategoryChips({required this.selected, required this.onSelected});

  final LibraryCategory? selected;
  final ValueChanged<LibraryCategory?> onSelected;

  @override
  Widget build(BuildContext context) {
    final definitions = libraryCategories
        .where(
          (LibraryCategoryDefinition definition) =>
              _searchableCategories.contains(definition.category),
        )
        .toList(growable: false);
    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      child: Row(
        children: <Widget>[
          _CategoryChip(
            label: 'All',
            icon: Icons.apps_rounded,
            isSelected: selected == null,
            onActivate: () => onSelected(null),
          ),
          for (final definition in definitions) ...<Widget>[
            const SizedBox(width: 10),
            _CategoryChip(
              label: definition.label,
              icon: definition.icon,
              isSelected: selected == definition.category,
              onActivate: () => onSelected(definition.category),
            ),
          ],
        ],
      ),
    );
  }
}

class _CategoryChip extends StatelessWidget {
  const _CategoryChip({
    required this.label,
    required this.icon,
    required this.isSelected,
    required this.onActivate,
  });

  final String label;
  final IconData icon;
  final bool isSelected;
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
          variant: TvControlVariant.selectable,
          isFocused: isFocused,
          isSelected: isSelected,
        );
        return AnimatedContainer(
          duration: TvTheme.focusDuration,
          curve: TvTheme.focusCurve,
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
          decoration: BoxDecoration(
            color: style.background,
            borderRadius: BorderRadius.circular(22),
            border: Border.all(color: style.border, width: 2),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              Icon(icon, size: 18, color: style.foreground),
              const SizedBox(width: 8),
              Text(
                label,
                style: TextStyle(
                  color: style.foreground,
                  fontWeight: FontWeight.w700,
                  fontSize: TvTheme.labelMediumSize,
                  height: 1,
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}

class _ResultsSummary extends StatelessWidget {
  const _ResultsSummary({
    required this.query,
    required this.resultCount,
    required this.isLoadingCatalog,
    required this.isSearchingRetro,
  });

  final String query;
  final int resultCount;
  final bool isLoadingCatalog;
  final bool isSearchingRetro;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final label = query.isEmpty ? 'Browse all' : '$resultCount results';
    return Row(
      children: <Widget>[
        Text(label, style: Theme.of(context).textTheme.titleLarge),
        if (isLoadingCatalog || isSearchingRetro) ...<Widget>[
          const SizedBox(width: 12),
          SizedBox(
            width: 16,
            height: 16,
            child: CircularProgressIndicator(
              strokeWidth: 2,
              color: tv.secondaryText,
            ),
          ),
        ],
      ],
    );
  }
}

class _NoSearchResults extends StatelessWidget {
  const _NoSearchResults();

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Padding(
      padding: EdgeInsets.symmetric(vertical: 56),
      child: Center(
        child: Column(
          children: <Widget>[
            Icon(Icons.search_off_rounded, size: 46, color: tv.secondaryText),
            const SizedBox(height: 12),
            const Text(
              'No matching content',
              style: TextStyle(fontWeight: FontWeight.w700),
            ),
            const SizedBox(height: 4),
            Text(
              'Try another title or category.',
              style: TextStyle(color: tv.secondaryText),
            ),
          ],
        ),
      ),
    );
  }
}

// _SearchHint, _SearchProblem, and _SearchLoadingState (below) build
// ordinary box content, like _NoSearchResults above - the caller wraps them
// in SliverToBoxAdapter at each use site inside the slivers list. Keep it
// that way rather than having build() return a sliver directly: these are
// plain StatelessWidgets with no type-level marker that they're sliver-only,
// so a future edit could easily reuse one outside a sliver context (e.g. a
// Column) or double-wrap it in another SliverToBoxAdapter, either of which
// would crash at runtime.
class _SearchHint extends StatelessWidget {
  const _SearchHint({required this.message});

  final String message;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 56),
      child: Center(
        child: Column(
          children: <Widget>[
            Icon(
              Icons.videogame_asset_outlined,
              size: 46,
              color: tv.secondaryText,
            ),
            const SizedBox(height: 12),
            Text(message, style: TextStyle(color: tv.secondaryText)),
          ],
        ),
      ),
    );
  }
}

class _SearchProblem extends StatelessWidget {
  const _SearchProblem({required this.message});

  final String message;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 56),
      child: Center(
        child: Column(
          children: <Widget>[
            Icon(Icons.cloud_off_rounded, size: 46, color: tv.secondaryText),
            const SizedBox(height: 12),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 24),
              child: Text(
                message,
                textAlign: TextAlign.center,
                style: TextStyle(color: tv.secondaryText),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _SearchLoadingState extends StatelessWidget {
  const _SearchLoadingState();

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 56),
      child: Center(
        child: SizedBox(
          width: 28,
          height: 28,
          child: CircularProgressIndicator(
            strokeWidth: 2.5,
            color: tv.secondaryText,
          ),
        ),
      ),
    );
  }
}

class _SearchBackdrop extends StatelessWidget {
  const _SearchBackdrop();

  @override
  Widget build(BuildContext context) {
    return const TvBackdrop(center: Alignment(0.45, -0.5));
  }
}
