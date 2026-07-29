import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import 'content_details.dart';
import 'dashboard_models.dart';
import 'library_models.dart';
import 'tv_components.dart';
import 'tv_theme.dart';

class TvSearchPage extends StatefulWidget {
  const TvSearchPage({super.key, this.initialQuery = ''});

  final String initialQuery;

  @override
  State<TvSearchPage> createState() => _TvSearchPageState();
}

class _TvSearchPageState extends State<TvSearchPage> {
  late final TextEditingController _controller = TextEditingController(
    text: widget.initialQuery,
  );
  late final FocusNode _textFocusNode = FocusNode(debugLabel: 'Search input');

  List<DashboardItem> get _results {
    final query = _controller.text.trim().toLowerCase();
    if (query.isEmpty) {
      return _searchableItems;
    }
    return _searchableItems
        .where((DashboardItem item) {
          return item.title.toLowerCase().contains(query) ||
              (item.description?.toLowerCase().contains(query) ?? false);
        })
        .toList(growable: false);
  }

  @override
  void initState() {
    super.initState();
    _controller.addListener(_handleTextChanged);
  }

  @override
  void dispose() {
    _controller
      ..removeListener(_handleTextChanged)
      ..dispose();
    _textFocusNode.dispose();
    super.dispose();
  }

  void _handleTextChanged() => setState(() {});

  @override
  Widget build(BuildContext context) {
    final results = _results;
    return LayoutBuilder(
      builder: (BuildContext context, BoxConstraints constraints) {
        final layout = _SearchLayout.fromConstraints(constraints);
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
                                  child: Text(
                                    _controller.text.trim().isEmpty
                                        ? 'Browse all'
                                        : '${results.length} results',
                                    style: Theme.of(
                                      context,
                                    ).textTheme.titleLarge,
                                  ),
                                ),
                                SliverToBoxAdapter(
                                  child: SizedBox(height: layout.gap),
                                ),
                                if (results.isEmpty)
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
                                    itemBuilder:
                                        (BuildContext context, int index) {
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
                                                      name:
                                                          '/details/${item.id}',
                                                    ),
                                                    builder:
                                                        (
                                                          BuildContext context,
                                                        ) => ContentDetailsPage(
                                                          item: item,
                                                          sourceShape:
                                                              TvTileShape
                                                                  .square,
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
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        Text('Search', style: Theme.of(context).textTheme.displaySmall),
        const SizedBox(height: 10),
        DecoratedBox(
          decoration: BoxDecoration(
            color: TvTheme.surface,
            borderRadius: BorderRadius.circular(10),
            border: Border.all(
              color: TvTheme.focus.withValues(alpha: 0.7),
              width: 2,
            ),
          ),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: Row(
              children: <Widget>[
                const Icon(Icons.search_rounded, color: TvTheme.focus),
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
                    cursorColor: TvTheme.focus,
                    decoration: const InputDecoration(
                      border: InputBorder.none,
                      hintText: 'Search games, apps, and media',
                      hintStyle: TextStyle(color: TvTheme.secondaryText),
                    ),
                  ),
                ),
                if (controller.text.isNotEmpty)
                  TvFocusable(
                    semanticLabel: 'Clear search',
                    onActivate: onClear,
                    builder: (BuildContext context, bool isFocused) => Icon(
                      Icons.backspace_outlined,
                      color: isFocused ? TvTheme.focus : TvTheme.secondaryText,
                    ),
                  ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}

class _NoSearchResults extends StatelessWidget {
  const _NoSearchResults();

  @override
  Widget build(BuildContext context) {
    return const Padding(
      padding: EdgeInsets.symmetric(vertical: 56),
      child: Center(
        child: Column(
          children: <Widget>[
            Icon(
              Icons.search_off_rounded,
              size: 46,
              color: TvTheme.secondaryText,
            ),
            SizedBox(height: 12),
            Text(
              'No matching content',
              style: TextStyle(fontWeight: FontWeight.w700),
            ),
            SizedBox(height: 4),
            Text(
              'Try another title or category.',
              style: TextStyle(color: TvTheme.secondaryText),
            ),
          ],
        ),
      ),
    );
  }
}

class _SearchBackdrop extends StatelessWidget {
  const _SearchBackdrop();

  @override
  Widget build(BuildContext context) {
    return const DecoratedBox(
      decoration: BoxDecoration(
        gradient: RadialGradient(
          center: Alignment(0.45, -0.5),
          radius: 1.25,
          colors: <Color>[Color(0xFF1F3A45), TvTheme.canvas],
          stops: <double>[0, 0.74],
        ),
      ),
    );
  }
}

final List<DashboardItem> _searchableItems = <DashboardItem>{
  ...gameLibrarySources.expand((LibrarySource source) => source.items),
  ...appLibrarySources.expand((LibrarySource source) => source.items),
}.toList(growable: false);
