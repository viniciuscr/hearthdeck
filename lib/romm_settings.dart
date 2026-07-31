import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'backend/hearthdeck_api_client.dart';
import 'catalog/catalog_repository_factory.dart';
import 'tv_components.dart';
import 'tv_theme.dart';
import 'virtual_keyboard.dart';

class RommSettingsPage extends StatefulWidget {
  const RommSettingsPage({super.key, this.apiClient});

  final HearthdeckApiClient? apiClient;

  @override
  State<RommSettingsPage> createState() => _RommSettingsPageState();
}

class _RommSettingsPageState extends State<RommSettingsPage> {
  final _urlController = TextEditingController();
  final _tokenController = TextEditingController();
  final _urlFocusNode = FocusNode(debugLabel: 'RomM URL');
  final _tokenFocusNode = FocusNode(debugLabel: 'RomM token');
  late final Future<HearthdeckApiClient?> _apiClient =
      widget.apiClient == null
      ? createRetroApiClient()
      : Future<HearthdeckApiClient?>.value(widget.apiClient);
  var _isLoading = true;
  var _isSaving = false;
  String? _error;
  HearthdeckRommSettings? _settings;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    _urlController.dispose();
    _tokenController.dispose();
    _urlFocusNode.dispose();
    _tokenFocusNode.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    try {
      final apiClient = await _apiClient;
      if (apiClient == null) {
        throw StateError('No Hearthdeck backend is connected.');
      }
      final settings = await apiClient.rommSettings();
      if (mounted) {
        setState(() {
          _settings = settings;
          _urlController.text = settings?.baseUrl ?? 'http://127.0.0.1:8080';
          _error = null;
          _isLoading = false;
        });
      }
    } catch (error) {
      if (mounted) {
        setState(() {
          _error = '$error';
          _isLoading = false;
        });
      }
    }
  }

  Future<void> _save() async {
    final baseUrl = _urlController.text.trim();
    final token = _tokenController.text.trim();
    if (token.isEmpty) {
      setState(() => _error = 'Enter a RomM client token to save this connection.');
      _tokenFocusNode.requestFocus();
      return;
    }
    setState(() {
      _isSaving = true;
      _error = null;
    });
    try {
      final apiClient = await _apiClient;
      if (apiClient == null) {
        throw StateError('No Hearthdeck backend is connected.');
      }
      final settings = await apiClient.updateRommSettings(
        baseUrl: baseUrl,
        token: token,
      );
      if (mounted) {
        setState(() {
          _settings = settings;
          _urlController.text = settings.baseUrl;
          _tokenController.clear();
        });
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('RomM connection saved.')),
        );
      }
    } catch (error) {
      if (mounted) {
        setState(() => _error = '$error');
      }
    } finally {
      if (mounted) {
        setState(() => _isSaving = false);
      }
    }
  }

  Future<void> _disconnect() async {
    setState(() {
      _isSaving = true;
      _error = null;
    });
    try {
      final apiClient = await _apiClient;
      if (apiClient == null) {
        throw StateError('No Hearthdeck backend is connected.');
      }
      await apiClient.clearRommSettings();
      if (mounted) {
        setState(() {
          _settings = null;
          _tokenController.clear();
        });
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('RomM connection removed.')),
        );
      }
    } catch (error) {
      if (mounted) {
        setState(() => _error = '$error');
      }
    } finally {
      if (mounted) {
        setState(() => _isSaving = false);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
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
                  const Positioned.fill(child: _RommSettingsBackdrop()),
                  if (_isLoading)
                    const Center(child: CircularProgressIndicator())
                  else
                    Center(
                      child: SingleChildScrollView(
                        padding: const EdgeInsets.all(32),
                        child: ConstrainedBox(
                          constraints: const BoxConstraints(maxWidth: 660),
                          child: DecoratedBox(
                            decoration: BoxDecoration(
                              color: tv.surface.withValues(alpha: 0.94),
                              borderRadius: BorderRadius.circular(14),
                              border: Border.all(color: tv.borderSubtle),
                            ),
                            child: Padding(
                              padding: const EdgeInsets.all(28),
                              child: Column(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: <Widget>[
                                  Row(
                                    children: <Widget>[
                                      Icon(
                                        Icons.videogame_asset_rounded,
                                        color: tv.accent,
                                        size: 34,
                                      ),
                                      const SizedBox(width: 14),
                                      Text(
                                        'Retro & RomM',
                                        style: Theme.of(
                                          context,
                                        ).textTheme.headlineSmall,
                                      ),
                                    ],
                                  ),
                                  const SizedBox(height: 12),
                                  Text(
                                    'Connect the local RomM server that manages your emulator library. Hearthdeck stores the token in its local service and never displays it again.',
                                    style: TextStyle(color: tv.secondaryText),
                                  ),
                                  const SizedBox(height: 28),
                                  _RommTextField(
                                    label: 'RomM server URL',
                                    hintText: 'http://127.0.0.1:8080',
                                    controller: _urlController,
                                    focusNode: _urlFocusNode,
                                    textInputAction: TextInputAction.next,
                                    onSubmitted: (_) =>
                                        _tokenFocusNode.requestFocus(),
                                  ),
                                  const SizedBox(height: 18),
                                  _RommTextField(
                                    label: 'RomM client token',
                                    hintText: 'rmm_...',
                                    controller: _tokenController,
                                    focusNode: _tokenFocusNode,
                                    obscureText: true,
                                    textInputAction: TextInputAction.done,
                                    onSubmitted: (_) => _save(),
                                  ),
                                  const SizedBox(height: 12),
                                  Text(
                                    _settings == null
                                        ? 'Not connected'
                                        : 'Connected to ${_settings!.baseUrl}',
                                    style: TextStyle(
                                      color: _settings == null
                                          ? tv.warning
                                          : tv.success,
                                    ),
                                  ),
                                  if (_error case final String error) ...<Widget>[
                                    const SizedBox(height: 12),
                                    Text(
                                      error,
                                      style: TextStyle(color: tv.warning),
                                    ),
                                  ],
                                  const SizedBox(height: 26),
                                  Wrap(
                                    spacing: 14,
                                    runSpacing: 14,
                                    children: <Widget>[
                                      _RommAction(
                                        label: _isSaving
                                            ? 'Saving...'
                                            : 'Save connection',
                                        icon: Icons.save_outlined,
                                        autofocus: true,
                                        onActivate: _isSaving ? () {} : _save,
                                      ),
                                      if (_settings != null)
                                        _RommAction(
                                          label: 'Disconnect',
                                          icon: Icons.link_off_rounded,
                                          primary: false,
                                          onActivate: _isSaving
                                              ? () {}
                                              : _disconnect,
                                        ),
                                    ],
                                  ),
                                ],
                              ),
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
  }
}

class _RommTextField extends StatelessWidget {
  const _RommTextField({
    required this.label,
    required this.hintText,
    required this.controller,
    required this.focusNode,
    required this.textInputAction,
    required this.onSubmitted,
    this.obscureText = false,
  });

  final String label;
  final String hintText;
  final TextEditingController controller;
  final FocusNode focusNode;
  final TextInputAction textInputAction;
  final ValueChanged<String> onSubmitted;
  final bool obscureText;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        Text(label, style: const TextStyle(fontWeight: FontWeight.w700)),
        const SizedBox(height: 8),
        TextField(
          controller: controller,
          focusNode: focusNode,
          obscureText: obscureText,
          enableSuggestions: !obscureText,
          autocorrect: false,
          textInputAction: textInputAction,
          onSubmitted: onSubmitted,
          style: Theme.of(context).textTheme.titleMedium,
          cursorColor: tv.focus,
          decoration: InputDecoration(
            hintText: hintText,
            hintStyle: TextStyle(color: tv.secondaryText),
            filled: true,
            fillColor: tv.canvas,
            border: OutlineInputBorder(
              borderRadius: BorderRadius.circular(8),
              borderSide: BorderSide(color: tv.borderSubtle),
            ),
            focusedBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(8),
              borderSide: BorderSide(color: tv.focus, width: 2),
            ),
          ),
        ),
      ],
    );
  }
}

class _RommAction extends StatelessWidget {
  const _RommAction({
    required this.label,
    required this.icon,
    required this.onActivate,
    this.autofocus = false,
    this.primary = true,
  });

  final String label;
  final IconData icon;
  final VoidCallback onActivate;
  final bool autofocus;
  final bool primary;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return TvFocusable(
      semanticLabel: label,
      autofocus: autofocus,
      onActivate: onActivate,
      builder: (BuildContext context, bool isFocused) {
        final style = TvControlStyle.resolve(
          tv,
          variant: primary
              ? TvControlVariant.primary
              : TvControlVariant.secondary,
          isFocused: isFocused,
        );
        return AnimatedContainer(
          duration: TvTheme.focusDuration,
          curve: TvTheme.focusCurve,
          padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 12),
          decoration: BoxDecoration(
            color: style.background,
            borderRadius: BorderRadius.circular(8),
            border: Border.all(color: style.border, width: isFocused ? 2 : 1),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              Icon(icon, color: style.foreground),
              const SizedBox(width: 10),
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

class _RommSettingsBackdrop extends StatelessWidget {
  const _RommSettingsBackdrop();

  @override
  Widget build(BuildContext context) => const TvBackdrop(
    center: Alignment(0.68, -0.48),
  );
}
