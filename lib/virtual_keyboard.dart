import 'dart:async';
import 'dart:io';

import 'package:flutter/widgets.dart';

abstract interface class VirtualKeyboard {
  Future<void> show();

  Future<void> hide();

  void didDismissExternally();
}

typedef VirtualKeyboardCommand = Future<ProcessResult> Function();

class GamepadOskVirtualKeyboard implements VirtualKeyboard {
  GamepadOskVirtualKeyboard({VirtualKeyboardCommand? command, bool? enabled})
    : _command = command ?? _toggle,
      _enabled = enabled ?? Platform.isLinux;

  final VirtualKeyboardCommand _command;
  final bool _enabled;
  var _visible = false;
  var _requestedVisible = false;
  Future<void> _pending = Future<void>.value();

  @override
  Future<void> show() => _setVisible(true);

  @override
  Future<void> hide() => _setVisible(false);

  @override
  void didDismissExternally() {
    _visible = false;
    _requestedVisible = false;
  }

  Future<void> _setVisible(bool visible) {
    if (!_enabled) {
      return Future<void>.value();
    }
    _requestedVisible = visible;
    _pending = _pending.then((_) async {
      if (_visible == _requestedVisible) {
        return;
      }
      try {
        final result = await _command();
        if (result.exitCode == 0) {
          _visible = !_visible;
        }
      } on ProcessException {
        // The virtual keyboard is optional outside the console session.
      }
    });
    return _pending;
  }

  static Future<ProcessResult> _toggle() =>
      Process.run('gamepad-osk', const <String>['--toggle']);
}

class VirtualKeyboardFocusObserver extends StatefulWidget {
  const VirtualKeyboardFocusObserver({
    required this.child,
    super.key,
    this.virtualKeyboard,
  });

  final Widget child;
  final VirtualKeyboard? virtualKeyboard;

  @override
  State<VirtualKeyboardFocusObserver> createState() =>
      _VirtualKeyboardFocusObserverState();
}

class _VirtualKeyboardFocusObserverState
    extends State<VirtualKeyboardFocusObserver> {
  late final VirtualKeyboard _virtualKeyboard =
      widget.virtualKeyboard ?? GamepadOskVirtualKeyboard();
  var _editableFocused = false;
  var _keyboardDismissedExternally = false;

  @override
  void initState() {
    super.initState();
    FocusManager.instance.addListener(_handleFocusChanged);
    WidgetsBinding.instance.addPostFrameCallback((_) => _syncFocus());
  }

  @override
  void dispose() {
    FocusManager.instance.removeListener(_handleFocusChanged);
    if (_editableFocused) {
      unawaited(_virtualKeyboard.hide());
    }
    super.dispose();
  }

  void _handleFocusChanged() => _syncFocus();

  void _syncFocus() {
    final editableFocused = hasWritableEditableTextFocus();
    if (_editableFocused == editableFocused) {
      return;
    }
    _editableFocused = editableFocused;
    if (editableFocused) {
      _keyboardDismissedExternally = false;
      unawaited(_virtualKeyboard.show());
    } else if (_keyboardDismissedExternally) {
      _keyboardDismissedExternally = false;
    } else {
      unawaited(_virtualKeyboard.hide());
    }
  }

  bool dismissFocusedEditableForNavigation() {
    if (!hasWritableEditableTextFocus()) {
      return false;
    }
    _keyboardDismissedExternally = true;
    _virtualKeyboard.didDismissExternally();
    return true;
  }

  bool dismissFocusedEditableForBack() {
    if (!hasWritableEditableTextFocus()) {
      return false;
    }
    return unfocusWritableEditableText();
  }

  @override
  Widget build(BuildContext context) => VirtualKeyboardFocusScope(
    dismissFocusedEditableForNavigation: dismissFocusedEditableForNavigation,
    dismissFocusedEditableForBack: dismissFocusedEditableForBack,
    child: widget.child,
  );
}

class VirtualKeyboardFocusScope extends InheritedWidget {
  const VirtualKeyboardFocusScope({
    required this.dismissFocusedEditableForNavigation,
    required this.dismissFocusedEditableForBack,
    required super.child,
    super.key,
  });

  final bool Function() dismissFocusedEditableForNavigation;
  final bool Function() dismissFocusedEditableForBack;

  static bool dismissFocusedEditableForNavigationOf(BuildContext context) {
    final scope = context
        .dependOnInheritedWidgetOfExactType<VirtualKeyboardFocusScope>();
    return scope?.dismissFocusedEditableForNavigation() ??
        unfocusWritableEditableText();
  }

  static bool dismissFocusedEditableForBackOf(BuildContext context) {
    final scope = context
        .dependOnInheritedWidgetOfExactType<VirtualKeyboardFocusScope>();
    return scope?.dismissFocusedEditableForBack() ??
        unfocusWritableEditableText();
  }

  @override
  bool updateShouldNotify(VirtualKeyboardFocusScope oldWidget) => false;
}

bool hasWritableEditableTextFocus() {
  final editable = _editableTextFor(FocusManager.instance.primaryFocus);
  return editable != null && !editable.readOnly;
}

bool unfocusWritableEditableText() {
  if (!hasWritableEditableTextFocus()) {
    return false;
  }
  FocusManager.instance.primaryFocus?.unfocus();
  return true;
}

void dismissTextInputOrPop(BuildContext context) {
  if (!unfocusWritableEditableText()) {
    Navigator.of(context).maybePop();
  }
}

EditableText? _editableTextFor(FocusNode? focusNode) {
  final context = focusNode?.context;
  if (context == null) {
    return null;
  }
  if (context.widget case final EditableText editable) {
    return editable;
  }
  EditableText? editable;
  context.visitAncestorElements((Element element) {
    if (element.widget case final EditableText found) {
      editable = found;
      return false;
    }
    return true;
  });
  return editable;
}
