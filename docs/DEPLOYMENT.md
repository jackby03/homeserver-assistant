# Deployment guide (appliance)

How to turn a laptop into a wall-mounted HomeServer Assistant appliance:
hardware preparation, ISO installation, first boot, network/DNS/DMZ, TLS
trust, remote access, Home Assistant onboarding, backups and updates.

Target OS: Ubuntu 24.04 LTS installed from `iso/` (see [ISO.md](ISO.md)).
All runtime configuration is owned by the daemon; this guide explains the
architecture and the operator steps.

---

## 1. Architecture overview

```
                    internet
                       │
              ┌────────┴─────────┐
              │ home router      │  DHCP + DNS (LAN), no port-forwards
              │ 192.168.1.1/24   │
              └────────┬─────────┘
                       │ eth0 (bridge member, no IP of its own)
        ┌──────────────┴───────────────────────────────────────────┐
        │ appliance (Ubuntu 24.04, hs-daemon)                      │
        │                                                          │
        │  br-lan  192.168.1.10/24  trusted LAN bridge             │
        │    ├─ Caddy :8443  ──► 127.0.0.1:8080  hs-daemon API     │
        │    ├─ kiosk :3000  (loopback only)                       │
        │    └─ Incus instances (Home Assistant, Music Assistant)  │
        │                                                          │
        │  br-dmz  10.20.30.1/24  isolated DMZ bridge (no NAT to   │
        │    │                    LAN, no default route)           │
        │    └─ Incus instances (untrusted/public-facing apps)     │
        │         └─ masqueraded egress via eth0 (nftables)        │
        │                                                          │
        │  vision: /run/homecore/vision.sock (root:hs-vision 2770) │
        │  kiosk:  http://127.0.0.1:3000/kiosk  (tty1)             │
        └──────────────────────────────────────────────────────────┘

Remote access (choose one, both outbound-only):
  Cloudflare Tunnel ──cloudflared──► Caddy 127.0.0.1:8443   (public hostnames)
  WireGuard         ──wg0─────────► Caddy br-lan:8443       (private clients)
```

**Intended security architecture:** the daemon binds the admin API to
`127.0.0.1:8080` and the kiosk UI/API to `127.0.0.1:3000/kiosk`; Caddy is the
only TLS entry point (`:8443`) and terminates TLS before proxying to loopback.
Port 8080/3000 are never exposed on the LAN, and the DMZ cannot reach either
of them or any host service other than DNS/NTP.

Key paths and ports:

| Component | Path / port | Notes |
| --- | --- | --- |
| daemon data dir | `/var/lib/homecore` (0700 root) | `config.json`, `state.redb`, `master.key`, apps, backups, firewall |
| admin API | `127.0.0.1:8080` | behind Caddy, never LAN-exposed |
| portal | `https://<hostname>.local:8443` | Caddy internal CA |
| kiosk | `http://127.0.0.1:3000/kiosk` | Chromium via cage on `tty1` |
| vision socket | `/run/homecore/vision.sock` | `hs-vision`, mode 0660, dir 2770 root:hs-vision |
| firewall baseline | `/etc/nftables.d/10-hs-network.nft` | boot fallback; daemon renders the live ruleset |
| live ruleset | `/var/lib/homecore/firewall/nftables.conf` | tables `inet homeserver` + `ip homeserver_nat` |
| LAN/DMZ bridges | `br-lan`, `br-dmz` | rendered by `hs-network` to `/etc/netplan/90-homeserver.yaml` |

---

## 2. Hardware preparation (laptop in a wall panel)

The appliance is designed around a used x86 laptop (ThinkPad T480/T480s class
is a common choice) mounted inside the wall or a DIN enclosure. Prepare:

### 2.1 BIOS/UEFI

| Setting | Value | Why |
| --- | --- | --- |
| Restore AC power on power loss | **On / "Power On"** | the appliance must boot after a blackout; no one will press the power button |
| Wake on AC / "Power On when AC detected" | On | same |
| Secure Boot | Off (or enroll your own keys) | the ISO's kernels are unsigned by this project; Incus and ZFS need standard modules |
| Virtualisation (VT-x/AMD-V) | On | Incus containers need namespaces; VMs need VT-x |
| Battery charge threshold (if present in firmware) | 80 % | belt-and-braces with the OS-level limit |
| Lid close action | Nothing / Ignore | `homeserver-power.service` also sets `HandleLidSwitch=ignore` |
| Boot order | USB first, then disk | for installation; remove USB afterwards |
| TPM disk encryption | optional | not enabled by the installer |

