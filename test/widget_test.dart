import 'package:flutter/material.dart';
import 'package:flutter_gamepads/flutter_gamepads.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gamepads/gamepads.dart';
import 'package:gamepads_platform_interface/gamepads_platform_interface.dart';
import 'package:gamepads_platform_interface/method_channel_gamepads_platform_interface.dart';
import 'package:hearthdeck/backend/hearthdeck_api_client.dart';
import 'package:hearthdeck/catalog/catalog_repository.dart';
import 'package:hearthdeck/catalog/mock_catalog_repository.dart';
import 'package:hearthdeck/content_details.dart';
import 'package:hearthdeck/dashboard_models.dart';
import 'package:hearthdeck/external_link.dart';
import 'package:hearthdeck/full_library.dart';
import 'package:hearthdeck/library_classification.dart';
import 'package:hearthdeck/main.dart';
import 'package:hearthdeck/platform_session.dart';
import 'package:hearthdeck/romm_settings.dart';
import 'package:hearthdeck/search.dart';
import 'package:hearthdeck/settings.dart';
import 'package:hearthdeck/settings/user_settings_repository.dart';
import 'package:hearthdeck/system_health.dart';
import 'package:hearthdeck/tv_components.dart';
import 'package:hearthdeck/tv_gamepad.dart';
import 'package:hearthdeck/tv_theme.dart';
import 'package:hearthdeck/virtual_keyboard.dart';
import 'package:shared_preferences/shared_preferences.dart';

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
    expect(find.text('Games'), findsOneWidget);
    expect(find.text('PC games'), findsOneWidget);
  });

  testWidgets('Retro opens the Games console tab', (WidgetTester tester) async {
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.text('Retro'));
    await tester.pumpAndSettle();

    expect(find.byType(FullLibraryPage), findsOneWidget);
    expect(find.text('Consoles'), findsOneWidget);
    expect(
      find.byKey(const ValueKey<String>('library-tile-romm-console-nes')),
      findsOneWidget,
    );
  });

  testWidgets('settings opens the RomM connection form', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.bySemanticsLabel('Settings'));
    await tester.pumpAndSettle();
    await tester.tap(find.bySemanticsLabel('System'));
    await tester.pumpAndSettle();
    final rommSettings = find.byKey(
      const ValueKey<String>('settings-option-romm'),
    );
    await tester.scrollUntilVisible(
      rommSettings,
      240,
      scrollable: find.byType(Scrollable).last,
    );
    await tester.tap(rommSettings);
    await tester.pumpAndSettle();

    expect(find.byType(RommSettingsPage), findsOneWidget);
    expect(find.text('RomM server URL'), findsOneWidget);
  });

  testWidgets('settings opens library classification management', (
    WidgetTester tester,
  ) async {
    final repository = _ClassificationCatalogRepository();
    await tester.pumpWidget(
      MaterialApp(home: SettingsPage(catalogRepository: repository)),
    );
    await tester.tap(find.bySemanticsLabel('System'));
    await tester.pumpAndSettle();
    final classification = find.byKey(
      const ValueKey<String>('settings-option-library-classification'),
    );
    await tester.scrollUntilVisible(
      classification,
      240,
      scrollable: find.byType(Scrollable).last,
    );
    await tester.tap(classification);
    await tester.pumpAndSettle();

    expect(find.byType(LibraryClassificationPage), findsOneWidget);
    expect(find.text('GOverlay'), findsOneWidget);
    await tester.tap(find.text('GOverlay'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('App'));
    await tester.pumpAndSettle();

    expect(repository.classifiedItemId, 'desktop:goverlay.desktop');
    expect(repository.classifiedKind, 'application');
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

  testWidgets('content details open an external metadata link', (
    WidgetTester tester,
  ) async {
    final externalLink = _FakeExternalLink();
    await tester.pumpWidget(
      MaterialApp(
        home: ContentDetailsPage(
          item: const DashboardItem(
            id: 'example',
            title: 'Example',
            icon: Icons.apps_rounded,
            colors: <Color>[Color(0xFF000000), Color(0xFF111111)],
            details: ContentDetails(
              summary: 'Example application',
              actions: <ContentAction>[
                ContentAction(
                  id: 'homepage',
                  label: 'Website',
                  icon: Icons.language_rounded,
                  url: 'https://example.org',
                ),
              ],
              facts: <ContentFact>[],
              galleryTitle: 'Screenshots',
              gallery: <ContentGalleryItem>[],
            ),
          ),
          sourceShape: TvTileShape.square,
          externalLink: externalLink,
        ),
      ),
    );

    await tester.pumpAndSettle();
    final website = find.text('Website', skipOffstage: false);
    await tester.ensureVisible(website);
    await tester.tap(find.text('Website'));
    await tester.pumpAndSettle();

    expect(externalLink.openedUrl, 'https://example.org');
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

  testWidgets('settings selects and persists a curated color palette', (
    WidgetTester tester,
  ) async {
    addTearDown(tester.view.reset);
    tester.view
      ..devicePixelRatio = 1
      ..physicalSize = const Size(1280, 900);
    SharedPreferences.setMockInitialValues(<String, Object>{});
    addTearDown(
      () => SharedPreferences.setMockInitialValues(<String, Object>{}),
    );
    final settingsRepository = await CachedUserSettingsRepository.load(
      preferences: await SharedPreferences.getInstance(),
    );
    await tester.pumpWidget(
      HearthdeckApp(settingsRepository: settingsRepository),
    );
    await tester.tap(find.bySemanticsLabel('Settings'));
    await tester.pumpAndSettle();

    await tester.tap(
      find.byKey(const ValueKey<String>('settings-option-personalization')),
    );
    await tester.pumpAndSettle();
    expect(find.text('Live role preview'), findsOneWidget);
    expect(find.text('Primary action'), findsOneWidget);
    expect(find.text('Ready'), findsOneWidget);
    await tester.tap(find.text('Quiet grid'));
    await tester.pumpAndSettle();
    await tester.scrollUntilVisible(
      find.text('Ember'),
      240,
      scrollable: find.byWidgetPredicate(
        (Widget widget) =>
            widget is Scrollable && widget.axisDirection == AxisDirection.down,
      ),
    );
    await tester.drag(find.byType(CustomScrollView), const Offset(0, -160));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Ember'));
    await tester.pumpAndSettle();

    final pageTitle = find.text('Appearance & color');
    expect(pageTitle, findsOneWidget);
    expect(TvThemeScope.of(tester.element(pageTitle)).mode, TvThemeMode.ember);
    expect(
      TvPalette.of(tester.element(pageTitle)).action,
      TvTheme.colorsFor(TvThemeMode.ember, null).primary,
    );
    expect(settingsRepository.settings.themeMode, TvThemeMode.ember);
    expect(settingsRepository.settings.backdropMode, TvBackdropMode.quietGrid);
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

  testWidgets('settings confirms exit to desktop before ending the session', (
    WidgetTester tester,
  ) async {
    final session = _FakePlatformSession();
    await tester.pumpWidget(
      MaterialApp(
        home: SettingsPage(
          catalogRepository: const MockCatalogRepository(),
          platformSession: session,
        ),
      ),
    );

    final exitToDesktop = find.byKey(
      const ValueKey<String>('settings-option-exit-to-desktop'),
    );
    await tester.scrollUntilVisible(
      exitToDesktop,
      240,
      scrollable: find.byType(Scrollable).last,
    );
    await tester.tap(exitToDesktop);
    await tester.pumpAndSettle();

    expect(find.text('Exit to desktop?'), findsOneWidget);
    expect(session.exitRequested, isFalse);

    await tester.tap(find.widgetWithText(FilledButton, 'Exit to desktop'));
    await tester.pumpAndSettle();

    expect(session.exitRequested, isTrue);
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
    await tester.scrollUntilVisible(
      find.text('RomM connection'),
      360,
      scrollable: find.byType(Scrollable).last,
    );
    expect(find.text('RomM connection'), findsOneWidget);
    await tester.scrollUntilVisible(
      find.text('Recent service events'),
      360,
      scrollable: find.byType(Scrollable).last,
    );
    expect(find.text('Recent service events'), findsOneWidget);
    expect(
      find.text('RomM console check completed (console_count=12)'),
      findsOneWidget,
    );
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

  testWidgets('virtual keyboard follows every editable text focus', (
    WidgetTester tester,
  ) async {
    final keyboard = _FakeVirtualKeyboard();
    final focusNode = FocusNode();
    await tester.pumpWidget(
      VirtualKeyboardFocusObserver(
        virtualKeyboard: keyboard,
        child: MaterialApp(
          home: Scaffold(body: TextField(focusNode: focusNode)),
        ),
      ),
    );
    await tester.pump();

    focusNode.requestFocus();
    await tester.pump();
    expect(keyboard.showRequests, 1);

    focusNode.unfocus();
    await tester.pump();

    expect(keyboard.hideRequests, 1);
    focusNode.dispose();
  });

  testWidgets(
    'controller back dismisses a focused text input before its route',
    (WidgetTester tester) async {
      Gamepads.normalizer = GamepadNormalizer.forPlatform(
        GamepadPlatform.linux,
      );
      addTearDown(() => Gamepads.normalizer = null);
      final keyboard = _FakeVirtualKeyboard();
      await tester.pumpWidget(HearthdeckApp(virtualKeyboard: keyboard));
      await tester.tap(find.bySemanticsLabel('Search'));
      await tester.pumpAndSettle();

      await _sendLinuxGamepadButton('1', 1.0);
      await _sendLinuxGamepadButton('1', 0.0);
      await tester.pumpAndSettle();

      expect(find.byType(TvSearchPage), findsOneWidget);
      expect(
        FocusManager.instance.primaryFocus?.debugLabel,
        isNot('Search input'),
      );
      expect(keyboard.hideRequests, 1);
    },
  );

  testWidgets('directional focus leaves text input after an OSK dismissal', (
    WidgetTester tester,
  ) async {
    final keyboard = _FakeVirtualKeyboard();
    await tester.pumpWidget(HearthdeckApp(virtualKeyboard: keyboard));
    await tester.tap(find.bySemanticsLabel('Search'));
    await tester.pumpAndSettle();

    Actions.invoke(
      tester.element(find.byKey(const ValueKey<String>('search-input'))),
      TvGamepadBindings.down,
    );
    await tester.pumpAndSettle();

    expect(
      FocusManager.instance.primaryFocus?.debugLabel,
      isNot('Search input'),
    );
    expect(keyboard.externalDismissRequests, 1);
    expect(keyboard.hideRequests, 0);
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

  testWidgets('Games library switches from PC games to RomM consoles', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.bySemanticsLabel('Full library'));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Consoles'));
    await tester.pumpAndSettle();

    expect(
      find.byKey(const ValueKey<String>('library-tile-romm-console-nes')),
      findsOneWidget,
    );
  });

  testWidgets('console tile opens its RomM platform overview', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const HearthdeckApp());
    await tester.tap(find.bySemanticsLabel('Full library'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Consoles'));
    await tester.pumpAndSettle();

    await tester.tap(
      find.byKey(const ValueKey<String>('library-tile-romm-console-nes')),
    );
    await tester.pumpAndSettle();

    expect(find.byType(ContentDetailsPage), findsOneWidget);
    expect(find.text('Nintendo Entertainment System'), findsOneWidget);
    expect(find.text('341 games in RomM'), findsOneWidget);
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
    expect(find.text('YOUR GAME STATS'), findsOneWidget);
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
  Future<HearthdeckDiagnostics> diagnostics() =>
      const MockCatalogRepository().diagnostics();

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
  Future<List<HearthdeckLibraryItem>> libraryItems() async =>
      const <HearthdeckLibraryItem>[];

  @override
  Future<void> updateLibraryClassification({
    required String itemId,
    required String? kind,
  }) async {}

  @override
  Future<void> launch(DashboardItem item) async {}

  @override
  Future<void> requestRescan() async {}

  @override
  Future<void> requestProviderRefresh(
    HearthdeckProviderHealth provider,
  ) async {}

  @override
  Stream<CatalogEvent> watch() =>
      Stream<CatalogEvent>.error(StateError('event feed unavailable'));
}

class _FakePlatformSession implements PlatformSession {
  bool exitRequested = false;

  @override
  bool get supportsExitToDesktop => true;

  @override
  Future<void> exitToDesktop() async {
    exitRequested = true;
  }
}

class _FakeVirtualKeyboard implements VirtualKeyboard {
  var showRequests = 0;
  var hideRequests = 0;
  var externalDismissRequests = 0;

  @override
  Future<void> show() async {
    showRequests += 1;
  }

  @override
  Future<void> hide() async {
    hideRequests += 1;
  }

  @override
  void didDismissExternally() {
    externalDismissRequests += 1;
  }
}

class _FakeExternalLink implements ExternalLink {
  String? openedUrl;

  @override
  Future<void> open(String url) async {
    openedUrl = url;
  }
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
        lastAttemptAt: null,
        lastSuccessAt: null,
        lastError: 'bridge unavailable',
      ),
    ],
  );

  @override
  Future<HearthdeckDiagnostics> diagnostics() async =>
      const HearthdeckDiagnostics(
        generatedAt: '2026-01-01T00:00:00Z',
        services: <HearthdeckServiceStatus>[
          HearthdeckServiceStatus(
            id: 'daemon',
            unit: 'hearthdeck-daemon.service',
            state: 'active',
            detail: 'active (running)',
          ),
        ],
        romm: HearthdeckRommDiagnostic(
          configured: true,
          status: 'ready',
          baseUrl: 'http://127.0.0.1:8080',
          consoleCount: 12,
          checkedAt: '2026-01-01T00:00:00Z',
          error: null,
        ),
        logs: HearthdeckLogTail(
          available: true,
          error: null,
          entries: <HearthdeckLogEntry>[
            HearthdeckLogEntry(
              timestamp: '2026-01-01T00:00:00Z',
              service: 'Daemon',
              level: 'info',
              message: 'RomM console check completed (console_count=12)',
            ),
          ],
        ),
      );
}

class _ClassificationCatalogRepository extends MockCatalogRepository {
  String? classifiedItemId;
  String? classifiedKind;

  @override
  Future<List<HearthdeckLibraryItem>> libraryItems() async =>
      <HearthdeckLibraryItem>[
        const HearthdeckLibraryItem(
          id: 'desktop:goverlay.desktop',
          sourceId: 'desktop-apps',
          title: 'GOverlay',
          kind: 'game',
          launchId: 'goverlay.desktop',
          icon: null,
          metadata: <String, dynamic>{
            'classification': <String, dynamic>{
              'kind': null,
              'discovered_kind': 'game',
              'overridden': false,
            },
          },
        ),
      ];

  @override
  Future<void> updateLibraryClassification({
    required String itemId,
    required String? kind,
  }) async {
    classifiedItemId = itemId;
    classifiedKind = kind;
  }
}
