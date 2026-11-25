#!/usr/bin/env bash
# =============================================================================
# HomeServer Assistant OS - appliance doctor
# =============================================================================
# Read-only health checks for the whole appliance:
#   daemon API (127.0.0.1:8080), Caddy TLS (8443), kiosk (3000), nftables/DMZ
#   isolation, Incus, camera/vision socket, audio, DNS and power policy.
#
# Exit code: 0 when every check passes (warnings allowed), 1 when at least one
# FAIL was reported.
#
# Deliberately NOT `set -e`: the doctor must run every check even when one
# fails. It runs read-only; do not add mutating commands here.
# =============================================================================
set -uo pipefail

readonly STATE_DIR="/var/lib/homecore"
readonly DATA_CONFIG="${STATE_DIR}/config.json"
readonly MASTER_KEY="${STATE_DIR}/master.key"
readonly VISION_SOCKET="/run/homecore/vision.sock"
readonly PORTAL_URL="https://localhost:8443"
readonly KIOSK_URL="http://127.0.0.1:3000/kiosk"
readonly ADMIN_API="http://127.0.0.1:8080"

failures=0
warnings=0

pass() { printf 'PASS  %s\n' "$*"; }
warn() {
    printf 'WARN  %s\n' "$*"
    warnings=$((warnings + 1))
}
fail() {
    printf 'FAIL  %s\n' "$*"
    failures=$((failures + 1))
}

section() {
    printf '\n== %s ==\n' "$*"
}

# ---------------------------------------------------------------------------
section "system"
printf 'hostname:   %s\n' "$(hostname)"
printf 'kernel:     %s\n' "$(uname -r)"
printf 'os:         %s\n' "$(grep -E '^PRETTY_NAME=' /etc/os-release 2>/dev/null | cut -d= -f2- | tr -d '"' || echo unknown)"
printf 'uptime:     %s\n' "$(uptime -p 2>/dev/null || uptime)"
if [[ -f "$DATA_CONFIG" ]]; then
    printf 'configured: %s\n' "$(jq -r '.initialized // false' "$DATA_CONFIG" 2>/dev/null || echo unknown)"
else
    printf 'configured: no (config.json missing: wizard not completed or daemon never started)\n'
fi

# ---------------------------------------------------------------------------
section "files and permissions"
check_file() {
    local path="$1" want_mode="$2" label="$3"
    if [[ ! -e "$path" ]]; then
        warn "${label} missing: ${path}"
        return
    fi
    local mode
    mode="$(stat -c '%a' "$path" 2>/dev/null || echo '?')"
    if [[ -n "$want_mode" && "$mode" != "$want_mode" ]]; then
        warn "${label} mode ${mode} (expected ${want_mode}): ${path}"
    else
        pass "${label} present (${mode}): ${path}"
    fi
}

check_file "$STATE_DIR" 700 "/var/lib/homecore"
check_file "$DATA_CONFIG" 600 "config.json"
check_file "$MASTER_KEY" 600 "master.key"
check_file "/run/homecore" 2770 "/run/homecore (root:hs-vision)"
check_file "$VISION_SOCKET" 660 "vision socket"

# ---------------------------------------------------------------------------
section "services"
check_service() {
    local unit="$1" optional="${2:-no}"
    if ! systemctl list-unit-files "$unit" >/dev/null 2>&1; then
        if [[ "$optional" == "yes" ]]; then
            warn "${unit}: not installed"
        else
            fail "${unit}: unit not found"
        fi
        return
    fi
    if systemctl is-active --quiet "$unit"; then
        pass "${unit}: active"
    else
        if [[ "$optional" == "yes" ]]; then
            warn "${unit}: inactive (optional)"
        else
            fail "${unit}: inactive ($(systemctl is-active "$unit" 2>/dev/null || true))"
        fi
    fi
}
check_service hs-daemon.service
check_service hs-vision.service yes
check_service caddy.service
check_service nftables.service
check_service incus.service
check_service avahi-daemon.service yes
check_service homeserver-power.service yes

if systemctl get-default 2>/dev/null | grep -q '^graphical.target$'; then
    pass "default target: graphical.target (kiosk enabled)"
else
    warn "default target is not graphical.target; kiosk will not start on boot"
fi
check_service hs-kiosk.service yes

