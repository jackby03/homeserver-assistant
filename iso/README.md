# ISO build and test

Reproducible Ubuntu 24.04 LTS (server) installation ISO for HomeServer
Assistant OS. The result is a single hybrid ISO (BIOS + UEFI) that installs the
appliance unattended, with no network access required during installation
except for the package cache (a local mirror can be used, see below).

## Flow

```
iso/autoinstall/{user-data,meta-data}     unattended install definition
        │
iso/rootfs-overlay/                       OS files installed into /target
infra/                                    units, firewall, Caddy, preseed, services
        │
        ▼
iso/build-iso.sh                          downloads + verifies base ISO,
        │                                 injects seed/assets, repacks with xorriso
        ▼
iso/output/homeserver-assistant-YYYYMMDD.iso (+ .manifest.json, SHA256)
        │
        ├──► write to USB (dd) ──► appliance boots ──► unattended install
        │                                              └── hs-firstboot.service
        │
        └──► iso/test-vm.sh               QEMU install + smoke assertions
```

## Build prerequisites (Linux x86_64 host)

```bash
sudo apt install xorriso curl coreutils gawk genisoimage \
                 qemu-system-x86 qemu-utils socat ovmf
# Optional: cloud-image-utils for the alternative external seed ISO.
# Rust + Node are only needed if you also build the binaries/UI to embed.
```

| Tool | Used for | Check |
| --- | --- | --- |
| `xorriso` | extract + repack the ISO, preserve BIOS/EFI boot | `xorriso --version` |
| `curl` or `wget` | download base ISO + `SHA256SUMS` | `curl --version` |
| `sha256sum`, `dd`, `sed`, `awk` | verification, patch, boot code | coreutils |
| `qemu-system-x86_64`, `qemu-img` | `--test` smoke run | `qemu-system-x86_64 --version` |
| `socat` | QEMU monitor screenshot (optional) | `socat -V` |
| `ovmf` | UEFI firmware in the test VM (optional) | `/usr/share/OVMF/OVMF_CODE*.fd` |

No root is required: the script only writes below `iso/work/` and
`iso/output/`.

## Build

```bash
# 0. Choose the console password. It is required and has no default.
export HS_CONSOLE_PASSWORD_HASH="$(openssl passwd -6 'a strong unique password')"

# 1. Build the appliance binaries (Linux target) and the portal UI.
cargo build --release -p hs-daemon -p hs-vision
(cd ui && pnpm install --frozen-lockfile && pnpm build)   # produces ui/dist

# 2. Build the ISO (downloads and verifies the newest 24.04 point release).
./iso/build-iso.sh \
    --binaries target/release \
    --ui-dir ui/dist

# Or fully offline from a local mirror/local ISO:
./iso/build-iso.sh --iso /srv/isos/ubuntu-24.04.3-live-server-amd64.iso \
    --binaries target/release --ui-dir ui/dist

# 3. Test it in QEMU (install + smoke assertions).
./iso/test-vm.sh --iso iso/output/homeserver-assistant-$(date -u +%Y%m%d).iso

# 4. Write to USB (double-check the device name!).
sudo dd if=iso/output/homeserver-assistant-YYYYMMDD.iso \
        of=/dev/sdX bs=8M status=progress oflag=sync
```

Build outputs:

| File | Content |
| --- | --- |
| `iso/output/homeserver-assistant-YYYYMMDD.iso` | bootable installer |
| `iso/output/....manifest.json` | base ISO name+SHA256, output SHA256, git commit, kernel params, flags |
| `iso/work/` | download cache + extraction tree (git-ignored; `--clean` removes it) |

`--clean` removes `iso/work/` and `iso/output/` (verified paths only) and exits.
`--keep-work` keeps the modified ISO tree for debugging (`iso/work/iso-root`).
`--output PATH` overrides the output file name/location.

## What the ISO contains

| ISO path | Purpose |
| --- | --- |
| `/nocloud/user-data`, `/nocloud/meta-data` | autoinstall seed (NoCloud) |
| `/hs/repo/` | `infra/`, `scripts/`, `docs/` + the ISO assets, staged from this repo |
| `/hs/bin/` | optional prebuilt `hs-daemon`, `hs-vision` |
| `/hs/ui/` | optional built React portal (`ui/dist`) |
| patched boot entries | `autoinstall "ds=nocloud;s=/cdrom/nocloud/"` (quoted: GRUB would otherwise stop at the `;`) |

The original Ubuntu boot images (`isolinux`/`grub` BIOS, `boot/grub/efi.img`
UEFI) are preserved; `build-iso.sh` reuses the hybrid MBR extracted from the
verified base ISO so no isolinux package is required.

## Autoinstall design

