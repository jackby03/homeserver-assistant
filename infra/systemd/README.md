# systemd units, users and runtime policy

Everything needed to run the appliance's local services under systemd, plus
the sysusers/tmpfiles/udev policy that fixes the on-disk layout.

| File | Installed as | Purpose |
| --- | --- | --- |
| `hs-daemon.service` | `/etc/systemd/system/hs-daemon.service` | Privileged API/composition root (`/usr/local/bin/hs-daemon serve`) |
| `hs-vision.service` | `/etc/systemd/system/hs-vision.service` | Unprivileged camera/vision service (`/run/homecore/vision.sock`) |
| `hs-kiosk.service` | `/etc/systemd/system/hs-kiosk.service` | `cage` + Chromium wall kiosk on `tty1` |
| `homeserver-power.service` | `/etc/systemd/system/homeserver-power.service` | Lid switch + battery charge limit policy |
| `hs-power-policy.sh` | `/usr/local/sbin/hs-power-policy.sh` | Script executed by the power unit and by udev |
| `udev/99-homeserver-power.rules` | `/etc/udev/rules.d/99-homeserver-power.rules` | Re-apply the power policy on AC/battery changes |
| `sysusers.d/homecore.conf` | `/etc/sysusers.d/homecore.conf` | `hs-vision` and `kiosk` system users/groups |
| `tmpfiles.d/homecore.conf` | `/etc/tmpfiles.d/homecore.conf` | `/var/lib/homecore`, `/run/homecore`, `/etc/homecore`, `/etc/caddy/apps.d` |
| `hs-firstboot.service` | `/etc/systemd/system/hs-firstboot.service` | One-shot first-boot provisioning (shipped in `iso/rootfs-overlay`) |

`scripts/install.sh` installs these files (it is what the ISO's autoinstall
`late-commands` run); the first-boot unit then performs the runtime steps.

## Install (on a Debian/Ubuntu host, from the repo checkout)

```bash
sudo ./scripts/install.sh                 # install units + helpers, no services started
sudo ./scripts/install.sh --start         # ... and enable/start them
sudo systemctl status hs-daemon hs-vision
```

The script is idempotent: re-running it refreshes the units and enables new
ones without touching `/var/lib/homecore`.

## Runtime layout

```
/var/lib/homecore         0700 root:root          config.json, state.redb, master.key, apps/ caddy/ backups/ firewall/
/run/homecore             2770 root:hs-vision     vision.sock (0660, group hs-vision)
/etc/homecore             0700 root:root          env (EnvironmentFile), caddy.env
/etc/caddy/apps.d         0755 root:root          daemon-rendered reverse-proxy snippets
/etc/nftables.d           0755 root:root          boot baseline firewall
/etc/netplan              0755 root:root          daemon-rendered 90-homeserver.yaml (LAN/DMZ bridges)
/var/lib/kiosk            0700 kiosk:kiosk        Chromium profile
/var/lib/hs-vision        0700 hs-vision:video    optional model cache
```

Loopback-only listeners (never exposed to the LAN by the daemon):

| Listener | Owner | Consumer |
| --- | --- | --- |
| `127.0.0.1:8080` | hs-daemon | Caddy (`https://<host>:8443`), `X-Forwarded-Proto: https` |
| `127.0.0.1:3000` | hs-daemon | kiosk Chromium, UI/API under `/kiosk` only |
| `/run/homecore/vision.sock` | hs-vision | hs-daemon |

## Hardening notes and caveats

`hs-daemon.service` runs as **root** because it needs Incus, nftables and
sysfs. It is still hardened:

- `NoNewPrivileges=true`, `ProtectSystem=strict`, `ProtectHome=true`,
  `PrivateTmp=true`;
- writable paths are reduced to an explicit allow list:
  `/var/lib/homecore`, `/var/lib/incus`, `/run/homecore`, `/etc/caddy/apps.d`,
  `/etc/nftables.d`, `/etc/netplan`, `/etc/systemd/logind.conf.d`;
- `Restart=on-failure`, `RestartSec=3`, `LimitNOFILE=65536`.

**Caveats to verify on the target kernel/systemd:**

