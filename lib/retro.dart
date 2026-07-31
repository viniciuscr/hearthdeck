import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import 'backend/hearthdeck_api_client.dart';
import 'catalog/catalog_repository_factory.dart';
import 'tv_components.dart';
import 'tv_theme.dart';
import 'virtual_keyboard.dart';

class RetroPage extends StatefulWidget {
  const RetroPage({super.key, this.apiClient});

  final HearthdeckApiClient? apiClient;

  @override
  State<RetroPage> createState() => _RetroPageState();
}

class _RetroPageState extends State<RetroPage> {
  late final Future<HearthdeckApiClient?> _apiClient = widget.apiClient == null
      ? createRetroApiClient()
      : Future<HearthdeckApiClient?>.value(widget.apiClient);
  late final Future<List<HearthdeckRetroConsole>> _consoles = _loadConsoles();

  Future<List<HearthdeckRetroConsole>> _loadConsoles() async {
    final apiClient = await _apiClient;
    if (apiClient == null) {
      throw StateError('No Hearthdeck backend is connected.');
    }
    return apiClient.retroConsoles();
  }

  @override
  Widget build(BuildContext context) {
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
            body: SafeArea(
              child: Stack(
                children: <Widget>[
                  const Positioned.fill(child: _RetroBackdrop()),
                  FutureBuilder<List<HearthdeckRetroConsole>>(
                    future: _consoles,
                    builder:
                        (
                          BuildContext context,
                          AsyncSnapshot<List<HearthdeckRetroConsole>> snapshot,
                        ) {
                          return _RetroContent(
                            consoles: snapshot.data,
                            error: snapshot.error,
                            isLoading:
                                snapshot.connectionState !=
                                ConnectionState.done,
                          );
                        },
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _RetroContent extends StatelessWidget {
  const _RetroContent({
    required this.consoles,
    required this.error,
    required this.isLoading,
  });

  final List<HearthdeckRetroConsole>? consoles;
  final Object? error;
  final bool isLoading;

  @override
  Widget build(BuildContext context) {
    const pagePadding = EdgeInsets.fromLTRB(48, 38, 48, 56);
    if (isLoading) {
      return const Center(child: CircularProgressIndicator());
    }
    final loadedConsoles = consoles;
    if (error != null || loadedConsoles == null) {
      return _RetroState(
        icon: Icons.link_off_rounded,
        title: 'RomM unavailable',
        message:
            'Configure the local RomM connection in Hearthdeck and try again.',
      );
    }
    if (loadedConsoles.isEmpty) {
      return const _RetroState(
        icon: Icons.sports_esports_outlined,
        title: 'No consoles found',
        message: 'Scan games in RomM, then reopen Retro.',
      );
    }
    return CustomScrollView(
      scrollCacheExtent: ScrollCacheExtent.viewport(2),
      slivers: <Widget>[
        SliverPadding(
          padding: pagePadding,
          sliver: SliverMainAxisGroup(
            slivers: <Widget>[
              SliverToBoxAdapter(
                child: Text(
                  'Retro',
                  style: Theme.of(context).textTheme.displaySmall,
                ),
              ),
              const SliverToBoxAdapter(child: SizedBox(height: 10)),
              SliverToBoxAdapter(
                child: Text(
                  '${loadedConsoles.length} consoles from your local RomM library',
                  style: TextStyle(color: TvPalette.of(context).secondaryText),
                ),
              ),
              const SliverToBoxAdapter(child: SizedBox(height: 34)),
              SliverGrid.builder(
                itemCount: loadedConsoles.length,
                gridDelegate: const SliverGridDelegateWithMaxCrossAxisExtent(
                  maxCrossAxisExtent: 230,
                  mainAxisSpacing: 18,
                  crossAxisSpacing: 18,
                  childAspectRatio: 1,
                ),
                itemBuilder: (BuildContext context, int index) {
                  final console = loadedConsoles[index];
                  return _ConsoleTile(console: console);
                },
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _ConsoleTile extends StatelessWidget {
  const _ConsoleTile({required this.console});

  final HearthdeckRetroConsole console;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return AnimatedContainer(
      duration: TvTheme.focusDuration,
      curve: TvTheme.focusCurve,
      padding: const EdgeInsets.all(18),
      decoration: BoxDecoration(
        gradient: LinearGradient(colors: _colorsFor(console.id)),
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: tv.borderSubtle),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          const Icon(Icons.videogame_asset_rounded, size: 42),
          const Spacer(),
          Text(
            console.displayName,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 4),
          Text(
            '${console.romCount} games',
            style: TextStyle(color: tv.secondaryText),
          ),
        ],
      ),
    );
  }
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