| Section | Value | Rationale |
| --- | --- | --- |
| `locale` | `en_US.UTF-8` | UI is localized at the application layer (`es`/`en`), OS stays English |
| `timezone` | `UTC` | per-app timezones are configuration, not OS state |
| `keyboard` | `us` | change for the site |
| `identity` | user `homeserver` | local console admin; the committed value is a placeholder. The build requires `HS_CONSOLE_PASSWORD_HASH` (`openssl passwd -6`) and refuses to produce an ISO without it |
| `ssh` | `install-server: false` | portal + local console only; enable with keys if required |
| `storage` | `layout: direct` (ext4) | simple single-disk layout; btrfs alternative documented in user-data; 4 GiB swap file created in late-commands |
| `refresh-installer` | `update: false` | offline-friendly, deterministic |
| `updates` | `security` | appliance updates are performed by the appliance |
| `late-commands` | `scripts/install.sh` + swap + kernel params + serial marker | the installer environment has `/target` but no working chroot for everything |
| `shutdown` | `reboot` | installations remove the USB and boot into `hs-firstboot` |

### Package list rationale

| Package | Why |
| --- | --- |
| `incus` + `zfsutils-linux` | app runtime; zfs pool with dir fallback |
| `nftables` | DMZ firewall (also required by `hs-daemon`) |
| `caddy` | TLS termination for the portal (`:8443` → `127.0.0.1:8080`) |
| `pipewire`, `pipewire-pulse`, `wireplumber` | audio for native Music Assistant + kiosk |
| `cage` | Wayland kiosk compositor for the wall panel |
| `chromium-browser`, `snapd` | kiosk browser (Ubuntu 24.04 ships Chromium as a snap); see the snap caveat below |
| `fonts-dejavu-core` | kiosk/UI font |
| `avahi-daemon`, `avahi-utils`, `libnss-mdns` | `<hostname>.local` mDNS name used by the portal and Caddy |
| `wireguard-tools` | optional road-warrior access |
| `jq`, `curl`, `ca-certificates`, `rsync` | diagnostics, health checks, backups |
| `lm-sensors`, `usbutils` | hardware diagnostics (`hs-doctor`) |

No Docker: on the appliance apps run as Incus containers; the compose files in
`infra/services/` are reference templates (`infra/services/README.md`).

### Chromium snap caveat

On Ubuntu 24.04 `chromium-browser` is a transitional package that installs the
Chromium snap. Consequences:

- the installer needs network access for that snap (or pre-seed it in a local
  snap store);
- snap auto-refresh can restart the browser; the kiosk unit uses
  `Restart=always` and the kiosk wrapper waits for the daemon, so this is
  survivable but should be pinned (`snap hold chromium`);
- the managed policy in `/etc/chromium/policies/managed/homeserver.json` is
  honoured by native Chromium builds; the snap may ignore some keys.

`iso/rootfs-overlay/usr/local/bin/hs-kiosk-start` prefers a native binary at
`/usr/bin/chromium` or `/usr/bin/chromium-browser` and falls back to
`/snap/bin/chromium`.

## First boot (after the unattended install)

1. GRUB boots the installed system into `graphical.target`.
2. `hs-firstboot.service` runs `/usr/local/sbin/hs-firstboot.sh`:
   users/dirs (`sysusers`/`tmpfiles`), hostname/caddy env from `config.json`
   when present, nftables baseline, Incus preseed, Caddy start + local CA trust,
   power policy, audio linger, `hs-daemon`/`hs-vision` start.
   It does **not** create `config.json`/`master.key` - the daemon generates
   those itself under `/var/lib/homecore` (0700/0600).
3. The serial console/journal prints the status block with the portal URL
   (`https://<hostname>.local:8443`).
4. The kiosk starts on `tty1` and opens `http://127.0.0.1:3000/kiosk`.

First-boot logs and state:

```bash
systemctl status hs-firstboot.service
journalctl -u hs-firstboot -b
cat /var/lib/homecore/.firstboot-done          # idempotency marker
/usr/local/sbin/hs-firstboot.sh --force        # re-provision
/usr/local/sbin/hs-doctor.sh                   # full health check
```

## QEMU test matrix

`iso/test-vm.sh` runs at least these scenarios on a Linux host:

| Scenario | Invocation | Expected |
| --- | --- | --- |
| BIOS + KVM, standard ISO | `test-vm.sh --iso out.iso` | install marker, HTTP 200, `initialized=false` |
| UEFI + KVM | install `ovmf`; same command auto-detects it | same assertions via OVMF |
| No hardware virtualisation (TCG) | run on a host without `/dev/kvm` | same assertions, much slower |
| ISO built without daemon binaries | `--allow-missing-daemon` | boot + Caddy `/healthz` pass; daemon step reported as WARN |
| Debug a failed boot | `--keep` | disk + serial logs kept under `iso/work/vm-test/` |
| Screenshot | needs `socat` (+ netpbm for PNG) | `screen.ppm`/`screen.png` in the work dir |

