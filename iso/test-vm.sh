#!/usr/bin/env bash
# =============================================================================
# HomeServer Assistant OS - QEMU/KVM smoke test for a built ISO
# =============================================================================
# Two-phase test, fully headless:
#
#   phase 1 (install): boot the ISO with 4 vCPU / 4 GiB / 40 GiB qcow2 and
#     -no-reboot, run the unattended autoinstall, wait for the "HomeServer
#     Assistant install complete" serial marker, then let QEMU exit on reboot.
#   phase 2 (smoke):   boot the installed disk, forward guest tcp/8443 to the
#     host and poll the real wizard endpoint:
#         GET https://homeserver.local:8443/api/v1/setup/status
#     expecting HTTP 200 and `"initialized": false` (fresh appliance).
#     Also checks Caddy /healthz, captures a screenshot through QMP/HMP when
#     socat is available, and prints the serial log path on failure.
#
# Exit code 0 = PASS, 1 = FAIL (all messages are English).
#
# Example:
#   ./iso/test-vm.sh --iso iso/output/homeserver-assistant-20260101.iso
#   ./iso/test-vm.sh --iso ... --keep --allow-missing-daemon
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
readonly DEFAULT_TIMEOUT_INSTALL=2400
readonly DEFAULT_TIMEOUT_BOOT=900

ISO=""
WORK_DIR="$SCRIPT_DIR/work/vm-test"
TIMEOUT_INSTALL="$DEFAULT_TIMEOUT_INSTALL"
TIMEOUT_BOOT="$DEFAULT_TIMEOUT_BOOT"
CPUS=4
MEMORY_MB=4096
DISK_SIZE="40G"
HOST_PORT=8443
ALLOW_MISSING_DAEMON=0
KEEP=0
SCREENSHOT=1

log() { printf '[test-vm] %s\n' "$*"; }
warn() { printf '[test-vm] WARNING: %s\n' "$*" >&2; }
die() {
    printf '[test-vm] ERROR: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Usage: test-vm.sh --iso PATH [OPTIONS]

Options:
  --iso PATH                Built HomeServer Assistant ISO (required).
  --work DIR                Work directory (default: iso/work/vm-test).
  --timeout-install SEC     Install phase timeout (default 2400).
  --timeout-boot SEC        Smoke phase timeout (default 900).
  --cpus N                  vCPUs (default 4).
  --memory MB               RAM in MiB (default 4096).
  --disk-size SIZE          qcow2 disk size (default 40G).
  --host-port PORT          Host port forwarded to guest 8443 (default 8443).
  --allow-missing-daemon    Do not fail when the ISO has no hs-daemon binary
                            (asserts only that the installed system boots).
  --no-screenshot           Skip the QEMU screenshot capture.
  --keep                    Keep the qcow2 disk and logs after the test.
  -h, --help                Show this help.

Dependencies: qemu-system-x86_64, qemu-img, coreutils, curl, timeout (coreutils),
              optionally OVMF firmware and socat (screenshot).
EOF
}

