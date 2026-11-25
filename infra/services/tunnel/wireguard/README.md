# WireGuard (road-warrior remote access)

Private remote access to the appliance for the owner's devices. No third party
is involved and nothing is published publicly. This is the recommended option
when a public HTTPS tunnel is not strictly required.

## Files

| File | Installed as | Notes |
| --- | --- | --- |
| `wg0.conf.example` | `/etc/wireguard/wg0.conf` | `0600 root:root`; replace every `__PLACEHOLDER__` |

## Key generation (on the appliance)

```bash
# Server keys.
umask 077
wg genkey | tee /root/wg-server.key | wg pubkey > /root/wg-server.pub
# Client keys - one pair per device, generated on the client when possible.
wg genkey | tee /root/wg-phone.key | wg pubkey > /root/wg-phone.pub
wg genpsk > /root/wg-phone.psk

# Install the example and fill in the generated values.
sudo install -m0600 infra/services/tunnel/wireguard/wg0.conf.example /etc/wireguard/wg0.conf
sudoedit /etc/wireguard/wg0.conf
sudo systemctl enable --now wg-quick@wg0
sudo wg show
```

Never commit `wg0.conf` with real keys. `*.conf` is not git-ignored globally,
so double-check `git status` before committing; only `.example` files belong in
the repo.

## Peer management

Adding a device (no restart of the interface needed):

1. Generate a key pair on the device.
2. Append a `[Peer]` block to `/etc/wireguard/wg0.conf` with
   `AllowedIPs = 10.99.0.<n>/32` and, preferably, a `PresharedKey`.
3. Apply atomically:

   ```bash
   sudo wg syncconf wg0 <(wg-quick strip wg0)
   ```

4. Hand the client-side config to the device through the portal or a password
   manager. Do not email it.

Removing a device: delete or comment its `[Peer]` block, run the same
`wg syncconf`, then rotate the server key only if the device's private key is
suspected compromised.

`AllowedIPs` semantics on the server side are **routing + source ACL**: a
packet from a peer whose source address is outside its `AllowedIPs` is
dropped. On the client side `AllowedIPs` decides what is routed into the
tunnel (`0.0.0.0/0` = full-tunnel, VPN IPs only = split-tunnel).

## Firewall integration

The appliance firewall already treats `wg0` as trusted LAN
(`@lan_ifaces = { "br-lan", "eth0", "wg0" }` in
`infra/firewall/nftables.conf`), which means:

| From wg0 | Allowed |
| --- | --- |
| admin portal `8443/tcp` | yes |
| LAN bridge / DMZ / internet | no (forward policy is drop) |

That default is deliberate: a VPN client is a phone/laptop that may be lost;
it gets the admin portal and nothing else.

### Full LAN access (opt-in)

To let VPN clients reach LAN devices (printers, TVs, ...), add explicit forward
rules and keep the scope tight:

```nft
table inet homeserver {
    chain forward {
        # Allow VPN clients to reach the LAN bridge only, not the DMZ.
        iifname "wg0" oifname "br-lan" ct state new counter accept
        # Replies from the LAN bridge back to the VPN.
        iifname "br-lan" oifname "wg0" ct state established,related accept
    }
}
```

Then add `10.10.20.0/24` (and/or the real LAN subnet) to both the server's
`AllowedIPs` for the peer and the client's `AllowedIPs`. Do not add `br-dmz`:
the DMZ must stay unreachable from the LAN side, including VPN.

## Provider configuration

WireGuard is selected in the portal with `services.tunnel_provider =
wireguard` and `services.enable_tunnel = true`; the daemon then generates
peer blocks and client configs from that state. The example file here is the
manual/reference baseline for a fresh appliance.

## Verification

```bash
sudo wg show                       # peers, handshakes, transfer counters
sudo systemctl status wg-quick@wg0
# From the client, the portal must answer and the LAN must not:
curl -k -fsS https://homeserver.local:8443/healthz
ping -c1 10.10.20.1 || echo "OK: LAN bridge unreachable (expected)"
```

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Handshake never completes | UDP `51820` forwarded on the router / client endpoint reachable; `wg show` counters |
| Handshake OK, portal unreachable | nftables input rule for `wg0` + 8443; `journalctl -k | grep hs-` |
| LAN unreachable | expected: full LAN access is opt-in (see above) |
| DNS fails for queued names | AdGuard listens on the appliance; if DMZ DNS is allowed, use the appliance LAN IP as client DNS |
