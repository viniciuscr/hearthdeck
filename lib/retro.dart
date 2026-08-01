import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';

import 'backend/hearthdeck_api_client.dart';
import 'catalog/catalog_repository_factory.dart';
import 'content_details.dart';
import 'dashboard_models.dart';
import 'library_models.dart';
import 'search.dart';
import 'tv_components.dart';
import 'tv_theme.dart';

class RetroPage extends StatefulWidget {
  const RetroPage({super.key, this.apiClient, this.embedded = false});

  final HearthdeckApiClient? apiClient;
  final bool embedded;

  @override
  State<RetroPage> createState() => _RetroPageState();
}

class _RetroPageState extends State<RetroPage> {
  late final Future<HearthdeckApiClient?> _apiClient = widget.apiClient == null
      ? createRetroApiClient()
      : Future<HearthdeckApiClient?>.value(widget.apiClient);
  late Future<List<HearthdeckRetroConsole>> _consoles = _loadConsoles();
  HearthdeckApiClient? _connectedApiClient;
  HearthdeckRetroConsole? _selectedConsole;
  HearthdeckRetroGamePage? _games;
  Object? _gamesError;
  var _isLoadingGames = false;
  var _isLoadingMore = false;

  Future<List<HearthdeckRetroConsole>> _loadConsoles() async {
    final apiClient = await _apiClient;
    if (apiClient == null) {
      throw StateError('No Hearthdeck backend is connected.');
    }
    _connectedApiClient = apiClient;
    return apiClient.retroConsoles();
  }

  Future<void> _selectConsole(
    HearthdeckRetroConsole console, {
    bool force = false,
  }) async {
    if (!force && _selectedConsole?.id == console.id && _games != null) {
      return;
    }
    setState(() {
      _selectedConsole = console;
      _games = null;
      _gamesError = null;
      _isLoadingGames = true;
    });
    try {
      final apiClient = await _apiClient;
      if (apiClient == null) {
        throw StateError('No Hearthdeck backend is connected.');
      }
      _connectedApiClient = apiClient;
      final games = await apiClient.retroGames(platformId: console.id);
      if (mounted && _selectedConsole?.id == console.id) {
        setState(() => _games = games);
      }
    } catch (error) {
      if (mounted && _selectedConsole?.id == console.id) {
        setState(() => _gamesError = error);
      }
    } finally {
      if (mounted && _selectedConsole?.id == console.id) {
        setState(() => _isLoadingGames = false);
      }
    }
  }

  Future<void> _loadMore() async {
    final console = _selectedConsole;
    final games = _games;
    if (console == null ||
        games == null ||
        _isLoadingMore ||
        games.offset + games.items.length >= games.total) {
      return;
    }
    setState(() => _isLoadingMore = true);
    try {
      final apiClient = await _apiClient;
      if (apiClient == null) {
        throw StateError('No Hearthdeck backend is connected.');
      }
      final nextPage = await apiClient.retroGames(
        platformId: console.id,
        limit: games.limit,
        offset: games.offset + games.items.length,
      );
      if (mounted && _selectedConsole?.id == console.id) {
        setState(() {
          _games = HearthdeckRetroGamePage(
            items: <HearthdeckRetroGame>[...games.items, ...nextPage.items],
            total: nextPage.total,
            limit: nextPage.limit,
            offset: games.offset,
          );
        });
      }
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Could not load more games: $error')),
        );
      }
    } finally {
      if (mounted) {
        setState(() => _isLoadingMore = false);
      }
    }
  }

  Future<void> _refresh() async {
    final console = _selectedConsole;
    if (console == null) {
      setState(() => _consoles = _loadConsoles());
      return;
    }
    await _selectConsole(console, force: true);
  }

  @override
  Widget build(BuildContext context) {
    // Escape/back is handled globally (see main.dart's HardwareKeyboard
    // listener), regardless of what has focus on this screen.
    return TvDirectionalFocusNavigation(
      child: widget.embedded
          ? _consoleBrowser()
          : Scaffold(body: SafeArea(child: _consoleBrowser())),
    );
  }

  void _openSearch() {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        settings: const RouteSettings(name: '/search'),
        builder: (BuildContext context) => TvSearchPage(
          initialCategory: LibraryCategory.consoleGames,
          apiClient: _connectedApiClient,
        ),
      ),
    );
  }

  Widget _consoleBrowser() => Stack(
    children: <Widget>[
      const Positioned.fill(child: _RetroBackdrop()),
      FutureBuilder<List<HearthdeckRetroConsole>>(
        future: _consoles,
        builder:
            (
              BuildContext context,
              AsyncSnapshot<List<HearthdeckRetroConsole>> snapshot,
            ) {
              final consoles = snapshot.data;
              if (snapshot.connectionState != ConnectionState.done) {
                return const Center(child: CircularProgressIndicator());
              }
              if (snapshot.hasError || consoles == null) {
                return const _RetroState(
                  icon: Icons.link_off_rounded,
                  title: 'RomM unavailable',
                  message:
                      'Configure the local RomM connection in Hearthdeck and try again.',
                );
              }
              if (consoles.isEmpty) {
                return const _RetroState(
                  icon: Icons.sports_esports_outlined,
                  title: 'No consoles found',
                  message: 'Scan games in RomM, then reopen Retro.',
                );
              }
              final selected = _selectedConsole ?? consoles.first;
              if (_selectedConsole == null) {
                WidgetsBinding.instance.addPostFrameCallback(
                  (_) => _selectConsole(selected),
                );
              }
              return _RetroContent(
                consoles: consoles,
                selectedConsole: selected,
                games: _games,
                gamesError: _gamesError,
                isLoadingGames: _isLoadingGames,
                isLoadingMore: _isLoadingMore,
                onConsoleSelected: _selectConsole,
                onLoadMore: _loadMore,
                onRefresh: _refresh,
                onSearch: _openSearch,
                gameItemFor: (HearthdeckRetroGame game) =>
                    retroGameToDashboardItem(
                      game,
                      apiClient: _connectedApiClient,
                    ),
                onOpenGame: (DashboardItem item) {
                  Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      settings: RouteSettings(name: '/retro/${item.id}'),
                      builder: (BuildContext context) => ContentDetailsPage(
                        item: item,
                        sourceShape: TvTileShape.square,
                      ),
                    ),
                  );
                },
              );
            },
      ),
    ],
  );
}

