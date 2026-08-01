import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';

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
                              child: Text(
                                _controller.text.trim().isEmpty
                                    ? 'Browse all'
                                    : '${results.length} results',
                                style: Theme.of(context).textTheme.titleLarge,
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
                      hintText: 'Search games, apps, and media',
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

class _SearchBackdrop extends StatelessWidget {
  const _SearchBackdrop();

  @override
  Widget build(BuildContext context) {
    return const TvBackdrop(center: Alignment(0.45, -0.5));
  }
}

final List<DashboardItem> _searchableItems = <DashboardItem>{
  ...gameLibrarySources.expand((LibrarySource source) => source.items),
  ...appLibrarySources.expand((LibrarySource source) => source.items),
}.toList(growable: false);
