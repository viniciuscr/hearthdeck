import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import 'backend/hearthdeck_api_client.dart';
import 'catalog/catalog_repository.dart';
import 'catalog/catalog_repository_factory.dart';
import 'tv_components.dart';
import 'tv_theme.dart';

class SystemHealthPage extends StatefulWidget {
  const SystemHealthPage({super.key, this.catalogRepository});

  final CatalogRepository? catalogRepository;

  @override
  State<SystemHealthPage> createState() => _SystemHealthPageState();
}

class _SystemHealthPageState extends State<SystemHealthPage> {
  late final CatalogRepository _catalogRepository =
      widget.catalogRepository ?? createCatalogRepository();
  HearthdeckHealth? _health;
  Object? _error;
  var _isLoading = true;

  @override
  void initState() {
    super.initState();
    _refresh();
  }

  Future<void> _refresh() async {
    setState(() {
      _isLoading = true;
      _error = null;
    });
    try {
      final health = await _catalogRepository.health();
      if (mounted) {
        setState(() {
          _health = health;
          _isLoading = false;
        });
      }
    } catch (error) {
      if (mounted) {
        setState(() {
          _error = error;
          _isLoading = false;
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
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
                  const Positioned.fill(child: _SystemHealthBackdrop()),
                  CustomScrollView(
                    scrollCacheExtent: ScrollCacheExtent.viewport(2),
                    slivers: <Widget>[
                      SliverPadding(
                        padding: const EdgeInsets.fromLTRB(48, 38, 48, 56),
                        sliver: SliverMainAxisGroup(
                          slivers: <Widget>[
                            SliverToBoxAdapter(
                              child: _HealthHeader(
                                health: _health,
                                isLoading: _isLoading,
                                onRefresh: _refresh,
                              ),
                            ),
                            const SliverToBoxAdapter(
                              child: SizedBox(height: 34),
                            ),
                            if (_error case final Object error)
                              SliverToBoxAdapter(
                                child: _HealthError(error: error),
                              )
                            else if (_isLoading)
                              const SliverToBoxAdapter(
                                child: Center(
                                  child: Padding(
                                    padding: EdgeInsets.all(42),
                                    child: CircularProgressIndicator(),
                                  ),
                                ),
                              )
                            else if (_health!.providers.isEmpty)
                              const SliverToBoxAdapter(
                                child: _NoProviderHealth(),
                              )
                            else
                              SliverGrid.builder(
                                itemCount: _health!.providers.length,
                                gridDelegate:
                                    const SliverGridDelegateWithMaxCrossAxisExtent(
                                      maxCrossAxisExtent: 520,
                                      mainAxisSpacing: 18,
                                      crossAxisSpacing: 18,
                                      childAspectRatio: 1.55,
                                    ),
                                itemBuilder:
                                    (BuildContext context, int index) =>
                                        _ProviderHealthCard(
                                          provider: _health!.providers[index],
                                        ),
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
  }
}

class _HealthHeader extends StatelessWidget {
  const _HealthHeader({
    required this.health,
    required this.isLoading,
    required this.onRefresh,
  });

  final HearthdeckHealth? health;
  final bool isLoading;
  final Future<void> Function() onRefresh;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final providerCount = health?.providers.length ?? 0;
    final degradedCount =
        health?.providers
            .where((provider) => provider.status == 'degraded')
            .length ??
        0;
    return Row(
      children: <Widget>[
        DecoratedBox(
          decoration: BoxDecoration(
            color: tv.accent.withValues(alpha: 0.12),
            shape: BoxShape.circle,
          ),
          child: Padding(
            padding: EdgeInsets.all(14),
            child: Icon(Icons.monitor_heart_outlined, color: tv.accent),
          ),
        ),
        const SizedBox(width: 18),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              Text(
                'System health',
                style: Theme.of(context).textTheme.displaySmall,
              ),
              const SizedBox(height: 5),
              Text(
                health == null
                    ? 'Connecting to the local Hearthdeck daemon'
                    : degradedCount == 0
                    ? '$providerCount providers reporting healthy state'
                    : '$degradedCount provider${degradedCount == 1 ? '' : 's'} needs attention',
                style: TextStyle(color: tv.secondaryText),
              ),
            ],
          ),
        ),
        TvFocusable(
          semanticLabel: 'Refresh system health',
          onActivate: isLoading ? null : () => onRefresh(),
          builder: (BuildContext context, bool isFocused) {
            final style = TvControlStyle.resolve(
              tv,
              variant: TvControlVariant.icon,
              isFocused: isFocused,
            );
            return AnimatedContainer(
              duration: TvTheme.focusDuration,
              curve: TvTheme.focusCurve,
              padding: const EdgeInsets.all(13),
              decoration: BoxDecoration(
                color: style.background,
                borderRadius: BorderRadius.circular(10),
                border: Border.all(color: style.border, width: 2),
              ),
              child: Icon(Icons.refresh_rounded, color: style.foreground),
            );
          },
        ),
      ],
    );
  }
}

class _ProviderHealthCard extends StatelessWidget {
  const _ProviderHealthCard({required this.provider});

  final HearthdeckProviderHealth provider;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final status = _ProviderStatus.from(provider.status, tv);
    return AnimatedContainer(
      duration: TvTheme.focusDuration,
      curve: TvTheme.focusCurve,
      padding: const EdgeInsets.all(22),
      decoration: BoxDecoration(
        color: tv.surface,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: status.color.withValues(alpha: 0.38)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          Row(
            children: <Widget>[
              Icon(status.icon, color: status.color),
              const SizedBox(width: 10),
              Expanded(
                child: Text(
                  _labelFor(provider.id),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: Theme.of(context).textTheme.titleLarge,
                ),
              ),
              _StatusPill(status: status),
            ],
          ),
          const Spacer(),
          Text(
            provider.kind == 'discovery'
                ? '${provider.recordCount ?? 0} launchable items'
                : '${provider.recordCount ?? 0} metadata records',
            style: TextStyle(color: tv.secondaryText),
          ),
          const SizedBox(height: 7),
          Text(
            provider.lastError ??
                (provider.lastSuccessAt == null
                    ? 'Awaiting first refresh'
                    : 'Last refresh ${_formatTimestamp(provider.lastSuccessAt!)}'),
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              color: provider.lastError == null ? tv.primaryText : tv.warning,
            ),
          ),
        ],
      ),
    );
  }

  static String _labelFor(String id) => id
      .split('-')
      .map(
        (String word) => word.isEmpty
            ? word
            : '${word[0].toUpperCase()}${word.substring(1)}',
      )
      .join(' ');

  static String _formatTimestamp(String value) {
    final timestamp = DateTime.tryParse(value)?.toLocal();
    if (timestamp == null) {
      return 'Last refresh unavailable';
    }
    return 'Last refresh ${timestamp.hour.toString().padLeft(2, '0')}:${timestamp.minute.toString().padLeft(2, '0')}';
  }
}

class _StatusPill extends StatelessWidget {
  const _StatusPill({required this.status});

  final _ProviderStatus status;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: status.color.withValues(alpha: 0.14),
        borderRadius: BorderRadius.circular(20),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
        child: Text(
          status.label,
          style: TextStyle(color: status.color, fontWeight: FontWeight.w700),
        ),
      ),
    );
  }
}