Serial console in the VM: `-serial file:iso/work/vm-test/serial-install.log`
for the install and `serial-boot.log` for the installed system. QEMU is
headless (`-display none`), 4 vCPU / 4 GiB RAM / 40 GiB qcow2 by default
(`--cpus`, `--memory`, `--disk-size` override), with `-no-reboot` in the
install phase so the test detects the post-install reboot.

## Reproducibility notes

- The base ISO is always verified against the upstream `SHA256SUMS`; the name
  and hash are recorded in the manifest.
- `SOURCE_DATE_EPOCH` (when set) is forwarded as `-volume_date
  all_file_dates=...` so file dates in the ISO are fixed. xorriso still writes
  a few volume descriptor fields (UUID/timestamps) per run, so **bit-identical
  output is not guaranteed**; what is guaranteed is an identical input tree,
  base ISO and manifest (except `built_at_utc`).
- The package cache inside the base ISO is not refreshed: the installed system
  runs `apt update` on first use. For long-lived offline installs, mirror the
  Ubuntu archive and point `/etc/apt/sources.list` at it via autoinstall
  (`apt.primary`/`geoip: false`).
- Build logs: `iso/build-iso.sh` prints every command it runs; the xorriso
  invocation is deterministic.

## Customisation

| Want to change | Edit |
| --- | --- |
| console password/hostname/locale | `iso/autoinstall/user-data` (`identity`, `locale`, `keyboard`); password injected via `HS_CONSOLE_PASSWORD_HASH` |
| packages | `iso/autoinstall/user-data` (`packages:`) |
| static network at install time | `iso/autoinstall/user-data` (`network:` section, commented) |
| install-time scripts | `iso/autoinstall/user-data` (`late-commands:`) |
| OS files installed on the appliance | `iso/rootfs-overlay/` (mirrors the target root) |
| systemd units / firewall / Caddy / preseed | `infra/` (copied by `scripts/install.sh`) |
| first-boot behaviour | `iso/rootfs-overlay/usr/local/sbin/hs-firstboot.sh` |
| health checks | `iso/rootfs-overlay/usr/local/sbin/hs-doctor.sh` |
| boot kernel parameters | `iso/build-iso.sh` (`KERNEL_EXTRA`) |

Keep the overlay minimal: anything that applies to a normal Debian/Ubuntu host
belongs in `infra/` and is installed by `scripts/install.sh`.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| `xorriso` not found | `sudo apt install xorriso` |
| SHA256 mismatch | retry the download; never bypass the check |
| ISO does not boot | was the base ISO hybrid? compare `fdisk -l` partition table; `test-vm.sh` reports which boot images were found |
| Installer stops at a prompt | serial log; the kernel parameter may not have been patched (`build-iso.sh` fails loudly if no boot entry matched) |
| Installer fails during late-commands | `/var/log/installer/` on the installed system (`/target`), and `/var/log/installer/autoinstall-user-data` |
| cloud-init does not find the seed | the ISO must contain `/nocloud/user-data` + `meta-data`, and the kernel line must have `autoinstall "ds=nocloud;s=/cdrom/nocloud/"` |
| Install finishes but no first boot | check `journalctl -b -u hs-firstboot` from the local console; SSH is disabled by default |
| Portal certificate warning | import `/var/lib/homecore/caddy-root-ca.crt` on the client, or `curl -k` for tests |
| Kiosk black screen | `journalctl -u hs-kiosk`, `ls /dev/dri`, cage/chromium presence; `hs-doctor.sh` |
| No daemon API after install | ISO was built without `--binaries`; rebuild or install `hs-daemon` and `systemctl start hs-daemon` |
| Reproducibility differences | set `SOURCE_DATE_EPOCH`; compare manifests, not just the ISO hash |

## Security notes

- There is **no default console password**. `user-data` contains an explicit
  placeholder and `iso/build-iso.sh` refuses to build without
  `HS_CONSOLE_PASSWORD_HASH` set to a SHA-512 crypt hash. Generate it with
  `openssl passwd -6 'a strong unique password'`.
- SSH is not installed; if you enable it, use `allow-pw: false` and keys.
- No secrets are embedded in the ISO: tunnel credentials, tokens and keys live
  in the encrypted vault created on first boot, or are generated on the
  appliance.
- The ISO itself is not signed by this repository. Production fleets should
  sign the output artifact with an internal key and verify the manifest hash
  before flashing.

## Executable bits

The repository may be checked out on Windows, where the executable bit is not
preserved. Both `iso/build-iso.sh` (when staging the overlay) and
`scripts/install.sh` (when installing) explicitly `chmod 0755` the scripts.
On a Linux checkout, `chmod +x iso/build-iso.sh iso/test-vm.sh scripts/install.sh
infra/systemd/hs-power-policy.sh iso/rootfs-overlay/usr/local/sbin/*.sh
iso/rootfs-overlay/usr/local/bin/*` is enough.
