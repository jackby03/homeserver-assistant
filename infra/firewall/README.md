# Firewall (nftables)

Baseline firewall for HomeServer Assistant OS. It mirrors the authoritative
renderer in `crates/hs-network/src/firewall.rs` (`render_nftables`) plus the
DMZ egress/NAT extension driven by `FirewallPlan::wan_interface` and
`FirewallPlan::allow_dmz_egress`. The daemon re-renders the live ruleset when
the network configuration changes.

| Item | Value |
| --- | --- |
| Filter table | `inet homeserver` (IPv4 + IPv6) |
| NAT table | `ip homeserver_nat` (DMZ postrouting masquerade only) |
| Runtime ruleset (daemon) | `/var/lib/homecore/firewall/nftables.conf` |
| Boot baseline | `/etc/nftables.d/10-hs-network.nft` |
| Loader | `/etc/nftables.conf` → `include "/etc/nftables.d/*.nft"` |
| Service | `nftables.service` (enabled by the ISO install) |
| Zones | `br-lan` trusted, `br-dmz` `10.20.30.0/24` isolated, `eth0` WAN |
| Admin port | `8443/tcp` (Caddy TLS → `127.0.0.1:8080`) |
| DMZ host allow list | DNS `53/{tcp,udp}`, NTP `123/udp` (only when `network.dmz.allow_dns`) |

## Threat model → rules

| # | Threat | Control |
| --- | --- | --- |
| T1 | DMZ container pivots to LAN hosts | `forward`: `iifname "br-dmz" oifname "br-lan"` → drop (core isolation rule, ordered before egress) |
| T2 | DMZ reaches the admin portal / host services | `input`: DMZ can only reach `53` and `123`; `8443` has an explicit counter drop; everything else is dropped and counted |
| T3 | DMZ bypasses DNS filtering | DMZ→host DNS is the only resolver path; DMZ→LAN is dropped, so `br-lan` services are unreachable |
| T4 | Public exposure of the admin portal | `8443` accepted only on `br-lan` from `lan_subnets` (LAN + optionally WireGuard if added to the set by the daemon) |
| T5 | DMZ apps need updates and outbound tunnels | `forward`: `br-dmz → eth0` accept **after T1**, paired with `ip homeserver_nat` masquerade of `10.20.30.0/24`; no home-router port-forward and no inbound path is created |
| T6 | Silent scanning / lateral movement | `counter` on the DMZ → host deny rules (visible in `nft list table inet homeserver` and the daemon audit trail) |

The DMZ is also isolated at the Incus layer: the `dmz` profile attaches to
`br-dmz` with `security.mac_filtering`, `security.ipv4_filtering` and
`security.port_isolation` (see `infra/incus-preseed.yaml`). The bridge itself
is created by netplan (`infra/network/netplan-90-homeserver.example.yaml`) and
has **no default route** and **no NAT to the LAN**.

### Why DMZ egress is allowed

Home Assistant/Music Assistant updates, the AdGuard filter-list refresh and the
outbound tunnel client (cloudflared) all need internet access from the DMZ.
Egress is **masqueraded on eth0** and only accepts `ct state new` packet flows
after the DMZ→LAN drop, so:

- the DMZ can reach the internet;
- the internet cannot initiate anything towards the DMZ (no port-forward, no
  inbound NAT);
- the DMZ still cannot reach the LAN, the portal or host services.

Set `allow_dmz_egress=false` in the plan (daemon config) to keep the DMZ fully
offline; then both the forward accept and the `homeserver_nat` masquerade rule
disappear.

### Known limits

- Traffic **within** one bridge is switched at L2 and never traverses
  `nftables`; sibling DMZ isolation comes from `security.port_isolation`, not
  from this file.
- The static baseline uses `192.168.1.0/24` as the LAN subnet placeholder (the
  daemon renders the real subnet when known, or interface-only matching when
  it is not). Adjust the `lan_subnets` set before applying this file by hand.
