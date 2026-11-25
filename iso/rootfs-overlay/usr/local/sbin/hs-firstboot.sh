#!/usr/bin/env bash
# =============================================================================
# HomeServer Assistant OS - first boot provisioning
# =============================================================================
# Runs once from hs-firstboot.service and is idempotent: a marker file makes a
# second run a no-op (use `hs-firstboot.sh --force` to re-provision, e.g. after
# manually removing /var/lib/incus or the Caddy CA).
#
# What it does (in order):
#   1. system users + runtime directories (sysusers/tmpfiles);
#   2. hostname from SystemConfig when a config already exists;
#   3. /etc/homecore/caddy.env from SystemConfig;
#   4. validate + start the nftables baseline (DMZ isolation active from boot);
#   5. Incus preseed (storage pool, br-lan/br-dmz, profiles, project);
#   6. start Caddy, trust its local CA system-wide and export the CA;
#   7. power policy (lid switch + 80 % charge limit) and audio session linger;
#   8. start hs-daemon / hs-vision (when their binaries are installed);
#   9. print the appliance status in English and mark the boot complete.
#
# Everything is English and only runs on the appliance (Linux + systemd).
# =============================================================================
set -euo pipefail

readonly STATE_DIR="/var/lib/homecore"
readonly MARKER="${STATE_DIR}/.firstboot-done"
readonly CONFIG_JSON="${STATE_DIR}/config.json"
readonly CADDY_ENV="/etc/homecore/caddy.env"
readonly PRESEEED="/opt/homeserver-assistant/infra/incus-preseed.yaml"
readonly CA_EXPORT="${STATE_DIR}/caddy-root-ca.crt"

log() {
    printf '[hs-firstboot] %s\n' "$*"
}

warn() {
    printf '[hs-firstboot] WARNING: %s\n' "$*" >&2
}

force=0
if [[ "${1:-}" == "--force" ]]; then
    force=1
fi

if [[ -f "$MARKER" && "$force" -eq 0 ]]; then
    log "first boot already completed (${MARKER}); nothing to do"
    exit 0
fi
if [[ ! -d /run/systemd/system ]]; then
    log "ERROR: systemd is not running; hs-firstboot must run on the appliance"
    exit 1
fi

install -d -m 0700 "$STATE_DIR"

# Read a value from SystemConfig without failing when the file does not exist
# yet (the daemon creates config.json on its first start).
config_get() {
    local filter="$1" value=""
    if [[ -f "$CONFIG_JSON" ]] && command -v jq >/dev/null 2>&1; then
        value="$(jq -r "$filter // empty" "$CONFIG_JSON" 2>/dev/null || true)"
    fi
    printf '%s' "$value"
}

# ---------------------------------------------------------------------------
# 1. Users and directories
# ---------------------------------------------------------------------------
log "step 1/9: system users and runtime directories"
systemd-sysusers || warn "systemd-sysusers failed (check /etc/sysusers.d/homecore.conf)"
systemd-tmpfiles --create || warn "systemd-tmpfiles failed (check /etc/tmpfiles.d/homecore.conf)"
# Only directories are created here: hs-daemon generates config.json,
# state.redb and master.key itself on first start (0600/0700 inside
# /var/lib/homecore). Never pre-create or overwrite them from first boot.
install -d -m 0700 "$STATE_DIR/apps" "$STATE_DIR/backups"

# ---------------------------------------------------------------------------
# 2. Hostname
# ---------------------------------------------------------------------------
log "step 2/9: hostname"
configured_hostname="$(config_get '.device.hostname')"
if [[ -n "$configured_hostname" ]]; then
    current_hostname="$(hostname)"
    if [[ "$configured_hostname" != "$current_hostname" ]]; then
        if hostnamectl set-hostname "$configured_hostname"; then
            log "hostname set to ${configured_hostname}"
        else
            warn "could not set hostname to ${configured_hostname}"
        fi
    else
        log "hostname already ${configured_hostname}"
    fi
else
    log "no SystemConfig yet; keeping hostname $(hostname)"
fi