class _RetroContent extends StatelessWidget {
  const _RetroContent({
    required this.consoles,
    required this.selectedConsole,
    required this.games,
    required this.gamesError,
    required this.isLoadingGames,
    required this.isLoadingMore,
    required this.onConsoleSelected,
    required this.onLoadMore,
    required this.onRefresh,
    required this.onSearch,
    required this.gameItemFor,
    required this.onOpenGame,
  });

  final List<HearthdeckRetroConsole> consoles;
  final HearthdeckRetroConsole selectedConsole;
  final HearthdeckRetroGamePage? games;
  final Object? gamesError;
  final bool isLoadingGames;
  final bool isLoadingMore;
  final ValueChanged<HearthdeckRetroConsole> onConsoleSelected;
  final VoidCallback onLoadMore;
  final VoidCallback onRefresh;
  final VoidCallback onSearch;
  final DashboardItem Function(HearthdeckRetroGame game) gameItemFor;
  final ValueChanged<DashboardItem> onOpenGame;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return LayoutBuilder(
      builder: (BuildContext context, BoxConstraints constraints) {
        final scale = (constraints.biggest.shortestSide / 720)
            .clamp(0.72, 1.3)
            .toDouble();
        final padding = (constraints.maxWidth * 0.04)
            .clamp(24 * scale, 76 * scale)
            .toDouble();
        final gap = 14 * scale;
        final maxTileExtent = (constraints.maxWidth * 0.16)
            .clamp(148 * scale, 244 * scale)
            .toDouble();
        final page = games;
        final gameItems = page?.items.map(gameItemFor).toList(growable: false);
        return CustomScrollView(
          scrollCacheExtent: ScrollCacheExtent.viewport(2),
          slivers: <Widget>[
            SliverPadding(
              padding: EdgeInsets.fromLTRB(padding, padding, padding, 56),
              sliver: SliverMainAxisGroup(
                slivers: <Widget>[
                  SliverToBoxAdapter(
                    child: Row(
                      children: <Widget>[
                        Expanded(
                          child: Text(
                            'Console games',
                            style: Theme.of(context).textTheme.displaySmall,
                          ),
                        ),
                        TvDetailAction(
                          action: const ContentAction(
                            id: 'search',
                            label: 'Search',
                            icon: Icons.search_rounded,
                          ),
                          onActivate: onSearch,
                        ),
                        const SizedBox(width: 10),
                        TvDetailAction(
                          action: const ContentAction(
                            id: 'refresh',
                            label: 'Refresh',
                            icon: Icons.refresh_rounded,
                          ),
                          onActivate: onRefresh,
                        ),
                      ],
                    ),
                  ),
                  SliverToBoxAdapter(child: SizedBox(height: gap * 0.75)),
                  SliverToBoxAdapter(
                    child: Text(
                      'Live from your local RomM library',
                      style: TextStyle(color: tv.secondaryText),
                    ),
                  ),
                  SliverToBoxAdapter(child: SizedBox(height: gap * 2)),
                  SliverToBoxAdapter(
                    child: _ConsoleTabs(
                      consoles: consoles,
                      selectedConsole: selectedConsole,
                      onSelected: onConsoleSelected,
                      gap: gap,
                    ),
                  ),
                  SliverToBoxAdapter(child: SizedBox(height: gap * 2)),
                  if (isLoadingGames)
                    const SliverToBoxAdapter(child: _GamesLoadingState())
                  else if (gamesError != null)
                    SliverToBoxAdapter(
                      child: _GamesErrorState(onRetry: onRefresh),
                    )
                  else if (page == null)
                    const SliverToBoxAdapter(child: _GamesLoadingState())
                  else ...<Widget>[
                    SliverToBoxAdapter(
                      child: Text(
                        '${selectedConsole.displayName} - ${page.total} games',
                        style: Theme.of(context).textTheme.titleLarge,
                      ),
                    ),
                    SliverToBoxAdapter(child: SizedBox(height: gap)),
                    if (gameItems!.isEmpty)
                      const SliverToBoxAdapter(
                        child: _RetroState(
                          icon: Icons.videogame_asset_off_rounded,
                          title: 'No games in this console',
                          message: 'Scan this platform in RomM and try again.',
                        ),
                      )
                    else
                      SliverGrid.builder(
                        itemCount: gameItems.length,
                        gridDelegate: SliverGridDelegateWithMaxCrossAxisExtent(
                          maxCrossAxisExtent: maxTileExtent,
                          mainAxisSpacing: gap,
                          crossAxisSpacing: gap,
                          childAspectRatio: 0.72,
                        ),
                        itemBuilder: (BuildContext context, int index) {
                          final item = gameItems[index];
                          return TvContentTile(
                            key: ValueKey<String>('retro-game-${item.id}'),
                            item: item,
                            shape: TvTileShape.square,
                            onActivate: () => onOpenGame(item),
                          );
                        },
                      ),
                    if (page.offset + page.items.length <
                        page.total) ...<Widget>[
                      SliverToBoxAdapter(child: SizedBox(height: gap * 1.5)),
                      SliverToBoxAdapter(
                        child: Center(
                          child: TvDetailAction(
                            action: ContentAction(
                              id: 'load-more',
                              label: isLoadingMore
                                  ? 'Loading games...'
                                  : 'Load more',
                              icon: Icons.expand_more_rounded,
                            ),
                            onActivate: isLoadingMore ? () {} : onLoadMore,
                          ),
                        ),
                      ),
                    ],
                  ],
                ],
              ),
            ),
          ],
        );
      },
    );
  }
}