1. **Incus socket access.** The daemon talks to `/var/lib/incus/unix.socket`.
   `/var/lib/incus` is in `ReadWritePaths`, but if Incus is configured with a
   different `core.socket` path, add it there or the daemon will fail with
   `permission denied` / `no such file`.
2. **Incus attach/exec namespaces.** Creating containers via the API works with
   the hardening above. If the daemon later attaches to an instance namespace
   (`nsenter`) it may need `CAP_SYS_ADMIN`; check `systemd-analyze security
   hs-daemon` and relax one directive at a time, documenting why. Do not add
   `CAP_SYS_ADMIN` pre-emptively.
3. **`ProtectSystem=strict`** makes `/usr/local/bin/hs-daemon` read-only (good)
   but also blocks writes below `/var/lib/systemd` etc. The daemon must not
   shell out to `systemctl` from inside a private mount namespace with a
   read-only `/run`; if you add `systemctl reload caddy`, use
   `busctl`/`systemd-run` from the host or add `/run/systemd` access.
4. **nftables**: the daemon validates a rendered ruleset with `nft -c -f` and
   only then applies it. `CAP_NET_ADMIN` is implied by running as root; keep
   `NoNewPrivileges=true` (it does not block nftables netlink operations).

`hs-vision.service` and `hs-kiosk.service` run as unprivileged users and rely
on group membership for device access (`video`, `render`, `input`, `audio`),
not on root.

### hs-vision

- `User=hs-vision`, `Group=video`, `SupplementaryGroups=video`: camera nodes
  are `root:video` `0660`, so the `video` group is what grants access.
- The socket directory `/run/homecore` is created by systemd-tmpfiles as
  `root:hs-vision` mode `2770`; the setgid bit makes the socket inherit group
  `hs-vision` and `UMask=0007` makes it `0660`. The daemon runs as root, so it
  can connect regardless of ownership.
- `PrivateDevices=true` with `DeviceAllow=/dev/video0 rw` and
  `DeviceAllow=/dev/video1 rw`: the service cannot open any other device node.

### hs-kiosk

- `User=kiosk`, `PAMName=login`, `TTYPath=/dev/tty1`, `StandardInput=tty`,
  `WantedBy=graphical.target`: logind creates the session and grants the seat
  to `cage`, which is the standard way to run a Wayland kiosk without a
  display manager. `hs-firstboot` sets `graphical.target` as the default.
- `XDG_RUNTIME_DIR` is a documented placeholder in the unit; the
  `hs-kiosk-start` wrapper (in `iso/rootfs-overlay/usr/local/bin/`) exports
  `/run/user/$(id -u kiosk)` so the UID does not have to be hard-coded.
- Chromium is expected at `/usr/bin/chromium-browser` (Debian/Ubuntu) and
  falls back to `/snap/bin/chromium` (Ubuntu 24.04 ships Chromium as a snap).
  The wrapper prefers a native binary when present; see `docs/DEPLOYMENT.md`
  for the snap caveat.
- `Restart=always` with `RestartSec=5`: if the browser crashes, the kiosk
  comes back on its own. A `curl` health gate in the wrapper waits for the
  daemon before launching Chromium, so boot order does not matter.

### homeserver-power

- Writes `/etc/systemd/logind.conf.d/90-homeserver.conf`
  (`HandleLidSwitch=ignore`, `HandleLidSwitchExternalPower=ignore`,
  `IdleAction=ignore`) because the appliance is a closed laptop in a wall
  panel: a lid event must never suspend it.
- Applies an 80 % charge limit to every battery that exposes
  `charge_control_end_threshold` (kernel >= 5.4, ThinkPads and many others).
  Unsupported batteries are logged and ignored - the script is a no-op there.
- BIOS/UEFI "restore power on AC" must still be enabled by hand; see
  `docs/DEPLOYMENT.md`. The script only handles what the OS can control.
- The udev rule restarts the unit on power_supply changes so a replaced battery
  or a re-plugged charger re-applies the threshold.

## Health checks

```bash
systemctl status hs-daemon hs-vision hs-kiosk homeserver-power
journalctl -u hs-daemon -b --no-pager
journalctl -u hs-vision -b --no-pager
systemd-analyze security hs-daemon        # hardening score (expect ~5-7 OK)
/usr/local/sbin/hs-doctor.sh              # shipped by the ISO
```
