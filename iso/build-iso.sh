#!/usr/bin/env bash
# =============================================================================
# HomeServer Assistant OS - build a reproducible Ubuntu 24.04 LTS server ISO
# =============================================================================
# Produces an unattended installation ISO:
#
#   ubuntu-24.04.x-live-server-amd64.iso  (verified against SHA256SUMS)
#     + /nocloud/{user-data,meta-data}    (autoinstall seed)
#     + /hs/repo/                         (infra, scripts, docs, iso assets)
#     + /hs/bin/                          (optional hs-daemon / hs-vision)
#     + /hs/ui/                           (optional built React UI)
#     + patched boot entries:
#         autoinstall "ds=nocloud;s=/cdrom/nocloud/"
#
# The original ISO is extracted, modified and repacked with xorriso, preserving
# the El Torito BIOS and EFI boot images. No root privileges are required.
#
# Examples:
#   ./iso/build-iso.sh                                  # download, verify, build
#   ./iso/build-iso.sh --iso ~/isos/ubuntu-24.04.3-live-server-amd64.iso
#   ./iso/build-iso.sh --binaries target/release --ui-dir ui/dist --test
#   ./iso/build-iso.sh --clean
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
REPO_ROOT="$(dirname -- "$SCRIPT_DIR")"
readonly REPO_ROOT
readonly DEFAULT_BASE_URL="https://releases.ubuntu.com/24.04/"
readonly VOLUME_ID="HS_ASSISTANT_24_04"
# Quoting matters: GRUB treats an unquoted semicolon as the end of a statement.
readonly KERNEL_EXTRA='autoinstall "ds=nocloud;s=/cdrom/nocloud/"'
readonly ISO_PATTERN='^ubuntu-24\.04(\.[0-9]+)?-live-server-amd64\.iso$'

WORK_DIR="$SCRIPT_DIR/work"
OUTPUT_DIR="$SCRIPT_DIR/output"
BASE_ISO=""
BASE_URL="$DEFAULT_BASE_URL"
OUTPUT_ISO=""
BINARIES_DIR=""
UI_DIR=""
DO_TEST=0

