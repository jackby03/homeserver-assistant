# Services (reference templates)

Templates for the optional homeserver workloads the appliance provisions:
AdGuard Home (DNS filter), Home Assistant, Music Assistant, the Caddy reverse
proxy and the outbound tunnels (Cloudflare Tunnel / WireGuard).

## Two deployment paths - read this first

| Path | Who runs the service | Where the real config lives |
| --- | --- | --- |
| **Appliance (supported)** | `hs-daemon` → Incus containers in the `homeserver` project (`hs-apps` orchestrator) | `/var/lib/homecore/apps/<app>/...`, rendered by the daemon from `SystemConfig` |
| **Standalone (reference)** | Docker Compose or the host package manager on a normal Linux host | `docker-compose.yml` + `conf/` files in this directory |

The ISO intentionally **does not install Docker**: on the appliance every app
runs as an Incus container or as a native systemd service. The compose files
in `infra/services/` exist so the same service definitions can be:

1. reviewed and versioned as infrastructure-as-code;
2. run on a developer machine or a non-Incus host;
3. used as the reference for the container image, ports, volumes and the
   configuration file the daemon renders.

Where the two paths differ, the compose file says so in a comment near the
relevant key.

## Directory map

| Path | Purpose |
| --- | --- |
| `dns-guard/` | AdGuard Home container + `AdGuardHome.yaml` template with DoH/DoT upstreams and blocklists |
| `home-assistant/` | Home Assistant container, adoption/onboarding notes |
| `music-assistant/` | Music Assistant container, audio limitations |
| `caddy/` | Caddyfile with internal TLS, daemon snippet contract, `apps.d/` |
| `tunnel/cloudflared/` | outbound-only tunnel example + unit (no inbound ports) |
| `tunnel/wireguard/` | WireGuard road-warrior example + peer management notes |

## Rules for these files

- **No secrets, ever.** Tokens, passwords, private keys and tunnel credentials
  appear only as `__PLACEHOLDER__` values or commented examples.
- Configuration files are **templates**: the daemon overwrites the rendered
  copy on the appliance. Editing the deployed file is not persistent.
- Pin image versions and bump them deliberately; `latest`/`stable` tags are
  called out with a comment where they are used for upstream reasons.
- Compose files are validated with `docker compose config` on a Linux host
  (`docs/ISO.md` lists the validation commands). They cannot be validated on
  the Windows development machine - see the final report of the infra task.

## File ownership on the appliance

| Path | Owner | Mode |
| --- | --- | --- |
| `/var/lib/homecore/apps/<app>/` | `root:root` | `0700` |
| `/etc/caddy/apps.d/` | `root:root` | `0755` |
| `/etc/cloudflared/` | `root:root` | `0700` (credentials) |
| `/etc/wireguard/` | `root:root` | `0700` (private keys) |
