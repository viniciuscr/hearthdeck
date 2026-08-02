import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';

import 'backend/hearthdeck_api_client.dart';
import 'catalog/catalog_repository.dart';
import 'catalog/catalog_repository_factory.dart';
import 'frontend_log.dart';
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
  HearthdeckDiagnostics? _diagnostics;
  Object? _error;
  String? _refreshingProviderId;
  Timer? _refreshTimer;
  var _refreshInFlight = false;
  var _isLoading = true;

  @override
  void initState() {
    super.initState();
    _refresh();
    _refreshTimer = Timer.periodic(
      const Duration(seconds: 5),
      (_) => unawaited(_refresh(showLoading: false)),
    );
  }

  @override
  void dispose() {
    _refreshTimer?.cancel();
    super.dispose();
  }

  Future<void> _refresh({bool showLoading = true}) async {
    if (_refreshInFlight) {
      return;
    }
    _refreshInFlight = true;
    if (showLoading) {
      setState(() {
        _isLoading = true;
        _error = null;
      });
    }
    try {
      final results = await Future.wait<Object>(<Future<Object>>[
        _catalogRepository.health(),
        _catalogRepository.diagnostics(),
      ]);
      if (mounted) {
        setState(() {
          _health = results[0] as HearthdeckHealth;
          _diagnostics = results[1] as HearthdeckDiagnostics;
          _isLoading = false;
          _error = null;
        });
      }
    } catch (error) {
      FrontendLog.instance.warning('Could not read service diagnostics: $error');
      if (mounted && (showLoading || _health == null)) {
        setState(() {
          _error = error;
          _isLoading = false;
        });
      }
    } finally {
      _refreshInFlight = false;
    }
  }

  Future<void> _refreshProvider(HearthdeckProviderHealth provider) async {
    setState(() => _refreshingProviderId = provider.id);
    try {
      await _catalogRepository.requestProviderRefresh(provider);
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text('${_labelFor(provider.id)} refresh requested.'),
          ),
        );
        await _refresh();
      }
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text(
              'Could not refresh ${_labelFor(provider.id)}: $error',
            ),
          ),
        );
      }
    } finally {
      if (mounted) {
        setState(() => _refreshingProviderId = null);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    // Escape/back is handled globally (see main.dart's HardwareKeyboard
    // listener), regardless of what has focus on this screen.
    return TvDirectionalFocusNavigation(
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
                            diagnostics: _diagnostics,
                            isLoading: _isLoading,
                            onRefresh: _refresh,
                          ),
                        ),
                        const SliverToBoxAdapter(child: SizedBox(height: 34)),
                        if (_error case final Object error)
                          SliverToBoxAdapter(child: _HealthError(error: error))
                        else if (_isLoading)
                          const SliverToBoxAdapter(
                            child: Center(
                              child: Padding(
                                padding: EdgeInsets.all(42),
                                child: CircularProgressIndicator(),
                              ),
                            ),
                          )
                        else ...<Widget>[
                          SliverToBoxAdapter(
                            child: _SectionTitle(
                              title: 'Local services',
                              subtitle:
                                  'Daemon, bridge socket, and bridge activation state',
                            ),
                          ),
                          const SliverToBoxAdapter(child: SizedBox(height: 14)),
                          SliverGrid.builder(
                            itemCount: _diagnostics!.services.length,
                            gridDelegate:
                                const SliverGridDelegateWithMaxCrossAxisExtent(
                                  maxCrossAxisExtent: 390,
                                  mainAxisSpacing: 14,
                                  crossAxisSpacing: 14,
                                  childAspectRatio: 2.05,
                                ),
                            itemBuilder: (BuildContext context, int index) =>
                                _ServiceStatusCard(
                                  service: _diagnostics!.services[index],
                                ),
                          ),
                          const SliverToBoxAdapter(child: SizedBox(height: 34)),
                          SliverToBoxAdapter(
                            child: _SectionTitle(
                              title: 'Library sources',
                              subtitle:
                                  'Last scan results for desktop apps, Heroic, and metadata sources',
                            ),
                          ),
                          const SliverToBoxAdapter(child: SizedBox(height: 14)),
                          if (_health!.providers.isEmpty)
                            const SliverToBoxAdapter(child: _NoProviderHealth())
                          else
                            SliverGrid.builder(
                              itemCount: _health!.providers.length,
                              gridDelegate:
                                  const SliverGridDelegateWithMaxCrossAxisExtent(
                                    maxCrossAxisExtent: 520,
                                    mainAxisSpacing: 18,
                                    crossAxisSpacing: 18,
                                    childAspectRatio: 1.35,
                                  ),
                              itemBuilder: (BuildContext context, int index) =>
                                  _ProviderHealthCard(
                                    provider: _health!.providers[index],
                                    isRefreshing:
                                        _refreshingProviderId ==
                                        _health!.providers[index].id,
                                    onRefresh: () => _refreshProvider(
                                      _health!.providers[index],
                                    ),
                                  ),
                            ),
                          const SliverToBoxAdapter(child: SizedBox(height: 34)),
                          SliverToBoxAdapter(
                            child: _SectionTitle(
                              title: 'RomM connection',
                              subtitle:
                                  'A live platform request verifies the configured local server',
                            ),
                          ),
                          const SliverToBoxAdapter(child: SizedBox(height: 14)),
                          SliverToBoxAdapter(
                            child: _RommDiagnosticCard(
                              diagnostic: _diagnostics!.romm,
                            ),
                          ),
                          const SliverToBoxAdapter(child: SizedBox(height: 34)),
                          SliverToBoxAdapter(
                            child: _SectionTitle(
                              title: 'Recent service events',
                              subtitle:
                                  'A live terminal for daemon, service API, bridge, RomM, and frontend events',
                            ),
                          ),
                          const SliverToBoxAdapter(child: SizedBox(height: 14)),
                          SliverToBoxAdapter(
                            child: _LogTerminalSection(logs: _diagnostics!.logs),
                          ),
                        ],
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
  }
}