# ---------------------------------------------------------------------------
section "daemon API (loopback 8080, behind Caddy)"
if curl -fsS --max-time 5 -o /tmp/hs-doctor-status.json "${ADMIN_API}/api/v1/setup/status"; then
    status_body="$(cat /tmp/hs-doctor-status.json)"
    printf '      body: %s\n' "$status_body"
    if printf '%s' "$status_body" | grep -q '"initialized"'; then
        pass "GET /api/v1/setup/status answered"
        if printf '%s' "$status_body" | grep -q '"initialized":true'; then
            pass "appliance initialized (wizard completed)"
        else
            warn "appliance not initialized: complete the wizard at ${PORTAL_URL}"
        fi
    else
        warn "setup/status answered but body does not contain 'initialized'"
    fi
else
    fail "daemon API not reachable on ${ADMIN_API}"
fi
rm -f /tmp/hs-doctor-status.json

if command -v ss >/dev/null 2>&1; then
    if ss -ltn 2>/dev/null | grep -q '127.0.0.1:8080'; then
        pass "127.0.0.1:8080 is listening"
    else
        fail "nothing listening on 127.0.0.1:8080"
    fi
    if ss -ltn 2>/dev/null | grep -q '127.0.0.1:3000'; then
        pass "127.0.0.1:3000 (kiosk) is listening"
    else
        warn "no listener on 127.0.0.1:3000 (kiosk API)"
    fi
    if ss -ltn 2>/dev/null | grep -q ':8443'; then
        pass "tcp/8443 (Caddy) is listening"
    else
        fail "no listener on tcp/8443 (Caddy)"
    fi
    if ss -ltn 2>/dev/null | grep -qE '(0\.0\.0\.0|\[::\]):(8080|3000)'; then
        fail "admin API or kiosk is bound to a non-loopback address"
    else
        pass "admin API and kiosk are not exposed beyond loopback"
    fi
fi

# ---------------------------------------------------------------------------
section "Caddy / TLS"
if command -v caddy >/dev/null 2>&1; then
    if caddy validate --config /etc/caddy/Caddyfile >/dev/null 2>&1; then
        pass "Caddyfile validates"
    else
        fail "Caddyfile does not validate (sudo caddy validate --config /etc/caddy/Caddyfile)"
    fi
fi
if curl -fsS --max-time 5 "${PORTAL_URL}/healthz" >/dev/null 2>&1; then
    pass "https://localhost:8443/healthz answered with a trusted certificate"
else
    if curl -fsS -k --max-time 5 "${PORTAL_URL}/healthz" >/dev/null 2>&1; then
        warn "Caddy answers but the local CA is not trusted by the host (run: caddy trust)"
    else
        fail "Caddy not answering on ${PORTAL_URL}/healthz"
    fi
fi
if [[ -f "${STATE_DIR}/caddy-root-ca.crt" ]]; then
    pass "exported local CA present: ${STATE_DIR}/caddy-root-ca.crt"
else
    warn "exported local CA missing (first boot may not have completed)"
fi
if curl -fsS --max-time 5 -o /dev/null "$KIOSK_URL"; then
    pass "kiosk page ${KIOSK_URL} answered"
else
    warn "kiosk page ${KIOSK_URL} did not answer"
fi

# ---------------------------------------------------------------------------
section "firewall / DMZ isolation"
if command -v nft >/dev/null 2>&1; then
    if nft list table inet homeserver >/dev/null 2>&1; then
        pass "table inet homeserver is loaded"
        ruleset="$(nft list table inet homeserver 2>/dev/null || true)"
        if printf '%s' "$ruleset" | grep -q 'DMZ must never reach LAN'; then
            pass "DMZ -> LAN drop rule present"
        else
            fail "DMZ -> LAN drop rule is MISSING (isolation broken)"
        fi
        if printf '%s' "$ruleset" | grep -q 'DMZ must never reach the administrator portal'; then
            pass "DMZ -> admin portal drop rule present"
        else
            fail "DMZ -> admin portal drop rule is MISSING"
        fi
    else
        fail "table inet homeserver is not loaded (run: systemctl restart nftables)"
    fi
    if nft list table ip homeserver_nat >/dev/null 2>&1; then
        pass "table ip homeserver_nat (DMZ egress masquerade) is loaded"
    else
        warn "table ip homeserver_nat missing: DMZ egress is disabled or not yet rendered"
    fi
    if command -v incus >/dev/null 2>&1 && incus network show br-dmz >/dev/null 2>&1; then
        pass "Incus network/br-dmz is configured"
    fi
else
    fail "nft command not found"
