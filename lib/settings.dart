import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import 'catalog/catalog_repository.dart';
import 'catalog/catalog_repository_factory.dart';
import 'settings_models.dart';
import 'system_health.dart';
import 'theme_settings.dart';
import 'tv_components.dart';
import 'tv_theme.dart';
import 'tv_two_pane.dart';

class SettingsPage extends StatefulWidget {
  const SettingsPage({super.key, this.catalogRepository});

  final CatalogRepository? catalogRepository;

  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  var _category = SettingsCategory.general;
  late final CatalogRepository _catalogRepository =
      widget.catalogRepository ?? createCatalogRepository();

  List<SettingsOption> get _options => settingsOptions[_category]!;

  void _selectCategory(SettingsCategory category) {
    setState(() => _category = category);
  }

  Future<void> _requestLibraryRescan() async {
    try {
      await _catalogRepository.requestRescan();
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('Library rescan requested.')),
        );
      }
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Could not request rescan: $error')),
        );
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (BuildContext context, BoxConstraints constraints) {
        final layout = _SettingsLayout.fromConstraints(constraints);
        final definition = settingsCategories.firstWhere(
          (SettingsCategoryDefinition definition) =>
              definition.category == _category,
        );
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
                      const Positioned.fill(child: _SettingsBackdrop()),
                      TvTwoPaneLayout(
                        rail: TvNavigationRail(
                          width: layout.railWidth,
                          compact: layout.isRailCompact,
                          headerBuilder: (BuildContext context, bool compact) =>
                              TvProfileRailHeader(
                                name: 'Alex',
                                compact: compact,
                                icon: Icons.settings_outlined,
                              ),
                          items: settingsCategories
                              .map(
                                (SettingsCategoryDefinition item) =>
                                    TvNavigationRailItem(
                                      id: item.category.name,
                                      label: item.label,
                                      icon: item.icon,
                                      isSelected: item.category == _category,
                                      onActivate: () =>
                                          _selectCategory(item.category),
                                    ),
                              )
                              .toList(growable: false),
                        ),
                        content: _SettingsContent(
                          definition: definition,
                          options: _options,
                          layout: layout,
                          onLibraryRescan: _requestLibraryRescan,
                          onThemeSettings: () => Navigator.of(context).push(
                            MaterialPageRoute<void>(
                              settings: const RouteSettings(
                                name: '/theme-settings',
                              ),
                              builder: (BuildContext context) =>
                                  const ThemeSettingsPage(),
                            ),
                          ),
                          onServiceStatus: () => Navigator.of(context).push(
                            MaterialPageRoute<void>(
                              settings: const RouteSettings(
                                name: '/system-health',
                              ),
                              builder: (BuildContext context) =>
                                  SystemHealthPage(
                                    catalogRepository: _catalogRepository,
                                  ),
                            ),
                          ),
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

class _SettingsLayout {
  const _SettingsLayout._({
    required this.railWidth,
    required this.pagePadding,
    required this.gap,
    required this.sectionGap,
    required this.cardExtent,
    required this.isRailCompact,
  });

  factory _SettingsLayout.fromConstraints(BoxConstraints constraints) {
    final width = constraints.maxWidth;
    final height = constraints.maxHeight;
    final scale = (math.min(width, height) / 720).clamp(0.72, 1.3).toDouble();
    final compact = width < 980 * scale;
    return _SettingsLayout._(
      railWidth: compact ? 72 * scale : 254 * scale,
      pagePadding: (width * 0.04).clamp(24 * scale, 76 * scale).toDouble(),
      gap: 16 * scale,
      sectionGap: 32 * scale,
      cardExtent: (width * 0.33).clamp(300 * scale, 560 * scale).toDouble(),
      isRailCompact: compact,
    );
  }

  final double railWidth;
  final double pagePadding;
  final double gap;
  final double sectionGap;
  final double cardExtent;
  final bool isRailCompact;
}

class _SettingsContent extends StatelessWidget {
  const _SettingsContent({
    required this.definition,
    required this.options,
    required this.layout,
    required this.onLibraryRescan,
    required this.onThemeSettings,
    required this.onServiceStatus,
  });

  final SettingsCategoryDefinition definition;
  final List<SettingsOption> options;
  final _SettingsLayout layout;
  final VoidCallback onLibraryRescan;
  final VoidCallback onThemeSettings;
  final VoidCallback onServiceStatus;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
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
                    Icon(definition.icon, size: 34, color: tv.focus),
                    const SizedBox(width: 14),
                    Expanded(
                      child: Text(
                        definition.label,
                        style: Theme.of(context).textTheme.displaySmall,
                      ),
                    ),
                  ],
                ),
              ),
              SliverToBoxAdapter(child: SizedBox(height: layout.sectionGap)),
              SliverGrid.builder(
                itemCount: options.length,
                gridDelegate: SliverGridDelegateWithMaxCrossAxisExtent(
                  maxCrossAxisExtent: layout.cardExtent,
                  mainAxisSpacing: layout.gap,
                  crossAxisSpacing: layout.gap,
                  childAspectRatio: 1.5,
                ),
                itemBuilder: (BuildContext context, int index) {
                  final option = options[index];
                  return TvOptionCard(
                    key: ValueKey<String>('settings-option-${option.id}'),
                    label: option.label,
                    description: option.description,
                    icon: option.icon,
                    autofocus: index == 0,
                    onActivate: switch (option.id) {
                      'rescan-library' => onLibraryRescan,
                      'service-status' => onServiceStatus,
                      'personalization' => onThemeSettings,
                      _ => () => _showSettingsMessage(context, option),
                    },
                  );
                },
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _SettingsBackdrop extends StatelessWidget {
  const _SettingsBackdrop();

  @override
  Widget build(BuildContext context) {
    return const TvBackdrop(center: Alignment(0.78, -0.55));
  }
}

void _showSettingsMessage(BuildContext context, SettingsOption option) {
  ScaffoldMessenger.of(context).hideCurrentSnackBar();
  ScaffoldMessenger.of(context).showSnackBar(
    SnackBar(content: Text('${option.label} is ready for configuration.')),
  );
}
