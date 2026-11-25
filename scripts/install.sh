#!/usr/bin/env bash
# =============================================================================
# HomeServer Assistant OS - static installer
# =============================================================================
# Installs the appliance's static configuration into a target root:
#   * rootfs overlay (first-boot unit, kiosk wrapper, policies, sysctl, ...);
#   * systemd units, sysusers/tmpfiles, udev rules;
#   * nftables baseline, Caddy config, Incus preseed and helper scripts;
#   * the prebuilt hs-daemon / hs-vision binaries when provided;
#   * enabled units and the graphical target (kiosk boots on tty1).
#
# Two callers:
#   1. ISO autoinstall late-commands (unattended install):
#        sudo bash /cdrom/hs/repo/scripts/install.sh \
#            --target /target --source /cdrom/hs/repo --binaries /cdrom/hs/bin
#   2. A Debian/Ubuntu host from the repo checkout:
#        sudo ./scripts/install.sh --start
#
# The script only *writes files* and enables units; it never deletes existing
# configuration and never touches /var/lib/homecore state. Runtime steps
# (users, Incus preseed, Caddy local CA trust, service start) are performed by
# hs-firstboot.service on the first boot, or immediately with --start.
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
REPO_ROOT="$(dirname -- "$SCRIPT_DIR")"
readonly REPO_ROOT
readonly OPT_DIR="/opt/homeserver-assistant"

TARGET="/"
SOURCE="$REPO_ROOT"
BINARIES=""
START=0

log() {
    printf '[hs-install] %s\n' "$*"
}

warn() {
    printf '[hs-install] WARNING: %s\n' "$*" >&2
}

die() {
    printf '[hs-install] ERROR: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Usage: install.sh [OPTIONS]

Options:
  --target DIR      Install into DIR instead of / (autoinstall: /target).
  --source DIR      Repository root to install from (default: parent of this
                    script).
  --binaries DIR    Directory containing prebuilt hs-daemon and hs-vision.
  --start           Also apply runtime steps now and start the services
                    (only valid with --target /).
  -h, --help        Show this help.

Environment:
  None. Everything is passed explicitly so the script works inside the
  installer environment where the repository may be mounted read-only.
EOF
}