class _HealthHeader extends StatelessWidget {
  const _HealthHeader({
    required this.health,
    required this.diagnostics,
    required this.isLoading,
    required this.onRefresh,
  });

  final HearthdeckHealth? health;
  final HearthdeckDiagnostics? diagnostics;
  final bool isLoading;
  final Future<void> Function() onRefresh;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final providers = health?.providers ?? const <HearthdeckProviderHealth>[];
    final degraded = providers.where(
      (provider) => provider.status == 'degraded',
    );
    final failedServices = diagnostics?.services.where(
      (service) => service.state == 'failed' || service.state == 'unavailable',
    );
    final needsAttention =
        degraded.isNotEmpty || (failedServices?.isNotEmpty ?? false);
    return Row(
      children: <Widget>[
        DecoratedBox(
          decoration: BoxDecoration(
            color: (needsAttention ? tv.warning : tv.accent).withValues(
              alpha: 0.12,
            ),
            shape: BoxShape.circle,
          ),
          child: Padding(
            padding: const EdgeInsets.all(14),
            child: Icon(
              needsAttention
                  ? Icons.warning_amber_rounded
                  : Icons.monitor_heart_outlined,
              color: needsAttention ? tv.warning : tv.accent,
            ),
          ),
        ),
        const SizedBox(width: 18),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              Text(
                'Service status',
                style: Theme.of(context).textTheme.displaySmall,
              ),
              const SizedBox(height: 5),
              Text(
                health == null
                    ? 'Connecting to the local Hearthdeck daemon'
                    : needsAttention
                    ? 'Review service and source errors below'
                    : '${providers.length} sources and local services reporting normally',
                style: TextStyle(color: tv.secondaryText),
              ),
            ],
          ),
        ),
        _RefreshButton(isLoading: isLoading, onRefresh: onRefresh),
      ],
    );
  }
}

class _RefreshButton extends StatelessWidget {
  const _RefreshButton({required this.isLoading, required this.onRefresh});

  final bool isLoading;
  final Future<void> Function() onRefresh;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return TvFocusable(
      semanticLabel: 'Refresh service status',
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
    );
  }
}

class _SectionTitle extends StatelessWidget {
  const _SectionTitle({required this.title, required this.subtitle});

  final String title;
  final String subtitle;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        Text(title, style: Theme.of(context).textTheme.titleLarge),
        const SizedBox(height: 4),
        Text(subtitle, style: TextStyle(color: tv.secondaryText)),
      ],
    );
  }
}

class _ServiceStatusCard extends StatelessWidget {
  const _ServiceStatusCard({required this.service});

