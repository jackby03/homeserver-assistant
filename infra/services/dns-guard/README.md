# AdGuard Home (DNS guard)

Filtering DNS resolver: DoH/DoT upstreams, blocklists, per-client statistics.
On the appliance it is deployed as an Incus container and rendered by
`hs-network` from `SystemConfig.network.dns`; this directory is the reference
standalone deployment.

## Files

| File | Purpose |
| --- | --- |
| `docker-compose.yml` | standalone container (host networking, pinned image) |
| `conf/AdGuardHome.yaml` | config template: upstreams, blocklists, rewrites, no secrets |

## Ports

| Port | Scope | Purpose |
| --- | --- | --- |
| `53/tcp`, `53/udp` | LAN (and DMZ when `dmz.allow_dns=true`) | DNS service |
| `3000/tcp` | loopback only (`127.0.0.1:3000`) | admin UI |

DNS is **not** exposed to the WAN, and the DMZ can only reach it because of the
explicit nftables allow list (see `infra/firewall/README.md`, threat T3).

## First start

```bash
cd infra/services/dns-guard
sudo mkdir -p work && sudo chown -R 0:0 conf work   # image runs as root
docker compose up -d
docker compose logs -f adguardhome
```

On the appliance the daemon does this for you; the first-boot wizard writes the
admin password (stored hashed by AdGuard), and the UI stays loopback-only.

## Point the LAN at it

1. Router DHCP: advertise the appliance LAN IP as DNS server (recommended), or
2. Per-device DNS for a pilot group.

Then verify filtering from a LAN client:

```bash
dig +short doubleclick.net @<appliance-lan-ip>   # expect 0.0.0.0 / blocked
dig +short example.com @<appliance-lan-ip>       # expect a public address
```

## How the daemon rewrites this file

`hs_network` maps the config to this YAML:

| `SystemConfig` field | Rendered key |
| --- | --- |
| `network.dns.provider` + `protocol` | `dns.upstream_dns` (exactly one list) |
| `network.dns.custom_upstream` | `dns.upstream_dns` when `provider=custom` |
| `network.dns.blocklists` | `filtering.filters[].url` (ids assigned in order) |
| `device.hostname`, `services.base_domain`, app subdomains | `dns.rewrites` |
| `network.dns.enabled` | app deployed / DNS rule removed from nftables |

Contract: the daemon **owns** `conf/AdGuardHome.yaml` on the appliance and
writes it atomically before restarting the container. Manual edits are lost on
the next configuration change - use the portal or `config.json` instead.

## Notes and caveats

- **DoT**: if `network.dns.protocol = dot`, the rendered upstream uses
  `tls://<host>` and AdGuard validates the certificate against the system
  trust store (which contains the Caddy local CA after first boot).
- **DMZ clients**: they may only talk to port 53 on the DMZ gateway. They are
  rate-limited by both AdGuard and nftables (`dmz.allow_dns=false` removes the
  allow rule entirely).
- **Encrypted upstream failure**: keep `bootstrap_dns` reachable, otherwise
  DoH hostnames cannot resolve. The daemon keeps plain bootstrap DNS as the
  only cleartext path.
- **Backups**: the app state is `work/` + `conf/`; the appliance backup job
  snapshots the whole app volume (see `docs/DEPLOYMENT.md`).
