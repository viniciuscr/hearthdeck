import 'package:flutter/material.dart';
import 'package:flutter_gamepads/flutter_gamepads.dart';

/// Requests the same navigator behavior as a keyboard Escape press.
class TvBackIntent extends Intent {
  const TvBackIntent();
}

/// Moves focus between TV controls without invoking native text editing.
class TvDirectionalFocusIntent extends Intent {
  const TvDirectionalFocusIntent(this.direction);

  final TraversalDirection direction;
}

abstract final class TvGamepadBindings {
  static const TvDirectionalFocusIntent up = TvDirectionalFocusIntent(
    TraversalDirection.up,
  );
  static const TvDirectionalFocusIntent down = TvDirectionalFocusIntent(
    TraversalDirection.down,
  );
  static const TvDirectionalFocusIntent left = TvDirectionalFocusIntent(
    TraversalDirection.left,
  );
  static const TvDirectionalFocusIntent right = TvDirectionalFocusIntent(
    TraversalDirection.right,
  );
  static const TvBackIntent back = TvBackIntent();

  static const Map<GamepadActivator, Intent> shortcuts =
      <GamepadActivator, Intent>{
        GamepadActivatorButton.dpadUp(): up,
        GamepadActivatorButton.dpadDown(): down,
        GamepadActivatorButton.dpadLeft(): left,
        GamepadActivatorButton.dpadRight(): right,
        GamepadActivatorAxis.leftStickUp(): up,
        GamepadActivatorAxis.leftStickDown(): down,
        GamepadActivatorAxis.leftStickLeft(): left,
        GamepadActivatorAxis.leftStickRight(): right,
        GamepadActivatorButton.a(): ActivateIntent(),
        GamepadActivatorButton.b(): back,
        GamepadActivatorButton.back(): back,
        GamepadActivatorAxis.rightStickUp(): ScrollIntent(
          direction: AxisDirection.up,
        ),
        GamepadActivatorAxis.rightStickDown(): ScrollIntent(
          direction: AxisDirection.down,
        ),
        GamepadActivatorAxis.rightStickLeft(): ScrollIntent(
          direction: AxisDirection.left,
        ),
        GamepadActivatorAxis.rightStickRight(): ScrollIntent(
          direction: AxisDirection.right,
        ),
      };

  static const Set<Intent> repeatIntents = <Intent>{
    up,
    down,
    left,
    right,
    ScrollIntent(direction: AxisDirection.up),
    ScrollIntent(direction: AxisDirection.down),
    ScrollIntent(direction: AxisDirection.left),
    ScrollIntent(direction: AxisDirection.right),
  };
}