log() { printf '[build-iso] %s\n' "$*"; }
warn() { printf '[build-iso] WARNING: %s\n' "$*" >&2; }
die() {
    printf '[build-iso] ERROR: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Usage: build-iso.sh [OPTIONS]

Options:
  --iso PATH        Use a local Ubuntu 24.04 live-server AMD64 ISO instead of
                    downloading one. Its SHA256 is reported in the manifest.
  --iso-name NAME   Exact ISO filename to download (default: newest matching
                    ubuntu-24.04*-live-server-amd64.iso from SHA256SUMS).
  --base-url URL    Mirror base URL for the ISO + SHA256SUMS
                    (default: https://releases.ubuntu.com/24.04/).
  --output PATH     Output ISO path (default: iso/output/homeserver-assistant-<date>.iso).
  --binaries DIR    Directory with prebuilt hs-daemon and hs-vision binaries.
  --ui-dir DIR      Directory with the built UI (ui/dist).
  --work DIR        Work directory (default: iso/work).
  --test            Run iso/test-vm.sh against the built ISO after building.
  --clean           Remove the work and output directories, then exit.
  --keep-work       Keep the extracted/modified ISO tree (debugging).
  -h, --help        Show this help.

Dependencies: xorriso, coreutils (sha256sum, dd), sed/awk/grep, curl or wget,
               optionally qemu-system-x86_64 + qemu-img + socat (for --test).
EOF
}

CLEAN=0
KEEP_WORK=0
ISO_NAME=""

while (($# > 0)); do
    case "$1" in
        --iso) [[ $# -ge 2 ]] || die "--iso needs a value"; BASE_ISO="$2"; shift 2 ;;
        --iso-name) [[ $# -ge 2 ]] || die "--iso-name needs a value"; ISO_NAME="$2"; shift 2 ;;
        --base-url) [[ $# -ge 2 ]] || die "--base-url needs a value"; BASE_URL="$2"; shift 2 ;;
        --output) [[ $# -ge 2 ]] || die "--output needs a value"; OUTPUT_ISO="$2"; shift 2 ;;
        --binaries) [[ $# -ge 2 ]] || die "--binaries needs a value"; BINARIES_DIR="$2"; shift 2 ;;
        --ui-dir) [[ $# -ge 2 ]] || die "--ui-dir needs a value"; UI_DIR="$2"; shift 2 ;;
        --work) [[ $# -ge 2 ]] || die "--work needs a value"; WORK_DIR="$2"; shift 2 ;;
        --test) DO_TEST=1; shift ;;
        --clean) CLEAN=1; shift ;;
        --keep-work) KEEP_WORK=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown argument: $1 (try --help)" ;;
    esac
done

require() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1 ($2)"
}

require xorriso "apt install xorriso"
require sha256sum "coreutils"
require dd "coreutils"
require sed "sed"
require awk "gawk/mawk"
require grep "grep"
if ! command -v curl >/dev/null 2>&1 && ! command -v wget >/dev/null 2>&1; then
    die "either curl or wget is required to download the base ISO"
fi
require sha256sum "coreutils"

fetch() {
    local url="$1" dest="$2"
    log "downloading ${url}"
    if command -v curl >/dev/null 2>&1; then
        curl -fL --retry 3 --connect-timeout 20 --progress-bar -o "$dest" "$url"
    else
        wget -q --show-progress -O "$dest" "$url"
    fi
}

# Remove the work/output trees after verifying the paths are the expected ones.
clean() {
    local abs_work abs_output
    abs_work="$(cd -- "$(dirname -- "$WORK_DIR")" && pwd)/$(basename -- "$WORK_DIR")"
    abs_output="$(cd -- "$(dirname -- "$OUTPUT_DIR")" && pwd)/$(basename -- "$OUTPUT_DIR")"
    if [[ -d "$abs_work" && "$abs_work" == "$SCRIPT_DIR/work" ]]; then
        log "removing work directory ${abs_work}"
        rm -rf -- "$abs_work"
    elif [[ -d "$abs_work" ]]; then
        die "refusing to remove unexpected work directory: ${abs_work}"
    fi
    if [[ -d "$abs_output" && "$abs_output" == "$SCRIPT_DIR/output" ]]; then
        log "removing output directory ${abs_output}"
        rm -rf -- "$abs_output"
    elif [[ -d "$abs_output" ]]; then
        die "refusing to remove unexpected output directory: ${abs_output}"
    fi
}

if ((CLEAN == 1)); then
    clean
    log "done"
    exit 0
fi

mkdir -p "$WORK_DIR" "$OUTPUT_DIR"

ISO_ROOT="$WORK_DIR/iso-root"
SEED_DIR="$ISO_ROOT/nocloud"
mkdir -p "$ISO_ROOT"

# ---------------------------------------------------------------------------
# 1. Resolve and verify the base ISO
# ---------------------------------------------------------------------------
resolve_base_iso() {
    if [[ -n "$BASE_ISO" ]]; then
        [[ -f "$BASE_ISO" ]] || die "base ISO not found: $BASE_ISO"
        BASE_ISO="$(cd -- "$(dirname -- "$BASE_ISO")" && pwd)/$(basename -- "$BASE_ISO")"
        BASE_ISO_SHA="$(sha256sum "$BASE_ISO" | awk '{print $1}')"
        log "using local base ISO: $BASE_ISO"
        log "sha256: $BASE_ISO_SHA"
        return 0
    fi

    local sha_file="$WORK_DIR/SHA256SUMS"
    fetch "${BASE_URL%/}/SHA256SUMS" "$sha_file"

    if [[ -z "$ISO_NAME" ]]; then
        # Newest point release matching the pattern (sort -V handles 24.04.10).
        ISO_NAME="$(awk '{f=$2; sub(/^\*/,"",f); print f}' "$sha_file" \
            | grep -E "$ISO_PATTERN" | sort -V | tail -n1)"
    fi
    [[ -n "$ISO_NAME" ]] || die "no ubuntu-24.04*-live-server-amd64.iso found in ${BASE_URL%/}/SHA256SUMS"

    BASE_ISO="$WORK_DIR/$ISO_NAME"
    if [[ ! -f "$BASE_ISO" ]]; then
        fetch "${BASE_URL%/}/$ISO_NAME" "$BASE_ISO"
    else
        log "reusing cached base ISO: $BASE_ISO"
    fi

    local expected actual
    expected="$(awk -v name="$ISO_NAME" '{f=$2; sub(/^\*/,"",f); if (f==name) {print $1; exit}}' "$sha_file")"
    [[ -n "$expected" ]] || die "no checksum for $ISO_NAME in SHA256SUMS"
    actual="$(sha256sum "$BASE_ISO" | awk '{print $1}')"
    if [[ "$actual" != "$expected" ]]; then
        rm -f "$BASE_ISO"
        die "SHA256 mismatch for $ISO_NAME (expected $expected, got $actual); refusing to build"
    fi
    BASE_ISO_SHA="$actual"
    log "base ISO verified: $ISO_NAME ($BASE_ISO_SHA)"
}

# ---------------------------------------------------------------------------
# 2. Extract the base ISO
# ---------------------------------------------------------------------------
extract_iso() {
    log "extracting $BASE_ISO into $ISO_ROOT"
    rm -rf -- "$ISO_ROOT"
    mkdir -p "$ISO_ROOT"
    # -osirrox on enables the extraction commands; -extract copies the tree.
    xorriso -indev "$BASE_ISO" -osirrox on -extract / "$ISO_ROOT" >/dev/null 2>&1 \
        || die "xorriso extraction failed"
    # Extracted files are read-only; the boot config patching needs write access.
    chmod -R u+w "$ISO_ROOT"
    [[ -d "$ISO_ROOT/casper" ]] || die "unexpected ISO layout: no /casper directory"
}

# ---------------------------------------------------------------------------
# 3. Inject the autoinstall seed, repository staging, binaries and UI
# ---------------------------------------------------------------------------
inject_content() {
    log "injecting autoinstall seed"
    mkdir -p "$SEED_DIR"
    install -m 0644 "$SCRIPT_DIR/autoinstall/user-data" "$SEED_DIR/user-data"
    install -m 0644 "$SCRIPT_DIR/autoinstall/meta-data" "$SEED_DIR/meta-data"

    log "staging repository assets under /hs/repo"
    mkdir -p "$ISO_ROOT/hs/repo"
    local dir
    for dir in infra scripts docs; do
        [[ -d "$REPO_ROOT/$dir" ]] && cp -a "$REPO_ROOT/$dir" "$ISO_ROOT/hs/repo/"
    done
    mkdir -p "$ISO_ROOT/hs/repo/iso"
    cp -a "$SCRIPT_DIR/autoinstall" "$ISO_ROOT/hs/repo/iso/"
    cp -a "$SCRIPT_DIR/rootfs-overlay" "$ISO_ROOT/hs/repo/iso/"
    [[ -f "$REPO_ROOT/LICENSE" ]] && install -m 0644 "$REPO_ROOT/LICENSE" "$ISO_ROOT/hs/repo/LICENSE"

    # Windows checkouts / archives lose the execute bit: restore it before the
    # late-commands run install.sh from the mounted ISO.
    find "$ISO_ROOT/hs/repo" -type f -name '*.sh' -exec chmod 0755 {} +
    find "$ISO_ROOT/hs/repo/iso/rootfs-overlay/usr/local" -type f -exec chmod 0755 {} + 2>/dev/null || true

    if [[ -n "$BINARIES_DIR" ]]; then
        [[ -d "$BINARIES_DIR" ]] || die "--binaries directory not found: $BINARIES_DIR"
        log "staging binaries from $BINARIES_DIR"
        mkdir -p "$ISO_ROOT/hs/bin"
        local binary found=0
        for binary in hs-daemon hs-vision; do
            if [[ -f "$BINARIES_DIR/$binary" ]]; then
                install -m 0755 "$BINARIES_DIR/$binary" "$ISO_ROOT/hs/bin/$binary"
                found=1
            else
                warn "binary missing: $BINARIES_DIR/$binary"
            fi
        done
        ((found == 1)) || warn "no appliance binaries staged: hs-daemon will not start until installed"
    else
        warn "no --binaries given: the ISO installs the OS but not hs-daemon/hs-vision"
        warn "build them on Linux: cargo build --release -p hs-daemon -p hs-vision"
        warn "and rebuild with: --binaries target/release"
    fi

    if [[ -n "$UI_DIR" ]]; then
        [[ -d "$UI_DIR" ]] || die "--ui-dir directory not found: $UI_DIR"
        log "staging UI from $UI_DIR"
        mkdir -p "$ISO_ROOT/hs/ui"
        cp -a "$UI_DIR/." "$ISO_ROOT/hs/ui/"
        find "$ISO_ROOT/hs/ui" -type f -exec chmod 0644 {} +
    else
        warn "no --ui-dir given: the portal UI will be missing until installed"
    fi
}

# ---------------------------------------------------------------------------
# 4. Patch the boot entries with the autoinstall kernel parameter
# ---------------------------------------------------------------------------
patch_boot() {
    log "patching boot entries with: ${KERNEL_EXTRA}"
    local patched=0 file

    # GRUB menu (UEFI boot and most BIOS paths on 24.04).
    for file in "$ISO_ROOT/boot/grub/grub.cfg" "$ISO_ROOT/boot/grub/loopback.cfg"; do
        [[ -f "$file" ]] || continue
        chmod u+w "$file"
        if grep -qE '^[[:space:]]*(linux|linuxefi)[[:space:]]+.*casper/vmlinuz' "$file"; then
            sed -i -E "s|^([[:space:]]*(linux|linuxefi)[[:space:]]+.*casper/vmlinuz.*)\$|\\1 ${KERNEL_EXTRA}|" "$file"
            log "patched ${file#"$ISO_ROOT"/}"
            patched=1
        fi
    done

    # isolinux (older/BIOS fallback images).
    for file in "$ISO_ROOT/isolinux/txt.cfg" "$ISO_ROOT/isolinux/isolinux.cfg"; do
        [[ -f "$file" ]] || continue
        chmod u+w "$file"
        if grep -qE '^[[:space:]]*append[[:space:]]' "$file"; then
            sed -i -E "s|^([[:space:]]*append[[:space:]].*)\$|\\1 ${KERNEL_EXTRA}|" "$file"
            log "patched ${file#"$ISO_ROOT"/}"
            patched=1
        fi
    done

    ((patched == 1)) || die "could not find a kernel command line to patch; the ISO layout may have changed (report this)"
}

# ---------------------------------------------------------------------------
# 5. Repack with xorriso, preserving BIOS + EFI El Torito boot images
# ---------------------------------------------------------------------------
repack_iso() {
    if [[ -z "$OUTPUT_ISO" ]]; then
        OUTPUT_ISO="$OUTPUT_DIR/homeserver-assistant-$(date -u +%Y%m%d).iso"
    fi
    mkdir -p "$(dirname -- "$OUTPUT_ISO")"

    local mbr_file="$WORK_DIR/isohdpfx.bin"
    local -a mbr_args=() bios_args=() efi_args=()

    # Hybrid MBR boot code: the first 432 bytes of the Ubuntu hybrid ISO.
    # Extracted from the verified base ISO so no isolinux package is required.
    dd if="$BASE_ISO" of="$mbr_file" bs=1 count=432 status=none \
        || die "could not extract the hybrid MBR from the base ISO"

    if [[ -f "$ISO_ROOT/isolinux/isolinux.bin" && -f "$ISO_ROOT/isolinux/boot.cat" ]]; then
        # Regenerate the boot catalog instead of shipping the stale one.
        rm -f "$ISO_ROOT/isolinux/boot.cat"
        mbr_args=(-isohybrid-mbr "$mbr_file")
        bios_args=(-c isolinux/boot.cat -b isolinux/isolinux.bin -no-emul-boot -boot-load-size 4 -boot-info-table)
        log "BIOS boot: isolinux"
    elif [[ -f "$ISO_ROOT/boot/grub/i386-pc/eltorito.img" ]]; then
        mbr_args=(-isohybrid-mbr "$mbr_file")
        bios_args=(-b boot/grub/i386-pc/eltorito.img -no-emul-boot -boot-load-size 4 -boot-info-table --grub2-boot-info)
        log "BIOS boot: grub2 eltorito.img"
    else
        warn "no BIOS boot image found: the ISO will be UEFI-only"
    fi

    # EFI System Partition image. `-eltorito-alt-boot` only makes sense when a
    # previous El Torito (BIOS) entry exists.
    local efi_img="" candidate
    for candidate in "$ISO_ROOT/boot/grub/efi.img" "$ISO_ROOT/efi.img"; do
        if [[ -f "$candidate" ]]; then
            efi_img="${candidate#"$ISO_ROOT"/}"
            break
        fi
    done
    if [[ -n "$efi_img" ]]; then
        if ((${#bios_args[@]} > 0)); then
            efi_args=(-eltorito-alt-boot -e "$efi_img" -no-emul-boot -isohybrid-gpt-basdat)
        else
            efi_args=(-e "$efi_img" -no-emul-boot -isohybrid-gpt-basdat)
        fi
        log "EFI boot: ${efi_img}"
    else
        warn "no EFI boot image found: the ISO will be BIOS-only"
    fi
    [[ ${#bios_args[@]} -gt 0 || ${#efi_args[@]} -gt 0 ]] || die "no boot image found; refusing to build an unbootable ISO"

    # Reproducibility: fix the file dates when SOURCE_DATE_EPOCH is provided.
    local -a repro_args=()
    if [[ -n "${SOURCE_DATE_EPOCH:-}" ]]; then
        local iso_date
        iso_date="$(date -u -d "@${SOURCE_DATE_EPOCH}" +%Y%m%d%H%M%S00 2>/dev/null || true)"
        if [[ -n "$iso_date" ]]; then
            repro_args=(-volume_date all_file_dates="$iso_date")
            log "reproducible dates from SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}"
        fi
    fi

    log "repacking ISO: $OUTPUT_ISO"
    rm -f "$OUTPUT_ISO"
    xorriso -as mkisofs \
        -r -V "$VOLUME_ID" -J -joliet-long -l -iso-level 3 \
        "${repro_args[@]}" \
        "${mbr_args[@]}" \
        "${bios_args[@]}" \
        "${efi_args[@]}" \
        -o "$OUTPUT_ISO" "$ISO_ROOT" \
        || die "xorriso repack failed"

    [[ -s "$OUTPUT_ISO" ]] || die "xorriso produced an empty ISO"
}

# ---------------------------------------------------------------------------
# 6. Manifest and report
# ---------------------------------------------------------------------------
write_manifest() {
    local output_sha base_name manifest
    output_sha="$(sha256sum "$OUTPUT_ISO" | awk '{print $1}')"
    base_name="$(basename -- "$BASE_ISO")"
    manifest="${OUTPUT_ISO%.iso}.manifest.json"
    local git_commit
    git_commit="$(cd -- "$REPO_ROOT" && git rev-parse --short HEAD 2>/dev/null || echo unknown)"
    local xorriso_version
    xorriso_version="$(xorriso --version 2>/dev/null | head -1 || echo unknown)"

    cat >"$manifest" <<EOF
{
  "product": "HomeServer Assistant OS",
  "built_at_utc": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "git_commit": "${git_commit}",
  "base_iso": "${base_name}",
  "base_iso_sha256": "${BASE_ISO_SHA}",
  "output_iso": "$(basename -- "$OUTPUT_ISO")",
  "output_iso_sha256": "${output_sha}",
  "volume_id": "${VOLUME_ID}",
  "kernel_extra": "${KERNEL_EXTRA}",
  "xorriso": "${xorriso_version}",
  "binaries_staged": $([[ -d "$ISO_ROOT/hs/bin" ]] && echo true || echo false),
  "ui_staged": $([[ -d "$ISO_ROOT/hs/ui" ]] && echo true || echo false),
  "source_date_epoch": "${SOURCE_DATE_EPOCH:-}"
}
EOF

    log "ISO:      $OUTPUT_ISO"
    log "SHA256:   $output_sha"
    log "manifest: $manifest"
    log "flash with: sudo dd if=${OUTPUT_ISO} of=/dev/sdX bs=8M status=progress oflag=sync"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-}"

log "HomeServer Assistant OS ISO build"
log "repo:   $REPO_ROOT"
log "work:   $WORK_DIR"
log "output: $OUTPUT_DIR"

avail_kb="$(df -Pk "$WORK_DIR" | awk 'NR==2 {print $4}')"
if [[ -n "$avail_kb" ]] && ((avail_kb < 12 * 1024 * 1024)); then
    warn "less than 12 GiB free in $WORK_DIR; the build needs ~4 GiB ISO + extracted tree"
fi

resolve_base_iso
extract_iso
inject_content
patch_boot
repack_iso
write_manifest

if ((KEEP_WORK == 0)); then
    log "removing extracted tree (use --keep-work to keep it)"
    rm -rf -- "$ISO_ROOT"
fi

if ((DO_TEST == 1)); then
    [[ -x "$SCRIPT_DIR/test-vm.sh" ]] || die "test-vm.sh is not executable"
    log "running QEMU smoke test"
    "$SCRIPT_DIR/test-vm.sh" --iso "$OUTPUT_ISO"
fi

log "build complete"
