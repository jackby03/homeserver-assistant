# ISO build reference

Technical reference for building, testing and customising the HomeServer
Assistant OS installation ISO (Ubuntu 24.04 LTS server, amd64). The quick
operator flow lives in [`iso/README.md`](../iso/README.md); this document is the
canonical detail for maintainers and CI.

> **Distribution status:** the ISO is **not published yet**. Builds are for
> development and hardware validation. Stable ISOs will be published on
> SourceForge with SHA256, manifest and signatures once the stability gate in
> [`PUBLISHING.md`](PUBLISHING.md) is met. GitHub releases remain source-only.

---

## 1. Artefacts

| Artefact | Built by | Content |
| --- | --- | --- |
| `iso/output/homeserver-assistant-YYYYMMDD.iso` | `iso/build-iso.sh` | unattended installer (BIOS + UEFI) |
| `iso/output/....manifest.json` | `iso/build-iso.sh` | base ISO name/hash, output hash, git commit, flags, xorriso version |
| `iso/work/` | build/test | download cache, extraction tree, QEMU disk and serial logs (git-ignored) |
| `iso/work/vm-test/` | `iso/test-vm.sh` | qcow2, serial logs, screenshot (git-ignored) |

The ISO contains the autoinstall seed (`/nocloud`), the repository assets
(`/hs/repo`), optional binaries (`/hs/bin`) and optional UI (`/hs/ui`). See
[`iso/README.md`](../iso/README.md) for the exact layout.

---

## 2. Prerequisites

### 2.1 Build host

Linux x86_64 (Debian/Ubuntu recommended; WSL2 works for building but not for
KVM testing). macOS/Windows can only produce the ISO if `xorriso`, `curl`,
`sha256sum` and GNU `sed` are available; the QEMU test path assumes Linux.

```bash
sudo apt install xorriso curl coreutils gawk genisoimage \
                 qemu-system-x86 qemu-utils socat ovmf
```

Optional tooling:

| Tool | Needed for |
| --- | --- |
| `cloud-image-utils` | `cloud-localds` (alternative external seed ISO, see §5) |
| `netpbm` | converting the QEMU screenshot `.ppm` to `.png` |
| `shellcheck` | linting the scripts before changing them |
| Rust 1.85+ | building `hs-daemon` / `hs-vision` to embed |
| Node 22 + pnpm | building `ui/dist` to embed |

`iso/build-iso.sh` checks for `xorriso`, `sha256sum`, `dd`, `sed`, `awk`,
`grep` and `curl`/`wget` and fails with an actionable message when missing.

### 2.2 Disk and network

- ~4 GiB for the base ISO download + ~4 GiB for the extraction tree (the build
  warns when less than 12 GiB is free).
- Network access to `https://releases.ubuntu.com/24.04/` for the base ISO, or
  a local mirror via `--base-url`, or a local file via `--iso`.
- Package installation inside the installer requires network access to the
  Ubuntu archive. For air-gapped builds, mirror the archive and point the
  autoinstall `apt:` section at it (§5.2).

### 2.3 Base ISO verification

`build-iso.sh` never trusts a downloaded ISO:

1. downloads `SHA256SUMS` from the same base URL;
2. selects the newest `ubuntu-24.04*-live-server-amd64.iso` entry (or
   `--iso-name NAME`);
3. downloads it (or reuses the cached file);
4. compares its SHA256 with the upstream value and aborts on mismatch.

The verified base name and hash are recorded in the manifest. Local ISOs
(`--iso PATH`) are hashed and reported, but the operator is responsible for
verifying them out-of-band.

---

## 3. Build commands

```bash
# The console password is mandatory and never committed.
# Generate a hash once: openssl passwd -6 'a strong unique password'
export HS_CONSOLE_PASSWORD_HASH='$6$...'

# Full build with daemon binaries and UI embedded (recommended).
cargo build --release -p hs-daemon -p hs-vision
(cd ui && pnpm install --frozen-lockfile && pnpm build)
./iso/build-iso.sh --binaries target/release --ui-dir ui/dist

# Build only the OS layer (no daemon yet, will warn).
./iso/build-iso.sh

# Offline / pinned base ISO.
./iso/build-iso.sh --iso /srv/isos/ubuntu-24.04.3-live-server-amd64.iso \
                   --binaries target/release --ui-dir ui/dist

# Pin a specific download and output location.
./iso/build-iso.sh --iso-name ubuntu-24.04.3-live-server-amd64.iso \
                   --output /srv/artifacts/hs-24.04.3.iso

# Clean the work/output trees (verified paths only) and exit.
./iso/build-iso.sh --clean

# Build then smoke-test in QEMU.
./iso/build-iso.sh --binaries target/release --ui-dir ui/dist --test
```

