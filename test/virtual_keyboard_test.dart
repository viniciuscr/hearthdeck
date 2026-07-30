import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:hearthdeck/virtual_keyboard.dart';

void main() {
  test('serializes rapid focus visibility changes', () async {
    var toggles = 0;
    final keyboard = GamepadOskVirtualKeyboard(
      enabled: true,
      command: () async {
        toggles += 1;
        return ProcessResult(0, 0, '', '');
      },
    );

    await keyboard.show();
    await keyboard.hide();

    expect(toggles, 2);
  });

  test('does not invoke the command outside Linux mode', () async {
    var invoked = false;
    final keyboard = GamepadOskVirtualKeyboard(
      enabled: false,
      command: () async {
        invoked = true;
        return ProcessResult(0, 0, '', '');
      },
    );

    await keyboard.show();
    await keyboard.hide();

    expect(invoked, isFalse);
  });
}
