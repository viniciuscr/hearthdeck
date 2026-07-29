import 'package:flutter/material.dart';
import 'package:flutter_gamepads/flutter_gamepads.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gamepads/gamepads.dart';
import 'package:gamepads_platform_interface/gamepads_platform_interface.dart';
import 'package:gamepads_platform_interface/method_channel_gamepads_platform_interface.dart';
import 'package:hearthdeck/backend/hearthdeck_api_client.dart';
import 'package:hearthdeck/main.dart';
import 'package:hearthdeck/tv_components.dart';
import 'package:hearthdeck/content_details.dart';
import 'package:hearthdeck/dashboard_models.dart';
import 'package:hearthdeck/catalog/mock_catalog_repository.dart';
import 'package:hearthdeck/catalog/catalog_repository.dart';
import 'package:hearthdeck/full_library.dart';
import 'package:hearthdeck/search.dart';
import 'package:hearthdeck/settings.dart';
import 'package:hearthdeck/system_health.dart';
import 'package:hearthdeck/tv_gamepad.dart';

void main() {
  testWidgets('dashboard renders reusable shelves and pinned applications', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());

    expect(find.text('Orbit'), findsOneWidget);
    expect(find.text('Discover something new'), findsOneWidget);
    expect(find.byType(TvShelf), findsNWidgets(3));
  });

  testWidgets('full library opens from the dashboard action', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());

    await tester.tap(find.bySemanticsLabel('Full library'));
    await tester.pumpAndSettle();

    expect(find.byType(FullLibraryPage), findsOneWidget);
    expect(find.text('Games library'), findsOneWidget);
    expect(find.text('All games'), findsOneWidget);
  });

  testWidgets('library opens details before dispatching a catalog launch', (
    WidgetTester tester,
  ) async {
    addTearDown(tester.view.reset);
    tester.view
      ..devicePixelRatio = 1
      ..physicalSize = const Size(1280, 900);
    await tester.pumpWidget(
      MaterialApp(
        home: FullLibraryPage(catalogRepository: const MockCatalogRepository()),
      ),
    );
    await tester.pump();

    final tile = find.byKey(
      const ValueKey<String>('library-tile-orbit-library'),
    );
    await tester.tap(tile);
    await tester.pumpAndSettle();

    expect(find.byType(ContentDetailsPage), findsOneWidget);
    expect(find.text('Play'), findsOneWidget);
  });

  testWidgets('library keeps loaded catalog when event stream fails', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        home: FullLibraryPage(
          catalogRepository: _EventFailingCatalogRepository(),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('All games'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('settings opens from the dashboard action', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());

    await tester.tap(find.bySemanticsLabel('Settings'));
    await tester.pumpAndSettle();

    expect(find.byType(SettingsPage), findsOneWidget);
    expect(find.text('General'), findsWidgets);
    expect(
      find.byKey(const ValueKey<String>('settings-option-network')),
      findsOneWidget,
    );
  });

  testWidgets('settings categories update the option grid', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.bySemanticsLabel('Settings'));
    await tester.pumpAndSettle();

    await tester.tap(find.bySemanticsLabel('Devices & connections'));
    await tester.pumpAndSettle();

    expect(find.text('Controllers'), findsOneWidget);
    expect(
      find.byKey(const ValueKey<String>('settings-option-controllers')),
      findsOneWidget,
    );
  });

  testWidgets('settings exposes a library rescan control', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        home: SettingsPage(catalogRepository: const MockCatalogRepository()),
      ),
    );

    await tester.tap(find.bySemanticsLabel('System'));
    await tester.pumpAndSettle();

    expect(
      find.byKey(const ValueKey<String>('settings-option-rescan-library')),
      findsOneWidget,
    );
  });

  testWidgets('settings opens live provider health', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        home: SettingsPage(catalogRepository: const _HealthCatalogRepository()),
      ),
    );

    await tester.tap(find.bySemanticsLabel('System'));
    await tester.pumpAndSettle();
    final serviceStatus = find.byKey(
      const ValueKey<String>('settings-option-service-status'),
    );
    await tester.scrollUntilVisible(
      serviceStatus,
      240,
      scrollable: find.byType(Scrollable).last,
    );
    await tester.tap(serviceStatus);
    await tester.pumpAndSettle();

    expect(find.byType(SystemHealthPage), findsOneWidget);
    expect(find.text('Desktop Apps'), findsOneWidget);
    expect(find.text('Attention'), findsOneWidget);
    expect(find.text('bridge unavailable'), findsOneWidget);
  });

  testWidgets('dashboard search opens a focused native text field', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());

    await tester.tap(find.bySemanticsLabel('Search'));
    await tester.pumpAndSettle();

    expect(find.byType(TvSearchPage), findsOneWidget);
    expect(find.byKey(const ValueKey<String>('search-input')), findsOneWidget);
    expect(FocusManager.instance.primaryFocus?.debugLabel, 'Search input');
  });

  testWidgets('native text input updates search results', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.bySemanticsLabel('Search'));
    await tester.pumpAndSettle();

    await tester.enterText(
      find.byKey(const ValueKey<String>('search-input')),
      'orbit',
    );
    await tester.pump();

    expect(
      find.byKey(const ValueKey<String>('search-tile-orbit-library')),
      findsOneWidget,
    );
    expect(
      find.byKey(const ValueKey<String>('search-tile-ember')),
      findsNothing,
    );
  });

  testWidgets('library search uses the same search route', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.bySemanticsLabel('Full library'));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Search library'));
    await tester.pumpAndSettle();

    expect(find.byType(TvSearchPage), findsOneWidget);
  });

  testWidgets('library source selection updates its item grid', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.bySemanticsLabel('Full library'));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Steam'));
    await tester.pumpAndSettle();

    expect(
      find.byKey(const ValueKey<String>('library-tile-ember-steam')),
      findsOneWidget,
    );
  });

  testWidgets('library supports directional focus into its content grid', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.bySemanticsLabel('Full library'));
    await tester.pumpAndSettle();

    expect(FocusManager.instance.primaryFocus?.debugLabel, 'Games');

    await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
    await tester.pumpAndSettle();

    expect(FocusManager.instance.primaryFocus?.debugLabel, isNot('Games'));
  });

  testWidgets('library filters slide in and apply to the content grid', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.bySemanticsLabel('Full library'));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Filter'));
    await tester.pumpAndSettle();

    expect(find.text('Filters'), findsOneWidget);
    expect(FocusManager.instance.primaryFocus?.debugLabel, 'Installed');

    await tester.tap(find.text('Strategy'));
    await tester.pump();
    await tester.tap(find.text('Show filtered items'));
    await tester.pumpAndSettle();

    expect(find.text('Filter (1)'), findsOneWidget);
    expect(
      find.byKey(const ValueKey<String>('library-tile-citadel')),
      findsOneWidget,
    );
    expect(
      find.byKey(const ValueKey<String>('library-tile-ember')),
      findsNothing,
    );
  });

  testWidgets(
    'Escape closes the filter side sheet before leaving the library',
    (WidgetTester tester) async {
      await tester.pumpWidget(const HearthdeckApp());
      await tester.tap(find.bySemanticsLabel('Full library'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Filter'));
      await tester.pumpAndSettle();

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();

      expect(find.text('Filters'), findsNothing);
      expect(find.byType(FullLibraryPage), findsOneWidget);
    },
  );

  testWidgets('activating content opens its reusable detail route', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());

    await tester.tap(find.text('Orbit'));
    await tester.pumpAndSettle();

    expect(find.byType(ContentDetailsPage), findsOneWidget);
    expect(find.text('Your game stats'), findsOneWidget);
    expect(find.text('Recent captures'), findsOneWidget);
    expect(find.text('Play'), findsOneWidget);
  });

  testWidgets('the details route can be dismissed with Escape', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.text('Orbit'));
    await tester.pumpAndSettle();

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();

    expect(find.byType(ContentDetailsPage), findsNothing);
    expect(find.text('Discover something new'), findsOneWidget);
  });

  testWidgets('the controller B button dismisses the details route', (
    WidgetTester tester,
  ) async {
    Gamepads.normalizer = GamepadNormalizer.forPlatform(GamepadPlatform.linux);
    addTearDown(() => Gamepads.normalizer = null);
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.text('Orbit'));
    await tester.pumpAndSettle();

    await _sendLinuxGamepadButton('1', 1.0);
    await _sendLinuxGamepadButton('1', 0.0);
    await tester.pumpAndSettle();

    expect(find.byType(ContentDetailsPage), findsNothing);
    expect(find.text('Discover something new'), findsOneWidget);
  });

  testWidgets('details route supports directional focus navigation', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.text('Orbit'));
    await tester.pumpAndSettle();

    expect(FocusManager.instance.primaryFocus?.debugLabel, 'Play');

    await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
    await tester.pumpAndSettle();

    expect(FocusManager.instance.primaryFocus?.debugLabel, 'Manage game');
  });

  testWidgets('controller directional intent navigates detail actions', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.text('Orbit'));
    await tester.pumpAndSettle();

    Actions.invoke(
      tester.element(find.byType(ContentDetailsPage)),
      TvGamepadBindings.right,
    );
    await tester.pumpAndSettle();

    expect(FocusManager.instance.primaryFocus?.debugLabel, 'Manage game');
  });

  testWidgets('shared artwork transition excludes tile captions', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());

    final tile = find.byKey(const ValueKey<String>('tile-orbit'));
    final artworkHero = find.descendant(of: tile, matching: find.byType(Hero));
    expect(artworkHero, findsOneWidget);
    expect(
      find.descendant(of: artworkHero, matching: find.text('Orbit')),
      findsNothing,
    );
    expect(
      find.descendant(of: tile, matching: find.text('Orbit')),
      findsOneWidget,
    );

    await tester.tap(find.text('Orbit'));
    await tester.pumpAndSettle();

    expect(find.byType(Hero), findsOneWidget);
  });

  testWidgets('remote directional input moves focus across a shelf', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());

    expect(FocusManager.instance.primaryFocus?.debugLabel, 'Orbit');

    await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
    await tester.pumpAndSettle();

    expect(FocusManager.instance.primaryFocus?.debugLabel, 'Stream');
  });

  testWidgets('section titles align with their shelf content at any width', (
    WidgetTester tester,
  ) async {
    addTearDown(tester.view.reset);
    tester.view.devicePixelRatio = 1;

    for (final size in <Size>[const Size(720, 720), const Size(1920, 1080)]) {
      tester.view.physicalSize = size;
      await tester.pumpWidget(const HearthdeckApp());

      final title = find.byKey(const ValueKey<String>('shelf-title-discover'));
      final firstTile = find.byKey(const ValueKey<String>('tile-library'));

      expect(tester.getTopLeft(title).dx, tester.getTopLeft(firstTile).dx);
    }
  });

  test('controller bindings map standard controls to TV intents', () {
    expect(
      TvGamepadBindings.shortcuts[const GamepadActivatorButton.dpadRight()],
      TvGamepadBindings.right,
    );
    expect(
      TvGamepadBindings.shortcuts[const GamepadActivatorAxis.leftStickDown()],
      TvGamepadBindings.down,
    );
    expect(
      TvGamepadBindings.shortcuts[const GamepadActivatorButton.a()],
      isA<ActivateIntent>(),
    );
    expect(
      TvGamepadBindings.shortcuts[const GamepadActivatorButton.b()],
      TvGamepadBindings.back,
    );
    expect(
      TvGamepadBindings.shortcuts[const GamepadActivatorButton.back()],
      TvGamepadBindings.back,
    );
  });
}

