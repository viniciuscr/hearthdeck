import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:shared_preferences/shared_preferences.dart';

import '../backend/hearthdeck_api_client.dart';
import '../backend/hearthdeck_endpoint.dart';
import '../backend/local_hearthdeck_client.dart';
import '../tv_theme.dart';

class UserSettings {
  const UserSettings({
    required this.themeMode,
    required this.backdropMode,
    required this.revision,
    this.isPending = false,
  });

  final TvThemeMode themeMode;
  final TvBackdropMode backdropMode;
  final int? revision;
  final bool isPending;

  bool get hasPendingSync => isPending;

  UserSettings copyWith({
    TvThemeMode? themeMode,
    TvBackdropMode? backdropMode,
    int? revision,
    bool? isPending,
  }) => UserSettings(
    themeMode: themeMode ?? this.themeMode,
    backdropMode: backdropMode ?? this.backdropMode,
    revision: revision ?? this.revision,
    isPending: isPending ?? this.isPending,
  );
}

abstract interface class UserSettingsRepository {
  UserSettings get settings;

  /// Saves locally before the caller updates visible state.
  Future<UserSettings> setThemeMode(TvThemeMode mode);

  /// Saves the static backdrop treatment with the same cache-first semantics.
  Future<UserSettings> setBackdropMode(TvBackdropMode mode);

  /// Retries a write left by an interrupted or offline previous app session.
  void retryPending();
}

class InMemoryUserSettingsRepository implements UserSettingsRepository {
  InMemoryUserSettingsRepository([TvThemeMode mode = TvThemeMode.system])
    : _settings = UserSettings(
        themeMode: mode,
        backdropMode: TvBackdropMode.edgeWash,
        revision: null,
      );

  UserSettings _settings;

  @override
  UserSettings get settings => _settings;

  @override
  Future<UserSettings> setThemeMode(TvThemeMode mode) async {
    _settings = _settings.copyWith(themeMode: mode);
    return _settings;
  }

  @override
  Future<UserSettings> setBackdropMode(TvBackdropMode mode) async {
    _settings = _settings.copyWith(backdropMode: mode);
    return _settings;
  }

  @override
  void retryPending() {}
}

class CachedUserSettingsRepository implements UserSettingsRepository {
  CachedUserSettingsRepository._({
    required this._preferences,
    required this._settings,
    this.createClient,
  });

  static const _cacheKey = 'user-settings-v1';
  static const _legacyThemeKey = 'theme-mode';

  final SharedPreferences _preferences;
  final Future<HearthdeckApiClient> Function()? createClient;
  UserSettings _settings;
  HearthdeckApiClient? _client;
  Future<void> _cacheQueue = Future<void>.value();
  Future<void>? _syncing;
  bool _syncAgain = false;

  static Future<CachedUserSettingsRepository> load({
    required SharedPreferences preferences,
    Future<HearthdeckApiClient> Function()? createClient,
  }) async {
    final cached = _readCached(preferences.getString(_cacheKey));
    if (cached != null) {
      return CachedUserSettingsRepository._(
        preferences: preferences,
        settings: cached,
        createClient: createClient,
      );
    }

    final legacyMode = _modeFromName(preferences.getString(_legacyThemeKey));
    final settings = UserSettings(
      themeMode: legacyMode ?? TvThemeMode.system,
      backdropMode: TvBackdropMode.edgeWash,
      revision: null,
      isPending: legacyMode != null && createClient != null,
    );
    if (legacyMode != null) {
      await _write(preferences, settings);
      await preferences.remove(_legacyThemeKey);
    }
    return CachedUserSettingsRepository._(
      preferences: preferences,
      settings: settings,
      createClient: createClient,
    );
  }

  @override
  UserSettings get settings => _settings;

  @override
  Future<UserSettings> setThemeMode(TvThemeMode mode) {
    return _updateAppearance(themeMode: mode);
  }

  @override
  Future<UserSettings> setBackdropMode(TvBackdropMode mode) {
    return _updateAppearance(backdropMode: mode);
  }

  Future<UserSettings> _updateAppearance({
    TvThemeMode? themeMode,
    TvBackdropMode? backdropMode,
  }) {
    final write = _enqueueCache(() async {
      final next = UserSettings(
        themeMode: themeMode ?? _settings.themeMode,
        backdropMode: backdropMode ?? _settings.backdropMode,
        revision: _settings.revision,
        isPending: createClient != null,
      );
      await _write(_preferences, next);
      _settings = next;
      return next;
    });
    write.then<void>((_) => _scheduleSync(), onError: (_, _) {});
    return write;
  }

  @override
  void retryPending() => _scheduleSync();

  void _scheduleSync() {
    if (createClient == null || !_settings.hasPendingSync) {
      return;
    }
    if (_syncing != null) {
      _syncAgain = true;
      return;
    }
    _syncing = _flushPending().whenComplete(() {
      _syncing = null;
      if (_syncAgain) {
        _syncAgain = false;
        _scheduleSync();
      }
    });
  }