  final HearthdeckServiceStatus service;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final status = _ServiceStatus.from(service.state, tv);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tv.surface,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: status.color.withValues(alpha: 0.38)),
      ),
      child: Padding(
        padding: const EdgeInsets.all(18),
        child: Row(
          children: <Widget>[
            Icon(status.icon, color: status.color, size: 30),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text(
                    _serviceName(service.id),
                    style: Theme.of(context).textTheme.titleMedium,
                  ),
                  const SizedBox(height: 4),
                  Text(
                    service.detail,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(color: tv.secondaryText),
                  ),
                ],
              ),
            ),
            _StatusPill(label: status.label, color: status.color),
          ],
        ),
      ),
    );
  }
}

class _ProviderHealthCard extends StatelessWidget {
  const _ProviderHealthCard({
    required this.provider,
    required this.isRefreshing,
    required this.onRefresh,
  });

  final HearthdeckProviderHealth provider;
  final bool isRefreshing;
  final VoidCallback onRefresh;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final status = _ProviderStatus.from(provider.status, tv);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tv.surface,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: status.color.withValues(alpha: 0.38)),
      ),
      child: Padding(
        padding: const EdgeInsets.all(20),
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
                _StatusPill(label: status.label, color: status.color),
              ],
            ),
            const SizedBox(height: 16),
            Text(
              provider.kind == 'discovery'
                  ? '${provider.recordCount ?? 0} discovered items'
                  : '${provider.recordCount ?? 0} metadata records',
              style: TextStyle(color: tv.secondaryText),
            ),
            const SizedBox(height: 6),
            Text(
              provider.lastError ?? _providerTiming(provider),
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                color: provider.lastError == null ? tv.primaryText : tv.warning,
              ),
            ),
            const Spacer(),
            _ProviderRefreshButton(
              label: isRefreshing ? 'Refreshing...' : 'Refresh source',
              onActivate: isRefreshing ? null : onRefresh,
            ),
          ],
        ),
      ),
    );
  }
}

class _ProviderRefreshButton extends StatelessWidget {
  const _ProviderRefreshButton({required this.label, required this.onActivate});

  final String label;
  final VoidCallback? onActivate;

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
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          decoration: BoxDecoration(
            color: style.background,
            borderRadius: BorderRadius.circular(7),
            border: Border.all(color: style.border),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              Icon(Icons.refresh_rounded, size: 22, color: style.foreground),
              const SizedBox(width: 7),
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

class _RommDiagnosticCard extends StatelessWidget {
  const _RommDiagnosticCard({required this.diagnostic});

  final HearthdeckRommDiagnostic diagnostic;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final status = _RommStatus.from(diagnostic.status, tv);
    final detail = switch (diagnostic.status) {
      'ready' =>
        '${diagnostic.consoleCount ?? 0} consoles available from ${diagnostic.baseUrl}',
      'not_configured' =>
        'No RomM server is configured. Set one up in Retro & RomM.',
      _ =>
        diagnostic.error ?? 'The configured RomM server could not be reached.',
    };
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tv.surface,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: status.color.withValues(alpha: 0.38)),
      ),
      child: Padding(
        padding: const EdgeInsets.all(22),
        child: Row(
          children: <Widget>[
            Icon(status.icon, color: status.color, size: 34),
            const SizedBox(width: 16),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Text('RomM', style: Theme.of(context).textTheme.titleLarge),
                  const SizedBox(height: 6),
                  Text(detail, style: TextStyle(color: tv.secondaryText)),
                  const SizedBox(height: 5),
                  Text(
                    'Checked ${_formatTimestamp(diagnostic.checkedAt)}',
                    style: TextStyle(
                      color: tv.secondaryText,
                      fontSize: TvTheme.labelSmallSize,
                    ),
                  ),
                ],
              ),
            ),
            _StatusPill(label: status.label, color: status.color),
          ],
        ),
      ),
    );
  }
}

/// A source tab shown in the log terminal's left rail.
class _LogSourceDef {
  const _LogSourceDef(this.id, this.label, this.icon);

  final String id;
  final String label;
  final IconData icon;
}

const _logSourceTabs = <_LogSourceDef>[
  _LogSourceDef('all', 'All', Icons.dvr_outlined),
  _LogSourceDef('daemon', 'Daemon', Icons.dns_outlined),
  _LogSourceDef('api', 'Service API', Icons.http_rounded),
  _LogSourceDef('bridge', 'Bridge', Icons.cable_rounded),
  _LogSourceDef('romm', 'RomM', Icons.videogame_asset_outlined),
  _LogSourceDef('frontend', 'Frontend', Icons.smartphone_rounded),
];

