# Caddy reverse proxy (internal TLS)

Single TLS entry point for the LAN portal and app subdomains. The daemon API
stays on `127.0.0.1:8080`; Caddy listens on `8443` and terminates TLS with the
appliance-local CA. This is the intended architecture - the admin API is never
bound to the LAN interface.

```
LAN client ──https://homeserver.local:8443──┐
Cloudflare Tunnel ──https://127.0.0.1:8443──┤
WireGuard client ──https://homeserver.local:8443──┘
                                            ▼
                                    Caddy :8443 (TLS)
                                     ├── /               -> 127.0.0.1:8080 (hs-daemon)
                                     ├── /api/*          -> 127.0.0.1:8080
                                     ├── /healthz        -> static "ok"
                                     └── import apps.d/* -> app vhosts (HTTPS :8443)
Kiosk Chromium ──http://127.0.0.1:3000──► hs-daemon kiosk API (not exposed)
```

## Files and install targets

| File | Installed as |
| --- | --- |
| `Caddyfile` | `/etc/caddy/Caddyfile` |
| `caddy.service.d/10-homeserver.conf` | `/etc/systemd/system/caddy.service.d/10-homeserver.conf` |
| `apps.d/00-placeholder.caddy` | `/etc/caddy/apps.d/00-placeholder.caddy` |
| `apps.d/example-app.caddy.example` | documentation only (not imported) |
| *(daemon-rendered)* | `/etc/caddy/apps.d/app-<slug>.caddy` |
| *(installer)* | `/etc/homecore/caddy.env` |

Install on a Debian/Ubuntu host:

```bash
sudo apt install caddy        # Ubuntu 24.04 universe; otherwise use the Caddy apt repo
sudo install -D -m0644 infra/services/caddy/Caddyfile /etc/caddy/Caddyfile
sudo install -D -m0644 infra/services/caddy/caddy.service.d/10-homeserver.conf \
  /etc/systemd/system/caddy.service.d/10-homeserver.conf
sudo install -D -m0644 infra/services/caddy/apps.d/00-placeholder.caddy /etc/caddy/apps.d/00-placeholder.caddy
sudo systemctl daemon-reload && sudo systemctl enable --now caddy
```

## Environment contract (`/etc/homecore/caddy.env`, mode 0644)

| Variable | Default in Caddyfile | Meaning |
| --- | --- | --- |
| `HS_HOSTNAME` | `homeserver` | `SystemConfig.device.hostname`; also the mDNS label (`<hostname>.local`) |
| `HS_BASE_DOMAIN` | `home.local` | `SystemConfig.services.base_domain`; app vhosts are `<app>.<base_domain>` |
| `HS_ADMIN_PORT` | `8443` | `SystemConfig.security.admin_web_port` |

The installer writes it from the autoinstall identity; the daemon keeps it in
sync when device/network settings change and restarts Caddy. `caddy validate`
runs as `ExecStartPre`, so a bad env file fails the unit instead of serving
half of the configuration.

## TLS (internal CA)

- `tls internal` issues certificates from Caddy's local PKI for
  `<hostname>.local`, `<hostname>.<base_domain>` and `localhost`.
- `hs-firstboot` runs `caddy trust` once the service is up, adding the local
  root CA to the appliance trust store (the kiosk and `curl` from the host then
  trust the portal).
- Other devices need the CA imported once:
  `/var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt`.
  Export it through the portal's "download CA" action once implemented, or
  copy it by USB/SSH. `docs/DEPLOYMENT.md` documents the flow.
- There is no ACME/Let's Encrypt path on purpose: the appliance must work
  without internet access, and a public certificate would require exposing
  port 80/443.

## `apps.d/` snippet contract (daemon-facing)

The daemon is the **only writer** of `app-*.caddy` files. This section is the
contract the `hs-apps` orchestrator must implement; golden tests for the
renderer should mirror it.

**Directory**

```
/etc/caddy/apps.d/          0755 root:root
├── 00-placeholder.caddy    installer-owned, comment-only (keeps the glob non-empty)
├── app-home-assistant.caddy        daemon-owned
└── app-music-assistant.caddy       daemon-owned
```

**Filename**

- `app-<slug>.caddy`, slug = `^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$` (DNS label).
- The daemon manages only files matching `^app-[a-z0-9-]+\.caddy$`; anything
  else in the directory is left untouched and may be operator-managed.

