import 'package:flutter/material.dart';
import 'package:flutter_gamepads/flutter_gamepads.dart';

/// Requests the same navigator behavior as a keyboard Escape press.
class TvBackIntent extends Intent {
  const TvBackIntent();
}

abstract final class TvGamepadBindings {
  static const DirectionalFocusIntent up = DirectionalFocusIntent(
    TraversalDirection.up,
  );
  static const DirectionalFocusIntent down = DirectionalFocusIntent(
    TraversalDirection.down,
  );
  static const DirectionalFocusIntent left = DirectionalFocusIntent(
    TraversalDirection.left,
  );
  static const DirectionalFocusIntent right = DirectionalFocusIntent(
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