/// A single normalized line for the terminal, merging the backend's
/// daemon/api/bridge/romm log tail with this running app's own in-memory
/// [FrontendLog] entries into one timeline.
class _TerminalLine {
  const _TerminalLine({
    required this.timestamp,
    required this.source,
    required this.level,
    required this.message,
  });

  factory _TerminalLine.fromBackend(HearthdeckLogEntry entry) =>
      _TerminalLine(
        timestamp: entry.timestamp == null
            ? null
            : DateTime.tryParse(entry.timestamp!)?.toLocal(),
        source: entry.source,
        level: entry.level,
        message: entry.message,
      );

  factory _TerminalLine.fromFrontend(FrontendLogEntry entry) => _TerminalLine(
    timestamp: entry.timestamp,
    source: 'frontend',
    level: entry.level,
    message: entry.message,
  );

  final DateTime? timestamp;
  final String source;
  final String level;
  final String message;
}

List<_TerminalLine> _combinedLogLines(HearthdeckLogTail logs) {
  final lines = <_TerminalLine>[
    ...logs.entries.map(_TerminalLine.fromBackend),
    ...FrontendLog.instance.entries.map(_TerminalLine.fromFrontend),
  ];
  lines.sort((a, b) {
    if (a.timestamp == null && b.timestamp == null) {
      return 0;
    }
    if (a.timestamp == null) {
      return 1;
    }
    if (b.timestamp == null) {
      return -1;
    }
    return b.timestamp!.compareTo(a.timestamp!);
  });
  return lines;
}

/// Replaces the old bordered-card log list with a single, fast terminal
/// view: a left rail to switch between log sources, and a monospace
/// scrolling pane on the right that auto-follows new lines unless the user
/// has scrolled up to read something older.
class _LogTerminalSection extends StatefulWidget {
  const _LogTerminalSection({required this.logs});

  final HearthdeckLogTail logs;

  @override
  State<_LogTerminalSection> createState() => _LogTerminalSectionState();
}

class _LogTerminalSectionState extends State<_LogTerminalSection> {
  String _selectedSource = 'all';
  final _scrollController = ScrollController();
  var _followLatest = true;

  @override
  void initState() {
    super.initState();
    _scrollController.addListener(_handleScroll);
  }

  @override
  void dispose() {
    _scrollController.removeListener(_handleScroll);
    _scrollController.dispose();
    super.dispose();
  }

  void _handleScroll() {
    if (!_scrollController.hasClients) {
      return;
    }
    final position = _scrollController.position;
    final atBottom = position.pixels >= position.maxScrollExtent - 24;
    if (atBottom != _followLatest) {
      setState(() => _followLatest = atBottom);
    }
  }

