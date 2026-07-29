import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';

import 'content_details.dart';
import 'dashboard_models.dart';
import 'tv_components.dart';
import 'tv_theme.dart';
import 'full_library.dart';
import 'search.dart';
import 'settings.dart';

class TvDashboard extends StatelessWidget {
  const TvDashboard({super.key});

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (BuildContext context, BoxConstraints constraints) {
        final metrics = TvLayoutMetrics.fromConstraints(constraints);
        return TvDirectionalFocusNavigation(
          child: Actions(
            actions: <Type, Action<Intent>>{
              DismissIntent: CallbackAction<DismissIntent>(
                onInvoke: (DismissIntent intent) {
                  ScaffoldMessenger.of(context).hideCurrentSnackBar();
                  return null;
                },
              ),
            },
            child: Scaffold(
              body: SafeArea(
                child: Stack(
                  children: <Widget>[
                    const Positioned.fill(child: _DashboardBackdrop()),
                    Column(
                      children: <Widget>[
                        Padding(
                          padding: metrics.pageInsets,
                          child: TvTopBar(
                            metrics: metrics,
                            onLibraryActivate: () => Navigator.of(context).push(
                              MaterialPageRoute<void>(
                                settings: const RouteSettings(name: '/library'),
                                builder: (BuildContext context) =>
                                    const FullLibraryPage(),
                              ),
                            ),
                            onSearchActivate: () => Navigator.of(context).push(
                              MaterialPageRoute<void>(
                                settings: const RouteSettings(name: '/search'),
                                builder: (BuildContext context) =>
                                    const TvSearchPage(),
                              ),
                            ),
                            onSettingsActivate: () =>
                                Navigator.of(context).push(
                                  MaterialPageRoute<void>(
                                    settings: const RouteSettings(
                                      name: '/settings',
                                    ),
                                    builder: (BuildContext context) =>
                                        const SettingsPage(),
                                  ),
                                ),
                          ),
                        ),
                        Expanded(
                          child: CustomScrollView(
                            scrollCacheExtent: ScrollCacheExtent.pixels(
                              metrics.squareExtent * 3,
                            ),
                            slivers: <Widget>[
                              SliverToBoxAdapter(
                                child: SizedBox(height: metrics.heroHeight),
                              ),
                              SliverList.builder(
                                itemCount: dashboardSections.length,
                                itemBuilder: (BuildContext context, int index) {
                                  final section = dashboardSections[index];
                                  return Padding(
                                    padding: EdgeInsets.only(
                                      bottom:
                                          index == dashboardSections.length - 1
                                          ? metrics.sectionGap
                                          : metrics.sectionGap,
                                    ),
                                    child: TvShelf(
                                      section: section,
                                      metrics: metrics,
                                      onActivate:
                                          (
                                            DashboardItem item,
                                            TvTileShape sourceShape,
                                          ) => Navigator.of(context).push(
                                            MaterialPageRoute<void>(
                                              settings: RouteSettings(
                                                name: '/details/${item.id}',
                                              ),
                                              builder: (BuildContext context) =>
                                                  ContentDetailsPage(
                                                    item: item,
                                                    sourceShape: sourceShape,
                                                  ),
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
        );
      },
    );
  }
}

class _DashboardBackdrop extends StatelessWidget {
  const _DashboardBackdrop();

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        gradient: RadialGradient(
          center: const Alignment(0.12, -0.22),
          radius: 1.15,
          colors: <Color>[tv.backdropGlow, tv.canvas],
          stops: const <double>[0, 0.72],
        ),
      ),
      child: const ColoredBox(color: Color(0x12000000)),
    );
  }
}