# ---------------------------------------------------------------------------
# 3. Caddy environment (hostname/base domain/admin port)
# ---------------------------------------------------------------------------
log "step 3/9: Caddy environment"
render_caddy_env() {
    local hostname base_domain admin_port tmp
    hostname="$(config_get '.device.hostname')"
    base_domain="$(config_get '.services.base_domain')"
    admin_port="$(config_get '.security.admin_web_port')"
    # Keep installer/first-boot defaults when SystemConfig is not available yet.
    hostname="${hostname:-homeserver}"
    base_domain="${base_domain:-home.local}"
    admin_port="${admin_port:-8443}"

    tmp="$(mktemp)"
    cat >"$tmp" <<EOF
# Managed by hs-firstboot/hs-daemon - do not edit.
# Values mirror SystemConfig (device.hostname, services.base_domain,
# security.admin_web_port) and are consumed by the Caddyfile.
HS_HOSTNAME=${hostname}
HS_BASE_DOMAIN=${base_domain}
HS_ADMIN_PORT=${admin_port}
EOF
    if [[ -f "$CADDY_ENV" ]] && cmp -s "$tmp" "$CADDY_ENV"; then
        log "caddy.env already current"
        rm -f "$tmp"
        return 0
    fi
    install -D -m 0644 "$tmp" "$CADDY_ENV"
    rm -f "$tmp"
    log "rendered ${CADDY_ENV} (${hostname}.${base_domain}:${admin_port})"
}
render_caddy_env

# ---------------------------------------------------------------------------
# 4. Firewall baseline (DMZ isolation must be active before workloads start)
# ---------------------------------------------------------------------------
log "step 4/9: nftables baseline"
if nft -c -f /etc/nftables.conf; then
    systemctl enable --now nftables.service || warn "could not start nftables.service"
    log "firewall loaded (table inet homeserver + ip hs-nat)"
else
    warn "nftables configuration invalid; DMZ isolation is NOT active"
fi

# ---------------------------------------------------------------------------
# 5. Network bridges (br-lan contains eth0, br-dmz 10.20.30.1/24)
# ---------------------------------------------------------------------------
# The bridges must exist before Incus starts workloads or the firewall matches,
# so install the reference netplan from the repository. cloud-init must not keep
# owning eth0: its 50-cloud-init.yaml would fight the bridge for the address.
log "step 5/10: network bridges"
readonly NETPLAN_TEMPLATE="/opt/homeserver-assistant/infra/network/netplan-90-homeserver.example.yaml"
if [[ -f "$NETPLAN_TEMPLATE" ]]; then
    install -d -m 0755 /etc/cloud/cloud.cfg.d
    cat >/etc/cloud/cloud.cfg.d/99-hs-disable-network-config.cfg <<'EOF'
# Installed by hs-firstboot. /etc/netplan/90-homeserver.yaml is the only owner
# of br-lan/br-dmz; cloud-init must not rewrite interfaces on later boots.
network: {config: disabled}
EOF
    rm -f /etc/netplan/50-cloud-init.yaml
    install -m 0600 "$NETPLAN_TEMPLATE" /etc/netplan/90-homeserver.yaml
    chmod 0600 /etc/netplan/90-homeserver.yaml
    if netplan generate && netplan apply; then
        log "bridges applied (br-lan carries the LAN address, br-dmz is isolated)"
    else
        warn "netplan apply failed; inspect /etc/netplan/90-homeserver.yaml and run 'netplan try'"
    fi
else
    warn "netplan template missing: ${NETPLAN_TEMPLATE}; bridges are NOT configured"
fi

# ---------------------------------------------------------------------------
# 6. Incus preseed
# ---------------------------------------------------------------------------
log "step 6/10: Incus preseed"
wait_for_incus() {
    local deadline=$((SECONDS + 90))
    while ((SECONDS < deadline)); do
        if incus version >/dev/null 2>&1; then
            return 0
        fi
        sleep 3
    done
    return 1
}

systemctl enable --now incus.service >/dev/null 2>&1 || warn "could not start incus.service"
if wait_for_incus; then
    if [[ -z "$(incus storage list --format csv 2>/dev/null || true)" ]]; then
        if [[ -f "$PRESEEED" ]]; then
            if incus admin init --preseed <"$PRESEEED"; then
                log "Incus preseed applied (br-lan, br-dmz, profiles, project homeserver)"
            else
                warn "Incus preseed failed; run 'incus admin init' manually"
            fi
        else
            warn "preseed file not found: ${PRESEEED}"
        fi
    else
        log "Incus already initialized (storage pools present); preseed skipped"
    fi
else
    warn "Incus did not become ready in time; preseed skipped (run manually later)"
fi

# ---------------------------------------------------------------------------
# 7. Caddy: start, trust the local CA, export it for client devices
# ---------------------------------------------------------------------------
log "step 7/10: Caddy and local CA"
systemctl enable --now caddy.service || warn "could not start caddy.service"

wait_for_caddy() {
    local deadline=$((SECONDS + 60))
    while ((SECONDS < deadline)); do
        if curl -fsS -k -o /dev/null --max-time 3 "https://localhost:8443/healthz"; then
            return 0
        fi
        sleep 2
    done
    return 1
}