  void _scheduleAutoScroll() {
    if (!_followLatest) {
      return;
    }
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_scrollController.hasClients) {
        return;
      }
      _scrollController.jumpTo(_scrollController.position.maxScrollExtent);
    });
  }

  String? _emptyMessage(List<_TerminalLine> filtered) {
    if (filtered.isNotEmpty) {
      return null;
    }
    if (_selectedSource == 'frontend') {
      return 'No frontend events yet.';
    }
    if (!widget.logs.available) {
      return widget.logs.error ?? 'No recent service events are available.';
    }
    return switch (_selectedSource) {
      'daemon' => 'No recent daemon events.',
      'api' => 'No recent service API requests.',
      'bridge' => 'No recent bridge events.',
      'romm' => 'No RomM connectivity events yet.',
      _ => 'No recent events to show.',
    };
  }

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return ListenableBuilder(
      listenable: FrontendLog.instance,
      builder: (BuildContext context, Widget? _) {
        final combined = _combinedLogLines(widget.logs);
        final filtered = _selectedSource == 'all'
            ? combined
            : combined
                  .where((line) => line.source == _selectedSource)
                  .toList(growable: false);
        final display = filtered.reversed.toList(growable: false);
        _scheduleAutoScroll();
        return DecoratedBox(
          decoration: BoxDecoration(
            color: tv.surface,
            borderRadius: BorderRadius.circular(14),
            border: Border.all(color: tv.borderSubtle),
          ),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(14),
            child: SizedBox(
              height: 420,
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: <Widget>[
                  _LogSourceRail(
                    selected: _selectedSource,
                    onSelected: (String id) =>
                        setState(() => _selectedSource = id),
                  ),
                  Expanded(
                    child: _LogTerminalViewport(
                      lines: display,
                      scrollController: _scrollController,
                      emptyMessage: _emptyMessage(filtered),
                    ),
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

class _LogSourceRail extends StatelessWidget {
  const _LogSourceRail({required this.selected, required this.onSelected});

  final String selected;
  final ValueChanged<String> onSelected;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        border: Border(right: BorderSide(color: tv.borderSubtle)),
      ),
      child: SizedBox(
        width: 172,
        child: ListView(
          padding: const EdgeInsets.all(10),
          children: <Widget>[
            for (final tab in _logSourceTabs) ...<Widget>[
              _LogSourceTabButton(
                tab: tab,
                isSelected: tab.id == selected,
                onActivate: () => onSelected(tab.id),
              ),
              const SizedBox(height: 6),
            ],
          ],
        ),
      ),
    );
  }
}

class _LogSourceTabButton extends StatelessWidget {
  const _LogSourceTabButton({
    required this.tab,
    required this.isSelected,
    required this.onActivate,
  });

  final _LogSourceDef tab;
  final bool isSelected;
  final VoidCallback onActivate;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return TvFocusable(
      semanticLabel: '${tab.label} logs',
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
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
          decoration: BoxDecoration(
            color: style.background,
            borderRadius: BorderRadius.circular(8),
            border: Border.all(color: style.border, width: 2),
          ),
          child: Row(
            children: <Widget>[
              Icon(tab.icon, size: 18, color: style.foreground),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  tab.label,
                  maxLines: 1,
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

class _LogTerminalViewport extends StatelessWidget {
  const _LogTerminalViewport({
    required this.lines,
    required this.scrollController,
    required this.emptyMessage,
  });

  final List<_TerminalLine> lines;
  final ScrollController scrollController;
  final String? emptyMessage;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    if (emptyMessage case final String message) {
      return ColoredBox(
        color: tv.canvas,
        child: Center(
          child: Padding(
            padding: const EdgeInsets.all(28),
            child: Text(
              message,
              textAlign: TextAlign.center,
              style: TextStyle(
                color: tv.secondaryText,
                fontFamily: 'monospace',
              ),
            ),
          ),
        ),
      );
    }
    return ColoredBox(
      color: tv.canvas,
      child: ListView.builder(
        controller: scrollController,
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        itemCount: lines.length,
        itemBuilder: (BuildContext context, int index) =>
            _TerminalLineRow(line: lines[index]),
      ),
    );
  }
}

class _TerminalLineRow extends StatelessWidget {
  const _TerminalLineRow({required this.line});

  final _TerminalLine line;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final color = switch (line.level) {
      'error' || 'warning' => tv.warning,
      'debug' => tv.secondaryText,
      _ => tv.info,
    };
    const monospace = TextStyle(fontFamily: 'monospace', fontSize: 13);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 2),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          Text(
            line.timestamp == null ? '--:--:--' : _formatClock(line.timestamp!),
            style: monospace.copyWith(color: tv.secondaryText),
          ),
          const SizedBox(width: 10),
          SizedBox(
            width: 58,
            child: Text(
              line.level.toUpperCase(),
              style: monospace.copyWith(color: color, fontWeight: FontWeight.w700),
            ),
          ),
          SizedBox(
            width: 74,
            child: Text(
              line.source,
              overflow: TextOverflow.ellipsis,
              style: monospace.copyWith(color: tv.secondaryText),
            ),
          ),
          Expanded(
            child: Text(
              line.message,
              style: monospace.copyWith(color: tv.primaryText),
            ),
          ),
        ],
      ),
    );
  }
}

String _formatClock(DateTime value) =>
    '${value.hour.toString().padLeft(2, '0')}:'
    '${value.minute.toString().padLeft(2, '0')}:'
    '${value.second.toString().padLeft(2, '0')}';

class _DiagnosticsEmptyState extends StatelessWidget {
  const _DiagnosticsEmptyState({required this.icon, required this.message});

  final IconData icon;
  final String message;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tv.surface,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: tv.borderSubtle),
      ),
      child: Padding(
        padding: const EdgeInsets.all(28),
        child: Row(
          children: <Widget>[
            Icon(icon, color: tv.secondaryText),
            const SizedBox(width: 12),
            Expanded(
              child: Text(message, style: TextStyle(color: tv.secondaryText)),
            ),
          ],
        ),
      ),
    );
  }
}