The OS-level power policy is applied by `homeserver-power.service`:
`/etc/systemd/logind.conf.d/90-homeserver.conf` ignores lid/suspend/idle
events, and `/sys/class/power_supply/BAT*/charge_control_end_threshold` is set
to 80 % when the firmware exposes it. BIOS "restore AC power" cannot be set by
the OS - do it by hand.

### 2.2 Physical mounting and airflow

- Remove the battery if the panel will be permanently powered? **No** - keep
  it: it acts as a UPS for clean shutdowns. Apply the 80 % charge limit.
- Leave the fan intake/exhaust unobstructed; a closed wooden panel overheats.
  Drill ventilation holes or use a small enclosure fan; monitor with
  `sensors` (installed).
- Keep the lid closed but *do not* disconnect the lid switch - the OS ignores
  it, and the panel is built around the closed chassis.
- Route the camera/microphone ribbon cables carefully; use a USB camera if the
  internal one is hard to reach (see below).
- Label the LAN cable and the power supply inside the panel.

### 2.3 Display / touchscreen

- The kiosk uses Wayland via `cage` on `tty1`; the panel appears as the only
  DRM connector. Touch works through `libinput` (`kiosk` is in the `input`
  group).
- Enable the kernel parameter `consoleblank=0` (the installer adds it) so the
  panel never blanks - `cage` does not implement DPMS.
- If the touchscreen is rotated, add a libinput/udev calibration rule; cage
  does not expose a rotation setting by default.
- Choose a panel with an external HDMI/USB controller when possible: internal
  laptop panels need an LVDS/eDP controller board.

### 2.4 Camera behind glass

- The camera must not be blocked by the panel glass; matte/IR-transparent
  glass or a small cut-out is required for the local vision service.
- `hs-vision` opens `/dev/video0` through the `video` group only; it cannot
  access other devices (`PrivateDevices=true`).
- An external USB camera is often better: predictable `/dev/video*` numbering,
  wider angle, better low-light. If you add a second camera, extend
  `DeviceAllow=` in `hs-vision.service` and the daemon's vision socket
  configuration together.
- Focus the camera at the entry point and check the view at night; the
  vision service runs fully local (no frames leave the appliance).

### 2.5 Microphone / audio output

The appliance drives a home stereo. Three wiring options:

| Option | Quality | Notes |
| --- | --- | --- |
| **USB DAC** (recommended) | best | appears as a PipeWire sink; select it in the portal (`SystemConfig.audio.sink_id`); no ground loop from the laptop's headphone jack |
| HDMI audio to a TV/receiver | good | second monitor output; select the HDMI sink |
| Bluetooth speaker/receiver | convenient | pair once with `bluetoothctl`; latency is higher; keep the adapter away from the metal panel |

The laptop's internal speakers are usually unusable behind a wall; always
prefer an external path. Audio routing is managed by `hs-system` (`wpctl`);
configure the sink in the portal, not with ad-hoc `pactl` commands.

### 2.6 Thermal and power budget

- Target < 60 °C CPU idle. Use `sensors`; consider `powertop` tuning only if
  measured, not by default.
- A wall-mounted laptop consumes ~8-15 W idle; the 80 % battery limit and
  `vm.swappiness=10` (installed) reduce heat and SSD wear.
- Do not use a laptop with a swollen battery; replace it before mounting.

---

## 3. Installation

1. Build or obtain the ISO (`iso/README.md`, `docs/ISO.md`). Verify its SHA256
   against the manifest.
2. Write it to a USB stick:
   `sudo dd if=homeserver-assistant-YYYYMMDD.iso of=/dev/sdX bs=8M status=progress oflag=sync`
3. Boot the laptop from USB. The installer runs unattended:
   locale `en_US.UTF-8`, timezone `UTC`, single ext4 root, 4 GiB swap file,
   user `homeserver`, no SSH server.
4. The system reboots into the installed OS. Remove the USB stick.
5. `hs-firstboot.service` provisions users/dirs, Incus (preseed: `local` pool,
   `br-lan`/`br-dmz`, profiles `lan`/`dmz`, project `homeserver`), the firewall
   baseline, Caddy + local CA trust, the power policy and the daemon.