class _ConsoleTabs extends StatelessWidget {
  const _ConsoleTabs({
    required this.consoles,
    required this.selectedConsole,
    required this.onSelected,
    required this.gap,
  });

  final List<HearthdeckRetroConsole> consoles;
  final HearthdeckRetroConsole selectedConsole;
  final ValueChanged<HearthdeckRetroConsole> onSelected;
  final double gap;

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      child: Row(
        children: <Widget>[
          for (final console in consoles) ...<Widget>[
            if (console != consoles.first) SizedBox(width: gap),
            _ConsoleTab(
              console: console,
              isSelected: console.id == selectedConsole.id,
              onActivate: () => onSelected(console),
            ),
          ],
        ],
      ),
    );
  }
}

class _ConsoleTab extends StatelessWidget {
  const _ConsoleTab({
    required this.console,
    required this.isSelected,
    required this.onActivate,
  });

  final HearthdeckRetroConsole console;
  final bool isSelected;
  final VoidCallback onActivate;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return TvFocusable(
      semanticLabel: console.displayName,
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
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              Icon(
                Icons.videogame_asset_rounded,
                size: 18,
                color: style.foreground,
              ),
              const SizedBox(width: 8),
              Text(
                console.displayName,
                style: TextStyle(
                  color: style.foreground,
                  fontWeight: FontWeight.w700,
                ),
              ),
              const SizedBox(width: 8),
              Text(
                '${console.romCount}',
                style: TextStyle(
                  color: style.foreground.withValues(alpha: 0.72),
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}

class _GamesLoadingState extends StatelessWidget {
  const _GamesLoadingState();

  @override
  Widget build(BuildContext context) => const Padding(
    padding: EdgeInsets.symmetric(vertical: 80),
    child: Center(child: CircularProgressIndicator()),
  );
}

class _GamesErrorState extends StatelessWidget {
  const _GamesErrorState({required this.onRetry});

  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) => Center(
    child: Padding(
      padding: const EdgeInsets.symmetric(vertical: 80),
      child: Column(
        children: <Widget>[
          const Icon(Icons.cloud_off_rounded, size: 46),
          const SizedBox(height: 14),
          Text(
            'Could not load games',
            style: Theme.of(context).textTheme.titleLarge,
          ),
          const SizedBox(height: 16),
          TvDetailAction(
            action: const ContentAction(
              id: 'retry',
              label: 'Try again',
              icon: Icons.refresh_rounded,
            ),
            onActivate: onRetry,
          ),
        ],
      ),
    ),
  );
}

class _RetroState extends StatelessWidget {
  const _RetroState({
    required this.icon,
    required this.title,
    required this.message,
  });

  final IconData icon;
  final String title;
  final String message;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(40),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            Icon(icon, size: 48, color: tv.secondaryText),
            const SizedBox(height: 16),
            Text(title, style: Theme.of(context).textTheme.titleLarge),
            const SizedBox(height: 6),
            Text(message, style: TextStyle(color: tv.secondaryText)),
          ],
        ),
      ),
    );
  }
}