class _ProviderStatus {
  const _ProviderStatus(this.label, this.color, this.icon);

  factory _ProviderStatus.from(String value, TvPalette tv) => switch (value) {
    'ready' => _ProviderStatus(
      'Ready',
      tv.success,
      Icons.check_circle_outline_rounded,
    ),
    'degraded' => _ProviderStatus(
      'Attention',
      tv.warning,
      Icons.error_outline_rounded,
    ),
    _ => _ProviderStatus('Starting', tv.info, Icons.hourglass_top_rounded),
  };

  final String label;
  final Color color;
  final IconData icon;
}

class _HealthError extends StatelessWidget {
  const _HealthError({required this.error});

  final Object error;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tv.warning.withValues(alpha: 0.14),
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: tv.warning.withValues(alpha: 0.55)),
      ),
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Row(
          children: <Widget>[
            Icon(Icons.cloud_off_rounded, color: tv.warning),
            const SizedBox(width: 14),
            Expanded(
              child: Text(
                'Could not read system health: $error',
                style: TextStyle(color: tv.primaryText),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _NoProviderHealth extends StatelessWidget {
  const _NoProviderHealth();

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Center(
      child: Padding(
        padding: EdgeInsets.all(42),
        child: Column(
          children: <Widget>[
            Icon(Icons.info_outline_rounded, size: 42, color: tv.secondaryText),
            const SizedBox(height: 12),
            const Text('No live provider data'),
            const SizedBox(height: 4),
            Text(
              'Service health is available when connected to a Hearthdeck daemon.',
              style: TextStyle(color: tv.secondaryText),
            ),
          ],
        ),
      ),
    );
  }
}

class _SystemHealthBackdrop extends StatelessWidget {
  const _SystemHealthBackdrop();

  @override
  Widget build(BuildContext context) {
    return const TvBackdrop(center: Alignment(-0.35, -0.55));
  }
}