  Future<void> _flushPending() async {
    try {
      while (_settings.isPending) {
        final pending = _settings;
        final client = await _getClient().timeout(const Duration(seconds: 2));
        HearthdeckUserSettings saved;
        try {
          saved = await client
              .updateUserSettings(
                themeMode: pending.themeMode.name,
                backdropMode: pending.backdropMode.wireName,
                revision: pending.revision,
              )
              .timeout(const Duration(seconds: 2));
        } on HearthdeckSettingsConflict catch (conflict) {
          if (!_matches(_settings, pending)) {
            return;
          }
          try {
            saved = await client
                .updateUserSettings(
                  themeMode: pending.themeMode.name,
                  backdropMode: pending.backdropMode.wireName,
                  revision: conflict.settings.revision,
                )
                .timeout(const Duration(seconds: 2));
          } on Object {
            _resetClient();
            return;
          }
        } on HearthdeckApiException catch (error) {
          if (error.statusCode == 401) {
            _resetClient();
          }
          return;
        } on Object {
          _resetClient();
          return;
        }
        await _commitServerSettings(pending, saved);
      }
    } on Object {
      _resetClient();
    }
  }

  Future<HearthdeckApiClient> _getClient() =>
      _client != null ? Future<HearthdeckApiClient>.value(_client) : _create();

  Future<HearthdeckApiClient> _create() async {
    final client = await createClient!();
    _client = client;
    return client;
  }

  void _resetClient() {
    _client?.close();
    _client = null;
  }

  Future<void> _commitServerSettings(
    UserSettings pending,
    HearthdeckUserSettings saved,
  ) => _enqueueCache(() async {
    final serverMode = _modeFromName(saved.themeMode);
    final serverBackdrop = _backdropFromName(saved.backdropMode);
    if (serverMode == null || serverBackdrop == null) {
      return;
    }
    final next = _matches(_settings, pending)
        ? UserSettings(
            themeMode: serverMode,
            backdropMode: serverBackdrop,
            revision: saved.revision,
          )
        : _settings.copyWith(revision: saved.revision, isPending: true);
    await _write(_preferences, next);
    _settings = next;
  });

  Future<T> _enqueueCache<T>(Future<T> Function() action) {
    final operation = _cacheQueue.then((_) => action());
    _cacheQueue = operation.then<void>((_) {}, onError: (_, _) {});
    return operation;
  }

  static UserSettings? _readCached(String? encoded) {
    if (encoded == null) {
      return null;
    }
    try {
      final json = jsonDecode(encoded) as Map<String, dynamic>;
      final themeMode = _modeFromName(json['theme_mode'] as String?);
      if (themeMode == null) {
        return null;
      }
      return UserSettings(
        themeMode: themeMode,
        backdropMode:
            _backdropFromName(json['backdrop_mode'] as String?) ??
            TvBackdropMode.edgeWash,
        revision: json['revision'] as int?,
        isPending: json['pending'] as bool? ?? false,
      );
    } on FormatException {
      return null;
    } on TypeError {
      return null;
    }
  }

  static TvThemeMode? _modeFromName(String? name) {
    for (final mode in TvThemeMode.values) {
      if (mode.name == name) {
        return mode;
      }
    }
    return null;
  }

  static TvBackdropMode? _backdropFromName(String? name) {
    for (final mode in TvBackdropMode.values) {
      if (mode.name == name || mode.wireName == name) {
        return mode;
      }
    }
    return null;
  }

  static bool _matches(UserSettings left, UserSettings right) =>
      left.themeMode == right.themeMode &&
      left.backdropMode == right.backdropMode &&
      left.revision == right.revision;

  static Future<void> _write(
    SharedPreferences preferences,
    UserSettings settings,
  ) => preferences.setString(
    _cacheKey,
    jsonEncode(<String, Object?>{
      'theme_mode': settings.themeMode.name,
      'backdrop_mode': settings.backdropMode.wireName,
      'revision': settings.revision,
      'pending': settings.isPending,
    }),
  );
}

Future<UserSettingsRepository> createUserSettingsRepository(
  SharedPreferences preferences,
) => CachedUserSettingsRepository.load(
  preferences: preferences,
  createClient: _settingsClientFactory(),
);

Future<HearthdeckApiClient> Function()? _settingsClientFactory() {
  const backendUrl = String.fromEnvironment('HEARTHDECK_BACKEND_URL');
  const token = String.fromEnvironment('HEARTHDECK_PAIRING_TOKEN');
  if (backendUrl.isNotEmpty && token.isNotEmpty) {
    return () async => HearthdeckApiClient(
      endpoint: HearthdeckEndpoint.parse(backendUrl),
      token: token,
    );
  }
  const useLocalCatalog = bool.fromEnvironment('HEARTHDECK_USE_LOCAL_CATALOG');
  if (useLocalCatalog && Platform.isLinux) {
    return createLocalHearthdeckClient;
  }
  return null;
}