class _RetroBackdrop extends StatelessWidget {
  const _RetroBackdrop();

  @override
  Widget build(BuildContext context) =>
      const TvBackdrop(center: Alignment(0.64, -0.46));
}

List<Color> _colorsFor(int id) {
  const palette = <List<Color>>[
    <Color>[Color(0xFF57307E), Color(0xFF201033)],
    <Color>[Color(0xFF8B3B3C), Color(0xFF331315)],
    <Color>[Color(0xFF176765), Color(0xFF0C2D31)],
    <Color>[Color(0xFF705B20), Color(0xFF31270B)],
    <Color>[Color(0xFF315E91), Color(0xFF142944)],
  ];
  return palette[id.abs() % palette.length];
}

String _gameDescription(HearthdeckRetroGame game) {
  final parts = <String>[
    if (game.releaseYear != null) '${game.releaseYear}',
    if (game.genres.isNotEmpty) game.genres.first,
  ];
  return parts.isEmpty ? 'RomM library' : parts.join(' - ');
}

/// Converts a live RomM game into the shared [DashboardItem] shape used by
/// tiles, detail routes, and search results. [apiClient] is used to build
/// authenticated cover-art URIs; pass the client that actually served
/// [game] (e.g. `retro.dart`'s connected client, or the one used to search).
DashboardItem retroGameToDashboardItem(
  HearthdeckRetroGame game, {
  HearthdeckApiClient? apiClient,
}) {
  final coverPath = game.coverPath;
  final artworkHeaders = apiClient?.authorizationHeaders;
  final artworkUrl = coverPath == null
      ? game.coverUrl
      : apiClient?.retroAssetUri(coverPath).toString();
  return DashboardItem(
    id: 'romm:${game.id}',
    title: game.title,
    description: _gameDescription(game),
    icon: Icons.sports_esports_rounded,
    colors: _colorsFor(game.id),
    artworkUrl: artworkUrl,
    artworkHeaders: coverPath == null ? null : artworkHeaders,
    artworkFallbackUrl: coverPath == null ? null : game.coverUrl,
    artworkFit: BoxFit.contain,
    artworkAspectRatio: 0.72,
    kind: TvContentKind.game,
    details: ContentDetails(
      summary: game.summary?.trim().isNotEmpty == true
          ? game.summary!.trim()
          : 'Metadata supplied by your local RomM library.',
      actions: const <ContentAction>[],
      facts: const <ContentFact>[],
      factSections: <ContentFactSection>[
        ContentFactSection(
          title: 'Game details',
          facts: <ContentFact>[
            if (game.genres.isNotEmpty)
              ContentFact(
                label: 'Genre',
                value: game.genres.join(', '),
                icon: Icons.category_outlined,
              ),
            if (game.releaseYear != null)
              ContentFact(
                label: 'Released',
                value: '${game.releaseYear}',
                icon: Icons.calendar_today_outlined,
              ),
            if (game.playerCount?.isNotEmpty == true)
              ContentFact(
                label: 'Players',
                value: game.playerCount!,
                icon: Icons.people_outline_rounded,
              ),
            if (game.regions.isNotEmpty)
              ContentFact(
                label: 'Region',
                value: game.regions.join(', '),
                icon: Icons.public_outlined,
              ),
            if (game.hasManual)
              const ContentFact(
                label: 'Manual',
                value: 'Available in RomM',
                icon: Icons.menu_book_outlined,
              ),
          ],
        ),
      ],
      galleryTitle: 'Screenshots',
      gallery: game.screenshotPaths
          .map(
            (String path) => ContentGalleryItem(
              label: '${game.title} screenshot',
              icon: Icons.image_outlined,
              colors: _colorsFor(game.id),
              artworkUrl: apiClient?.retroAssetUri(path).toString(),
              artworkHeaders: artworkHeaders,
            ),
          )
          .toList(growable: false),
    ),
  );
}
