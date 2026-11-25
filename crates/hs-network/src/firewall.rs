//! nftables firewall rendering, DMZ isolation validation and application.
//!
//! The appliance is the only router between the trusted LAN (`br-lan`), the
//! isolated DMZ (`br-dmz`) and the internet. This module owns the nftables
//! rules that enforce the DMZ security invariant:
//!
//! > workloads in the DMZ must never reach the LAN, the administrator portal or
//! > any host service other than the explicit DNS/NTP allow list.
//!
//! DMZ workloads may opt in to **outbound internet access** through the
//! physical uplink ([`FirewallPlan::wan_interface`] plus
//! [`FirewallPlan::allow_dmz_egress`]). This is required by Home Assistant
//! updates and by the outbound tunnel client (cloudflared/WireGuard): the
//! forward chain accepts `DMZ -> WAN` explicitly, and a separate
//! `table ip homeserver_nat` masquerades DMZ traffic behind the appliance
//! uplink so no port has to be opened on the home router. The DMZ stays
//! forbidden from the LAN and from the host.
//!
//! [`render_nftables`] is a pure function so the exact ruleset can be reviewed
//! and golden-tested. [`validate_isolation`] re-parses a rendered ruleset and
//! rejects it when any invariant is missing or contradicted; [`apply`] renders,
//! re-validates, writes atomically and runs `nft -f` on Linux.
//!
//! The WAN accept is deliberately narrow: it is only rendered when
//! `allow_dmz_egress` is true **and** a WAN interface is configured, it stays
//! after the explicit `DMZ -> LAN` drop, and it is paired with a NAT
//! masquerade rule validated by [`validate_isolation`].

use std::path::Path;

use ipnet::Ipv4Net;

use crate::error::{NetworkError, NetworkResult};
use crate::fsutil::atomic_write;

/// NTP port allowed from the LAN (and from the DMZ when DNS is allowed).
const NTP_PORT: u16 = 123;
/// mDNS port used by the device discovery service.
const MDNS_PORT: u16 = 5353;
/// SSDP source port used by UPnP discovery replies.
const SSDP_SOURCE_PORT: u16 = 1900;

/// Complete description of the firewall to render.
///
/// Bridge names are the host-side Linux bridge interfaces, not the Incus
/// network names. Subnets are used in addition to the interface matching so a
/// spoofed source address cannot bypass isolation.
#[derive(Debug, Clone)]
pub struct FirewallPlan {
    /// Trusted LAN bridge interface, usually `br-lan`.
    pub lan_bridge: String,
    /// Isolated DMZ bridge interface, usually `br-dmz`.
    pub dmz_bridge: String,
    /// Administrative portal TCP port exposed on the host.
    pub admin_port: u16,
    /// Local DNS resolver port (TCP and UDP) exposed on the host.
    pub dns_port: u16,
    /// Whether DMZ workloads may reach the host resolver and NTP.
    pub dmz_allow_dns: bool,
    /// DMZ subnet in CIDR notation.
    pub dmz_subnet: Ipv4Net,
    /// LAN subnet in CIDR notation, when known.
    pub lan_subnet: Option<Ipv4Net>,
    /// Physical uplink interface used for outbound internet access
    /// (e.g. `eth0`).
    ///
    /// Required when [`Self::allow_dmz_egress`] is true: it is the `oifname`
    /// of the DMZ egress forward rule and of the NAT masquerade rule.
    pub wan_interface: Option<String>,
    /// Whether DMZ workloads may reach the internet through
    /// [`Self::wan_interface`].
    ///
    /// Egress is **opt-in** and defaults to disabled when constructing a plan:
    /// the daemon enables it for the update/tunnel path. Even when enabled the
    /// DMZ still cannot reach the LAN, the administrator portal or other host
    /// services; the forward chain drops `DMZ -> LAN` before the WAN accept and
    /// the egress is masqueraded, so no router port is exposed.
    pub allow_dmz_egress: bool,
}

/// Validate a firewall plan before rendering or applying it.
///
/// # Errors
/// Returns [`NetworkError::Invalid`] when a bridge or WAN interface name is
/// unsafe or too long, the LAN and DMZ bridges are equal, the WAN interface
/// collides with a bridge, a port is zero or duplicated, the DMZ subnet is the
/// whole IPv4 space, the LAN and DMZ subnets overlap, or DMZ egress is enabled
/// without a WAN interface.
pub fn validate_plan(plan: &FirewallPlan) -> NetworkResult<()> {
    validate_bridge_name(&plan.lan_bridge, "lan_bridge")?;
    validate_bridge_name(&plan.dmz_bridge, "dmz_bridge")?;
    if plan.lan_bridge == plan.dmz_bridge {
        return Err(NetworkError::Invalid(
            "the LAN and DMZ bridges must be different interfaces".to_owned(),
        ));
    }
    if let Some(wan) = &plan.wan_interface {
        validate_bridge_name(wan, "wan_interface")?;
        if wan == &plan.lan_bridge || wan == &plan.dmz_bridge {
            return Err(NetworkError::Invalid(
                "wan_interface must be a physical uplink, not the LAN or DMZ bridge".to_owned(),
            ));
        }
    }
    if plan.allow_dmz_egress && plan.wan_interface.is_none() {
        return Err(NetworkError::Invalid(
            "allow_dmz_egress requires a configured wan_interface".to_owned(),
        ));
    }
    if plan.admin_port == 0 {
        return Err(NetworkError::Invalid(
            "admin_port must be between 1 and 65535".to_owned(),
        ));
    }
    if plan.dns_port == 0 {
        return Err(NetworkError::Invalid(
            "dns_port must be between 1 and 65535".to_owned(),
        ));
    }
    if plan.admin_port == plan.dns_port {
        return Err(NetworkError::Invalid(
            "admin_port and dns_port must be different".to_owned(),
        ));
    }
    if plan.dmz_subnet.prefix_len() == 0 {
        return Err(NetworkError::Invalid(
            "the DMZ subnet must not be the whole IPv4 address space".to_owned(),
        ));
    }
    if let Some(lan) = plan.lan_subnet {
        if subnets_overlap(&lan, &plan.dmz_subnet) {
            return Err(NetworkError::Invalid(
                "the LAN and DMZ subnets must not overlap".to_owned(),
            ));
        }
    }
    Ok(())
}