if wait_for_caddy; then
    log "Caddy is serving https://<hostname>.local:8443"
    # caddy trust needs root to update the system trust store but reads the CA
    # from the service user's data directory.
    if HOME=/var/lib/caddy caddy trust >/dev/null 2>&1; then
        log "Caddy local CA installed in the system trust store"
    else
        warn "caddy trust failed; clients may see a certificate warning"
    fi
    ca_src=""
    for candidate in /var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt \
                     /root/.local/share/caddy/pki/authorities/local/root.crt; do
        if [[ -f "$candidate" ]]; then
            ca_src="$candidate"
            break
        fi
    done
    if [[ -n "$ca_src" ]]; then
        install -m 0644 "$ca_src" "$CA_EXPORT"
        log "CA exported to ${CA_EXPORT} (import it on phones/laptops once)"
    else
        warn "Caddy root CA not found; export it manually from /var/lib/caddy"
    fi
else
    warn "Caddy did not answer on https://localhost:8443/healthz"
fi

# ---------------------------------------------------------------------------
# 8. Power policy and audio session
# ---------------------------------------------------------------------------
log "step 8/10: power policy and audio"
if [[ -x /usr/local/sbin/hs-power-policy.sh ]]; then
    /usr/local/sbin/hs-power-policy.sh --reload-logind || warn "power policy failed"
else
    warn "hs-power-policy.sh is missing"
fi
# PipeWire/wireplumber run per user session; linger keeps them available after
# the console session exits so native Music Assistant can keep playing.
if id homeserver >/dev/null 2>&1; then
    loginctl enable-linger homeserver || warn "could not enable linger for homeserver"
fi

# ---------------------------------------------------------------------------
# 9. Appliance services
# ---------------------------------------------------------------------------
log "step 9/10: appliance services"
if [[ -x /usr/local/bin/hs-daemon ]]; then
    systemctl enable hs-daemon.service >/dev/null 2>&1 || true
    if systemctl restart hs-daemon.service; then
        log "hs-daemon started (API on 127.0.0.1:8080 behind Caddy)"
    else
        warn "hs-daemon failed to start; inspect: journalctl -u hs-daemon -b"
    fi
else
    warn "hs-daemon binary missing (/usr/local/bin/hs-daemon):"
    warn "  install it with 'scripts/install.sh --binaries <dir>' or build it, then:"
    warn "  systemctl start hs-daemon"
fi

if [[ -x /usr/local/bin/hs-vision && -e /dev/video0 ]]; then
    systemctl enable hs-vision.service >/dev/null 2>&1 || true
    systemctl restart hs-vision.service || warn "hs-vision failed to start (camera busy?)"
else
    if [[ ! -e /dev/video0 ]]; then
        log "no camera at /dev/video0; hs-vision stays stopped"
    fi
fi

# Kiosk browser: Ubuntu 24.04 installs Chromium as a snap. On an offline
# install the snap may be missing; report it clearly instead of failing boot.
if [[ -x /snap/bin/chromium || -x /usr/bin/chromium || -x /usr/bin/chromium-browser ]]; then
    log "kiosk browser found"
else
    warn "no Chromium binary found; hs-kiosk will fail until 'snap install chromium' succeeds"
fi

# ---------------------------------------------------------------------------
# 10. Status
# ---------------------------------------------------------------------------
log "step 10/10: first boot complete"
lan_ip="$(hostname -I 2>/dev/null | awk '{print $1}' || true)"
appliance_hostname="$(hostname)"
cat <<EOF
[hs-firstboot] -----------------------------------------------------------------
[hs-firstboot] HomeServer Assistant is provisioned.
[hs-firstboot]
[hs-firstboot]   Portal:   https://${appliance_hostname}.local:8443
[hs-firstboot]             (or https://${lan_ip}:8443 after trusting the CA)
[hs-firstboot]   Kiosk:    http://127.0.0.1:3000/kiosk (tty1, graphical.target)
[hs-firstboot]   CA file:  ${CA_EXPORT}
[hs-firstboot]   Doctor:   /usr/local/sbin/hs-doctor.sh
[hs-firstboot]   Logs:     journalctl -u hs-daemon -b
[hs-firstboot]
[hs-firstboot] Next: open the portal in a browser and complete the wizard
[hs-firstboot]       (language, admin password, DNS/DMZ, Home Assistant).
[hs-firstboot]       SSH is disabled by default - see docs/DEPLOYMENT.md.
[hs-firstboot] -----------------------------------------------------------------
EOF

date -u '+%Y-%m-%dT%H:%M:%SZ' >"$MARKER"
chmod 0600 "$MARKER"
log "marker written: ${MARKER}"