while (($# > 0)); do
    case "$1" in
        --target)
            [[ $# -ge 2 ]] || die "--target requires a value"
            TARGET="$2"
            shift 2
            ;;
        --source)
            [[ $# -ge 2 ]] || die "--source requires a value"
            SOURCE="$2"
            shift 2
            ;;
        --binaries)
            [[ $# -ge 2 ]] || die "--binaries requires a value"
            BINARIES="$2"
            shift 2
            ;;
        --start)
            START=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1 (try --help)"
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Preconditions
# ---------------------------------------------------------------------------
[[ "$(id -u)" -eq 0 ]] || die "must run as root (installs system files)"

[[ -d "$SOURCE" ]] || die "source directory not found: $SOURCE"
[[ -d "$SOURCE/iso/rootfs-overlay" ]] || die "missing $SOURCE/iso/rootfs-overlay"
[[ -f "$SOURCE/infra/systemd/hs-daemon.service" ]] || die "missing infra/systemd units in $SOURCE"

TARGET="$(cd -- "$TARGET" 2>/dev/null && pwd)" || die "target directory does not exist"
# Normalise: when installing into the live root, no prefix is needed.
ROOT=""
if [[ "$TARGET" != "/" ]]; then
    ROOT="$TARGET"
    [[ -f "$ROOT/etc/os-release" ]] || warn "$TARGET does not look like a Linux root (no /etc/os-release)"
fi

if [[ "$START" -eq 1 && -n "$ROOT" ]]; then
    die "--start is only valid with --target / (installing into the live root)"
fi

install_file() {
    local src="$1" dest="$2" mode="$3"
    [[ -f "$src" ]] || die "missing file: $src"
    install -D -m "$mode" "$src" "${ROOT}${dest}"
}

# ---------------------------------------------------------------------------
# 1. Rootfs overlay (ISO-specific files: first boot unit, kiosk wrapper,
#    chromium policy, sysctl, logind drop-in, nftables include loader).
# ---------------------------------------------------------------------------
log "installing rootfs overlay from $SOURCE/iso/rootfs-overlay"
cp -a "$SOURCE/iso/rootfs-overlay/." "${ROOT}/"

# Windows checkouts lose the execute bit; restore it explicitly (the ISO build
# script does the same before packaging).
for script_file in "${ROOT}/usr/local/bin/hs-kiosk-start" "${ROOT}/usr/local/sbin/"*.sh; do
    if [[ -e "$script_file" ]]; then
        chmod 0755 "$script_file"
    fi
done

# ---------------------------------------------------------------------------
# 2. systemd units, power helper, udev, sysusers/tmpfiles
# ---------------------------------------------------------------------------
log "installing systemd units"
install -d -m 0755 "${ROOT}/etc/systemd/system"
for unit in hs-daemon.service hs-vision.service hs-kiosk.service homeserver-power.service; do
    install_file "$SOURCE/infra/systemd/$unit" "/etc/systemd/system/$unit" 0644
done

install -d -m 0755 "${ROOT}/usr/local/sbin"
install_file "$SOURCE/infra/systemd/hs-power-policy.sh" "/usr/local/sbin/hs-power-policy.sh" 0755

install -d -m 0755 "${ROOT}/etc/udev/rules.d"
install_file "$SOURCE/infra/systemd/udev/99-homeserver-power.rules" "/etc/udev/rules.d/99-homeserver-power.rules" 0644

install -d -m 0755 "${ROOT}/etc/sysusers.d"
install_file "$SOURCE/infra/systemd/sysusers.d/homecore.conf" "/etc/sysusers.d/homecore.conf" 0644

install -d -m 0755 "${ROOT}/etc/tmpfiles.d"
install_file "$SOURCE/infra/systemd/tmpfiles.d/homecore.conf" "/etc/tmpfiles.d/homecore.conf" 0644

# ---------------------------------------------------------------------------
# 3. Firewall baseline (the daemon re-renders this file at runtime)
# ---------------------------------------------------------------------------
log "installing nftables baseline"
install -d -m 0755 "${ROOT}/etc/nftables.d"
install_file "$SOURCE/infra/firewall/nftables.conf" "/etc/nftables.d/10-hs-network.nft" 0644

# /etc/nftables.conf is shipped by the rootfs overlay, but keep a fallback so
# the ruleset still loads if the overlay is ever trimmed.
if [[ ! -f "${ROOT}/etc/nftables.conf" ]]; then
    install -d -m 0755 "${ROOT}/etc"
    printf '%s\n' '#!/usr/sbin/nft -f' \
        '# Loader for the daemon-rendered appliance rulesets.' \
        'include "/etc/nftables.d/*.nft"' \
        >"${ROOT}/etc/nftables.conf"
    chmod 0644 "${ROOT}/etc/nftables.conf"
fi

# ---------------------------------------------------------------------------
# 4. Caddy reverse proxy + local CA layout
# ---------------------------------------------------------------------------
log "installing Caddy reverse proxy configuration"
install -d -m 0755 "${ROOT}/etc/caddy/apps.d"
install_file "$SOURCE/infra/services/caddy/Caddyfile" "/etc/caddy/Caddyfile" 0644
install_file "$SOURCE/infra/services/caddy/apps.d/00-placeholder.caddy" \
    "/etc/caddy/apps.d/00-placeholder.caddy" 0644
install -d -m 0755 "${ROOT}/etc/systemd/system/caddy.service.d"
install_file "$SOURCE/infra/services/caddy/caddy.service.d/10-homeserver.conf" \
    "/etc/systemd/system/caddy.service.d/10-homeserver.conf" 0644

# ---------------------------------------------------------------------------
# 5. Copy the deployment assets for offline troubleshooting:
#    /opt/homeserver-assistant/{infra,scripts,docs,iso}
# ---------------------------------------------------------------------------
log "staging deployment assets under $OPT_DIR"
install -d -m 0755 "${ROOT}${OPT_DIR}"
for dir in infra scripts docs; do
    if [[ -d "$SOURCE/$dir" ]]; then
        cp -a "$SOURCE/$dir" "${ROOT}${OPT_DIR}/"
    fi
done
if [[ -d "$SOURCE/iso/rootfs-overlay" ]]; then
    install -d -m 0755 "${ROOT}${OPT_DIR}/iso"
    cp -a "$SOURCE/iso/rootfs-overlay" "${ROOT}${OPT_DIR}/iso/"
    cp -a "$SOURCE/iso/autoinstall" "${ROOT}${OPT_DIR}/iso/"
fi

# ---------------------------------------------------------------------------
# 6. Service binaries (Rust build output, injected by the ISO builder)
# ---------------------------------------------------------------------------
if [[ -n "$BINARIES" ]]; then
    if [[ -d "$BINARIES" ]]; then
        for binary in hs-daemon hs-vision; do
            if [[ -f "$BINARIES/$binary" ]]; then
                install -D -m 0755 "$BINARIES/$binary" "${ROOT}/usr/local/bin/$binary"
                log "installed /usr/local/bin/$binary"
            else
                warn "binary not found in $BINARIES: $binary (services will fail until it is installed)"
            fi
        done
    else
        warn "--binaries directory not found: $BINARIES (skipping binaries)"
    fi
else
    log "no --binaries directory given: hs-daemon/hs-vision are not installed by this script"
    log "  build them on Linux with: cargo build --release -p hs-daemon -p hs-vision"
fi

# Optional built React UI: the ISO stages it at <staging>/ui (sibling of
# <staging>/repo); the daemon serves it from HS_STATIC_DIR.
UI_SRC="$(dirname -- "$SOURCE")/ui"
if [[ -d "$UI_SRC" ]]; then
    log "installing portal UI from $UI_SRC"
    install -d -m 0755 "${ROOT}/usr/share/homeserver-assistant/ui"
    cp -a "$UI_SRC/." "${ROOT}/usr/share/homeserver-assistant/ui/"
    find "${ROOT}/usr/share/homeserver-assistant/ui" -type f -exec chmod 0644 {} +
elif [[ -d "$REPO_ROOT/ui/dist" ]]; then
    log "installing portal UI from $REPO_ROOT/ui/dist"
    install -d -m 0755 "${ROOT}/usr/share/homeserver-assistant/ui"
    cp -a "$REPO_ROOT/ui/dist/." "${ROOT}/usr/share/homeserver-assistant/ui/"
else
    warn "built UI not found (ui/dist): the portal will return an error until it is installed"
fi

# ---------------------------------------------------------------------------
# 7. Environment files (no secrets; the vault holds those)
# ---------------------------------------------------------------------------
log "writing environment files under /etc/homecore"
install -d -m 0700 "${ROOT}/etc/homecore"

# Written before tmpfiles so the tmpfiles `f` entries see existing files.
if [[ ! -s "${ROOT}/etc/homecore/env" ]]; then
    cat >"${ROOT}/etc/homecore/env" <<'EOF'
# Environment for hs-daemon.service (read as root, mode 0640).
# Keep this file free of secrets: tokens and keys belong in the encrypted
# secret vault under /var/lib/homecore.
RUST_LOG=info
# HS_DATA_DIR is optional: the daemon defaults to /var/lib/homecore.
# HS_DATA_DIR=/var/lib/homecore
# Built React UI served by the daemon (installed by the ISO / this script).
HS_STATIC_DIR=/usr/share/homeserver-assistant/ui
EOF
    chmod 0640 "${ROOT}/etc/homecore/env"
fi

if [[ ! -s "${ROOT}/etc/homecore/caddy.env" ]]; then
    cat >"${ROOT}/etc/homecore/caddy.env" <<'EOF'
# Environment for caddy.service and the Caddyfile (mode 0644, no secrets).
# The daemon keeps these in sync with SystemConfig; hs-firstboot seeds them
# and the installer writes safe defaults so Caddy can start before the
# daemon's first run.
HS_HOSTNAME=homeserver
HS_BASE_DOMAIN=home.local
HS_ADMIN_PORT=8443
EOF
    chmod 0644 "${ROOT}/etc/homecore/caddy.env"
fi

# ---------------------------------------------------------------------------
# 8. Users and runtime directories (best effort: systemd tools may be missing
#    when installing into a minimal target).
# ---------------------------------------------------------------------------
if command -v systemd-sysusers >/dev/null 2>&1; then
    log "creating hs-vision/kiosk system users"
    systemd-sysusers --root "$TARGET" || warn "systemd-sysusers failed (users will be created on first boot)"
fi
if command -v systemd-tmpfiles >/dev/null 2>&1; then
    log "creating runtime directories"
    systemd-tmpfiles --root "$TARGET" --create || warn "systemd-tmpfiles failed (directories will be created on first boot)"
fi

# ---------------------------------------------------------------------------
# 9. Enable the units
# ---------------------------------------------------------------------------
enable_unit() {
    local unit="$1"
    if [[ -n "$ROOT" ]]; then
        systemctl --root="$ROOT" enable "$unit" 2>/dev/null || warn "could not enable $unit"
    else
        systemctl enable "$unit" 2>/dev/null || warn "could not enable $unit"
    fi
}

log "enabling services"
for unit in hs-daemon.service hs-vision.service homeserver-power.service \
            hs-firstboot.service nftables.service caddy.service incus.service \
            avahi-daemon.service; do
    enable_unit "$unit"
done

# The kiosk is WantedBy=graphical.target: make that the boot default so a
# closed-lid appliance boots straight into the wall UI.
if [[ -n "$ROOT" ]]; then
    systemctl --root="$ROOT" set-default graphical.target 2>/dev/null \
        || warn "could not set graphical.target as default"
else
    systemctl set-default graphical.target 2>/dev/null || warn "could not set graphical.target as default"
fi
enable_unit hs-kiosk.service

# ---------------------------------------------------------------------------
# 10. Optional runtime application on the live host
# ---------------------------------------------------------------------------
if [[ "$START" -eq 1 ]]; then
    log "applying runtime configuration and starting services"
    systemctl daemon-reload
    systemd-sysusers
    systemd-tmpfiles --create
    systemctl enable --now nftables.service
    systemctl enable --now caddy.service
    systemctl enable --now incus.service || warn "incus failed to start; check 'incus admin init' prerequisites"
    systemctl enable --now hs-daemon.service || warn "hs-daemon failed to start (binary installed? see journalctl -u hs-daemon)"
    systemctl enable --now hs-vision.service || warn "hs-vision failed to start (camera present?)"
    systemctl enable --now homeserver-power.service || warn "homeserver-power failed"
    log "runtime applying done; run: hs-doctor.sh"
fi

log "done"
log "target:   $TARGET"
log "assets:   $OPT_DIR"
log "next:     first boot provisions users, Incus preseed and the Caddy local CA"
log "          (systemctl status hs-firstboot.service; journalctl -u hs-firstboot -b)"