Exit codes: `0` success, `1` any failure (missing tool, checksum mismatch, boot
patch failure, xorriso failure, failed `--test`). No step is skipped silently.

### 3.1 What `build-iso.sh` does (step by step)

1. **Resolve + verify** the base ISO (§2.3).
2. **Extract** with `xorriso -indev BASE -osirrox on -extract / iso-root`;
   `chmod -R u+w` because extracted files are read-only.
3. **Inject**:
   - `iso/autoinstall/user-data` (with `__CONSOLE_PASSWORD_HASH__` replaced from
    `HS_CONSOLE_PASSWORD_HASH`) + `meta-data` → `/nocloud/`;
   - `infra/`, `scripts/`, `docs/`, `iso/autoinstall`, `iso/rootfs-overlay` →
     `/hs/repo/`;
   - `--binaries` → `/hs/bin/` (mode 0755);
   - `--ui-dir` → `/hs/ui/` (mode 0644);
   - restores `0755` on all `*.sh`, including the rootfs overlay scripts.
4. **Patch boot** entries in `boot/grub/grub.cfg`,
   `boot/grub/loopback.cfg` and (when present) `isolinux/txt.cfg`:
   append `autoinstall "ds=nocloud;s=/cdrom/nocloud/"` to every kernel command
   line. The quotes are required: GRUB would otherwise end the statement at the
   first `;`. If no entry matches, the build fails instead of producing an
   interactive ISO.
5. **Repack** with `xorriso -as mkisofs`:
   - volume ID `HS_ASSISTANT_24_04`, Rock Ridge/Joliet, ISO level 3;
   - BIOS: `isolinux` El Torito (`-b isolinux/isolinux.bin -c
     isolinux/boot.cat`) or GRUB2 `boot/grub/i386-pc/eltorito.img`;
   - UEFI: `boot/grub/efi.img` with `-eltorito-alt-boot`,
     `-isohybrid-gpt-basdat`;
   - hybrid MBR: first 432 bytes extracted from the verified base ISO, so no
     `isolinux` package is needed;
   - the boot catalog is regenerated rather than copied.
6. **Write the manifest** with the base/output hashes and build metadata.
7. Optional `--test` delegates to `iso/test-vm.sh`.

`--keep-work` preserves `iso/work/iso-root` for inspecting exactly what was
packed; the default removes it after a successful build.

---

## 4. Autoinstall details

The seed is a NoCloud cloud-init datasource:

```
ISO ├── nocloud/user-data       #cloud-config + autoinstall: version 1
    └── nocloud/meta-data       instance-id / local-hostname
kernel command line: autoinstall "ds=nocloud;s=/cdrom/nocloud/"
```

`subiquity` reads `/cdrom/nocloud/user-data`, switches to automated mode
because of the `autoinstall` kernel parameter, and runs the sections in
`autoinstall:`. `interactive-sections` is empty: nothing prompts.

