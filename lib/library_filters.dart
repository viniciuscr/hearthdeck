import 'package:flutter/material.dart';

import 'dashboard_models.dart';
import 'tv_components.dart';
import 'tv_side_sheet.dart';
import 'tv_theme.dart';

enum LibraryFilter {
  installed,
  cloudReady,
  action,
  strategy,
  multiplayer,
  singlePlayer,
}

class LibraryFilterDefinition {
  const LibraryFilterDefinition({
    required this.filter,
    required this.label,
    required this.icon,
  });

  final LibraryFilter filter;
  final String label;
  final IconData icon;
}

const List<LibraryFilterDefinition> libraryFilterDefinitions =
    <LibraryFilterDefinition>[
      LibraryFilterDefinition(
        filter: LibraryFilter.installed,
        label: 'Installed',
        icon: Icons.download_done_outlined,
      ),
      LibraryFilterDefinition(
        filter: LibraryFilter.cloudReady,
        label: 'Cloud ready',
        icon: Icons.cloud_queue_rounded,
      ),
      LibraryFilterDefinition(
        filter: LibraryFilter.action,
        label: 'Action',
        icon: Icons.bolt_rounded,
      ),
      LibraryFilterDefinition(
        filter: LibraryFilter.strategy,
        label: 'Strategy',
        icon: Icons.account_tree_outlined,
      ),
      LibraryFilterDefinition(
        filter: LibraryFilter.multiplayer,
        label: 'Multiplayer',
        icon: Icons.groups_outlined,
      ),
      LibraryFilterDefinition(
        filter: LibraryFilter.singlePlayer,
        label: 'Single player',
        icon: Icons.person_outline_rounded,
      ),
    ];

class LibraryFilterState {
  const LibraryFilterState({this.selected = const <LibraryFilter>{}});

  final Set<LibraryFilter> selected;

  bool get isEmpty => selected.isEmpty;

  LibraryFilterState toggle(LibraryFilter filter) {
    final next = Set<LibraryFilter>.of(selected);
    if (!next.add(filter)) {
      next.remove(filter);
    }
    return LibraryFilterState(selected: next);
  }

  LibraryFilterState clear() => const LibraryFilterState();

  bool matches(DashboardItem item) {
    if (selected.isEmpty) {
      return true;
    }
    final tags = _tagsFor(item);
    return selected.every(tags.contains);
  }
}

Set<LibraryFilter> _tagsFor(DashboardItem item) {
  final tags = <LibraryFilter>{LibraryFilter.installed};
  if (item.id.contains('orbit') || item.id.contains('cloud')) {
    tags.add(LibraryFilter.cloudReady);
  }
  if (item.kind == TvContentKind.game) {
    tags.add(LibraryFilter.singlePlayer);
  }
  if (item.id.contains('ember') ||
      item.id.contains('violet') ||
      item.id.contains('forge')) {
    tags.add(LibraryFilter.action);
  }
  if (item.id.contains('citadel')) {
    tags.add(LibraryFilter.strategy);
  }
  if (item.id.contains('arcade') || item.id.contains('drift')) {
    tags.add(LibraryFilter.multiplayer);
  }
  return tags;
}

class LibraryFilterSheet extends StatefulWidget {
  const LibraryFilterSheet({
    required this.initialState,
    required this.onApply,
    super.key,
  });

  final LibraryFilterState initialState;
  final ValueChanged<LibraryFilterState> onApply;

  @override
  State<LibraryFilterSheet> createState() => _LibraryFilterSheetState();
}

class _LibraryFilterSheetState extends State<LibraryFilterSheet> {
  late LibraryFilterState _state = widget.initialState;

  @override
  Widget build(BuildContext context) {
    return TvSideSheet(
      title: 'Filters',
      child: Padding(
        padding: const EdgeInsets.all(18),
        child: Column(
          children: <Widget>[
            TvSideSheetAction(
              label: 'Clear all filters',
              icon: Icons.clear_all_rounded,
              onActivate: () => setState(() => _state = _state.clear()),
            ),
            const SizedBox(height: 20),
            Expanded(
              child: ListView.separated(
                itemCount: libraryFilterDefinitions.length,
                separatorBuilder: (BuildContext context, int index) =>
                    const SizedBox(height: 9),
                itemBuilder: (BuildContext context, int index) {
                  final definition = libraryFilterDefinitions[index];
                  return _LibraryFilterOption(
                    definition: definition,
                    isSelected: _state.selected.contains(definition.filter),
                    autofocus: index == 0,
                    onActivate: () => setState(
                      () => _state = _state.toggle(definition.filter),
                    ),
                  );
                },
              ),
            ),
            const SizedBox(height: 18),
            TvSideSheetAction(
              label: _state.isEmpty ? 'Save and close' : 'Show filtered items',
              icon: Icons.check_rounded,
              isPrimary: true,
              onActivate: () {
                widget.onApply(_state);
                Navigator.of(context).pop();
              },
            ),
          ],
        ),
      ),
    );
  }
}

class _LibraryFilterOption extends StatelessWidget {
  const _LibraryFilterOption({
    required this.definition,
    required this.isSelected,
    required this.autofocus,
    required this.onActivate,
  });

  final LibraryFilterDefinition definition;
  final bool isSelected;
  final bool autofocus;
  final VoidCallback onActivate;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return TvFocusable(
      semanticLabel: definition.label,
      autofocus: autofocus,
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
          constraints: const BoxConstraints(minHeight: 50),
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
          decoration: BoxDecoration(
            color: style.background,
            borderRadius: BorderRadius.circular(8),
            border: Border.all(color: style.border, width: 2),
          ),
          child: Row(
            children: <Widget>[
              Icon(definition.icon, color: style.foreground),
              const SizedBox(width: 12),
              Expanded(
                child: Text(
                  definition.label,
                  style: TextStyle(
                    color: style.foreground,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ),
              Icon(
                isSelected
                    ? Icons.check_circle_rounded
                    : Icons.add_circle_outline_rounded,
                color: isFocused
                    ? style.foreground
                    : (isSelected ? tv.accent : tv.secondaryText),
              ),
            ],
          ),
        );
      },
    );
  }
}
