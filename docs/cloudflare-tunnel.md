# Cloudflare Tunnel for Immich (Podman Quadlet)

The Immich instance documented in [`immich-backup.md`](immich-backup.md) is
reachable from the internet at `https://photos.viniciuscr.com.br` through a
Cloudflare Tunnel. Nothing is port-forwarded on the router and no inbound port
is opened: `cloudflared` dials out to Cloudflare, and Cloudflare forwards
requests back down that connection.

Checked against cloudflared **2026.10.5** and Podman **6.1.4** on 2026-10-27.

## Decision

- **Runtime:** rootless Podman under the desktop user `vinicius`, declared as a
  Quadlet container in `~/.config/containers/systemd/`, started at boot with
  `loginctl enable-linger vinicius`.
- **Networking:** `Network=host`. The tunnel ingress is
  `photos.viniciuscr.com.br → http://localhost:2283`, so cloudflared must see
  the host's loopback where Immich publishes its port.
- **Secret:** the tunnel token lives in `~/.config/cloudflared/tunnel.env`
  (mode `0600`), not in the unit file and not in this repository.
- **Image:** `docker.io/cloudflare/cloudflared:latest`, pulled explicitly. No
  `podman auto-update`; a tunnel is small and an unattended change is not worth
  the surprise.
- **Scope:** this is machine infrastructure for the Immich deployment, not part
  of the Hearthdeck runtime. It does not touch the daemon, bridge, or protocol.

Why not join the Immich pod: that pod runs in the rootless Podman namespace of
the dedicated `immich` user. Rootless namespaces are per-user, so `vinicius`
cannot add a container to it. Immich publishes `2283` on the host, which is
exactly what the ingress expects, so host networking is the smaller change.

Why not a Compose file: the rest of the Immich stack is already Quadlet. One
container does not justify a second deployment mechanism.

## This machine

| Resource | Value |
| --- | --- |
| Public hostname | `photos.viniciuscr.com.br` |
| Tunnel ID | `8cd11421-7df0-4698-944e-06319d264632` |
| Local upstream | `http://localhost:2283` (Immich server) |
| Container | `cloudflared-immich` |
| Unit | `cloudflared-immich.service` (generated from Quadlet) |
| Metrics | `127.0.0.1:20241`, loopback only |
| Autostart | user lingering + `WantedBy=default.target` |

The ingress rule and the public hostname are configured in the Cloudflare
dashboard, not on disk. The token only identifies the tunnel; it does not carry
the ingress rules.

## 1. Create the tunnel and its token

In the Cloudflare dashboard: **Networks → Tunnels → Create a tunnel**, choose
`cloudflared`, name it, then add a public hostname:

- **Subdomain / domain:** `photos.viniciuscr.com.br`
- **Service type:** HTTP
- **URL:** `localhost:2283`

Copy the connector token. It is shown once; if it is lost, rotate it from the
same page. The token is a credential: anyone holding it can run a connector for
this tunnel.

## 2. Store the token

```sh
mkdir -p ~/.config/cloudflared
chmod 700 ~/.config/cloudflared
printf 'TUNNEL_TOKEN=%s\n' '<token>' > ~/.config/cloudflared/tunnel.env
chmod 600 ~/.config/cloudflared/tunnel.env
```

An environment file keeps the token out of the unit and out of `ps` output from
other users. It is still visible to `podman inspect` and to root; it is not a
vault.

## 3. Describe the container

`~/.config/containers/systemd/cloudflared-immich.container`:

```ini
[Unit]
Description=Cloudflare Tunnel for Immich
Wants=network-online.target
After=network-online.target

[Container]
ContainerName=cloudflared-immich
Image=docker.io/cloudflare/cloudflared:latest
Network=host
EnvironmentFile=%h/.config/cloudflared/tunnel.env
Exec=tunnel --no-autoupdate --metrics 127.0.0.1:20241 run
NoNewPrivileges=true

[Service]
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
```

Notes on the non-obvious lines:

