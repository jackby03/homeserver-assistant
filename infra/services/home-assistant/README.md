# Home Assistant

Home automation server. The appliance provisions it as an Incus container and
integrates with it through the REST API (`hs-homeassistant`); this directory is
the reference standalone deployment.

## Files

| File | Purpose |
| --- | --- |
| `docker-compose.yml` | standalone container (host networking, config volume) |

## Ports

| Port | Scope | Purpose |
| --- | --- | --- |
| `8123/tcp` | LAN / reverse proxy | web UI + REST API + WebSocket |

The public entry point is **not** port 8123 but the Caddy app vhost
`https://ha.<base_domain>:8443` (snippet rendered by the daemon into
`/etc/caddy/apps.d/`). Direct `:8123` access is a convenience on the LAN.

## Adoption (first boot)

1. Open `http://<appliance-lan-ip>:8123` (or the Caddy vhost) and create the
   owner account. Nothing is created by the installer; the account is yours.
2. In Home Assistant, add the appliance as a trusted proxy when running behind
   Caddy so client IPs are preserved. In `configuration.yaml`:

   ```yaml
   http:
     use_x_forwarded_for: true
     trusted_proxies:
       - 127.0.0.1
       - ::1
   ```

3. Create a **long-lived access token** (Profile -> Security -> Long-lived
   access tokens) and paste it into the HomeServer Assistant portal when the
   wizard asks for the Home Assistant URL/token. The token is stored encrypted
   in the appliance secret vault - never in `config.json` or the repo.
4. Set the Home Assistant URL (Settings -> System -> Network) to the Caddy
   vhost so notifications and links use HTTPS.
5. In the portal, ask the assistant to provision automations/scenes; every
   mutating call goes through `hs-homeassistant` with the configured token.

## Configuration notes

- **TLS**: Home Assistant itself serves plain HTTP on 8123; TLS is terminated
  by Caddy. Do not enable HA's `http.ssl_certificate` on the appliance.
- **WebSocket**: Caddy's `reverse_proxy` handles the `/api/websocket` upgrade
  automatically. Do not add extra `Connection`/`Upgrade` headers.
- **Zigbee/ZWave sticks**: passthrough is configured in the app settings
  (USB device mapping) and requires a container restart. The host must not
  bind the device through another service.
- **Discovery**: host networking keeps mDNS/SSDP working. In NAT mode the DMZ
  cannot reach Home Assistant at all (by design); keep HA on the `lan` profile.
- **Backups**: the state is `config/` plus the Home Assistant database. The
  appliance backup job snapshots the app volume and offers a restore that
  recreates the container (see `docs/DEPLOYMENT.md`).