6. The serial console/journal prints the portal URL. On a laptop panel, the
   kiosk opens `http://127.0.0.1:3000/kiosk`; if the daemon is not up yet the
   wrapper retries, then shows a connection page.

Verify from a LAN computer:

```bash
#!/usr/bin/env bash
# Replace with the appliance address shown by hs-firstboot.
curl -k https://homeserver.local:8443/healthz          # -> ok
curl -k https://homeserver.local:8443/api/v1/setup/status
```

If `curl` from another machine cannot resolve `homeserver.local`, use the LAN
IP printed by the installer (`hostnamectl`/`ip -br addr show br-lan`).

---

## 4. First boot and TLS trust

`hs-firstboot.sh` writes the marker `/var/lib/homecore/.firstboot-done` when
done. It does **not** create `config.json`/`master.key`: `hs-daemon` generates
those on its first start, so the wizard starts from a clean state.

Trust the appliance-local CA once per client device (phones, laptops):

1. From a machine that already trusts it (the appliance itself or with
   `curl -k`), copy `/var/lib/homecore/caddy-root-ca.crt` (exported by first
   boot) or the Caddy root
   `/var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt`.
2. Import it as a trusted root CA (Linux: `update-ca-certificates`; macOS:
   Keychain; Windows: `certutil -addstore -user Root`; Android/iOS: install
   profile).
3. Open `https://homeserver.local:8443` and complete the wizard:
   language, administrator password (Argon2id), device name, timezone, DNS
   provider, DMZ enablement, Home Assistant URL/token.

The internal CA is per appliance. A factory reset/reinstall generates a new
CA; re-import it on clients (delete the old one to avoid confusion).

`hs-doctor.sh` reports whether the local CA is trusted on the appliance and
whether the portal answers.

---

## 5. Network, DNS and DMZ

### 5.1 LAN

Bridge naming is fixed in `SystemConfig` and shared by netplan, Incus, the
firewall and these docs: LAN bridge `br-lan`, DMZ bridge `br-dmz`, physical
uplink `eth0`. `hs-network` renders them into
`/etc/netplan/90-homeserver.yaml` (reference template:
`infra/network/netplan-90-homeserver.example.yaml`).

- The installer uses DHCP for the first boot. For a stable portal URL, either
  reserve a DHCP lease on the router for the appliance MAC, or configure a
  static address in the wizard (`network.mode = static`); `hs-network` renders
  the netplan file with `br-lan` holding the address and `eth0` as a bridge
  member only (never both).
- Point the router's DHCP DNS option at the appliance LAN address to activate
  filtering for every device, or configure a pilot group first.

### 5.2 DNS filtering

`network.dns` selects the engine (AdGuard Home / Technitium), the encrypted
upstream (DoH/DoT; Cloudflare, Cloudflare Family, Quad9, Control D or a custom
endpoint) and the blocklists. `hs-network` renders the engine configuration;
the daemon deploys the app and keeps the blocklist URLs in sync.

AdGuard's admin UI is loopback-only (`127.0.0.1:3000`); reach it through an
SSH tunnel (`ssh -L 3000:127.0.0.1:3000 homeserver@<lan-ip>`, if SSH is
enabled) or expose it deliberately as an authenticated Caddy app vhost.

> Known integration gap: the current `hs_network::netplan` renderer writes a
> resolver of `127.0.0.1` for the host, and AdGuard is deployed as an Incus
> container. The daemon must either add an Incus proxy device for port 53 or
> point the host resolver at the container's address; check
> `hs-network`/`hs-apps` before relying on host-side filtering.

### 5.3 VLANs and the DMZ

The DMZ is a separate bridge (`br-dmz`, `10.20.30.1/24`) with:

- **no NAT** and no route to `br-lan`;
- nftables rules that allow only DNS (`53`) and NTP (`123`) from the DMZ to
  the host, when `network.dmz.allow_dns = true`;
- an explicit, counted `DMZ → LAN` drop (the core isolation invariant);
- **masqueraded internet egress** (`table ip homeserver_nat`), so DMZ apps can
  update and reach their tunnel endpoints without any router port-forward.

