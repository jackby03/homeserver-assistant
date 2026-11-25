//! ARP table discovery adapter.
//!
//! The ARP/neighbour table is the only protocol that reliably reveals the MAC
//! address of a device, which is what the vendor lookup and the stable device id
//! depend on.
//!
//! - Linux: reads `/proc/net/arp` (no privileges, no external tool), falling
//!   back to `arp -a`.
//! - Other Unix systems: runs `arp -a`.
//! - Windows: unsupported at runtime (returns no devices) because the appliance
//!   targets Linux; the parsers are still unit tested on every platform.
//!
//! The two parsers are pure and public so they can be tested with captured
//! output from every supported platform.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::time::Duration;

use async_trait::async_trait;
use hs_core::models::DiscoveryProtocol;

use crate::vendor::{normalize_mac, vendor_for_mac};
use crate::{DiscoveryError, DiscoveryResult, DiscoverySource, RawDevice};

/// ARP table discovery source.
#[derive(Debug, Default)]
pub struct ArpSource;

impl ArpSource {
    /// Create an ARP discovery source.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DiscoverySource for ArpSource {
    fn protocol(&self) -> DiscoveryProtocol {
        DiscoveryProtocol::Arp
    }

    async fn discover(&self, _timeout: Duration) -> DiscoveryResult<Vec<RawDevice>> {
        tokio::task::spawn_blocking(read_arp_table)
            .await
            .map_err(|err| DiscoveryError::Transport(format!("ARP reader failed: {err}")))
    }
}

/// Read the operating system ARP table (blocking).
#[cfg(unix)]
fn read_arp_table() -> Vec<RawDevice> {
    #[cfg(target_os = "linux")]
    {
        match std::fs::read_to_string("/proc/net/arp") {
            Ok(text) => {
                let devices = parse_proc_arp(&text);
                if !devices.is_empty() {
                    return devices;
                }
            }
            Err(error) => {
                tracing::debug!(%error, "could not read /proc/net/arp");
            }
        }
    }
    match std::process::Command::new("arp").arg("-a").output() {
        Ok(output) if output.status.success() => {
            parse_arp_output(&String::from_utf8_lossy(&output.stdout))
        }
        Ok(output) => {
            tracing::debug!(status = %output.status, "`arp -a` failed");
            Vec::new()
        }
        Err(error) => {
            tracing::debug!(%error, "`arp -a` is not available");
            Vec::new()
        }
    }
}

/// The appliance discovery runs on Linux; other platforms return no devices.
#[cfg(not(unix))]
fn read_arp_table() -> Vec<RawDevice> {
    tracing::debug!("ARP table inspection is only available on Unix systems");
    Vec::new()
}

/// Parse the Linux `/proc/net/arp` format.
///
/// Example line:
/// `10.20.30.5  0x1  0x2  b8:27:eb:12:34:56  *  br-lan`
///
/// Incomplete or failed entries (`0x0` flags, all-zero MAC) are skipped, as are
/// malformed addresses.
#[must_use]
pub fn parse_proc_arp(text: &str) -> Vec<RawDevice> {
    let mut devices = Vec::new();
    let mut seen: BTreeSet<Ipv4Addr> = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("IP address") {
            continue;
        }
        let columns: Vec<&str> = line.split_whitespace().collect();
        if columns.len() < 4 {
            continue;
        }
        let Ok(ip) = columns[0].parse::<Ipv4Addr>() else {
            continue;
        };
        // Flags: 0x2 = complete entry, 0x0 = incomplete.
        if columns[2].eq_ignore_ascii_case("0x0") {
            continue;
        }
        let Some(mac) = normalize_mac(columns[3]) else {
            continue;
        };
        if !seen.insert(ip) {
            continue;
        }
        devices.push(RawDevice {
            ip: ip.to_string(),
            vendor: vendor_for_mac(&mac).map(str::to_owned),
            mac: Some(mac),
            hostname: None,
            protocol: DiscoveryProtocol::Arp,
        });
    }
    devices
}

/// Parse the output of `arp -a` (Linux/BSD/macOS/Windows formats).
///
/// Supported shapes:
/// - `? (192.168.1.5) at b8:27:eb:12:34:56 [ether] on eth0` (Linux),
/// - `living-room (192.168.1.5) at b8:27:eb:12:34:56 on en0 ifscope [ethernet]`
///   (BSD/macOS),
/// - `192.168.1.1   b8-27-eb-12-34-56   dynamic` (Windows).
///
/// Incomplete entries are skipped; duplicate addresses are collapsed.
#[must_use]
pub fn parse_arp_output(text: &str) -> Vec<RawDevice> {
    let mut devices = Vec::new();
    let mut seen: BTreeSet<Ipv4Addr> = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("Interface:")
            || line.starts_with("Internet Address")
            || line.starts_with("IP address")
        {
            continue;
        }
        let lowercase = line.to_ascii_lowercase();
        if lowercase.contains("incomplete") || lowercase.contains("no entry") {
            continue;
        }

        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(ip) = tokens.iter().find_map(|token| {
            token
                .trim_matches(|c| c == '(' || c == ')' || c == ',')
                .parse::<Ipv4Addr>()
                .ok()
        }) else {
            continue;
        };
        let mac = tokens
            .iter()
            .filter_map(|token| mac_from_token(token))
            .next();
        let Some(mac) = mac else {
            continue;
        };
        if !seen.insert(ip) {
            continue;
        }
        devices.push(RawDevice {
            ip: ip.to_string(),
            vendor: vendor_for_mac(&mac).map(str::to_owned),
            mac: Some(mac),
            hostname: hostname_from_arp_line(line, ip),
            protocol: DiscoveryProtocol::Arp,
        });
    }
    devices
}