/// Render the complete nftables ruleset for `plan`.
///
/// The output is deterministic and starts with the `nft -f` shebang so it can
/// be applied directly. When DMZ egress is enabled it also contains the
/// `table ip homeserver_nat` postrouting masquerade table. Bridge and WAN names
/// are sanitized to the nft-safe character set; use [`validate_plan`] before
/// applying to reject unsafe names instead of silently sanitizing them.
#[must_use]
pub fn render_nftables(plan: &FirewallPlan) -> String {
    let lan_bridge = sanitize_interface(&plan.lan_bridge);
    let dmz_bridge = sanitize_interface(&plan.dmz_bridge);
    let admin_port = plan.admin_port;
    let dns_port = plan.dns_port;
    let dmz_subnet = plan.dmz_subnet;
    let egress_wan = if plan.allow_dmz_egress {
        plan.wan_interface.as_deref().map(sanitize_interface)
    } else {
        None
    };

    let mut ruleset = String::new();
    ruleset.push_str("#!/usr/sbin/nft -f\n");
    ruleset.push_str("# HomeServer Assistant - managed nftables ruleset. Do not edit by hand.\n");
    ruleset.push_str("#\n");
    ruleset.push_str("# Security invariants enforced by this ruleset:\n");
    ruleset.push_str("#   * the DMZ bridge can never forward traffic to the LAN bridge;\n");
    ruleset.push_str("#   * DMZ workloads can only reach host services on the explicit allow\n");
    ruleset
        .push_str("#     list (the local DNS resolver and NTP), and only when DNS is allowed;\n");
    ruleset.push_str("#   * the administrator portal is reachable from the trusted LAN only;\n");
    ruleset.push_str("#   * DMZ workloads may only reach the internet through the appliance\n");
    ruleset.push_str("#     uplink NAT (updates and the outbound tunnel client), never the LAN;\n");
    ruleset.push_str("#   * any other inbound or forwarded traffic is dropped.\n");
    ruleset.push_str("#\n");
    ruleset
        .push_str("# Regenerate with `hs_network::firewall::render_nftables` and validate with\n");
    ruleset.push_str("# `hs_network::firewall::validate_isolation` before applying.\n");
    ruleset.push('\n');
    ruleset.push_str("table inet homeserver {\n");

    // Source based matching as defence in depth on top of interface matching.
    ruleset.push_str("    # Subnet sets. Matching the source subnet in addition to the bridge\n");
    ruleset.push_str("    # interface makes source-address spoofing ineffective.\n");
    ruleset.push_str("    set dmz_subnets {\n");
    ruleset.push_str("        type ipv4_addr\n");
    ruleset.push_str("        flags interval\n");
    ruleset.push_str(&format!("        elements = {{ {dmz_subnet} }}\n"));
    ruleset.push_str("    }\n");
    if let Some(lan_subnet) = plan.lan_subnet {
        ruleset.push_str("    set lan_subnets {\n");
        ruleset.push_str("        type ipv4_addr\n");
        ruleset.push_str("        flags interval\n");
        ruleset.push_str(&format!("        elements = {{ {lan_subnet} }}\n"));
        ruleset.push_str("    }\n");
    }
    ruleset.push('\n');

    // --- Input chain: traffic addressed to the host ------------------------
    ruleset.push_str("    chain input {\n");
    ruleset.push_str("        type filter hook input priority filter; policy drop;\n");
    ruleset.push('\n');
    ruleset.push_str("        ct state established,related accept comment \"return traffic for host-initiated flows\"\n");
    ruleset.push_str("        ct state invalid counter drop comment \"invalid packets\"\n");
    ruleset.push('\n');
    ruleset.push_str("        iifname \"lo\" accept comment \"loopback\"\n");
    ruleset.push('\n');
    ruleset
        .push_str("        # --- Trusted LAN -------------------------------------------------\n");
    ruleset.push_str("        # The administrator portal is only reachable from the LAN.\n");
    let lan_match = match plan.lan_subnet {
        Some(_) => format!("iifname \"{lan_bridge}\" ip saddr @lan_subnets"),
        None => format!("iifname \"{lan_bridge}\""),
    };
    ruleset.push_str(&format!(
        "        {lan_match} tcp dport {admin_port} accept comment \"admin portal from LAN\"\n"
    ));
    ruleset.push_str(&format!(
        "        {lan_match} tcp dport {dns_port} accept comment \"DNS from LAN (TCP)\"\n"
    ));
    ruleset.push_str(&format!(
        "        {lan_match} udp dport {dns_port} accept comment \"DNS from LAN (UDP)\"\n"
    ));
    ruleset.push_str(&format!(
        "        {lan_match} udp dport {NTP_PORT} accept comment \"NTP from LAN\"\n"
    ));
    ruleset.push_str(&format!(
        "        {lan_match} udp dport {MDNS_PORT} accept comment \"mDNS discovery from LAN\"\n"
    ));
    ruleset.push_str(&format!(
        "        {lan_match} udp sport {SSDP_SOURCE_PORT} accept comment \"SSDP discovery replies from LAN\"\n"
    ));
    ruleset.push_str(&format!(
        "        {lan_match} icmp type echo-request accept comment \"ICMP diagnostics from LAN\"\n"
    ));
    ruleset.push('\n');
    ruleset
        .push_str("        # --- Isolated DMZ ------------------------------------------------\n");
    if plan.dmz_allow_dns {
        ruleset.push_str("        # DMZ workloads may reach the host resolver and NTP only.\n");
        ruleset.push_str(&format!(
            "        iifname \"{dmz_bridge}\" ip saddr @dmz_subnets tcp dport {dns_port} accept comment \"DMZ DNS (TCP)\"\n"
        ));
        ruleset.push_str(&format!(
            "        iifname \"{dmz_bridge}\" ip saddr @dmz_subnets udp dport {dns_port} accept comment \"DMZ DNS (UDP)\"\n"
        ));
        ruleset.push_str(&format!(
            "        iifname \"{dmz_bridge}\" ip saddr @dmz_subnets udp dport {NTP_PORT} accept comment \"DMZ NTP\"\n"
        ));
    } else {
        ruleset.push_str(
            "        # DMZ -> host DNS/NTP access is disabled (dmz_allow_dns = false).\n",
        );
    }
    ruleset.push_str(
        "        # Explicit defence in depth: the portal is never reachable from the DMZ.\n",
    );
    ruleset.push_str(&format!(
        "        iifname \"{dmz_bridge}\" ip saddr @dmz_subnets tcp dport {admin_port} counter drop comment \"DMZ must never reach the administrator portal\"\n"
    ));
    ruleset.push_str(
        "        # Everything else from the DMZ is dropped by this explicit rule (counted\n",
    );
    ruleset.push_str("        # so attempts are auditable when the policy drop is not enough).\n");
    ruleset.push_str(&format!(
        "        iifname \"{dmz_bridge}\" ip saddr @dmz_subnets counter drop comment \"DMZ must never reach other host services\"\n"
    ));
    ruleset.push_str("    }\n");
    ruleset.push('\n');

    // --- Forward chain: traffic routed between bridges ---------------------
    ruleset.push_str("    chain forward {\n");
    ruleset.push_str("        type filter hook forward priority filter; policy drop;\n");
    ruleset.push('\n');
    ruleset.push_str("        ct state established,related accept comment \"return traffic for permitted forwarded flows\"\n");
    ruleset
        .push_str("        ct state invalid counter drop comment \"invalid forwarded packets\"\n");
    ruleset.push('\n');
    ruleset
        .push_str("        # Only the trusted LAN may initiate forwarded flows towards the DMZ.\n");
    ruleset.push_str(&format!(
        "        iifname \"{lan_bridge}\" oifname \"{dmz_bridge}\" accept comment \"LAN may reach DMZ workloads\"\n"
    ));
    ruleset
        .push_str("        # The DMZ must never reach the LAN. This is the core isolation rule.\n");
    ruleset.push_str(&format!(
        "        iifname \"{dmz_bridge}\" oifname \"{lan_bridge}\" drop comment \"DMZ must never reach LAN\"\n"
    ));
    match &egress_wan {
        Some(wan) => {
            ruleset.push_str(
                "        # DMZ outbound internet is explicitly allowed through the appliance\n",
            );
            ruleset.push_str(
                "        # uplink: required for updates and for the outbound tunnel client\n",
            );
            ruleset.push_str(
                "        # (cloudflared/WireGuard). The DMZ -> LAN drop above still wins.\n",
            );
            ruleset.push_str(&format!(
                "        iifname \"{dmz_bridge}\" oifname \"{wan}\" accept comment \"DMZ outbound internet (updates, tunnel)\"\n"
            ));
        }
        None if plan.allow_dmz_egress => {
            ruleset.push_str(
                "        # DMZ egress was requested (allow_dmz_egress = true) but no WAN\n",
            );
            ruleset
                .push_str("        # interface is configured: outbound internet stays blocked.\n");
        }
        None => {
            ruleset.push_str(
                "        # DMZ outbound internet is disabled (allow_dmz_egress = false).\n",
            );
        }
    }
    ruleset.push_str(
        "        # No other forwarding is permitted; the policy drop above handles it.\n",
    );
    ruleset.push_str("    }\n");
    ruleset.push('\n');

    // --- Output chain: host initiated traffic ------------------------------
    ruleset.push_str("    chain output {\n");
    ruleset.push_str("        type filter hook output priority filter; policy accept;\n");
    ruleset.push('\n');
    ruleset
        .push_str("        # Host-initiated traffic (encrypted DNS, updates, assistant API) is\n");
    ruleset.push_str("        # allowed: the appliance itself is the trusted endpoint.\n");
    ruleset.push_str("    }\n");
    ruleset.push_str("}\n");

    // --- NAT table: masquerade DMZ outbound internet -----------------------
    if let Some(wan) = &egress_wan {
        ruleset.push('\n');
        ruleset.push_str(
            "# NAT table: masquerade DMZ egress behind the appliance uplink so updates and\n",
        );
        ruleset.push_str(
            "# the outbound tunnel client (cloudflared/WireGuard) work without opening any\n",
        );
        ruleset.push_str("# port on the home router.\n");
        ruleset.push_str("table ip homeserver_nat {\n");
        ruleset.push_str("    chain postrouting {\n");
        ruleset.push_str("        type nat hook postrouting priority srcnat; policy accept;\n");
        ruleset.push('\n');
        ruleset.push_str(&format!(
            "        oifname \"{wan}\" ip saddr {dmz_subnet} masquerade comment \"DMZ outbound internet\"\n"
        ));
        ruleset.push_str("    }\n");
        ruleset.push_str("}\n");
    }

    ruleset
}

