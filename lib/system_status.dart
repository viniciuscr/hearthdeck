import 'dart:io';

/// Battery charge, read from the Linux `/sys/class/power_supply` sysfs
/// interface. `null` (from [readBatteryStatus]) means no battery was found,
/// e.g. on a desktop with no battery, or a non-Linux dev build.
class BatteryStatus {
  const BatteryStatus({required this.percent, required this.charging});

  final int percent;
  final bool charging;
}

/// Reads the first battery under `/sys/class/power_supply/BAT*`.
/// ponytail: sysfs instead of a UPower/D-Bus client; add the latter if a
/// device exposes battery info some other way.
BatteryStatus? readBatteryStatus() {
  if (!Platform.isLinux) {
    return null;
  }
  final base = Directory('/sys/class/power_supply');
  if (!base.existsSync()) {
    return null;
  }
  for (final entry in base.listSync()) {
    final name = entry.path.split(Platform.pathSeparator).last;
    if (!name.startsWith('BAT')) {
      continue;
    }
    try {
      final percent = int.parse(
        File('${entry.path}/capacity').readAsStringSync().trim(),
      );
      final status = File(
        '${entry.path}/status',
      ).readAsStringSync().trim();
      return BatteryStatus(percent: percent, charging: status == 'Charging');
    } on FileSystemException {
      continue;
    } on FormatException {
      continue;
    }
  }
  return null;
}

/// Whether any non-loopback network interface currently has an address.
/// A best-effort "are we online" check without a network-connectivity
/// dependency; it doesn't distinguish Wi-Fi from Ethernet.
Future<bool> isNetworkConnected() async {
  try {
    final interfaces = await NetworkInterface.list(
      includeLoopback: false,
      includeLinkLocal: false,
    );
    return interfaces.any((NetworkInterface i) => i.addresses.isNotEmpty);
  } on SocketException {
    return false;
  }
}