**Content**

- One or more **complete site blocks**; no global options block, no `import`,
  no `admin` directive. The file is included at the top level of the Caddyfile.
- Every site must use `tls internal` (the appliance never has a public cert).
- Site address: `https://<subdomain>.{$HS_BASE_DOMAIN:home.local}-...` is not
  allowed (env placeholders are expanded by Caddy, but the daemon must render
  the literal base domain so `caddy validate` catches typos); render
  `https://<subdomain>.<base_domain>:<admin_port>` explicitly.
- The block must repeat the baseline headers (`Strict-Transport-Security`,
  `X-Content-Type-Options`, `Referrer-Policy`, `-Server`) because it is an
  independent site.
- Upstream: `reverse_proxy <app-address>:<port>` with `flush_interval -1` for
  WebSocket/SSE apps. Never proxy to `127.0.0.1:8080` (the daemon) or
  `127.0.0.1:3000` (the kiosk API).
- No secrets in the file: Caddy snippets are world-readable (`0644`).

**Write protocol**

1. Render the full snippet to `/etc/caddy/apps.d/.app-<slug>.caddy.tmp`.
2. `fsync` + `rename()` into `app-<slug>.caddy` (atomic replace).
3. Run `caddy validate --config /etc/caddy/Caddyfile` (as root; same binary the
   unit uses).
4. On success: `systemctl reload caddy` (atomic config swap; Caddy keeps the
   old config if the new one fails to load).
5. On failure: restore the previous file and do **not** reload; surface the
   validation error through the API/event bus.
6. Removing an app removes `app-<slug>.caddy` and reloads Caddy.

Files that belong to apps no longer in the catalog are garbage-collected on
daemon startup (same pattern, same prefix).

**Example**: [`apps.d/example-app.caddy.example`](apps.d/example-app.caddy.example)
(Home Assistant on `https://ha.<base_domain>:8443`).

## Validation and smoke tests

```bash
# Static validation (requires caddy installed; run as root for file access).
sudo caddy validate --config /etc/caddy/Caddyfile

# Reload after adding snippets.
sudo systemctl reload caddy && systemctl status caddy --no-pager

# TLS + routing from the appliance.
curl -fsS --cacert /var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt \
  https://homeserver.local:8443/healthz
curl -fsS --cacert /var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt \
  https://homeserver.local:8443/api/v1/setup/status

# From another LAN device (accept the private CA once, or use -k for a test).
curl -k -fsS https://homeserver.local:8443/healthz

# Kiosk is loopback-only: this must fail from the LAN.
nc -z -w 3 homeserver.local 3000 && echo "UNEXPECTED" || echo "OK: kiosk not exposed"
```

## Security notes

- **No admin API** (`admin off`): no local socket or HTTP endpoint can rewrite
  the config; only files + `systemctl reload`.
- **CSP**: the portal UI must not need inline scripts. `style-src
  'unsafe-inline'` is kept because the React UI injects a few inline styles;
  tighten once the UI ships with hashed styles.
- **HSTS** is short (1 day) on purpose - see the Caddyfile comment.
- **App vhosts are LAN-only by default** because port 8443 is open to the LAN
  netfilter set only (see `infra/firewall/nftables.conf`). Public exposure is a
  separate, explicit action (Cloudflare Tunnel + Cloudflare Access, or
  WireGuard - see `tunnel/`).
- **DMZ apps** are reached by Caddy through the host route. They are never
  accessible from the LAN directly unless the optional LAN -> DMZ forward rule
  is enabled.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| `caddy validate` fails after a snippet render | Read the reported line; the daemon restores the previous snippet, verify `/etc/caddy/apps.d/` contains no `.tmp` leftovers |
| Browser certificate warning | CA not imported on that client; compare with `openssl s_client -connect homeserver.local:8443 -CAfile root.crt` |
| 502 from the portal | Daemon not listening: `ss -ltnp | grep 8080`, `journalctl -u hs-daemon` |
| App vhost 404 | Snippet not imported: `systemctl reload caddy`, check `/etc/caddy/apps.d/` and `journalctl -u caddy` |
| WebSocket disconnects | Missing `flush_interval -1` in the app snippet |
