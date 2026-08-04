# Docker test sandbox (Linux, visual)

A disposable Linux container to build and visually test Hearthdeck's Flutter
UI plus the real `hearthdeck-daemon`/`hearthdeck-bridge` services, without
real hardware, DRM, or Gamescope. It runs the app under Xvfb and exposes that
virtual display over VNC-in-a-browser (noVNC), so you can watch and interact
with it from the host.

This is a **dev/test sandbox**, not a substitute for the real Arch Linux
packaging build in `.github/workflows/linux-build.yml`. It mirrors that job's
toolchain (Arch `base-devel`, GTK3, Flutter 3.44.8, Rust 1.97.1) so a failure
here is a strong signal CI would fail too, but it skips
packaging/systemd/Gamescope and just runs the daemon/bridge/UI as plain
foreground processes (similar to what `scripts/dev` does for local
development on other platforms).

## Requirements

- Docker Desktop, running.
- On Apple Silicon: Flutter does not ship a Linux-desktop SDK for arm64, so
  this image always runs as `linux/amd64` under emulation. In Docker
  Desktop's settings, enabling **"Use Rosetta for x86_64/amd64 emulation"**
  (Settings > General) is supposed to make this dramatically faster/more
  reliable than plain QEMU emulation -- see the known limitation below if it
  doesn't seem to be taking effect.
- This network's corporate proxy (Zscaler) blocks Docker Hub/ghcr.io and
  direct PyPI downloads outright, and TLS-intercepts everything else. If you
  hit certificate errors or 403s building the image, see `docker/certs/` and
  `docker/pip.conf.example`.

## Known limitation: QEMU segfaults on Apple Silicon

On at least one Apple Silicon Mac (this repo's), the container reliably
builds and runs right up until Flutter's engine tries to render its first
frame, then crashes with `qemu: uncaught target signal 11 (Segmentation
fault)`. The Rust toolchain hit the exact same `qemu: uncaught target signal
11` crash earlier in the build (fixed by retrying/reordering steps -- see
`Dockerfile` comments -- but the underlying cause is the same).

Both crash sites (`rustc`, and Mesa's `llvmpipe` software OpenGL renderer)
are LLVM-JIT-heavy programs; this looks like a QEMU TCG binary-translation
bug affecting that class of workload specifically, not anything wrong with
Hearthdeck. Enabling Docker Desktop's "Use Rosetta for x86_64/amd64
emulation" setting (which replaces QEMU with Apple's Rosetta 2 translator)
*should* fix this, but toggling it by editing
`~/Library/Group Containers/group.com.docker/settings-store.json` directly
and fully restarting Docker Desktop did not change the behavior -- the
crashes were identical before and after. It's possible this needs to be
toggled through Docker Desktop's actual Settings UI (Settings > General >
"Use Rosetta for x86_64/amd64 emulation" > **Apply & Restart**) rather than a
raw settings-file edit, since the GUI flow may perform additional
provisioning steps.

If you hit this:

1. Try the GUI toggle above and retry `docker compose up --build`.
2. Otherwise, this sandbox should work without any of this on a real
   x86_64 machine (Intel Mac, x86_64 Linux box, or a cloud VM) -- there's no
   emulation involved there at all.
3. The real CI (`.github/workflows/linux-build.yml`) already runs on x86_64
   GitHub-hosted runners and proves the project builds cleanly; this sandbox
   is only for *visually* exercising the UI, which needs the rendering step
   that's currently blocked here.

## Usage

```sh
cd docker
docker compose up --build
```

First run downloads/builds everything (Flutter engine artifacts, Rust crates,
the daemon/bridge, the Flutter app) and can take several minutes, especially
under emulation. Build caches are kept in named Docker volumes, so repeat
runs are much faster.

Once you see `[docker] launching the Flutter app`, open:

```
http://localhost:6080/vnc.html
```

and click **Connect** (no password) to see and interact with the running
app -- mouse and keyboard both work through noVNC.

Stop it with `Ctrl+C`, or `docker compose down` from another terminal.

## What's actually running

- `hearthdeck-bridge` and `hearthdeck-daemon`: built in debug mode and run as
  plain foreground processes with an isolated `XDG_RUNTIME_DIR`/database, no
  systemd involved.
- A one-time local pairing is performed automatically against the daemon.
- The Flutter app (`flutter run -d linux`) is launched paired to that daemon,
  so you're seeing the same live-catalog code path used on real hardware --
  just with whatever (if anything) the container's minimal filesystem has to
  discover. Expect most library sources to be empty; that's expected, not a
  bug, since there's no real desktop environment/games installed in the
  container. This is primarily for checking layout, navigation, and pages
  like Settings/System health, not real catalog contents.
- Gamescope/DRM/systemd/RetroArch are intentionally out of scope here; see
  `docs/kiosk-session.md` for how the real Kiosk session works on hardware.

## Other modes

Launch just the Flutter UI against built-in mock data, skipping the
daemon/bridge build entirely (faster, and useful for isolating whether an
issue is in the UI or the backend services):

```sh
HEARTHDECK_DOCKER_MODE=ui-only docker compose up --build
```

Run an interactive shell instead of auto-launching everything (Xvfb/openbox/
x11vnc/noVNC still start, so you can drive things manually, e.g. run
`flutter test`, `cargo test --manifest-path services/Cargo.toml --workspace`,
or `flutter run -d linux` yourself):

```sh
HEARTHDECK_DOCKER_MODE=shell docker compose up --build
```

then, from another terminal:

```sh
docker compose exec hearthdeck bash
```

## Cleaning up

```sh
docker compose down -v   # also removes the cached build volumes
```