- `br-lan` also carries the WireGuard interface? No: `wg0` is not part of the
  rendered ruleset. Remote-access peers reach the appliance through the LAN
  address once routed by the VPN; if you need portal access over an extra
  interface, add it to the rendered plan (and update the golden tests).

## Quick validation (safe, no state change)

```bash
# Syntax and semantic check; nothing is loaded.
sudo nft -c -f infra/firewall/nftables.conf

# Compare the boot baseline with the live daemon-rendered ruleset.
sudo diff -u /etc/nftables.d/10-hs-network.nft \
            /var/lib/homecore/firewall/nftables.conf || true
```

## Apply

```bash
# 1. Keep a rollback copy of the currently loaded ruleset.
sudo nft list ruleset | sudo tee /var/lib/homecore/firewall/last-good.nft >/dev/null

# 2. Install the baseline at the deployed path.
sudo install -o root -g root -m 0644 \
  infra/firewall/nftables.conf /etc/nftables.d/10-hs-network.nft

# 3. Load everything (the include wrapper loads all nftables.d snippets).
sudo nft -f /etc/nftables.conf

# 4. Confirm the tables are present and enabled at boot.
sudo nft list table inet homeserver
sudo nft list table ip homeserver_nat
systemctl is-enabled nftables.service
```

The daemon never shells out: `hs_network::firewall::apply` renders, re-runs
`validate_isolation`, writes atomically to
`/var/lib/homecore/firewall/nftables.conf` and invokes `nft -f`. A failed
validation or load leaves the previous ruleset active.

## Rollback

```bash
# Restore the snapshot taken before the change.
sudo nft -f /var/lib/homecore/firewall/last-good.nft

# Or reload from the boot baseline.
sudo systemctl restart nftables.service
```

A wrong ruleset can drop the remote SSH session. The appliance still accepts
loopback and LAN DHCP/DNS/ICMP, so a LAN console or the serial console
(`ttyS0`, 115200 8N1) remains usable.

## Verification from a DMZ container

```bash
# Throwaway DMZ workload on the appliance.
incus launch images:alpine/3.20 hs-dmz-test --profile dmz --project homeserver

# --- Expected to FAIL (T2): host admin portal is blocked --------------------
incus exec hs-dmz-test -- nc -z -w 3 10.20.30.1 8443 && echo "UNEXPECTED: admin port reachable" || echo "OK: admin port blocked"
incus exec hs-dmz-test -- nc -z -w 3 10.20.30.1 8080 && echo "UNEXPECTED: API port reachable" || echo "OK: API port blocked"

# --- Expected to FAIL (T1): no route into the LAN ---------------------------
incus exec hs-dmz-test -- ping -c 1 -W 2 192.168.1.1 && echo "UNEXPECTED: LAN reachable" || echo "OK: LAN blocked"

# --- Expected to SUCCEED (allow list): DNS + NTP ----------------------------
incus exec hs-dmz-test -- nslookup example.com 10.20.30.1

# --- Expected to SUCCEED (T5): masqueraded DMZ egress -----------------------
incus exec hs-dmz-test -- wget -q -T 5 -O /dev/null https://example.com && echo "OK: DMZ egress works (masqueraded)"

# --- Counters ----------------------------------------------------------------
incus exec hs-dmz-test -- nc -z -w 1 10.20.30.1 8443 || true
sudo nft list table inet homeserver | grep -A3 'br-dmz'

# Cleanup.
incus delete hs-dmz-test --project homeserver
```

If `nc` is unavailable, use `busybox nc` or
`timeout 3 bash -c '</dev/tcp/10.20.30.1/8443'`.

## File ownership / permissions

| Path | Owner | Mode |
| --- | --- | --- |
| `/etc/nftables.conf` | `root:root` | `0644` |
| `/etc/nftables.d/` | `root:root` | `0755` |
| `/etc/nftables.d/10-hs-network.nft` | `root:root` | `0644` |
| `/var/lib/homecore/firewall/` | `root:root` | `0700` |

`hs-daemon` runs as root and writes only inside its data directory (the
systemd unit additionally allows `/etc/nftables.d` and `/etc/netplan` for
future baseline updates). Everything else is read-only to the unit.
