# Hearthdeck

A local-first, controller-first TV library and launcher for macOS and Linux.

## Product Foundations

Start with [Product Foundations](docs/product-foundations.md). It defines the
non-negotiable controller navigation contract, catalog and metadata model,
service boundaries, and low-resource rules that every feature must follow.

The current product scope is live desktop and installed Heroic game discovery
on Linux, AppStream application enrichment, and a separate live RomM browser.
Full Library uses the live catalog; dashboard and search are still fixture
surfaces. See [backend architecture](docs/backend-architecture.md) and
[metadata enrichment](docs/metadata-enrichment.md) for implementation detail.

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
Install the initial `hearthdeck-*.pkg.tar.zst` from a workflow run; it configures
the Hearthdeck repository so future updates arrive through `pacman -Syu`:

```sh
sudo pacman -U hearthdeck-*.pkg.tar.zst
```

The package starts local services automatically at the next user login and when
Hearthdeck launches. To start them immediately without opening the client, run
`systemctl --user daemon-reload && systemctl --user start hearthdeck.target`.

The target owns the API daemon and bridge socket; the bridge process starts on
demand for host requests. The package's client pairs with its loopback daemon
automatically. See
`docs/arch-package.md` for package contents and troubleshooting.

## Kiosk Session

See `docs/kiosk-session.md` for the full session architecture, an incident
writeup of exactly how this regressed once already, and a "do not" list
before changing anything about how it starts.

The Arch package installs a **Hearthdeck Kiosk** Wayland session with no
desktop shell. Its session script runs Gamescope directly on the DRM/KMS seat
with Hearthdeck as its only child, for the lowest possible memory and CPU
footprint. There is no panel, launcher, wallpaper, or other desktop component
running underneath; Gamescope is the entire compositor for the session.

Hearthdeck launches a separate, on-demand nested Gamescope instance only when
it starts a managed desktop application or game. That instance is unrelated to
the outer Kiosk session compositor: it uses no DRM or memory until a launch is
requested and is torn down when the launch ends.

Hearthdeck launches approved Linux desktop entries in transient systemd user
services, so the host can identify and stop the active managed application. Remote
clients can inspect host capabilities and request an installation for host-side
approval, but cannot execute a package manager or arbitrary host command.

Controller input continues through Hearthdeck's direct Linux gamepad reader.
Audio stays on the existing PipeWire/WirePlumber services; Wi-Fi and Bluetooth
stay on NetworkManager and BlueZ system services. Those services remain active,
but their configuration UIs are not yet implemented in Hearthdeck.

Choose **Settings > General > Exit to desktop** to leave Hearthdeck Kiosk and
return to the display manager's login screen.

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

## RomM

Hearthdeck can list consoles from a local RomM instance in the **Retro**
section. Open **Settings > System > Retro & RomM** and enter the local server
URL plus a RomM client token with the `platforms.read` permission. The token is
stored in the Hearthdeck daemon and is never returned through the paired API.
Selecting a game and pressing **Play** launches it through RetroArch. The
design and phased roadmap for this integration is tracked in
`docs/retroarch-integration.md`.

The Arch package installs an optional `romm.service` that starts an existing
Podman Compose deployment alongside the Hearthdeck session. Copy
`/usr/share/doc/hearthdeck/romm.env.example` to
`~/.config/hearthdeck/romm.env`; it defaults to
`/mnt/external/romM/podman-compose.yaml`. Without that config file the unit is
skipped. Its status appears in Settings' service status view next to the daemon
and bridge.

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
