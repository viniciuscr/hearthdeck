# Disposable Linux sandbox for visually testing Hearthdeck's Flutter UI plus
# the real hearthdeck-daemon/hearthdeck-bridge services, without needing real
# hardware, DRM, or Gamescope. It exposes a VNC session over HTTP (noVNC) so
# the app's window can be viewed/driven from a browser on the host.
#
# This is a dev/test sandbox, not a substitute for the real Arch Linux
# packaging build in .github/workflows/linux-build.yml. It intentionally
# mirrors that job's toolchain (Arch base-devel, GTK3, Flutter 3.44.8, Rust
# 1.97.1) so a failure here is a strong signal CI would fail too, but it
# skips packaging/systemd/gamescope and just runs the daemon/bridge/UI as
# plain foreground processes.
#
# NOTE: Flutter does not ship a Linux-desktop SDK for arm64, so this image
# must run as linux/amd64 even on Apple Silicon hosts (see docker-compose.yml
# `platform:`). Docker Desktop emulates this via QEMU/Rosetta; enable "Use
# Rosetta for x86_64/amd64 emulation" in Docker Desktop's settings for
# tolerable build times.
#
# ARG so this can be overridden (e.g. `--build-arg BASE_IMAGE=archlinux:base-devel`)
# on networks that can reach Docker Hub directly. Defaults to the internal
# Artifactory mirror because plain `archlinux:base-devel`/`ghcr.io/...`
# pulls are blocked by this org's Zscaler policy ("Docker and mirrors"
# category denied) -- discovered while building this image, see docker/README.md.
ARG BASE_IMAGE=nn-docker-remote.artifactory.insim.biz/library/archlinux:base-devel
FROM --platform=linux/amd64 ${BASE_IMAGE}

ARG FLUTTER_VERSION=3.44.8
ARG RUST_VERSION=1.97.1

WORKDIR /

# Trust any corporate MITM-proxy CA(s) dropped in docker/certs/ (see
# docker/certs/README.md) before anything else touches the network, so
# pacman/curl/git/pip don't fail with certificate-verification errors on
# networks that intercept outbound HTTPS.
COPY docker/certs/ /usr/local/share/ca-certificates-extra/
RUN find /usr/local/share/ca-certificates-extra -maxdepth 1 -name '*.pem' \
    -exec cp {} /etc/ca-certificates/trust-source/anchors/ \; \
  && update-ca-trust

# Same idea for pip: only copied in if docker/pip.conf exists (see
# docker/pip.conf.example) -- needed on networks that block direct PyPI
# downloads (files.pythonhosted.org) in favor of an internal mirror.
COPY docker/pip.con[f] /etc/

# Build toolchain, matching .github/workflows/linux-build.yml's package job,
# plus a virtual display stack (Xvfb + openbox + x11vnc) and noVNC/websockify
# to view that display from a browser.
#
# `DisableSandboxSyscalls`: pacman's per-download seccomp sandbox (the
# 'alpm' user) fails under QEMU/Rosetta's amd64-on-arm64 emulation ("error
# restricting syscalls via seccomp"), so it has to be turned off to run this
# image on Apple Silicon at all. Harmless here since this is a disposable
# build-time container, not a persistent multi-tenant host.
# `ParallelDownloads = 1`: concurrent HTTPS downloads appear to starve each
# other badly under this same emulation, occasionally tripping pacman's
# "operation too slow" abort even though a single sequential download is
# fast; downloading one file at a time makes that much less likely.
RUN sed -i \
    -e 's/^#DisableSandboxSyscalls/DisableSandboxSyscalls/' \
    -e 's/^ParallelDownloads.*/ParallelDownloads = 1/' \
    /etc/pacman.conf \
  && pacman-key --init \
  && pacman-key --populate archlinux

# Split from the pacman-key step above so a flaky download here (see the
# ParallelDownloads note) only needs to retry package sync/install, not
# redo key generation. `retry` wraps the two genuinely flaky steps (pacman's
# own sync/install, and the individually-slow network calls below); each of
# those still isn't 100% reliable even sequentially under emulation, so a
# handful of attempts with a short backoff smooths that over.
RUN retry() { \
      for _ in 1 2 3 4 5; do "$@" && return 0; sleep 5; done; \
      return 1; \
    }; \
    retry pacman --sync --refresh --sysupgrade --noconfirm \
  && retry pacman --sync --noconfirm \
    clang cmake curl git gtk3 jq just ninja pkgconf sqlite unzip xz zsh \
    rustup \
    xorg-server-xvfb openbox x11vnc \
    python python-pip \
    noto-fonts ttf-dejavu \
    dbus \
  && retry pip install --break-system-packages --no-cache-dir websockify \
  && retry git clone --depth 1 --branch v1.5.0 https://github.com/novnc/noVNC.git /opt/noVNC \
  && retry rustup toolchain install "$RUST_VERSION" \
  && rustup default "$RUST_VERSION" \
  && retry curl -fsSL "https://storage.googleapis.com/flutter_infra_release/releases/stable/linux/flutter_linux_${FLUTTER_VERSION}-stable.tar.xz" -o /tmp/flutter.tar.xz \
  && tar -xJf /tmp/flutter.tar.xz -C /opt \
  && rm /tmp/flutter.tar.xz \
  && git config --system --add safe.directory /opt/flutter \
  && pacman -Scc --noconfirm

ENV PATH="/opt/flutter/bin:/root/.cargo/bin:${PATH}"
# Xvfb has no real GPU/DRI behind it, so Flutter's engine must fall back to
# Mesa's software rasterizer instead of trying (and failing) to use GL.
ENV LIBGL_ALWAYS_SOFTWARE=1
ENV DISPLAY=:99
# Tells linux/runner/my_application.cc to skip gtk_window_fullscreen()'s
# async WM round-trip, which never completes under this sandbox's minimal
# non-compositing window manager and otherwise leaves the app waiting
# forever for a first frame that never renders. See that file for details.
ENV HEARTHDECK_DOCKER_SANDBOX=1

# Downloads engine artifacts only (no project source needed yet), so this
# layer stays cached across rebuilds even when app code changes.
RUN flutter config --enable-linux-desktop --no-analytics \
  && flutter precache --linux

WORKDIR /workspace

COPY docker/entrypoint.sh /usr/local/bin/hearthdeck-docker-entrypoint
RUN chmod +x /usr/local/bin/hearthdeck-docker-entrypoint

EXPOSE 6080
ENTRYPOINT ["/usr/local/bin/hearthdeck-docker-entrypoint"]