/// Validate that a rendered ruleset still enforces DMZ isolation.
///
/// This is the regression gate for the security invariant: it is called before
/// [`apply`] and must be kept in sync with [`render_nftables`]. It rejects a
/// ruleset when:
///
/// - the `inet homeserver` table or the `input`/`forward` chains are missing;
/// - either chain does not use `policy drop`;
/// - the `established,related` accept rule is missing or comes after a drop;
/// - no inter-bridge drop rule exists (the DMZ -> LAN drop);
/// - a cross-bridge accept has no matching reverse drop or contradicts a drop;
/// - a DMZ-sourced accept targets the trusted side instead of the WAN uplink;
/// - a DMZ egress accept is not backed by a `table ip homeserver_nat`
///   `postrouting` masquerade rule on the same WAN interface;
/// - the input chain does not explicitly drop DMZ traffic, or allows DMZ
///   traffic to a port other than the DNS/NTP allow list.
///
/// # Errors
/// Returns [`NetworkError::Invalid`] describing the violated invariant.
pub fn validate_isolation(ruleset: &str) -> NetworkResult<()> {
    let parsed = parse_ruleset(ruleset);
    if !parsed.inet_table {
        return Err(NetworkError::Invalid(
            "ruleset does not define `table inet homeserver`".to_owned(),
        ));
    }
    let forward = find_chain(&parsed.chains, "forward").ok_or_else(|| {
        NetworkError::Invalid("ruleset does not define the `forward` chain".to_owned())
    })?;
    let input = find_chain(&parsed.chains, "input").ok_or_else(|| {
        NetworkError::Invalid("ruleset does not define the `input` chain".to_owned())
    })?;

    if forward.policy != Some(ChainPolicy::Drop) {
        return Err(NetworkError::Invalid(
            "the `forward` chain must use policy drop".to_owned(),
        ));
    }
    if input.policy != Some(ChainPolicy::Drop) {
        return Err(NetworkError::Invalid(
            "the `input` chain must use policy drop".to_owned(),
        ));
    }

    check_established_first(&forward.rules, "forward")?;
    check_established_first(&input.rules, "input")?;

    let mut accepts: Vec<(&str, &str)> = Vec::new();
    let mut drops: Vec<(&str, &str)> = Vec::new();
    for rule in &forward.rules {
        let (Some(source), Some(destination)) = (
            interface_name(rule, "iifname"),
            interface_name(rule, "oifname"),
        ) else {
            continue;
        };
        if source == destination {
            continue;
        }
        match rule_action(rule) {
            Some(RuleAction::Drop) => drops.push((source, destination)),
            Some(RuleAction::Accept) => accepts.push((source, destination)),
            None => {}
        }
    }

    if drops.is_empty() {
        return Err(NetworkError::Invalid(
            "the `forward` chain does not drop any inter-bridge traffic: DMZ isolation is missing"
                .to_owned(),
        ));
    }

    // Bridges that are explicitly dropped as forward sources are the untrusted
    // (DMZ) bridges; the targets of those drops are the trusted side. The DMZ
    // may only reach the WAN uplink and never the trusted side.
    let dmz_bridges: Vec<&str> = drops.iter().map(|(source, _)| *source).collect();
    let trusted_bridges: Vec<&str> = drops.iter().map(|(_, destination)| *destination).collect();

    let mut egress_accepts: Vec<(&str, &str)> = Vec::new();
    for (source, destination) in &accepts {
        let source_is_dmz = dmz_bridges.iter().any(|bridge| bridge == source)
            || (source.to_ascii_lowercase().contains("dmz")
                && !trusted_bridges.iter().any(|bridge| bridge == source));
        if source_is_dmz {
            // A DMZ-sourced accept is only tolerated as outbound internet
            // egress: the destination must not be a bridge of the forwarding
            // graph (the trusted side) but the WAN uplink, which is mentioned
            // exactly once in the forward chain.
            let destination_is_trusted = trusted_bridges.iter().any(|bridge| bridge == destination)
                || forward_interface_mentions(&forward.rules, destination) > 1;
            if destination_is_trusted {
                return Err(NetworkError::Invalid(format!(
                    "forward rule accepts traffic from the untrusted `{source}` bridge towards the trusted `{destination}` side"
                )));
            }
            egress_accepts.push((*source, *destination));
            continue;
        }

        if drops.iter().any(|(drop_source, drop_destination)| {
            drop_source == source && drop_destination == destination
        }) {
            return Err(NetworkError::Invalid(format!(
                "the `forward` chain both accepts and drops traffic from `{source}` to `{destination}`"
            )));
        }
        if accepts.iter().any(|(other_source, other_destination)| {
            other_source == destination && other_destination == source
        }) {
            return Err(NetworkError::Invalid(format!(
                "bidirectional forwarding between `{source}` and `{destination}` is not allowed"
            )));
        }
        let has_reverse_drop = drops.iter().any(|(drop_source, drop_destination)| {
            drop_source == destination && drop_destination == source
        });
        if !has_reverse_drop {
            return Err(NetworkError::Invalid(format!(
                "cross-bridge accept `{source}` -> `{destination}` has no matching reverse drop"
            )));
        }
    }

    // Outbound DMZ internet must be masqueraded behind the appliance uplink;
    // otherwise it cannot work without opening ports on the home router.
    if !egress_accepts.is_empty() {
        if !parsed.nat_table {
            return Err(NetworkError::Invalid(
                "DMZ egress is accepted in the `forward` chain but no `table ip homeserver_nat` is rendered"
                    .to_owned(),
            ));
        }
        let postrouting = find_chain(&parsed.chains, "postrouting").ok_or_else(|| {
            NetworkError::Invalid(
                "the `homeserver_nat` table is missing the `postrouting` chain".to_owned(),
            )
        })?;
        for (_dmz, wan) in &egress_accepts {
            let masqueraded = postrouting.rules.iter().any(|rule| {
                rule.contains("masquerade") && interface_name(rule, "oifname") == Some(*wan)
            });
            if !masqueraded {
                return Err(NetworkError::Invalid(format!(
                    "DMZ egress through `{wan}` is accepted but not masqueraded by `table ip homeserver_nat`"
                )));
            }
        }
    }

    let mut dmz_input_drop = false;
    for rule in &input.rules {
        let mentions_dmz = rule.contains("@dmz_subnets")
            || dmz_bridges
                .iter()
                .any(|bridge| rule.contains(&format!("\"{bridge}\"")));
        if !mentions_dmz {
            continue;
        }
        match rule_action(rule) {
            Some(RuleAction::Drop) => {
                // Require an unconditional DMZ drop (no port match) so the
                // "drop everything else" intent is explicit and auditable, not
                // just implied by the chain policy.
                if !rule.contains("dport") {
                    dmz_input_drop = true;
                }
            }
            Some(RuleAction::Accept) => {
                // A DMZ -> host accept rule is only tolerated when it is
                // explicitly part of the DNS/NTP allow list (the rendered rules
                // carry a `DMZ DNS`/`DMZ NTP` comment, which is what makes the
                // invariant auditable).
                let lower = rule.to_ascii_lowercase();
                if !(lower.contains("dns") || lower.contains("ntp")) {
                    return Err(NetworkError::Invalid(format!(
                        "DMZ input rule is outside the explicit DNS/NTP allow list: {rule}"
                    )));
                }
            }
            None => {}
        }
    }
    if !dmz_input_drop {
        return Err(NetworkError::Invalid(
            "the `input` chain does not explicitly drop DMZ traffic addressed to host services"
                .to_owned(),
        ));
    }

    Ok(())
}