Optionally map the DMZ to a dedicated VLAN: create a VLAN sub-interface on the
uplink (e.g. `eth0.20`), make it a bridge member of `br-dmz`-equivalent
netplan stanza, and keep the same subnet/firewall model. The identity of the
bridge name (`br-dmz`) is what the firewall renderer matches, so rename it in
`SystemConfig.network.dmz.bridge` and in the netplan template together.

### 5.4 Firewall verification

```bash
sudo nft list table inet homeserver          # isolation rules + counters
sudo nft list table ip homeserver_nat        # DMZ egress masquerade
incus exec <dmz-instance> -- nc -z -w2 10.20.30.1 8443 && echo BAD || echo "OK: blocked"
```

Do not add DMZ → LAN port-forwards "temporarily": the drop rule must stay
before any accept, and `hs_network::firewall::validate_isolation` rejects
rulesets that lose it.

---

## 6. Remote access

Both supported providers are **outbound-only**; no inbound port is opened on
the home router.

### 6.1 Cloudflare Tunnel (public hostnames, optional)

1. Install `cloudflared` from Cloudflare's apt repository (the ISO does not
   ship it).
2. `cloudflared tunnel login`, `cloudflared tunnel create homeserver`.
3. Install `infra/services/tunnel/cloudflared/config.yml.example` as
   `/etc/cloudflared/config.yml`, replace the UUID and hostnames.
4. `cloudflared tunnel route dns homeserver home.example.com`.
5. Enable the systemd unit (`cloudflared.service.example`).
6. **Protect the portal with Cloudflare Access** (owner email + MFA). The
   tunnel only proxies to Caddy on loopback
   (`https://127.0.0.1:8443` with `noTLSVerify: true`); Caddy's ingress rules
   and headers apply unchanged.
7. Enable the provider in the portal (`services.enable_tunnel = true`,
   `tunnel_provider = cloudflare_tunnel`) only after it works.

Never publish DMZ apps without authentication in front of them; the DMZ is
untrusted by design. The tunnel does not change the firewall: DMZ egress works
through the masquerade rule, and no inbound path to the DMZ exists.

### 6.2 WireGuard (private clients, recommended)

1. Generate keys on the appliance (`wg genkey | wg pubkey`), install
   `infra/services/tunnel/wireguard/wg0.conf.example` as `/etc/wireguard/wg0.conf`
   (`0600`), add one `[Peer]` per device with a `/32` `AllowedIPs`.
2. `sudo systemctl enable --now wg-quick@wg0`.
3. Add the client config (split tunnel to `10.99.0.0/24` + `br-lan` if full LAN
   access is explicitly enabled).
4. The nftables input chain accepts the admin portal from LAN interfaces;
   add `wg0` to the rendered plan's LAN set if the daemon does not do it yet
   (see `infra/firewall/nftables.conf` comments and the firewall golden tests).

WireGuard clients can reach the portal only by default - full LAN access is an
explicit forward rule documented in
`infra/services/tunnel/wireguard/README.md`.

---

## 7. Home Assistant onboarding

1. Deploy Home Assistant (app catalog, LAN profile). It runs on `br-lan` with
   host networking semantics inside the container; mDNS/SSDP discovery works
   because the instance is a first-class LAN member.
2. Open `http://<app-lan-ip>:8123` (or the Caddy vhost
   `https://ha.<base_domain>:8443` rendered by the daemon) and create the
   owner account.
3. In Home Assistant, trust the reverse proxy for client IPs:

   ```yaml
   http:
     use_x_forwarded_for: true
     trusted_proxies: [127.0.0.1, ::1]
   ```

4. Create a long-lived access token and paste it in the wizard together with
   the Home Assistant URL. The token is stored in the encrypted vault, never in
   `config.json` or this repository.
5. Let the assistant provision automations/labels through `hs-homeassistant`;
   every mutating call is authenticated and audited.

Do not enable HA's own TLS: Caddy terminates TLS. Do not expose HA directly to
the internet; use the tunnel with Cloudflare Access.

---

## 8. Backups

Back up three layers, in this order of importance:

| Layer | What | How |
| --- | --- | --- |
| appliance state | `/var/lib/homecore` (`config.json`, `state.redb`, `master.key`, app configs) | daemon backup job to an external disk/NAS; `master.key` is required to decrypt the vault, keep it offline too |
| Incus instances | app volumes/snapshots | `incus snapshot create <instance>` + `incus export --instance-only`; scheduled via the daemon |
| appliance OS | `/etc` (systemd, caddy, netplan, nftables) | `tar -C / -czf etc-backup.tar.gz etc` after configuration changes; small and fast |