while (($# > 0)); do
    case "$1" in
        --iso) [[ $# -ge 2 ]] || die "--iso needs a value"; ISO="$2"; shift 2 ;;
        --work) [[ $# -ge 2 ]] || die "--work needs a value"; WORK_DIR="$2"; shift 2 ;;
        --timeout-install) [[ $# -ge 2 ]] || die "--timeout-install needs a value"; TIMEOUT_INSTALL="$2"; shift 2 ;;
        --timeout-boot) [[ $# -ge 2 ]] || die "--timeout-boot needs a value"; TIMEOUT_BOOT="$2"; shift 2 ;;
        --cpus) [[ $# -ge 2 ]] || die "--cpus needs a value"; CPUS="$2"; shift 2 ;;
        --memory) [[ $# -ge 2 ]] || die "--memory needs a value"; MEMORY_MB="$2"; shift 2 ;;
        --disk-size) [[ $# -ge 2 ]] || die "--disk-size needs a value"; DISK_SIZE="$2"; shift 2 ;;
        --host-port) [[ $# -ge 2 ]] || die "--host-port needs a value"; HOST_PORT="$2"; shift 2 ;;
        --allow-missing-daemon) ALLOW_MISSING_DAEMON=1; shift ;;
        --no-screenshot) SCREENSHOT=0; shift ;;
        --keep) KEEP=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown argument: $1 (try --help)" ;;
    esac
done

[[ -n "$ISO" ]] || die "--iso is required (build one with iso/build-iso.sh)"
[[ -f "$ISO" ]] || die "ISO not found: $ISO"
command -v qemu-system-x86_64 >/dev/null 2>&1 || die "qemu-system-x86_64 not found (apt install qemu-system-x86)"
command -v qemu-img >/dev/null 2>&1 || die "qemu-img not found (apt install qemu-utils)"
command -v timeout >/dev/null 2>&1 || die "timeout not found (coreutils)"
command -v curl >/dev/null 2>&1 || die "curl not found"

mkdir -p "$WORK_DIR"
WORK_DIR="$(cd -- "$WORK_DIR" && pwd)"
DISK="$WORK_DIR/disk.qcow2"
SERIAL_INSTALL="$WORK_DIR/serial-install.log"
SERIAL_BOOT="$WORK_DIR/serial-boot.log"
MONITOR_SOCK="$WORK_DIR/monitor.sock"
SCREENSHOT_FILE="$WORK_DIR/screen.ppm"
INSTALL_MARKER="HomeServer Assistant install complete"

QEMU_PID=""

# Invoked indirectly through the EXIT trap below.
# shellcheck disable=SC2317
cleanup() {
    if [[ -n "$QEMU_PID" ]] && kill -0 "$QEMU_PID" 2>/dev/null; then
        log "stopping QEMU (pid ${QEMU_PID})"
        kill "$QEMU_PID" 2>/dev/null || true
        wait "$QEMU_PID" 2>/dev/null || true
    fi
    rm -f "$MONITOR_SOCK"
    if ((KEEP == 0)); then
        log "removing work directory ${WORK_DIR} (use --keep to keep it)"
        rm -rf -- "$WORK_DIR"
    else
        log "artifacts kept in ${WORK_DIR}"
    fi
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Acceleration and firmware detection
# ---------------------------------------------------------------------------
ACCEL_ARGS=()
CPU_ARGS=( -cpu max )
if [[ -r /dev/kvm && -w /dev/kvm ]]; then
    ACCEL_ARGS=( -accel kvm )
    CPU_ARGS=( -cpu host )
    log "KVM acceleration enabled"
else
    ACCEL_ARGS=( -accel "tcg,thread=multi" )
    warn "no /dev/kvm: falling back to TCG emulation (the test will be slow)"
fi

FIRMWARE_ARGS=()
OVMF_CODE=""
OVMF_VARS=""
for candidate in \
    /usr/share/OVMF/OVMF_CODE_4M.fd \
    /usr/share/OVMF/OVMF_CODE.fd \
    /usr/share/edk2/ovmf/OVMF_CODE.fd \
    /usr/share/edk2/x64/OVMF_CODE.fd; do
    if [[ -f "$candidate" ]]; then
        OVMF_CODE="$candidate"
        break
    fi
done
if [[ -n "$OVMF_CODE" ]]; then
    for candidate in \
        /usr/share/OVMF/OVMF_VARS_4M.fd \
        /usr/share/OVMF/OVMF_VARS.fd \
        /usr/share/edk2/ovmf/OVMF_VARS.fd \
        /usr/share/edk2/x64/OVMF_VARS.fd; do
        if [[ -f "$candidate" ]]; then
            OVMF_VARS="$candidate"
            break
        fi
    done
    if [[ -n "$OVMF_VARS" ]]; then
        cp "$OVMF_VARS" "$WORK_DIR/OVMF_VARS.fd"
        FIRMWARE_ARGS=(
            "-drive" "if=pflash,format=raw,readonly=on,file=$OVMF_CODE"
            "-drive" "if=pflash,format=raw,file=$WORK_DIR/OVMF_VARS.fd"
        )
        log "using OVMF (UEFI) firmware: ${OVMF_CODE}"
    else
        warn "OVMF CODE found but no VARS template; falling back to BIOS"
    fi
else
    log "OVMF not installed: using SeaBIOS (BIOS boot path)"
fi

# Global array populated by build_qemu_args; bash arrays cannot be returned
# from functions, and `timeout` needs a real executable plus its arguments.
QEMU_ARGS=()

build_qemu_args() {
    local serial_log="$1"
    shift
    QEMU_ARGS=(
        qemu-system-x86_64
        -machine q35
        "${ACCEL_ARGS[@]}" "${CPU_ARGS[@]}"
        -m "$MEMORY_MB" -smp "$CPUS"
        "${FIRMWARE_ARGS[@]}"
        -display none
        -monitor "unix:${MONITOR_SOCK},server,nowait"
        -serial "file:${serial_log}"
        -netdev "user,id=net0,hostfwd=tcp:127.0.0.1:${HOST_PORT}-:8443"
        -device "virtio-net-pci,netdev=net0"
        -device "virtio-rng-pci"
        "$@"
    )
}

# ---------------------------------------------------------------------------
# Phase 1: unattended install
# ---------------------------------------------------------------------------
phase_install() {
    log "phase 1/2: installing from ${ISO} (timeout ${TIMEOUT_INSTALL}s)"
    rm -f "$DISK" "$SERIAL_INSTALL"
    qemu-img create -f qcow2 "$DISK" "$DISK_SIZE" >/dev/null

    local qemu_args=(
        -drive "file=${DISK},if=virtio,format=qcow2,cache=unsafe"
        -cdrom "$ISO"
        -boot order=d
        -no-reboot
    )

    build_qemu_args "$SERIAL_INSTALL" "${qemu_args[@]}"
    local rc=0
    timeout --signal=TERM --kill-after=30 "$TIMEOUT_INSTALL" "${QEMU_ARGS[@]}" || rc=$?

    QEMU_PID=""
    if ((rc == 124)); then
        tail -n 40 "$SERIAL_INSTALL" 2>/dev/null || true
        die "install phase timed out after ${TIMEOUT_INSTALL}s (serial log: ${SERIAL_INSTALL})"
    fi

    if grep -q "$INSTALL_MARKER" "$SERIAL_INSTALL" 2>/dev/null; then
        log "PASS  install serial marker found"
    else
        warn "install serial marker not found; serial log tail:"
        tail -n 40 "$SERIAL_INSTALL" 2>/dev/null || true
        die "the unattended install did not report completion"
    fi
}

# ---------------------------------------------------------------------------
# Phase 2: boot the installed system and assert the wizard endpoint
# ---------------------------------------------------------------------------
# curl options for talking to the guest through the user-mode NAT forward.
# The guest certificate is issued for homeserver.local by the appliance-local
# CA, so the host maps that name to the forwarded port and skips verification
# (the CA is internal and different on every install).
CURL_ARGS=(
    --silent --show-error --fail --insecure --max-time 5
    --resolve "homeserver.local:${HOST_PORT}:127.0.0.1"
)

wait_for_url() {
    local url="$1" deadline=$((SECONDS + TIMEOUT_BOOT))
    while ((SECONDS < deadline)); do
        if curl "${CURL_ARGS[@]}" -o /dev/null "$url"; then
            return 0
        fi
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then
            return 1
        fi
        sleep 5
    done
    return 1
}

capture_screenshot() {
    ((SCREENSHOT == 1)) || return 0
    if ! command -v socat >/dev/null 2>&1; then
        warn "socat not installed: screenshot skipped"
        return 0
    fi
    if [[ ! -S "$MONITOR_SOCK" ]]; then
        warn "QEMU monitor socket missing: screenshot skipped"
        return 0
    fi
    rm -f "$SCREENSHOT_FILE"
    # Human monitor command; the QMP/HMP greeting is harmless.
    if printf 'screendump %s\n' "$SCREENSHOT_FILE" \
        | timeout 10 socat - "UNIX-CONNECT:${MONITOR_SOCK}" >/dev/null 2>&1; then
        if [[ -s "$SCREENSHOT_FILE" ]]; then
            if command -v pnmtopng >/dev/null 2>&1; then
                pnmtopng "$SCREENSHOT_FILE" >"${SCREENSHOT_FILE%.ppm}.png" 2>/dev/null || true
                log "screenshot: ${SCREENSHOT_FILE%.ppm}.png"
            else
                log "screenshot: ${SCREENSHOT_FILE} (install netpbm for PNG)"
            fi
        else
            warn "screenshot file was not produced (headless console may be blank)"
        fi
    else
        warn "could not talk to the QEMU monitor; screenshot skipped"
    fi
}

phase_smoke() {
    log "phase 2/2: booting installed system (timeout ${TIMEOUT_BOOT}s)"
    rm -f "$SERIAL_BOOT" "$MONITOR_SOCK"
    local qemu_args=(
        -drive "file=${DISK},if=virtio,format=qcow2,cache=unsafe"
        -boot order=c
    )

    # Background QEMU; the PID is managed by the trap.
    build_qemu_args "$SERIAL_BOOT" "${qemu_args[@]}"
    "${QEMU_ARGS[@]}" &
    QEMU_PID=$!
    sleep 2

    local base="https://homeserver.local:${HOST_PORT}"
    local failure=0

    # --- Caddy TLS endpoint (proves the installed OS, Caddy config and the
    #     local CA layout are correct even before the daemon answers).
    if wait_for_url "${base}/healthz"; then
        log "PASS  Caddy /healthz answered over TLS"
    else
        warn "Caddy /healthz did not answer; serial log tail:"
        tail -n 40 "$SERIAL_BOOT" 2>/dev/null || true
        failure=1
    fi

    # --- Wizard API (requires hs-daemon in the ISO).
    local status_code body
    if wait_for_url "${base}/api/v1/setup/status"; then
        status_code="$(curl "${CURL_ARGS[@]}" -o "$WORK_DIR/setup-status.json" -w '%{http_code}' \
            "${base}/api/v1/setup/status" || echo 000)"
        body="$(cat "$WORK_DIR/setup-status.json" 2>/dev/null || true)"
        if [[ "$status_code" == "200" ]]; then
            log "PASS  GET /api/v1/setup/status returned HTTP 200"
        else
            warn "GET /api/v1/setup/status returned HTTP ${status_code}"
            failure=1
        fi
        if printf '%s' "$body" | grep -q '"initialized"[[:space:]]*:[[:space:]]*false'; then
            log "PASS  appliance reports initialized=false (fresh install)"
        elif printf '%s' "$body" | grep -q '"initialized"[[:space:]]*:[[:space:]]*true'; then
            warn "appliance reports initialized=true (ISO was not a fresh install?)"
        else
            warn "setup/status body did not contain an initialized flag: ${body}"
            failure=1
        fi
    else
        if ((ALLOW_MISSING_DAEMON == 1)); then
            warn "hs-daemon endpoint unreachable (--allow-missing-daemon: not a failure)"
            warn "rebuild the ISO with --binaries <target/release> to test the daemon"
        else
            warn "hs-daemon endpoint unreachable within ${TIMEOUT_BOOT}s; serial log tail:"
            tail -n 40 "$SERIAL_BOOT" 2>/dev/null || true
            failure=1
        fi
    fi

    capture_screenshot

    if [[ -n "$QEMU_PID" ]] && kill -0 "$QEMU_PID" 2>/dev/null; then
        kill "$QEMU_PID" 2>/dev/null || true
        wait "$QEMU_PID" 2>/dev/null || true
        QEMU_PID=""
    fi

    return "$failure"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
log "ISO: ${ISO}"
log "sha256: $(sha256sum "$ISO" | awk '{print $1}')"

phase_install
if phase_smoke; then
    printf '\n[test-vm] RESULT: PASS\n'
    exit 0
fi
printf '\n[test-vm] RESULT: FAIL (serial logs: %s, %s)\n' "$SERIAL_INSTALL" "$SERIAL_BOOT"
exit 1
