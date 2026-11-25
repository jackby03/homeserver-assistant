# Cloudflare Tunnel (outbound publishing)

Optional way to reach the appliance from outside the home **without opening
any inbound port**. `cloudflared` keeps an outbound connection to Cloudflare
and Cloudflare forwards the configured hostnames to it.

## Files

| File | Installed as | Notes |
| --- | --- | --- |
| `config.yml.example` | `/etc/cloudflared/config.yml` | `0640 root:root`, contains the tunnel UUID |
| `cloudflared.service.example` | `/etc/systemd/system/cloudflared.service` | example unit for config-file mode |
| *(created on the appliance)* | `/etc/cloudflared/<uuid>.json` | credentials, `0600 root:root`, **never committed** |

## Setup (on the appliance, once the portal works locally)

```bash
# 1. Install cloudflared from Cloudflare's apt repository.
#    (The ISO does not ship it: it is optional and adds an external update
#    channel. `docs/DEPLOYMENT.md` has the exact commands.)
sudo apt install cloudflared

# 2. Authenticate the browser once and create a named tunnel.
cloudflared tunnel login
cloudflared tunnel create homeserver
#    -> prints the tunnel UUID and writes /root/.cloudflared/<uuid>.json

# 3. Move the credentials to the service location.
sudo install -d -m0700 /etc/cloudflared
sudo install -m0600 /root/.cloudflared/<uuid>.json /etc/cloudflared/<uuid>.json

# 4. Render and install the config/unit (replace placeholders first).
sudo install -m0640 infra/services/tunnel/cloudflared/config.yml.example /etc/cloudflared/config.yml
sudo install -m0644 infra/services/tunnel/cloudflared/cloudflared.service.example /etc/systemd/system/cloudflared.service
sudoedit /etc/cloudflared/config.yml

# 5. Create the DNS records and start the tunnel.
cloudflared tunnel route dns homeserver home.example.com
sudo systemctl daemon-reload && sudo systemctl enable --now cloudflared
systemctl status cloudflared --no-pager
journalctl -u cloudflared -b --no-pager
```

## Ingress rules

All ingress endpoints point at **Caddy on loopback** (`https://127.0.0.1:8443`),
never directly at a container or at port 8080/3000:

- the tunnel reuses the same TLS vhosts as the LAN, so CSP/headers/auth are
  identical;
- `originRequest.noTLSVerify: true` is safe because that hop never leaves the
  host (Caddy's certificate is from the appliance CA);
- `httpHostHeader` must match the Caddy site name so the right vhost answers;
- a catch-all `http_status:404` prevents accidentally publishing anything not
  explicitly listed.

## Security warnings

1. **Never publish the admin portal without Cloudflare Access.** The portal
   uses application authentication, but internet-exposed login endpoints are
   brute-forced constantly. A Cloudflare Access policy (owner email + MFA) is
   the expected configuration; the tunnel README in `docs/DEPLOYMENT.md` walks
   through it.
2. **Do not publish DMZ apps without authentication in front of them.** The
   DMZ is isolated from the LAN precisely because those apps are untrusted.
   Publishing them shifts the trust boundary to Cloudflare Access.
3. **Kiosk and daemon API ports are loopback-only** and must never appear in
   an ingress rule (`3000`, `8080`).
4. **The credentials file is a secret.** `*.json` credential files are in
   `.gitignore`; rotate the tunnel (`cloudflared tunnel delete` + create) if it
   ever leaks.
5. **Tunnel provider in config**: the portal only enables this path when
   `services.enable_tunnel = true` and `services.tunnel_provider =
   cloudflare_tunnel`. That is intentionally a separate opt-in from installing
   the binary.

## Alternative: DNS-only access via WireGuard

If remote access is only needed for the owner's devices, prefer
`../wireguard/` over a public tunnel: it opens no inbound port either (the
server is the *client* of the VPN in a road-warrior setup or the UDP port is
limited to a VPN endpoint if you self-host the listener), and nothing is
published to a third party. See the comparison in `docs/DEPLOYMENT.md`.