Sections and rationale are tabulated in
[`iso/README.md`](../iso/README.md#autoinstall-design). Key points:

- **Storage**: `layout: direct` (single ext4 root). A 4 GiB swap file and an
  fstab entry are created in `late-commands` because curtin's storage model has
  no swap-file type. Btrfs is documented as a commented alternative in
  `user-data`.
- **Identity**: `homeserver` / example hash for "ubuntu" - **change it** with
  `openssl passwd -6` before building production images.
- **SSH**: `install-server: false`; the appliance is managed through the portal
  and the local console. Enable with `allow-pw: false` + `authorized-keys`.
- **Late commands** (run in the installer environment, writing into `/target`):
  1. `scripts/install.sh --target /target --source /cdrom/hs/repo --binaries /cdrom/hs/bin`
     installs the overlay, units, firewall/Caddy/preseed, enables services and
     stages `/opt/homeserver-assistant`;
  2. swap file + fstab;
  3. `GRUB_CMDLINE_LINUX_DEFAULT="quiet consoleblank=0"` + `update-grub`;
  4. serial marker `HomeServer Assistant install complete` (used by
     `test-vm.sh`);
  5. ensure `/var/lib/homecore` exists (0700) and enable `hs-firstboot`.
- `shutdown: reboot`: the machine restarts into the installed system.

### 4.1 Seed variants

| Variant | How |
| --- | --- |
| Embedded seed (default) | `/nocloud/` + kernel parameter, single ISO |
| External seed ISO | `cloud-localds seed.iso iso/autoinstall/user-data iso/autoinstall/meta-data`, attach as a second CD-ROM in QEMU; the kernel parameter `ds=nocloud` finds the volume label `cidata`. Useful when the base ISO must stay untouched. |
| HTTP seed | `ds=nocloud;s=http://host/autoinstall/` for lab PXE-like flows; not supported by the build script. |

Keep the embedded path for appliances: one file to flash, no separate seed.

---

## 5. Customisation

### 5.1 Package set and services

Edit the `packages:` list in `iso/autoinstall/user-data`. The rationale table
in [`iso/README.md`](../iso/README.md#package-list-rationale) explains why each
package is present; adding compilers/Docker is explicitly out of scope (the
appliance deploys Incus containers).

To change enabled services, edit `scripts/install.sh` (the `enable_unit` list)
and/or `iso/rootfs-overlay/`.

### 5.2 Offline / mirrored installs

Add to `user-data`:

```yaml
apt:
  primary:
    - arches: [amd64]
      uri: http://mirror.lan/ubuntu
  geoip: false
```

and, for air-gapped sites, pre-seed Chromium's snap in the mirror
(`snap download chromium` + a local snap store) because Ubuntu 24.04 installs
Chromium as a snap.

### 5.3 Kernel parameters and kiosk behaviour

- Boot parameters are appended by `KERNEL_EXTRA` in `iso/build-iso.sh` and by
  the install late-command (both already set `consoleblank=0`).
- The kiosk launcher is
  `iso/rootfs-overlay/usr/local/bin/hs-kiosk-start`; it opens exactly
  `http://127.0.0.1:3000/kiosk` (the daemon's kiosk route) and waits for that
  endpoint.
- Chromium managed policy:
  `iso/rootfs-overlay/etc/chromium/policies/managed/homeserver.json`.
- Wayland session: `infra/systemd/hs-kiosk.service` (`cage` + `PAMName=login`
  + `tty1`); logind autologin is not used, the PAM session provides the seat.

### 5.4 Adding files to the installed system

Two options, in order of preference:

1. **Normal host configuration** → put it under `infra/` and install it from
   `scripts/install.sh` (applies to any Debian/Ubuntu host, testable outside
   the ISO).
2. **ISO-specific file** → add it under `iso/rootfs-overlay/` mirroring the
   target path (e.g. `etc/foo.conf` → `/etc/foo.conf`). The overlay is copied
   verbatim; keep it minimal.

Never add secrets to either tree.

---

## 6. QEMU test matrix and assertions

`iso/test-vm.sh`:

| Phase | What happens | Assertions |
| --- | --- | --- |
| install | boot ISO, `-no-reboot`, timeout 2400 s | serial log contains `HomeServer Assistant install complete` |
| smoke | boot installed disk, host-forward 8443 | `https://homeserver.local:8443/healthz` = 200; `/api/v1/setup/status` = HTTP 200 and `"initialized": false`; screenshot captured when `socat` exists |

Matrix to run before a release:

| # | Scenario | Command | Notes |
| --- | --- | --- | --- |
| 1 | BIOS + KVM | `test-vm.sh --iso out.iso` | default; fastest |
| 2 | UEFI + KVM | same, with `ovmf` installed | auto-detects OVMF; exercises the EFI image |
| 3 | TCG (no KVM) | same on a host without `/dev/kvm` | slow (~30-60 min); checks CPU-feature independence |
| 4 | No daemon embedded | `test-vm.sh --iso out.iso --allow-missing-daemon` | asserts OS + Caddy only |
| 5 | Non-default sizing | `--cpus 2 --memory 2048 --disk-size 20G` | low-end hardware floor |
| 6 | UEFI Secure Boot off | covered by 2 | Secure Boot is out of scope (unsigned kernels) |

Resource defaults: 4 vCPU, 4 GiB RAM, 40 GiB qcow2, `-machine q35`,
virtio disk/net/rng, `-display none`, serial to file. `--keep` preserves the
disk and logs for a failed run; artifacts are removed otherwise.

Failures to investigate first:

- `serial-install.log` tail (installer/subiquity errors);
- `/var/log/installer/` inside the installed disk (`/target` during install) -
  mount the qcow2 with `qemu-nbd` if needed;
- `serial-boot.log` for `hs-firstboot`/systemd errors on the installed system.

---

## 7. Reproducibility

| Aspect | Guarantee |
| --- | --- |
| base ISO | SHA256 verified against upstream `SHA256SUMS`, recorded in the manifest |
| staged tree | deterministic copy of the repository at the recorded git commit |
| file dates | fixed when `SOURCE_DATE_EPOCH` is set (`-volume_date all_file_dates`) |
| ISO bytes | **not guaranteed** bit-identical: xorriso writes volume UUID/timestamps per run |
| manifest | identical except `built_at_utc` for identical inputs |

Recommended release procedure:

```bash
export SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)"
./iso/build-iso.sh --binaries target/release --ui-dir ui/dist
sha256sum iso/output/homeserver-assistant-*.iso
# sign the manifest hash with the release key, publish ISO + manifest + signature
```

To make builds closer to bit-reproducible, run the build twice and compare the
manifests and extracted trees (`xorriso -indev out.iso -osirrox on -extract /`),
not only the ISO hash.

---

## 8. Troubleshooting

### Build

| Symptom | Cause / fix |
| --- | --- |
| `required command not found: xorriso` | `sudo apt install xorriso` |
| SHA256 mismatch | corrupt/mirror-mismatched download; the script deletes the file and aborts. Retry, or use `--iso` with a verified local copy |
| no `ubuntu-24.04*` entry in SHA256SUMS | the base URL is wrong or the point release was rotated; use `--base-url`/`--iso-name` |
| `unexpected ISO layout: no /casper` | the input is not a live-server ISO |
| `could not find a kernel command line to patch` | the ISO layout changed; inspect `iso/work/iso-root/boot/grub/grub.cfg` (keep `--keep-work`) and adapt the `patch_boot` patterns |
| xorriso repack failed | run the printed command with `-as mkisofs -help`; verify the extracted tree is writable (`chmod -R u+w`) |
| ISO boots but shows the interactive installer | kernel parameter not applied: check `cat /proc/cmdline` in the installer console (Alt-F2 console) |
| ISO does not boot at all | BIOS/EFI boot images were not found; the build logs which ones it used; UEFI-only build on a BIOS VM will not boot |

### Installer

| Symptom | Check |
| --- | --- |
| subiquity asks questions | seed not found or `autoinstall` missing from kernel cmdline; see §4 |
| install fails in late-commands | installer log at `/var/log/installer/` (after failure, Alt-F2 shell) and `/var/log/installer/autoinstall-user-data` |
| package install fails | network/DNS in the installer; Ethernet link on `eth0`; `apt:` mirror override (§5.2) |
| `/target` partially configured | re-run `bash /cdrom/hs/repo/scripts/install.sh --target /target --source /cdrom/hs/repo` from the installer shell |

### First boot (installed system)

| Symptom | Check |
| --- | --- |
| no portal | `journalctl -u hs-firstboot -b`, `journalctl -u hs-daemon -b`, `curl -k https://localhost:8443/healthz` |
| CA warning | re-run `caddy trust` / re-import `/var/lib/homecore/caddy-root-ca.crt` |
| Incus preseed failed | `journalctl -u hs-firstboot -b | grep -i incus`; run `incus admin init --preseed < /opt/homeserver-assistant/infra/incus-preseed.yaml` manually |
| kiosk black/blank | `journalctl -u hs-kiosk -b`, `ls -l /dev/dri`, `command -v cage chromium`, `loginctl session-status` |
| first boot repeats every boot | `/var/lib/homecore/.firstboot-done` missing/permission problem; the unit skips when present |
| no `hs-daemon` binary | ISO built without `--binaries`; install it and `systemctl start hs-daemon` |

### Cloud-init seed

- The seed must be readable at install time: files named exactly
  `user-data` and `meta-data` in `/nocloud/`; `meta-data` must contain
  `instance-id`.
- `#cloud-config` must be the first line of `user-data`; a leading blank line
  makes cloud-init ignore the file.
- Validate the YAML before building: `python3 -c "import yaml,sys; yaml.safe_load(open('iso/autoinstall/user-data'))"`
  (the file starts with `#cloud-config`, which is a comment for YAML, so this
  parses cleanly).
- On the installed system, the installer's copy of the seed is kept under
  `/var/log/installer/` for forensic comparison.

---

## 9. Maintenance checklist (when changing the ISO)

1. Update `iso/autoinstall/user-data` and the tables in
   `iso/README.md`/`docs/ISO.md` together.
2. Update `infra/` units/assets rather than the overlay when the file also
   belongs on a non-ISO host.
3. Bump the pinned Caddy/Chromium/Incus expectations in
   `infra/services/*/docker-compose.yml` separately from the ISO.
4. Run the static checks that do not need a Linux shell:
   `python3 scripts/check-assets.py` (YAML/JSON parsing, systemd units,
   nftables invariants, autoinstall seed, secrets/Windows-path scan) and
   `shellcheck -s bash iso/*.sh scripts/*.sh infra/systemd/hs-power-policy.sh`
   plus the overlay scripts. `bash -n` is only available on a Unix-like host;
   CI must run it there.
5. Build with `SOURCE_DATE_EPOCH`, run the QEMU matrix (§6) and keep the
   manifest with the artifact.
