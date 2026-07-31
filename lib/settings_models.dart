import 'package:flutter/material.dart';

enum SettingsCategory {
  general,
  account,
  system,
  devices,
  preferences,
  accessibility,
}

class SettingsCategoryDefinition {
  const SettingsCategoryDefinition({
    required this.category,
    required this.label,
    required this.icon,
  });

  final SettingsCategory category;
  final String label;
  final IconData icon;
}

class SettingsOption {
  const SettingsOption({
    required this.id,
    required this.label,
    required this.description,
    required this.icon,
  });

  final String id;
  final String label;
  final String description;
  final IconData icon;
}

const List<SettingsCategoryDefinition> settingsCategories =
    <SettingsCategoryDefinition>[
      SettingsCategoryDefinition(
        category: SettingsCategory.general,
        label: 'General',
        icon: Icons.tune_rounded,
      ),
      SettingsCategoryDefinition(
        category: SettingsCategory.account,
        label: 'Account',
        icon: Icons.person_outline_rounded,
      ),
      SettingsCategoryDefinition(
        category: SettingsCategory.system,
        label: 'System',
        icon: Icons.memory_rounded,
      ),
      SettingsCategoryDefinition(
        category: SettingsCategory.devices,
        label: 'Devices & connections',
        icon: Icons.devices_other_outlined,
      ),
      SettingsCategoryDefinition(
        category: SettingsCategory.preferences,
        label: 'Preferences',
        icon: Icons.favorite_outline_rounded,
      ),
      SettingsCategoryDefinition(
        category: SettingsCategory.accessibility,
        label: 'Ease of access',
        icon: Icons.accessibility_new_rounded,
      ),
    ];

const Map<SettingsCategory, List<SettingsOption>> settingsOptions =
    <SettingsCategory, List<SettingsOption>>{
      SettingsCategory.general: <SettingsOption>[
        SettingsOption(
          id: 'network',
          label: 'Network settings',
          description: 'Wi-Fi, ethernet, and connection status',
          icon: Icons.wifi_rounded,
        ),
        SettingsOption(
          id: 'personalization',
          label: 'Appearance & color',
          description: 'System colors, palettes, and visual style',
          icon: Icons.palette_outlined,
        ),
        SettingsOption(
          id: 'display',
          label: 'TV & display options',
          description: 'Resolution, refresh rate, and HDR',
          icon: Icons.tv_outlined,
        ),
        SettingsOption(
          id: 'family',
          label: 'Online safety & family',
          description: 'Privacy and family controls',
          icon: Icons.family_restroom_outlined,
        ),
        SettingsOption(
          id: 'audio',
          label: 'Volume & audio output',
          description: 'Speakers, headset, and surround sound',
          icon: Icons.volume_up_outlined,
        ),
        SettingsOption(
          id: 'power',
          label: 'Power mode & startup',
          description: 'Sleep, wake, and startup behavior',
          icon: Icons.power_settings_new_rounded,
        ),
        SettingsOption(
          id: 'exit-to-desktop',
          label: 'Exit to desktop',
          description: 'Close Hearthdeck and return to the desktop',
          icon: Icons.logout_rounded,
        ),
      ],
      SettingsCategory.account: <SettingsOption>[
        SettingsOption(
          id: 'profile',
          label: 'Profile',
          description: 'Name, avatar, and public details',
          icon: Icons.badge_outlined,
        ),
        SettingsOption(
          id: 'signin',
          label: 'Sign-in & security',
          description: 'Passkeys and trusted devices',
          icon: Icons.lock_outline_rounded,
        ),
        SettingsOption(
          id: 'subscriptions',
          label: 'Subscriptions',
          description: 'Manage connected services',
          icon: Icons.receipt_long_outlined,
        ),
        SettingsOption(
          id: 'privacy',
          label: 'Privacy',
          description: 'Data and activity preferences',
          icon: Icons.shield_outlined,
        ),
      ],
      SettingsCategory.system: <SettingsOption>[
        SettingsOption(
          id: 'romm',
          label: 'Retro & RomM',
          description: 'Connect your local retro game library',
          icon: Icons.videogame_asset_rounded,
        ),
        SettingsOption(
          id: 'service-status',
          label: 'Service status',
          description: 'Provider health, errors, and last refreshes',
          icon: Icons.monitor_heart_outlined,
        ),
        SettingsOption(
          id: 'rescan-library',
          label: 'Rescan library',
          description: 'Refresh games, apps, and provider sources',
          icon: Icons.refresh_rounded,
        ),
        SettingsOption(
          id: 'updates',
          label: 'Updates',
          description: 'System and application updates',
          icon: Icons.system_update_alt_rounded,
        ),
        SettingsOption(
          id: 'storage',
          label: 'Storage',
          description: 'Manage installed content and free space',
          icon: Icons.storage_outlined,
        ),
        SettingsOption(
          id: 'language',
          label: 'Language & region',
          description: 'Locale, time format, and keyboard',
          icon: Icons.language_rounded,
        ),
        SettingsOption(
          id: 'about',
          label: 'About this device',
          description: 'System version and diagnostics',
          icon: Icons.info_outline_rounded,
        ),
      ],
      SettingsCategory.devices: <SettingsOption>[
        SettingsOption(
          id: 'controllers',
          label: 'Controllers',
          description: 'Pair and configure game controllers',
          icon: Icons.gamepad_outlined,
        ),
        SettingsOption(
          id: 'bluetooth',
          label: 'Bluetooth',
          description: 'Headsets, remotes, and accessories',
          icon: Icons.bluetooth_rounded,
        ),
        SettingsOption(
          id: 'remote',
          label: 'Remote features',
          description: 'Connect companion devices',
          icon: Icons.phonelink_rounded,
        ),
        SettingsOption(
          id: 'hdmi',
          label: 'HDMI control',
          description: 'TV power, volume, and input control',
          icon: Icons.settings_input_hdmi_rounded,
        ),
      ],
      SettingsCategory.preferences: <SettingsOption>[
        SettingsOption(
          id: 'home',
          label: 'Home experience',
          description: 'Rows, pinned items, and recommendations',
          icon: Icons.home_outlined,
        ),
        SettingsOption(
          id: 'notifications',
          label: 'Notifications',
          description: 'Alerts and quiet hours',
          icon: Icons.notifications_outlined,
        ),
        SettingsOption(
          id: 'capture',
          label: 'Captures',
          description: 'Screenshots, recordings, and sharing',
          icon: Icons.photo_camera_outlined,
        ),
        SettingsOption(
          id: 'defaults',
          label: 'Default apps',
          description: 'Choose preferred services',
          icon: Icons.checklist_rounded,
        ),
      ],
      SettingsCategory.accessibility: <SettingsOption>[
        SettingsOption(
          id: 'display-accessibility',
          label: 'Display & text',
          description: 'Contrast, text size, and magnification',
          icon: Icons.text_fields_rounded,
        ),
        SettingsOption(
          id: 'input-accessibility',
          label: 'Input',
          description: 'Controller remapping and virtual keyboard',
          icon: Icons.keyboard_alt_outlined,
        ),
        SettingsOption(
          id: 'audio-accessibility',
          label: 'Audio',
          description: 'Mono audio and captions',
          icon: Icons.hearing_outlined,
        ),
        SettingsOption(
          id: 'narrator',
          label: 'Narrator',
          description: 'Screen reader and spoken feedback',
          icon: Icons.record_voice_over_outlined,
        ),
      ],
    };