/// Extract a MAC address from a whitespace separated token.
///
/// Requires a MAC separator (`:` or `-`, or exactly two dots for Cisco style)
/// so an IPv4 address is never misread as a MAC address.
fn mac_from_token(token: &str) -> Option<String> {
    let has_separator =
        token.contains(':') || token.contains('-') || token.matches('.').count() == 2;
    if !has_separator {
        return None;
    }
    normalize_mac(token)
}

/// Extract the hostname printed by `arp -a` before the address, when present.
fn hostname_from_arp_line(line: &str, ip: Ipv4Addr) -> Option<String> {
    let end = line.find('(')?;
    let hostname = line[..end].trim();
    if hostname.is_empty() || hostname == "?" || hostname.parse::<Ipv4Addr>().ok() == Some(ip) {
        return None;
    }
    let hostname = hostname.trim_end_matches('.').to_ascii_lowercase();
    if hostname.is_empty() {
        None
    } else {
        Some(hostname)
    }
}

#[cfg(test)]
mod tests {
    use hs_core::models::DiscoveryProtocol;

    use super::{parse_arp_output, parse_proc_arp};

    const PROC_ARP: &str =
        "IP address       HW type     Flags       HW address            Mask     Device\n\
        10.20.30.5       0x1         0x2         b8:27:eb:12:34:56     *        br-lan\n\
        10.20.30.6       0x1         0x2         ac:67:b2:aa:bb:cc     *        br-lan\n\
        10.20.30.7       0x1         0x0         00:00:00:00:00:00     *        br-lan\n\
        10.20.30.8       0x1         0x2         34:94:54:00:00:01     *        br-dmz\n\
        not-an-ip        0x1         0x2         b8:27:eb:00:00:00     *        br-lan\n";

    const LINUX_ARP: &str = "? (192.168.1.5) at b8:27:eb:12:34:56 [ether] on eth0\n\
        living-room-tv (192.168.1.6) at ac:67:b2:aa:bb:cc [ether] on eth0\n\
        ? (192.168.1.7) at <incomplete> on eth0\n";

    const WINDOWS_ARP: &str = "Interface: 192.168.1.10 --- 0x5\n\
        Internet Address      Physical Address      Type\n\
        192.168.1.1           b8-27-eb-12-34-56     dynamic\n\
        192.168.1.20          ac-67-b2-aa-bb-cc     dynamic\n\
        192.168.1.30          ff-ff-ff-ff-ff-ff     static\n";

    #[test]
    fn parses_proc_net_arp() {
        let devices = parse_proc_arp(PROC_ARP);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].ip, "10.20.30.5");
        assert_eq!(devices[0].mac.as_deref(), Some("b8:27:eb:12:34:56"));
        assert_eq!(devices[0].vendor.as_deref(), Some("Raspberry Pi"));
        assert_eq!(devices[0].protocol, DiscoveryProtocol::Arp);
        assert_eq!(devices[1].vendor.as_deref(), Some("Espressif"));
        assert_eq!(devices[2].vendor.as_deref(), Some("Shelly"));
    }

    #[test]
    fn parses_linux_arp_a_output() {
        let devices = parse_arp_output(LINUX_ARP);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].ip, "192.168.1.5");
        assert_eq!(devices[0].hostname, None);
        assert_eq!(devices[1].ip, "192.168.1.6");
        assert_eq!(devices[1].hostname.as_deref(), Some("living-room-tv"));
        assert_eq!(devices[1].vendor.as_deref(), Some("Espressif"));
    }

    #[test]
    fn parses_windows_arp_a_output() {
        let devices = parse_arp_output(WINDOWS_ARP);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].ip, "192.168.1.1");
        assert_eq!(devices[0].mac.as_deref(), Some("b8:27:eb:12:34:56"));
        assert_eq!(devices[1].mac.as_deref(), Some("ac:67:b2:aa:bb:cc"));
        // Broadcast-only entries are parsed but resolve to an unknown vendor.
        assert_eq!(devices[2].vendor, None);
    }

    #[test]
    fn ignores_garbage_and_duplicates() {
        let text = "garbage without ip\n\
            Interface: 192.168.1.10 --- 0x5\n\
            192.168.1.5   b8-27-eb-12-34-56   dynamic\n\
            192.168.1.5   b8-27-eb-12-34-56   dynamic\n";
        let devices = parse_arp_output(text);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].ip, "192.168.1.5");
        assert!(parse_proc_arp("").is_empty());
        assert!(parse_arp_output("").is_empty());
    }

    #[test]
    fn parses_dot_separated_mac_from_bsd_output() {
        let text = "? (10.0.0.9) at a4:83:e7:11:22:33 on en0 ifscope [ethernet]\n";
        let devices = parse_arp_output(text);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].ip, "10.0.0.9");
        assert_eq!(devices[0].vendor.as_deref(), Some("Apple"));
    }
}