fi

# ---------------------------------------------------------------------------
section "Incus"
if command -v incus >/dev/null 2>&1; then
    if incus version >/dev/null 2>&1; then
        pass "incus responds ($(incus version 2>/dev/null | head -1))"
        pools="$(incus storage list --format csv 2>/dev/null || true)"
        if [[ -n "$pools" ]]; then
            pass "storage pools: $(printf '%s' "$pools" | cut -d, -f1 | tr '\n' ' ')"
        else
            fail "no Incus storage pool (preseed not applied?)"
        fi
        instances="$(incus list --format csv -c n 2>/dev/null | wc -l)"
        pass "instances: ${instances}"
    else
        fail "incus CLI cannot talk to the Incus daemon"
    fi
else
    fail "incus command not found"
fi

# ---------------------------------------------------------------------------
section "camera / vision"
if [[ -e /dev/video0 ]]; then
    pass "/dev/video0 present ($(stat -c '%a %U:%G' /dev/video0 2>/dev/null))"
else
    warn "/dev/video0 not present (no camera or driver not loaded)"
fi
if [[ -S "$VISION_SOCKET" ]]; then
    pass "vision socket present ($(stat -c '%a %U:%G' "$VISION_SOCKET" 2>/dev/null))"
else
    warn "vision socket missing: ${VISION_SOCKET}"
fi

# ---------------------------------------------------------------------------
section "audio"
if id homeserver >/dev/null 2>&1 && command -v wpctl >/dev/null 2>&1; then
    uid="$(id -u homeserver)"
    if runuser -u homeserver -- env "XDG_RUNTIME_DIR=/run/user/${uid}" wpctl status >/dev/null 2>&1; then
        sink="$(runuser -u homeserver -- env "XDG_RUNTIME_DIR=/run/user/${uid}" wpctl inspect @DEFAULT_AUDIO_SINK@ 2>/dev/null | head -3 | tr '\n' ' ')"
        pass "PipeWire session reachable for homeserver${sink:+ (default sink found)}"
    else
        warn "PipeWire session not reachable for homeserver (user session/logind linger?)"
    fi
else
    warn "wpctl not available or homeserver user missing; audio check skipped"
fi

# ---------------------------------------------------------------------------
section "DNS"
if getent hosts homeserver.local >/dev/null 2>&1; then
    pass "homeserver.local resolves"
else
    warn "homeserver.local does not resolve (avahi/libnss-mdns)"
fi
if getent hosts example.com >/dev/null 2>&1; then
    pass "external DNS resolution works"
else
    fail "external DNS resolution failed (check the resolver / network)"
fi
if systemctl is-active --quiet avahi-daemon.service; then
    pass "avahi-daemon active (mDNS)"
else
    warn "avahi-daemon inactive: <hostname>.local will not resolve on the LAN"
fi

# ---------------------------------------------------------------------------
section "power"
if [[ -f /etc/systemd/logind.conf.d/90-homeserver.conf ]]; then
    pass "logind lid policy drop-in present"
else
    warn "logind lid policy drop-in missing (run hs-power-policy.sh)"
fi
thresholds=0
for path in /sys/class/power_supply/BAT*/charge_control_end_threshold; do
    [[ -e "$path" ]] || continue
    thresholds=$((thresholds + 1))
    printf '      %s = %s%%\n' "$path" "$(cat "$path" 2>/dev/null || echo '?')"
done
if ((thresholds == 0)); then
    warn "no battery charge limit attribute found"
else
    pass "battery charge limit attribute available"
fi

# ---------------------------------------------------------------------------
section "storage"
df -h / /var/lib/incus 2>/dev/null || true
root_use="$(df --output=pcent / 2>/dev/null | tail -1 | tr -dc '0-9')"
if [[ -n "$root_use" ]] && ((root_use >= 90)); then
    fail "root filesystem is ${root_use}% full"
elif [[ -n "$root_use" ]]; then
    pass "root filesystem usage ${root_use}%"
fi

# ---------------------------------------------------------------------------
section "summary"
if ((failures == 0)); then
    printf 'RESULT: OK (%d warning(s))\n' "$warnings"
    exit 0
fi
printf 'RESULT: %d failure(s), %d warning(s) - see FAIL lines above\n' "$failures" "$warnings"
printf 'Logs: journalctl -u hs-daemon -b / journalctl -u caddy -b / journalctl -u hs-firstboot -b\n'
exit 1