- `Network=host` is what makes `localhost:2283` mean the host's Immich port.
  `host.containers.internal` would also work for the published port, but host
  networking matches the ingress URL and avoids a second address to explain.
- `--metrics 127.0.0.1:20241` is deliberate. The default binds `[::]:20241`,
  which with host networking would expose the metrics endpoint on the LAN.
- `NoNewPrivileges=true` is the Quadlet spelling of
  `--security-opt no-new-privileges`. `SecurityOpt=` is not a Quadlet key.
- `TUNNEL_TOKEN` is read from the environment; the `run` subcommand needs no
  `--token` argument.

## 4. Start it and check

```sh
systemctl --user daemon-reload
systemctl --user start cloudflared-immich
systemctl --user status cloudflared-immich
journalctl --user -u cloudflared-immich -n 30 --no-pager
```

A healthy start logs four `Registered tunnel connection` lines (one per
Cloudflare edge), `Updated to new configuration` with the ingress JSON, and a
passing connectivity pre-check. The ICMP proxy warning about
`ping_group_range` is expected and harmless: this tunnel only proxies HTTP, so
it never uses ICMP.

Verify from the public side, not just locally:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' https://photos.viniciuscr.com.br/
curl -sS https://photos.viniciuscr.com.br/api/server/ping
```

Both should return `200`. Immich answering on the LAN but the hostname failing
points at Cloudflare-side DNS or ingress, not at the container.

## 5. Start at boot

Two independent things must hold, and they are easy to confuse:

1. The user manager must exist without a login: `loginctl enable-linger vinicius`.
2. The generated unit must be pulled in by the user's default target:
   `[Install] WantedBy=default.target`.

Check both:

```sh
loginctl show-user vinicius -p Linger        # Linger=yes
systemctl --user list-dependencies default.target | grep cloudflared
```

This is the same arrangement as the Immich stack, which runs as the `immich`
user with lingering enabled. Without lingering, the tunnel would only start
when `vinicius` logs in — on this machine that usually happens, but it is not
the same guarantee.

A reboot starts cloudflared and Immich in parallel, from different user
managers. Cloudflared may connect before Immich is listening; requests get a
502 until Immich is up, then succeed without restarting the tunnel.

## 6. Operations

```sh
# State and logs
systemctl --user status cloudflared-immich
podman logs -f cloudflared-immich
journalctl --user -u cloudflared-immich -f

# Apply a unit-file change
systemctl --user daemon-reload
systemctl --user restart cloudflared-immich

# Update the connector image
podman pull docker.io/cloudflare/cloudflared:latest
systemctl --user restart cloudflared-immich

# Rotate the token: replace TUNNEL_TOKEN in the env file, then
systemctl --user restart cloudflared-immich
```

After a Podman upgrade, run `systemctl --user daemon-reload` and check the unit
before assuming the tunnel survived. Quadlet behaviour is tied to the Podman
version.

## 7. Traps

- **Do not commit the token.** It belongs in
  `~/.config/cloudflared/tunnel.env`, mode `0600`. If it appears in a chat,
  transcript, or shell history, rotate it from the Cloudflare dashboard and
  restart the service.
- **Do not switch to bridge networking without changing the ingress.** With
  the default bridge, `localhost:2283` inside the container is the container,
  not the host; the tunnel would answer 502.
- **Do not expose port 20241.** Metrics are bound to loopback on purpose. They
  are unauthenticated.
- **Do not open 2283 on the router.** The tunnel is outbound-only. The existing
  LAN-only ufw rule for `2283` is still all that is needed.
- **A 502 is usually Immich, not the tunnel.** Check
  `systemctl --user status immich-server` (as the `immich` user) before
  restarting cloudflared.
- **Cloudflare-side changes are invisible on disk.** If the hostname or service
  changes, edit it in the dashboard; the local configuration picks it up at the
  next connection refresh. A stale ingress URL looks like a local failure.
- **Protect the public hostname.** Immich still requires its own login, but
  Cloudflare Access in front of `photos.viniciuscr.com.br` is the cheap way to
  keep the login page off the open internet. The tunnel does not add
  authentication by itself.