Notes:

- `state.redb` must be copied while the daemon is stopped or through the
  daemon's backup API (redb is not crash-consistent across a raw copy).
- `master.key` is a secret: `0600 root:root`, never synced to a public cloud.
- Test a restore at least once: install the ISO, restore `/var/lib/homecore`,
  start the daemon and verify the wizard says initialized.
- The appliance offers a restore path that recreates containers from exported
  instance backups; a Home Assistant snapshot is not enough on its own (the
  appliance metadata is not inside it).

---

## 9. Updates and rollback

- **OS updates**: `apt` security updates are applied by the appliance update
  job (or `sudo apt update && sudo apt upgrade`); reboot when the kernel
  changes. Take a `/etc` backup first (`§8`).
- **Appliance binaries**: install the new `hs-daemon`/`hs-vision` under
  `/usr/local/bin` (atomic `install`), then `systemctl restart hs-daemon`.
  Keep the previous binary until the new version passes `/usr/local/sbin/hs-doctor.sh`.
- **Apps**: the daemon upgrades containers image-by-image and keeps a pre-upgrade
  Incus snapshot; rollback is `incus restore <instance> <snapshot>` through the
  portal. Pin app images in the catalog rather than using `latest`.
- **Firewall/netplan**: rendered from `config.json`; the daemon validates the
  nftables ruleset (`validate_isolation`) and applies it atomically. A failed
  render leaves the running ruleset untouched; manual copy:
  `/var/lib/homecore/firewall/last-good.nft`.
- **Full rollback**: reinstall the ISO and restore `/var/lib/homecore` + Incus
  exports. The CA changes on reinstall, so re-import it on clients.

---

## 10. Security checklist

Before exposing the appliance or trusting it with family data:

- [ ] Console password changed from the ISO default; SSH disabled or key-only.
- [ ] Wizard completed; admin password strong (Argon2id), unique.
- [ ] `hs-doctor.sh` reports no FAIL, and `nft list table inet homeserver`
      shows the DMZ → LAN drop and the DMZ → portal drop.
- [ ] Portal CA imported on every client device; no `curl -k` in habitual use.
- [ ] DNS filtering enabled with an encrypted upstream; blocklists update.
- [ ] DMZ enabled only for untrusted apps; DMZ instances have no LAN access
      (verified with `nc`/`ping` from a DMZ container).
- [ ] Remote access is outbound-only: either WireGuard with per-device keys,
      or Cloudflare Tunnel behind Cloudflare Access (MFA). No router
      port-forwards to 8080/3000/8443.
- [ ] `master.key` and any tunnel credentials backed up offline; no secrets in
      `config.json`, compose files or the repository.
- [ ] Camera and microphone use is disclosed to the household; the vision
      service keeps frames local (enforced by architecture, not policy).
- [ ] Automatic updates monitored; kernel/app rollback path tested once.
- [ ] Physical: panel locked/anchored, ventilation present, battery not
      swollen, power supply strain-relieved.

---

## 11. Known integration gaps (tracked by the team)

These are documented so operators are not surprised; they are being closed by
the daemon/network workstreams:

1. `hs_network::netplan` currently renders the physical interface (`eth0`)
   directly. The bridge model documented here (`br-lan` containing `eth0`,
   `br-dmz`) requires the netplan renderer extension; until then, the
   installer's `/etc/netplan` is the distro default and the daemon's rendered
   file must be validated with `netplan try` before applying.
2. DMZ egress/NAT is implemented in the static ruleset and in the
   `FirewallPlan` fields (`wan_interface`, `allow_dmz_egress`), and the
   renderer is being extended to emit `table ip homeserver_nat`. Verify with
   `nft list table ip homeserver_nat` after enabling the DMZ.
3. Host-side DNS (`127.0.0.1`) assumes a resolver on the host or an Incus proxy
   device for the DNS app; see §5.2.
4. Caddy app snippet rendering (`/etc/caddy/apps.d/app-*.caddy`) is specified
   in `infra/services/caddy/README.md`; the orchestrator that writes it is
   still being implemented.
