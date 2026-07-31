import 'package:flutter/material.dart';

import 'backend/hearthdeck_api_client.dart';
import 'catalog/catalog_repository.dart';
import 'catalog/catalog_repository_factory.dart';
import 'tv_components.dart';
import 'tv_theme.dart';
import 'virtual_keyboard.dart';

class LibraryClassificationPage extends StatefulWidget {
  const LibraryClassificationPage({super.key, this.catalogRepository});

  final CatalogRepository? catalogRepository;

  @override
  State<LibraryClassificationPage> createState() =>
      _LibraryClassificationPageState();
}

class _LibraryClassificationPageState extends State<LibraryClassificationPage> {
  late final CatalogRepository _catalogRepository =
      widget.catalogRepository ?? createCatalogRepository();
  List<HearthdeckLibraryItem>? _items;
  Object? _error;
  String? _savingItemId;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    try {
      final items = await _catalogRepository.libraryItems();
      if (mounted) {
        setState(() {
          _items = List<HearthdeckLibraryItem>.of(items)
            ..sort((left, right) => left.title.compareTo(right.title));
          _error = null;
        });
      }
    } catch (error) {
      if (mounted) {
        setState(() => _error = error);
      }
    }
  }

  Future<void> _chooseClassification(HearthdeckLibraryItem item) async {
    final choice = await showDialog<String?>(
      context: context,
      builder: (BuildContext context) => _ClassificationDialog(item: item),
    );
    if (!mounted || choice == null || choice == _dismissed) {
      return;
    }
    setState(() => _savingItemId = item.id);
    try {
      await _catalogRepository.updateLibraryClassification(
        itemId: item.id,
        kind: choice == _automatic ? null : choice,
      );
      await _load();
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Could not classify ${item.title}: $error')),
        );
      }
    } finally {
      if (mounted) {
        setState(() => _savingItemId = null);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final items = _items;
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
        child: Scaffold(
          body: SafeArea(
            child: Stack(
              children: <Widget>[
                const Positioned.fill(
                  child: TvBackdrop(center: Alignment(0.45, -0.45)),
                ),
                CustomScrollView(
                  slivers: <Widget>[
                    SliverPadding(
                      padding: const EdgeInsets.fromLTRB(36, 26, 36, 44),
                      sliver: SliverMainAxisGroup(
                        slivers: <Widget>[
                          SliverToBoxAdapter(
                            child: Row(
                              children: <Widget>[
                                TvFocusable(
                                  semanticLabel: 'Back to Settings',
                                  autofocus: true,
                                  onActivate: () => Navigator.of(context).pop(),
                                  builder:
                                      (BuildContext context, bool isFocused) {
                                        final style = TvControlStyle.resolve(
                                          tv,
                                          variant: TvControlVariant.icon,
                                          isFocused: isFocused,
                                        );
                                        return AnimatedContainer(
                                          duration: TvTheme.focusDuration,
                                          width: 46,
                                          height: 46,
                                          decoration: BoxDecoration(
                                            color: style.background,
                                            borderRadius: BorderRadius.circular(
                                              10,
                                            ),
                                            border: Border.all(
                                              color: style.border,
                                              width: 2,
                                            ),
                                          ),
                                          child: Icon(
                                            Icons.arrow_back_rounded,
                                            color: style.foreground,
                                          ),
                                        );
                                      },
                                ),
                                const SizedBox(width: 18),
                                Expanded(
                                  child: Column(
                                    crossAxisAlignment:
                                        CrossAxisAlignment.start,
                                    children: <Widget>[
                                      Text(
                                        'Library classification',
                                        style: Theme.of(
                                          context,
                                        ).textTheme.displaySmall,
                                      ),
                                      const SizedBox(height: 5),
                                      Text(
                                        'Correct game and app grouping. Your choices survive library rescans.',
                                        style: TextStyle(
                                          color: tv.secondaryText,
                                        ),
                                      ),
                                    ],
                                  ),
                                ),
                                _RefreshClassificationButton(onActivate: _load),
                              ],
                            ),
                          ),
                          const SliverToBoxAdapter(child: SizedBox(height: 28)),
                          if (items == null && _error == null)
                            const SliverFillRemaining(
                              hasScrollBody: false,
                              child: Center(child: CircularProgressIndicator()),
                            )
                          else if (_error case final Object error)
                            SliverFillRemaining(
                              hasScrollBody: false,
                              child: Center(
                                child: Text(
                                  'Could not load the library: $error',
                                ),
                              ),
                            )
                          else if (items!.isEmpty)
                            SliverFillRemaining(
                              hasScrollBody: false,
                              child: Center(
                                child: Text(
                                  'No discovered entries yet. Rescan the library, then return here.',
                                  style: TextStyle(color: tv.secondaryText),
                                ),
                              ),
                            )
                          else
                            SliverList.separated(
                              itemCount: items.length,
                              separatorBuilder:
                                  (BuildContext context, int index) =>
                                      const SizedBox(height: 10),
                              itemBuilder: (BuildContext context, int index) {
                                final item = items[index];
                                return _ClassificationRow(
                                  item: item,
                                  isSaving: _savingItemId == item.id,
                                  onActivate: () => _chooseClassification(item),
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
    );
  }
}

const String _automatic = 'automatic';
const String _dismissed = 'dismissed';

class _ClassificationRow extends StatelessWidget {
  const _ClassificationRow({
    required this.item,
    required this.isSaving,
    required this.onActivate,
  });

  final HearthdeckLibraryItem item;
  final bool isSaving;
  final VoidCallback onActivate;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final classification = _classificationFor(item);
    return TvFocusable(
      semanticLabel: 'Classify ${item.title}',
      onActivate: isSaving ? null : onActivate,
      builder: (BuildContext context, bool isFocused) {
        final style = TvControlStyle.resolve(
          tv,
          variant: TvControlVariant.secondary,
          isFocused: isFocused,
        );
        return AnimatedContainer(
          duration: TvTheme.focusDuration,
          curve: TvTheme.focusCurve,
          padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 16),
          decoration: BoxDecoration(
            color: isFocused ? style.background : tv.surface,
            borderRadius: BorderRadius.circular(12),
            border: Border.all(color: style.border, width: 2),
          ),
          child: Row(
            children: <Widget>[
              Icon(
                item.kind == 'game'
                    ? Icons.sports_esports_rounded
                    : Icons.apps_rounded,
                color: isFocused ? style.foreground : tv.accent,
              ),
              const SizedBox(width: 16),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: <Widget>[
                    Text(
                      item.title,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: isFocused ? style.foreground : tv.primaryText,
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                    const SizedBox(height: 4),
                    Text(
                      '${_sourceLabel(item.sourceId)} · ${classification.detectedLabel}',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: isFocused
                            ? style.foreground.withValues(alpha: 0.76)
                            : tv.secondaryText,
                      ),
                    ),
                  ],
                ),
              ),
              if (isSaving)
                const SizedBox(
                  width: 22,
                  height: 22,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              else
                _KindPill(
                  label: classification.effectiveLabel,
                  isOverridden: classification.isOverridden,
                ),
              const SizedBox(width: 12),
              Icon(Icons.tune_rounded, color: style.foreground),
            ],
          ),
        );
      },
    );
  }
}

class _KindPill extends StatelessWidget {
  const _KindPill({required this.label, required this.isOverridden});

  final String label;
  final bool isOverridden;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final color = isOverridden ? tv.accent : tv.secondaryText;
    return DecoratedBox(
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.15),
        borderRadius: BorderRadius.circular(20),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 11, vertical: 6),
        child: Text(
          label,
          style: TextStyle(color: color, fontWeight: FontWeight.w700),
        ),
      ),
    );
  }
}

class _ClassificationDialog extends StatelessWidget {
  const _ClassificationDialog({required this.item});

  final HearthdeckLibraryItem item;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final classification = _classificationFor(item);
    return AlertDialog(
      backgroundColor: tv.surface,
      title: Text('Classify ${item.title}'),
      content: Text(
        'Detected as ${classification.detectedLabel.toLowerCase()}. Choose how it appears in Hearthdeck.',
        style: TextStyle(color: tv.secondaryText),
      ),
      actions: <Widget>[
        TextButton(
          onPressed: () => Navigator.of(context).pop(_dismissed),
          child: const Text('Cancel'),
        ),
        TextButton(
          onPressed: () => Navigator.of(context).pop(_automatic),
          child: const Text('Use detected'),
        ),
        TextButton(
          onPressed: () => Navigator.of(context).pop('application'),
          child: const Text('App'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop('game'),
          child: const Text('Game'),
        ),
      ],
    );
  }
}

class _RefreshClassificationButton extends StatelessWidget {
  const _RefreshClassificationButton({required this.onActivate});

  final Future<void> Function() onActivate;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return TvFocusable(
      semanticLabel: 'Refresh library classification',
      onActivate: () => onActivate(),
      builder: (BuildContext context, bool isFocused) {
        final style = TvControlStyle.resolve(
          tv,
          variant: TvControlVariant.icon,
          isFocused: isFocused,
        );
        return AnimatedContainer(
          duration: TvTheme.focusDuration,
          width: 46,
          height: 46,
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

class _Classification {
  const _Classification({
    required this.effectiveKind,
    required this.detectedKind,
    required this.isOverridden,
  });

  final String effectiveKind;
  final String detectedKind;
  final bool isOverridden;

  String get effectiveLabel => _kindLabel(effectiveKind);
  String get detectedLabel => 'Detected ${_kindLabel(detectedKind)}';
}

_Classification _classificationFor(HearthdeckLibraryItem item) {
  final raw = item.metadata['classification'];
  if (raw is Map<String, dynamic>) {
    final detected = raw['discovered_kind'] as String? ?? item.kind;
    return _Classification(
      effectiveKind: item.kind,
      detectedKind: detected,
      isOverridden: raw['overridden'] as bool? ?? false,
    );
  }
  return _Classification(
    effectiveKind: item.kind,
    detectedKind: item.kind,
    isOverridden: false,
  );
}

String _kindLabel(String kind) => switch (kind) {
  'game' => 'Game',
  _ => 'App',
};

String _sourceLabel(String sourceId) => sourceId
    .split('-')
    .map(
      (String word) =>
          word.isEmpty ? word : '${word[0].toUpperCase()}${word.substring(1)}',
    )
    .join(' ');