/// Write a rendered ruleset to `path` atomically.
///
/// The file is written through a temporary sibling and renamed into place so a
/// crash can never leave a partially written firewall configuration behind.
///
/// # Errors
/// Returns [`NetworkError::Io`] when the directory cannot be created or the
/// file cannot be written.
pub fn write_ruleset(path: &Path, rules: &str) -> NetworkResult<()> {
    atomic_write(path, rules)
}

/// Render, validate, write and apply the ruleset with `nft -f`.
///
/// On platforms other than Linux this returns [`NetworkError::Unsupported`]
/// without touching the target file. On Linux the freshly rendered ruleset is
/// re-validated with [`validate_isolation`] before it is written and applied.
///
/// # Errors
/// Returns [`NetworkError::Invalid`] for an invalid plan or a rejected ruleset,
/// and [`NetworkError::Apply`] when `nft` is missing or fails.
pub async fn apply(plan: &FirewallPlan, target_path: &Path) -> NetworkResult<()> {
    validate_plan(plan)?;

    if !cfg!(target_os = "linux") {
        return Err(NetworkError::Unsupported(
            "nftables can only be applied on Linux".to_owned(),
        ));
    }

    let ruleset = render_nftables(plan);
    validate_isolation(&ruleset)?;
    write_ruleset(target_path, &ruleset)?;

    let output = tokio::process::Command::new("nft")
        .arg("-f")
        .arg(target_path)
        .output()
        .await
        .map_err(|err| NetworkError::Apply(format!("failed to run `nft -f`: {err}")))?;
    if !output.status.success() {
        return Err(NetworkError::Apply(format!(
            "`nft -f {}` failed: {}",
            target_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// Apply an already rendered ruleset file (used by rollback).
///
/// The file is re-validated before touching the kernel, so a corrupted backup
/// cannot leave the appliance without its isolation invariants.
///
/// # Errors
/// Returns [`NetworkError::Unsupported`] off Linux, [`NetworkError::Io`] when
/// the file cannot be read and [`NetworkError::Apply`] when `nft -f` fails.
pub async fn apply_ruleset(target_path: &Path) -> NetworkResult<()> {
    if !cfg!(target_os = "linux") {
        return Err(NetworkError::Unsupported(
            "nftables can only be applied on Linux".to_owned(),
        ));
    }
    let ruleset = std::fs::read_to_string(target_path)
        .map_err(|err| NetworkError::Io(format!("failed to read {target_path:?}: {err}")))?;
    validate_isolation(&ruleset)?;
    let output = tokio::process::Command::new("nft")
        .arg("-f")
        .arg(target_path)
        .output()
        .await
        .map_err(|err| NetworkError::Apply(format!("failed to run `nft -f`: {err}")))?;
    if !output.status.success() {
        return Err(NetworkError::Apply(format!(
            "`nft -f {}` failed: {}",
            target_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// Validate a Linux network interface name used inside the ruleset.
fn validate_bridge_name(name: &str, field: &str) -> NetworkResult<()> {
    let valid = !name.is_empty()
        && name.len() <= 15
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'));
    if valid {
        Ok(())
    } else {
        Err(NetworkError::Invalid(format!(
            "{field} `{name}` is not a valid Linux interface name"
        )))
    }
}

/// Strip characters that could break out of an nft double-quoted string.
fn sanitize_interface(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
        .collect();
    if cleaned.is_empty() {
        "invalid0".to_owned()
    } else {
        cleaned
    }
}

/// Whether two IPv4 networks share at least one address.
fn subnets_overlap(left: &Ipv4Net, right: &Ipv4Net) -> bool {
    left.contains(&right.network()) || right.contains(&left.network())
}

/// Policy declared on a base chain header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChainPolicy {
    /// Default drop.
    Drop,
    /// Default accept.
    Accept,
}

/// Verdict declared by a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleAction {
    /// Drop the packet.
    Drop,
    /// Accept the packet.
    Accept,
}

/// Minimal representation of a parsed chain, used by [`validate_isolation`].
struct ChainBlock<'a> {
    name: &'a str,
    policy: Option<ChainPolicy>,
    rules: Vec<&'a str>,
}

/// Parsed representation of a rendered ruleset.
struct ParsedRuleset<'a> {
    /// Whether the `table inet homeserver` filter table is present.
    inet_table: bool,
    /// Whether the `table ip homeserver_nat` NAT table is present.
    nat_table: bool,
    /// Base chains found in the file (both tables).
    chains: Vec<ChainBlock<'a>>,
}

/// Parse the table presence flags and the base chains of a ruleset.
fn parse_ruleset(ruleset: &str) -> ParsedRuleset<'_> {
    let mut inet_table = false;
    let mut nat_table = false;
    let mut chains: Vec<ChainBlock<'_>> = Vec::new();
    let mut current: Option<ChainBlock<'_>> = None;

    for line in ruleset.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        match current.as_mut() {
            None => {
                if trimmed.starts_with("table inet homeserver") {
                    inet_table = true;
                } else if trimmed.starts_with("table ip homeserver_nat") {
                    nat_table = true;
                } else if let Some(rest) = trimmed.strip_prefix("chain ") {
                    let name = rest.trim_end_matches('{').trim();
                    current = Some(ChainBlock {
                        name,
                        policy: None,
                        rules: Vec::new(),
                    });
                }
            }
            Some(block) => {
                if trimmed == "}" {
                    if let Some(finished) = current.take() {
                        chains.push(finished);
                    }
                } else if trimmed.starts_with("type filter hook") {
                    if trimmed.contains("policy drop") {
                        block.policy = Some(ChainPolicy::Drop);
                    } else if trimmed.contains("policy accept") {
                        block.policy = Some(ChainPolicy::Accept);
                    }
                } else {
                    block.rules.push(trimmed);
                }
            }
        }
    }
    if let Some(finished) = current.take() {
        chains.push(finished);
    }
    ParsedRuleset {
        inet_table,
        nat_table,
        chains,
    }
}

/// Find a base chain by name.
fn find_chain<'a>(chains: &'a [ChainBlock<'a>], name: &str) -> Option<&'a ChainBlock<'a>> {
    chains.iter().find(|chain| chain.name == name)
}

/// Ensure the `established,related` accept rule exists and precedes any drop.
fn check_established_first(rules: &[&str], chain: &str) -> NetworkResult<()> {
    let established = rules
        .iter()
        .position(|rule| rule.contains("ct state established,related accept"))
        .ok_or_else(|| {
            NetworkError::Invalid(format!(
                "the `{chain}` chain is missing the `ct state established,related accept` rule"
            ))
        })?;
    let first_drop = rules
        .iter()
        .position(|rule| rule_action(rule) == Some(RuleAction::Drop));
    if let Some(first_drop) = first_drop {
        if established > first_drop {
            return Err(NetworkError::Invalid(format!(
                "the `{chain}` chain drops traffic before accepting established,related traffic"
            )));
        }
    }
    Ok(())
}

/// Extract the verdict of a rule, ignoring its trailing comment.
fn rule_action(rule: &str) -> Option<RuleAction> {
    let matching = rule.split(" comment ").next().unwrap_or(rule);
    if matching.split_whitespace().any(|token| token == "drop") {
        Some(RuleAction::Drop)
    } else if matching.split_whitespace().any(|token| token == "accept") {
        Some(RuleAction::Accept)
    } else {
        None
    }
}

/// Extract the value of a quoted `iifname`/`oifname` match from a rule.
fn interface_name<'a>(rule: &'a str, keyword: &str) -> Option<&'a str> {
    let needle = format!("{keyword} \"");
    let start = rule.find(&needle)? + needle.len();
    let rest = &rule[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Count how many forward rules mention `interface` as source or destination.
///
/// Used by [`validate_isolation`] to distinguish the WAN uplink (mentioned only
/// by the DMZ egress accept) from a bridge that participates in the forwarding
/// graph (for example the trusted LAN bridge).
fn forward_interface_mentions(rules: &[&str], interface: &str) -> usize {
    rules
        .iter()
        .filter(|rule| {
            interface_name(rule, "iifname") == Some(interface)
                || interface_name(rule, "oifname") == Some(interface)
        })
        .count()
}

#[cfg(test)]
mod tests {
    use ipnet::Ipv4Net;

    use super::{
        apply, render_nftables, validate_isolation, validate_plan, write_ruleset, FirewallPlan,
    };
    use crate::error::NetworkError;

    fn plan() -> FirewallPlan {
        FirewallPlan {
            lan_bridge: "br-lan".to_owned(),
            dmz_bridge: "br-dmz".to_owned(),
            admin_port: 8443,
            dns_port: 53,
            dmz_allow_dns: true,
            dmz_subnet: "10.20.30.0/24".parse().expect("dmz subnet"),
            lan_subnet: Some("192.168.1.0/24".parse().expect("lan subnet")),
            wan_interface: Some("eth0".to_owned()),
            allow_dmz_egress: true,
        }
    }

    #[test]
    fn rendered_ruleset_has_required_structure() {
        let rendered = render_nftables(&plan());
        assert!(rendered.starts_with("#!/usr/sbin/nft -f"));
        assert!(rendered.contains("table inet homeserver {"));
        assert_eq!(
            rendered
                .matches("type filter hook forward priority filter; policy drop;")
                .count(),
            1
        );
        assert_eq!(
            rendered
                .matches("type filter hook input priority filter; policy drop;")
                .count(),
            1
        );
        assert!(rendered.contains("ct state established,related accept"));
        assert!(rendered.contains("ct state invalid counter drop"));
        assert!(rendered.contains("iifname \"br-lan\" oifname \"br-dmz\" accept"));
        assert!(rendered.contains("iifname \"br-dmz\" oifname \"br-lan\" drop"));
        assert!(rendered.contains("DMZ must never reach LAN"));
        assert!(rendered.contains("DMZ must never reach the administrator portal"));
    }

    #[test]
    fn dmz_egress_renders_forward_accept_and_nat() {
        let rendered = render_nftables(&plan());
        assert!(
            rendered.contains(
                "iifname \"br-dmz\" oifname \"eth0\" accept comment \"DMZ outbound internet (updates, tunnel)\""
            ),
            "missing DMZ egress forward accept:\n{rendered}"
        );
        assert!(rendered.contains("table ip homeserver_nat {"));
        assert!(rendered.contains("type nat hook postrouting priority srcnat; policy accept;"));
        assert!(rendered.contains(
            "oifname \"eth0\" ip saddr 10.20.30.0/24 masquerade comment \"DMZ outbound internet\""
        ));

        // The DMZ -> LAN drop must be evaluated before the WAN egress accept.
        let dmz_lan_drop = rendered
            .find("iifname \"br-dmz\" oifname \"br-lan\" drop")
            .expect("dmz -> lan drop");
        let dmz_wan_accept = rendered
            .find("iifname \"br-dmz\" oifname \"eth0\" accept")
            .expect("dmz -> wan accept");
        assert!(
            dmz_lan_drop < dmz_wan_accept,
            "DMZ -> LAN drop must precede the DMZ -> WAN accept"
        );
        validate_isolation(&rendered).expect("egress ruleset must be isolated");
    }

    #[test]
    fn dmz_egress_disabled_renders_no_wan_accept_or_nat() {
        let mut plan = plan();
        plan.allow_dmz_egress = false;
        let rendered = render_nftables(&plan);
        assert!(!rendered.contains("iifname \"br-dmz\" oifname \"eth0\" accept"));
        assert!(!rendered.contains("homeserver_nat"));
        assert!(!rendered.contains("masquerade"));
        assert!(rendered.contains("DMZ outbound internet is disabled"));
        // The isolation rules are unaffected.
        assert!(rendered.contains("iifname \"br-dmz\" oifname \"br-lan\" drop"));
        validate_isolation(&rendered).expect("non-egress ruleset must be isolated");
    }

    #[test]
    fn dmz_egress_without_wan_is_rejected_and_not_rendered() {
        let mut plan = plan();
        plan.wan_interface = None;
        plan.allow_dmz_egress = true;
        assert!(matches!(
            validate_plan(&plan),
            Err(NetworkError::Invalid(_))
        ));

        let rendered = render_nftables(&plan);
        assert!(rendered.contains("no WAN"));
        assert!(!rendered.contains("homeserver_nat"));
        assert!(!rendered.contains("oifname \"eth0\" accept"));
        validate_isolation(&rendered).expect("rendered egress-disabled ruleset must be isolated");
    }

    #[test]
    fn golden_dmz_lan_drop_uses_plan_bridge_names() {
        let rendered = render_nftables(&plan());
        assert!(
            rendered.contains("iifname \"br-dmz\" oifname \"br-lan\" drop"),
            "missing golden DMZ -> LAN drop rule:\n{rendered}"
        );

        let mut custom = plan();
        custom.lan_bridge = "lan0".to_owned();
        custom.dmz_bridge = "dmz0".to_owned();
        let rendered = render_nftables(&custom);
        assert!(rendered.contains("iifname \"dmz0\" oifname \"lan0\" drop"));
        assert!(rendered.contains("iifname \"lan0\" oifname \"dmz0\" accept"));
    }

    #[test]
    fn dmz_cannot_reach_admin_port() {
        let rendered = render_nftables(&plan());
        let admin_drop = rendered
            .lines()
            .find(|line| line.contains("tcp dport 8443") && line.contains(" drop"))
            .expect("DMZ -> admin port drop rule");
        assert!(admin_drop.contains("iifname \"br-dmz\""));
        assert!(admin_drop.contains("ip saddr @dmz_subnets"));
        assert!(admin_drop.contains("counter drop"));
    }

    #[test]
    fn lan_only_admin_accept() {
        let rendered = render_nftables(&plan());
        let admin_accept = rendered
            .lines()
            .find(|line| line.contains("tcp dport 8443") && line.contains(" accept"))
            .expect("LAN -> admin port accept rule");
        assert!(admin_accept.contains("iifname \"br-lan\""));
        assert!(admin_accept.contains("ip saddr @lan_subnets"));
        assert!(
            !rendered.contains("iifname \"br-dmz\" ip saddr @dmz_subnets tcp dport 8443 accept")
        );
    }

    #[test]
    fn drops_come_after_established_accept() {
        let rendered = render_nftables(&plan());
        let established = rendered
            .find("ct state established,related accept")
            .expect("established rule");
        let first_drop = rendered.find("counter drop").expect("drop rule");
        assert!(
            established < first_drop,
            "established accept must precede drop rules"
        );
        let dmz_lan_drop = rendered
            .find("iifname \"br-dmz\" oifname \"br-lan\" drop")
            .expect("dmz -> lan drop");
        assert!(established < dmz_lan_drop);
    }

    #[test]
    fn dmz_dns_allow_list_is_configurable() {
        let mut plan = plan();
        plan.dmz_allow_dns = false;
        let rendered = render_nftables(&plan);
        assert!(!rendered.contains("DMZ DNS (TCP)"));
        assert!(!rendered.contains("DMZ DNS (UDP)"));
        assert!(!rendered.contains("DMZ NTP"));
        assert!(rendered.contains("dmz_allow_dns = false"));
        // The explicit portal drop is still present even when DNS is off.
        assert!(rendered.contains("DMZ must never reach the administrator portal"));
    }

    #[test]
    fn rendered_ruleset_passes_isolation_validation() {
        for dmz_allow_dns in [true, false] {
            for allow_dmz_egress in [true, false] {
                let mut plan = plan();
                plan.dmz_allow_dns = dmz_allow_dns;
                plan.allow_dmz_egress = allow_dmz_egress;
                let rendered = render_nftables(&plan);
                validate_isolation(&rendered).expect("rendered ruleset must be isolated");
            }
        }
        let mut no_lan_subnet = plan();
        no_lan_subnet.lan_subnet = None;
        let rendered = render_nftables(&no_lan_subnet);
        assert!(rendered.contains("iifname \"br-lan\" tcp dport 8443 accept"));
        validate_isolation(&rendered).expect("ruleset without LAN subnet must be isolated");

        let mut no_wan = plan();
        no_wan.wan_interface = None;
        no_wan.allow_dmz_egress = false;
        let rendered = render_nftables(&no_wan);
        validate_isolation(&rendered).expect("ruleset without WAN must be isolated");
    }

    #[test]
    fn validation_rejects_dmz_to_lan_accept() {
        let rendered = render_nftables(&plan());
        let tampered = rendered.replace(
            "iifname \"br-dmz\" oifname \"br-lan\" drop",
            "iifname \"br-dmz\" oifname \"br-lan\" accept",
        );
        assert_ne!(rendered, tampered);
        assert!(
            matches!(validate_isolation(&tampered), Err(NetworkError::Invalid(_))),
            "validator must reject a DMZ -> LAN accept rule"
        );
    }

    #[test]
    fn validation_rejects_missing_drops() {
        let rendered = render_nftables(&plan());
        let without_isolation = rendered.replace(
            "        iifname \"br-dmz\" oifname \"br-lan\" drop comment \"DMZ must never reach LAN\"\n",
            "",
        );
        assert_ne!(rendered, without_isolation);
        assert!(
            matches!(
                validate_isolation(&without_isolation),
                Err(NetworkError::Invalid(_))
            ),
            "validator must reject a ruleset without the DMZ -> LAN drop"
        );

        let without_input_drop = rendered.replace(
            "        iifname \"br-dmz\" ip saddr @dmz_subnets counter drop comment \"DMZ must never reach other host services\"\n",
            "",
        );
        assert!(
            matches!(
                validate_isolation(&without_input_drop),
                Err(NetworkError::Invalid(_))
            ),
            "validator must reject a ruleset without the DMZ -> host drop"
        );
    }

    #[test]
    fn validation_rejects_wrong_policies_and_missing_table() {
        let rendered = render_nftables(&plan());
        let accept_forward = rendered.replace(
            "type filter hook forward priority filter; policy drop;",
            "type filter hook forward priority filter; policy accept;",
        );
        assert!(matches!(
            validate_isolation(&accept_forward),
            Err(NetworkError::Invalid(_))
        ));

        let no_table = rendered.replace("table inet homeserver {", "table inet other {");
        assert!(matches!(
            validate_isolation(&no_table),
            Err(NetworkError::Invalid(_))
        ));

        let empty = String::new();
        assert!(matches!(
            validate_isolation(&empty),
            Err(NetworkError::Invalid(_))
        ));
    }

    #[test]
    fn validation_rejects_reordered_established_rule() {
        let rendered = render_nftables(&plan());
        let established_line =
            "        ct state established,related accept comment \"return traffic for permitted forwarded flows\"\n";
        let invalid_line =
            "        ct state invalid counter drop comment \"invalid forwarded packets\"\n";
        let tampered = rendered
            .replace(established_line, "@@ESTABLISHED@@")
            .replace(invalid_line, established_line)
            .replace("@@ESTABLISHED@@", invalid_line);
        assert_ne!(rendered, tampered);
        assert!(matches!(
            validate_isolation(&tampered),
            Err(NetworkError::Invalid(_))
        ));
    }

    #[test]
    fn validation_rejects_dmz_wan_accept_retargeted_to_lan() {
        let rendered = render_nftables(&plan());
        let tampered = rendered.replace(
            "iifname \"br-dmz\" oifname \"eth0\" accept comment \"DMZ outbound internet (updates, tunnel)\"",
            "iifname \"br-dmz\" oifname \"br-lan\" accept comment \"DMZ outbound internet (updates, tunnel)\"",
        );
        assert_ne!(rendered, tampered);
        assert!(
            matches!(validate_isolation(&tampered), Err(NetworkError::Invalid(_))),
            "validator must reject DMZ egress retargeted to the LAN bridge"
        );
    }

    #[test]
    fn validation_rejects_missing_nat_table() {
        let rendered = render_nftables(&plan());
        let nat_start = rendered
            .find("\ntable ip homeserver_nat")
            .expect("NAT table in egress ruleset");
        let without_nat = format!("{}\n", &rendered[..nat_start]);
        assert!(!without_nat.contains("homeserver_nat"));
        assert!(
            matches!(
                validate_isolation(&without_nat),
                Err(NetworkError::Invalid(_))
            ),
            "validator must reject DMZ egress without the NAT table"
        );
    }

    #[test]
    fn validation_rejects_missing_masquerade() {
        let rendered = render_nftables(&plan());
        let tampered = rendered.replace(
            "        oifname \"eth0\" ip saddr 10.20.30.0/24 masquerade comment \"DMZ outbound internet\"\n",
            "",
        );
        assert_ne!(rendered, tampered);
        assert!(
            matches!(validate_isolation(&tampered), Err(NetworkError::Invalid(_))),
            "validator must reject DMZ egress without a NAT masquerade rule"
        );
    }

    #[test]
    fn validation_rejects_dmz_accept_outside_allow_list() {
        let rendered = render_nftables(&plan());
        let tampered = rendered.replace(
            "        iifname \"br-dmz\" ip saddr @dmz_subnets tcp dport 8443 counter drop",
            "        iifname \"br-dmz\" ip saddr @dmz_subnets tcp dport 8443 accept",
        );
        // The rule now reads `accept comment "DMZ must never..."` and must be rejected.
        assert_ne!(rendered, tampered);
        assert!(matches!(
            validate_isolation(&tampered),
            Err(NetworkError::Invalid(_))
        ));
    }

    #[test]
    fn plan_validation_rejects_unsafe_values() {
        validate_plan(&plan()).expect("default plan is valid");

        let mut bad = plan();
        bad.lan_bridge = "br lan; flush ruleset".to_owned();
        assert!(matches!(validate_plan(&bad), Err(NetworkError::Invalid(_))));

        let mut bad = plan();
        bad.dmz_bridge = bad.lan_bridge.clone();
        assert!(matches!(validate_plan(&bad), Err(NetworkError::Invalid(_))));

        let mut bad = plan();
        bad.admin_port = 0;
        assert!(matches!(validate_plan(&bad), Err(NetworkError::Invalid(_))));

        let mut bad = plan();
        bad.dns_port = bad.admin_port;
        assert!(matches!(validate_plan(&bad), Err(NetworkError::Invalid(_))));

        let mut bad = plan();
        bad.dmz_subnet = "0.0.0.0/0".parse().expect("subnet");
        assert!(matches!(validate_plan(&bad), Err(NetworkError::Invalid(_))));

        let mut bad = plan();
        bad.dmz_subnet = "192.168.1.0/24".parse().expect("subnet");
        assert!(matches!(validate_plan(&bad), Err(NetworkError::Invalid(_))));

        let mut bad = plan();
        bad.wan_interface = None;
        bad.allow_dmz_egress = true;
        assert!(
            matches!(validate_plan(&bad), Err(NetworkError::Invalid(_))),
            "egress without a WAN interface must be rejected"
        );

        let mut bad = plan();
        bad.wan_interface = Some("br-dmz".to_owned());
        assert!(
            matches!(validate_plan(&bad), Err(NetworkError::Invalid(_))),
            "the WAN interface must not be the DMZ bridge"
        );

        let mut bad = plan();
        bad.wan_interface = Some("eth 0".to_owned());
        assert!(matches!(validate_plan(&bad), Err(NetworkError::Invalid(_))));

        let mut disabled = plan();
        disabled.allow_dmz_egress = false;
        disabled.wan_interface = None;
        validate_plan(&disabled).expect("egress disabled without WAN is valid");
    }

    #[test]
    fn unsafe_bridge_names_are_sanitized_in_output() {
        let mut plan = plan();
        plan.dmz_bridge = "br-dmz\" ; flush ruleset #".to_owned();
        let rendered = render_nftables(&plan);
        assert!(!rendered.contains("flush ruleset"));
        assert!(rendered.contains("iifname \"br-dmzflushruleset\""));
    }

    #[test]
    fn writes_ruleset_atomically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nftables").join("homeserver.nft");
        let rendered = render_nftables(&plan());
        write_ruleset(&path, &rendered).expect("write ruleset");
        let written = std::fs::read_to_string(&path).expect("read ruleset");
        assert_eq!(written, rendered);
        write_ruleset(&path, "# replaced\n").expect("replace ruleset");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read replaced ruleset"),
            "# replaced\n"
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[tokio::test]
    async fn apply_is_unsupported_outside_linux() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("homeserver.nft");
        let error = apply(&plan(), &target)
            .await
            .expect_err("apply must be unsupported on this platform");
        assert!(matches!(error, NetworkError::Unsupported(_)));
        assert!(!target.exists(), "unsupported apply must not write a file");
    }

    #[cfg(not(target_os = "linux"))]
    #[tokio::test]
    async fn apply_rejects_invalid_plan_before_platform_check() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut invalid = plan();
        invalid.admin_port = 0;
        let error = apply(&invalid, &dir.path().join("rules.nft"))
            .await
            .expect_err("invalid plan must be rejected");
        assert!(matches!(error, NetworkError::Invalid(_)));
    }

    #[test]
    fn subnets_overlap_detection() {
        let a: Ipv4Net = "10.0.0.0/8".parse().expect("net");
        let b: Ipv4Net = "10.20.30.0/24".parse().expect("net");
        let c: Ipv4Net = "192.168.1.0/24".parse().expect("net");
        assert!(super::subnets_overlap(&a, &b));
        assert!(super::subnets_overlap(&b, &a));
        assert!(!super::subnets_overlap(&a, &c));
    }
}
