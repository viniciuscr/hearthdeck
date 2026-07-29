# Hearthdeck

A responsive Flutter TV launcher for macOS and Linux.

## Local Services

The Linux host backend lives under `services/`. `hearthdeck-daemon` exposes the
paired-client API and SQLite state; `hearthdeck-bridge` performs local desktop-entry
scans and allowlisted launches over a user-only Unix socket. See
`services/README.md` and `contracts/openapi.yaml`.

The Flutter catalog uses a repository boundary. It defaults to mock content on
macOS and uses the live API only when both values are provided:

```sh
just app-live https://hearthdeck.local:38400 <paired-token> macos
```

Backend discovery providers are documented in `docs/backend-architecture.md`.
Logging and operational investigation are documented in `docs/observability.md`.
Metadata source and local-catalog policy are documented in
`docs/metadata-enrichment.md`.

The daemon queues an initial scan at startup. Full Library and Settings can
request later rescans; live API clients reload their catalog after the daemon
emits a source-aware `library_changed` WebSocket event.

## Run

```sh
mise exec -- flutter run
```

## Tasks

Use `just` for common development and deployment commands:

```sh
just list
just setup
just check
just app
just dev
```

`just dev` creates an isolated local pairing and starts Flutter in live catalog
mode. It is the normal way to verify discovered applications end-to-end.

On the Linux host, build and install the local services with:

```sh
just build-services
just install-services
```

## CachyOS and Arch Linux

GitHub Actions produces an `x86_64` pacman package containing the Flutter
client, the local bridge and daemon, a desktop entry, and systemd user units.
Download `hearthdeck-*.pkg.tar.zst` from the workflow run, then install it:

```sh
sudo pacman -U hearthdeck-*.pkg.tar.zst
systemctl --user daemon-reload
systemctl --user disable --now hearthdeck-bridge.service hearthdeck-daemon.service
systemctl --user enable --now hearthdeck.target
```

The target owns the API daemon and bridge socket; the bridge process starts on
demand for host requests. The package's client pairs with its loopback daemon
automatically. See
`docs/arch-package.md` for package contents and troubleshooting.

## Console Session

The Arch package depends on the distribution `gamescope` package and installs a
**Hearthdeck Console** Wayland session. Select it from a display manager or set
it as the autologin session to boot directly into Hearthdeck without KDE or
another desktop shell. Gamescope is not bundled because its GPU/DRM components
must match the host graphics stack.

Hearthdeck launches approved Linux desktop entries in transient systemd user
scopes, so the host can identify and stop the active managed application. Remote
clients can inspect host capabilities and request an installation for host-side
approval, but cannot execute a package manager or arbitrary host command.

## Controller support

The dashboard uses `flutter_gamepads`, backed by Flame Engine's cross-platform
`gamepads` plugin. It supports native macOS Game Controller input and Linux
controllers normalized with SDL's controller database.

| Control | Dashboard action |
| --- | --- |
| D-pad / left stick | Move focus |
| A | Activate focused control |
| B / Back | Go back or dismiss transient UI |
| Right stick | Scroll the current shelf or page |

Keyboard arrows, Enter, and Space remain supported as equivalent input.

## Search

Dashboard and library search use the same search route. It autofocuses a
standard Flutter `TextField`, so hardware keyboards and the platform IME, such
as CachyOS KDE's virtual keyboard, provide text input. Search results update
from the shared content data as the query changes.

## Content details

Selecting a dashboard item opens a maintained detail route with a shared-element
transition. `DashboardItem` supports a `ContentDetails` payload for custom
actions, facts, optional progress, and gallery content. Items without one use
the default layout for their `TvContentKind` (`game`, `media`, `application`,
or `system`), keeping future content sources independent from the UI layout.

## Shared Screens

Library and Settings share `TvTwoPaneLayout`, `TvNavigationRail`, and
`TvOptionCard` from `lib/tv_two_pane.dart`. The shell derives compact rail
geometry from its constraints, keeps all rail controls controller-focusable,
and is intended for future two-pane TV surfaces.

## Getting Started

This project is a starting point for a Flutter application.

A few resources to get you started if this is your first Flutter project:

- [Learn Flutter](https://docs.flutter.dev/get-started/learn-flutter)
- [Write your first Flutter app](https://docs.flutter.dev/get-started/codelab)
- [Flutter learning resources](https://docs.flutter.dev/reference/learning-resources)

For help getting started with Flutter development, view the
[online documentation](https://docs.flutter.dev/), which offers tutorials,
samples, guidance on mobile development, and a full API reference.