Future<void> _sendLinuxGamepadButton(String key, double value) {
  final platform =
      GamepadsPlatformInterface.instance
          as MethodChannelGamepadsPlatformInterface;
  return platform.platformCallHandler(
    MethodCall('onGamepadEvent', <String, dynamic>{
      'gamepadId': 'test-controller',
      'time': DateTime.now().millisecondsSinceEpoch,
      'type': 'button',
      'key': key,
      'value': value,
    }),
  );
}

class _EventFailingCatalogRepository implements CatalogRepository {
  @override
  Future<HearthdeckHealth> health() => const MockCatalogRepository().health();

  @override
  Future<CatalogData> load() async {
    return const CatalogData(
      gameSources: <CatalogSource>[
        CatalogSource(id: 'test', label: 'All games', items: <DashboardItem>[]),
      ],
      appSources: <CatalogSource>[],
    );
  }

  @override
  Future<void> launch(DashboardItem item) async {}

  @override
  Future<void> requestRescan() async {}

  @override
  Stream<CatalogEvent> watch() =>
      Stream<CatalogEvent>.error(StateError('event feed unavailable'));
}

class _HealthCatalogRepository extends MockCatalogRepository {
  const _HealthCatalogRepository();

  @override
  Future<HearthdeckHealth> health() async => const HearthdeckHealth(
    version: '0.1.0',
    lanEnabled: false,
    transport: 'http',
    providers: <HearthdeckProviderHealth>[
      HearthdeckProviderHealth(
        id: 'desktop-apps',
        kind: 'discovery',
        status: 'degraded',
        recordCount: null,
        lastSuccessAt: null,
        lastError: 'bridge unavailable',
      ),
    ],
  );
}
