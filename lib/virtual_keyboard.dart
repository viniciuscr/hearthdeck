import 'dart:async';
import 'dart:io';

import 'package:flutter/widgets.dart';

abstract interface class VirtualKeyboard {
  Future<void> show();

  Future<void> hide();
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

  @override
  void initState() {
    super.initState();
    FocusManager.instance.addListener(_handleFocusChanged);
    WidgetsBinding.instance.addPostFrameCallback((_) => _handleFocusChanged());
  }

  @override
  void dispose() {
    FocusManager.instance.removeListener(_handleFocusChanged);
    if (_editableFocused) {
      unawaited(_virtualKeyboard.hide());
    }
    super.dispose();
  }

  void _handleFocusChanged() {
    final editable = _editableTextFor(FocusManager.instance.primaryFocus);
    final editableFocused = editable != null && !editable.readOnly;
    if (_editableFocused == editableFocused) {
      return;
    }
    _editableFocused = editableFocused;
    if (editableFocused) {
      unawaited(_virtualKeyboard.show());
    } else {
      unawaited(_virtualKeyboard.hide());
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

  @override
  Widget build(BuildContext context) => widget.child;
}
