import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'tv_components.dart';
import 'tv_theme.dart';

class TvSideSheet extends StatelessWidget {
  const TvSideSheet({
    required this.title,
    required this.child,
    super.key,
    this.onClose,
    this.widthFactor = 0.3,
  });

  final String title;
  final Widget child;
  final VoidCallback? onClose;
  final double widthFactor;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    final screenWidth = MediaQuery.sizeOf(context).width;
    final width = (screenWidth * widthFactor).clamp(300.0, 480.0);
    return Drawer(
      width: width,
      backgroundColor: tv.surface,
      surfaceTintColor: Colors.transparent,
      elevation: 18,
      semanticLabel: title,
      shape: const RoundedRectangleBorder(),
      child: SafeArea(
        child: Actions(
          actions: <Type, Action<Intent>>{
            DismissIntent: CallbackAction<DismissIntent>(
              onInvoke: (DismissIntent intent) {
                _close(context);
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
                  _close(context);
                  return KeyEventResult.handled;
                }
                return KeyEventResult.ignored;
              },
              child: Column(
                children: <Widget>[
                  Padding(
                    padding: const EdgeInsets.fromLTRB(22, 18, 14, 12),
                    child: Row(
                      children: <Widget>[
                        Expanded(
                          child: Text(
                            title,
                            style: Theme.of(context).textTheme.titleLarge,
                          ),
                        ),
                        TvFocusable(
                          semanticLabel: 'Close $title',
                          onActivate: () => _close(context),
                          builder: (BuildContext context, bool isFocused) {
                            return AnimatedContainer(
                              duration: TvTheme.focusDuration,
                              width: 38,
                              height: 38,
                              decoration: BoxDecoration(
                                color: isFocused ? tv.focus : tv.surfaceMuted,
                                borderRadius: BorderRadius.circular(8),
                              ),
                              child: Icon(
                                Icons.close_rounded,
                                color: isFocused ? tv.canvas : tv.primaryText,
                              ),
                            );
                          },
                        ),
                      ],
                    ),
                  ),
                  Divider(
                    height: 1,
                    color: tv.borderSubtle,
                  ),
                  Expanded(child: child),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  void _close(BuildContext context) {
    onClose?.call();
    Navigator.of(context).pop();
  }
}

class TvSideSheetAction extends StatelessWidget {
  const TvSideSheetAction({
    required this.label,
    required this.icon,
    required this.onActivate,
    super.key,
    this.isPrimary = false,
  });

  final String label;
  final IconData icon;
  final VoidCallback onActivate;
  final bool isPrimary;

  @override
  Widget build(BuildContext context) {
    final tv = TvPalette.of(context);
    return TvFocusable(
      semanticLabel: label,
      onActivate: onActivate,
      builder: (BuildContext context, bool isFocused) {
        final background = isFocused
            ? tv.focus
            : isPrimary
            ? tv.action
            : tv.surfaceMuted;
        final foreground = isFocused
            ? tv.canvas
            : isPrimary
            ? tv.onAction
            : tv.primaryText;
        return AnimatedContainer(
          duration: TvTheme.focusDuration,
          curve: TvTheme.focusCurve,
          constraints: const BoxConstraints(minHeight: 46),
          padding: const EdgeInsets.symmetric(horizontal: 15, vertical: 10),
          decoration: BoxDecoration(
            color: background,
            borderRadius: BorderRadius.circular(8),
            border: Border.all(
              color: isFocused ? tv.primaryText : Colors.transparent,
              width: 2,
            ),
          ),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: <Widget>[
              Icon(icon, color: foreground, size: 20),
              const SizedBox(width: 9),
              Flexible(
                child: Text(
                  label,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    color: foreground,
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

double sideSheetWidthFor(double viewportWidth, {double widthFactor = 0.3}) =>
    math.max(300, math.min(480, viewportWidth * widthFactor));