class _StatusPill extends StatelessWidget {
  const _StatusPill({required this.label, required this.color});

  final String label;
  final Color color;

  @override
  Widget build(BuildContext context) => DecoratedBox(
    decoration: BoxDecoration(
      color: color.withValues(alpha: 0.14),
      borderRadius: BorderRadius.circular(20),
    ),
    child: Padding(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
      child: Text(
        label,
        style: TextStyle(color: color, fontWeight: FontWeight.w700),
      ),
    ),
  );
}

class _ProviderStatus {
  const _ProviderStatus(this.label, this.color, this.icon);

  factory _ProviderStatus.from(String value, TvPalette tv) => switch (value) {
    'ready' => _ProviderStatus(
      'Ready',
      tv.success,
      Icons.check_circle_outline_rounded,
    ),
    'refreshing' => _ProviderStatus('Refreshing', tv.info, Icons.sync_rounded),
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

class _ServiceStatus {
  const _ServiceStatus(this.label, this.color, this.icon);

  factory _ServiceStatus.from(String value, TvPalette tv) => switch (value) {
    'active' => _ServiceStatus(
      'Active',
      tv.success,
      Icons.check_circle_outline_rounded,
    ),
    'activating' => _ServiceStatus(
      'Starting',
      tv.info,
      Icons.hourglass_top_rounded,
    ),
    'inactive' => _ServiceStatus(
      'On demand',
      tv.info,
      Icons.pause_circle_outline_rounded,
    ),
    'failed' || 'unavailable' => _ServiceStatus(
      'Attention',
      tv.warning,
      Icons.error_outline_rounded,
    ),
    _ => _ServiceStatus(
      'Unknown',
      tv.secondaryText,
      Icons.help_outline_rounded,
    ),
  };

  final String label;
  final Color color;
  final IconData icon;
}

class _RommStatus {
  const _RommStatus(this.label, this.color, this.icon);

  factory _RommStatus.from(String value, TvPalette tv) => switch (value) {
    'ready' => _RommStatus('Connected', tv.success, Icons.dns_rounded),
    'not_configured' => _RommStatus(
      'Not set up',
      tv.info,
      Icons.link_off_rounded,
    ),
    _ => _RommStatus('Attention', tv.warning, Icons.error_outline_rounded),
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
                'Could not read service diagnostics: $error',
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
  Widget build(BuildContext context) => const _DiagnosticsEmptyState(
    icon: Icons.info_outline_rounded,
    message: 'No live library source data is available.',
  );
}

class _SystemHealthBackdrop extends StatelessWidget {
  const _SystemHealthBackdrop();

  @override
  Widget build(BuildContext context) =>
      const TvBackdrop(center: Alignment(-0.35, -0.55));
}

String _labelFor(String id) => id
    .split('-')
    .map(
      (String word) =>
          word.isEmpty ? word : '${word[0].toUpperCase()}${word.substring(1)}',
    )
    .join(' ');

String _serviceName(String id) => switch (id) {
  'session' => 'Hearthdeck session',
  'daemon' => 'Hearthdeck daemon',
  'bridge_socket' => 'Bridge socket',
  'bridge' => 'Host bridge',
  _ => _labelFor(id),
};

String _providerTiming(HearthdeckProviderHealth provider) {
  if (provider.status == 'refreshing') {
    return 'Refresh in progress';
  }
  if (provider.lastSuccessAt != null) {
    return 'Last success ${_formatTimestamp(provider.lastSuccessAt!)}';
  }
  if (provider.lastAttemptAt != null) {
    return 'Last attempt ${_formatTimestamp(provider.lastAttemptAt!)}';
  }
  return 'Awaiting first refresh';
}

String _formatTimestamp(String value) {
  final timestamp = DateTime.tryParse(value)?.toLocal();
  if (timestamp == null) {
    return 'unavailable';
  }
  return '${timestamp.hour.toString().padLeft(2, '0')}:${timestamp.minute.toString().padLeft(2, '0')}';
}
