# Hearthdeck

A local-first, controller-first TV library and launcher for Linux.

## Product Foundations

Start with [Product Foundations](docs/product-foundations.md). It defines the
non-negotiable controller navigation contract, catalog and metadata model,
service boundaries, and low-resource rules that every feature must follow.

The current product scope is live desktop and installed Heroic game discovery
on Linux, AppStream application enrichment, and a separate live RomM browser.
See [backend architecture](docs/backend-architecture.md) and
[metadata enrichment](docs/metadata-enrichment.md) for implementation detail.

## Frontend

The TV UI is a Rust/iced desktop application, `hearthdeck-frontend` under
`services/`, built on libcosmic and rendered as a COSMIC layer-shell surface. It
talks to the daemon over the paired HTTP/WebSocket API, configured through the
`HEARTHDECK_BACKEND_URL` and `HEARTHDECK_PAIRING_TOKEN` environment variables.

```sh
just run-frontend         # release build + run
just run-frontend-debug   # debug build + run
```

`just dev` starts the bridge and daemon with an isolated local pairing and then
launches the frontend against them. It is the normal way to verify discovered
applications end-to-end.

## Local Services

The Linux host backend lives under `services/`. `hearthdeck-daemon` exposes the
paired-client API and SQLite state; `hearthdeck-bridge` performs local desktop-entry
scans and allowlisted launches over a user-only Unix socket. See
`services/README.md` and `contracts/openapi.yaml`.

Backend discovery providers are documented in `docs/backend-architecture.md`.
Logging and operational investigation are documented in `docs/observability.md`.
Metadata source and local-catalog policy are documented in
`docs/metadata-enrichment.md`.

The daemon queues an initial scan at startup. The frontend and Settings can
request later rescans; live API clients reload their catalog after the daemon
emits a source-aware `library_changed` WebSocket event.

## Tasks

Use `just` for common development and deployment commands:

```sh
just list
just setup
just check
just dev
```

On the Linux host, build and install the local services with:

```sh
just build-services
just install-services
```

## CachyOS and Arch Linux

GitHub Actions produces an `x86_64` pacman package containing the frontend, the
local bridge and daemon, a desktop entry, and systemd user units.
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

The frontend reads controllers directly through `gilrs`, which normalizes Linux
controllers with SDL's controller database.

| Control | Dashboard action |
| --- | --- |
| D-pad / left stick | Move focus |
| A | Activate focused control |
| B / Back | Go back or dismiss transient UI |
| Right stick | Scroll the current shelf or page |

Keyboard arrows, Enter, and Space remain supported as equivalent input.

## Search

The library search filters the current catalog as the query changes, using the
same controller-focusable text input as the rest of the frontend.

## RomM

Hearthdeck can list consoles from a local RomM instance in the **Retro**
section. Open **Settings > System > Retro & RomM** and enter the local server
URL plus a RomM client token with the `platforms.read` permission. The token is
stored in the Hearthdeck daemon and is never returned through the paired API.
Selecting a game and pressing **Play** launches it through RetroArch. The
design and phased roadmap for this integration is tracked in
`docs/retroarch-integration.md`.

The Arch package installs an optional `romm.service` that starts an existing
Podman Compose deployment alongside the Hearthdeck session. It uses
`/mnt/external/romM/podman-compose.yaml` by default and skips cleanly when that
file is absent. `~/.config/hearthdeck/romm.env` can override the path. Its
status appears in Settings' service status view next to the daemon and bridge.
Combined session output is also available at `~/hearthdeck.log`.
