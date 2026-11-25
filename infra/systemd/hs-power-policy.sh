#!/usr/bin/env bash
# =============================================================================
# HomeServer Assistant OS - power policy helper
# =============================================================================
# Called by homeserver-power.service (boot + udev power events) and by
# hs-firstboot.sh (before the kiosk session exists).
#
# The appliance is a closed laptop mounted inside a wall: a lid event must
# never suspend it, and the battery must not sit at 100 % for months because
# that is what kills a wall-mounted battery pack.
#
# What this script can control:
#   * logind lid/power/idle policy (drop-in, requires a logind restart);
#   * the firmware-exposed 80 % charge limit through sysfs.
# What it cannot control (manual BIOS/UEFI step, see docs/DEPLOYMENT.md):
#   * "restore power when AC returns" / Wake-on-AC.
#
# Idempotent: safe to call on every power_supply event.
# =============================================================================
set -euo pipefail

readonly LOGIND_DROPIN_DIR="/etc/systemd/logind.conf.d"
readonly LOGIND_DROPIN="${LOGIND_DROPIN_DIR}/90-homeserver.conf"
readonly CHARGE_LIMIT_PERCENT=80

# Do not restart logind unless asked: restarting it on a live machine
# terminates graphical sessions (including the kiosk). hs-firstboot passes
# --reload-logind before the kiosk exists; udev does not.
reload_logind=0

print_usage() {
    cat <<'EOF'
Usage: hs-power-policy.sh [--reload-logind]

  --reload-logind  Restart systemd-logind when the drop-in changed. Only safe
                   before graphical sessions exist (first boot).
EOF
}

log() {
    printf '[hs-power] %s\n' "$*"
}

while (($# > 0)); do
    case "$1" in
        --reload-logind) reload_logind=1 ;;
        -h|--help) print_usage; exit 0 ;;
        *) log "unknown argument: $1"; print_usage >&2; exit 2 ;;
    esac
    shift
done

# ---------------------------------------------------------------------------
# 1. logind policy: never suspend on lid close, ignore idle/keys.
# ---------------------------------------------------------------------------
install_logind_dropin() {
    local tmp changed=0
    tmp="$(mktemp)"
    cat >"$tmp" <<'EOF'
# Managed by homeserver-power.service (HomeServer Assistant OS).
# Do not edit: changes are overwritten on the next boot.
# The appliance is a closed laptop in a wall panel; a lid switch event must
# never suspend it and input idle must never power it off.
[Login]
HandleLidSwitch=ignore
HandleLidSwitchExternalPower=ignore
HandleLidSwitchDocked=ignore
HandleSuspendKey=ignore
HandleHibernateKey=ignore
HandlePowerKey=poweroff
IdleAction=ignore
EOF

    if [[ -f "$LOGIND_DROPIN" ]] && cmp -s "$tmp" "$LOGIND_DROPIN"; then
        log "logind policy already current (${LOGIND_DROPIN})"
    else
        install -D -o root -g root -m 0644 "$tmp" "$LOGIND_DROPIN"
        log "installed ${LOGIND_DROPIN}"
        changed=1
    fi
    rm -f "$tmp"

    if ((changed == 0)); then
        return 0
    fi
    if ((reload_logind == 1)); then
        if [[ -d /run/systemd/system ]]; then
            log "restarting systemd-logind to apply the new lid policy"
            if ! systemctl restart systemd-logind.service; then
                log "WARNING: systemd-logind restart failed; reboot to apply the policy"
            fi
        else
            log "systemd is not running (chroot/install phase); policy applies on next boot"
        fi
    else
        log "NOTE: restart systemd-logind (or reboot) to apply the lid policy"
    fi
}

# ---------------------------------------------------------------------------
# 2. Battery charge limit through the standard sysfs attribute.
#    Supported by many ThinkPads, some Dell/Framework/Lenovo models. Firmware
#    that does not expose the attribute is simply skipped.
# ---------------------------------------------------------------------------
apply_charge_limit() {
    local path applied=0
    # The glob is intentionally unquoted; a non-matching glob expands to the
    # literal pattern, which the -e test then rejects.
    for path in /sys/class/power_supply/BAT*/charge_control_end_threshold; do
        [[ -e "$path" ]] || continue
        if [[ ! -w "$path" ]]; then
            log "SKIP ${path}: not writable (firmware or permissions)"
            continue
        fi
        if ! printf '%s\n' "${CHARGE_LIMIT_PERCENT}" >"$path" 2>/dev/null; then
            log "SKIP ${path}: write rejected by the kernel/firmware"
            continue
        fi
        log "battery $(basename "$(dirname "$path")") charge limit -> ${CHARGE_LIMIT_PERCENT}% (kernel reports $(cat "$path")%)"
        applied=$((applied + 1))
    done

    if ((applied == 0)); then
        log "no battery with charge_control_end_threshold found; charge limit not applied"
    fi
}

# ---------------------------------------------------------------------------
# 3. Status line for the serial console / hs-doctor.
# ---------------------------------------------------------------------------
report_status() {
    local supply
    local -a batts=()
    for supply in /sys/class/power_supply/BAT* /sys/class/power_supply/AC* /sys/class/power_supply/ADP*; do
        [[ -e "$supply" ]] || continue
        batts+=("$(basename "$supply")")
    done
    if ((${#batts[@]} == 0)); then
        log "power supplies: none detected (desktop or VM)"
    else
        log "power supplies: ${batts[*]}"
    fi
}

install_logind_dropin
apply_charge_limit
report_status
log "done"

exit 0
