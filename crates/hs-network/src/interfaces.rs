//! Best-effort local network interface detection.
//!
//! The onboarding wizard and the network page display the physical interfaces
//! available on the appliance so the operator can pick the LAN uplink. The
//! detection is intentionally tolerant: on platforms where the MAC address
//! cannot be read it stays `None`, and failures to enumerate interfaces return
//! an empty list instead of an error (the UI then offers the default `eth0`).

use std::collections::BTreeMap;

use get_if_addrs::{get_if_addrs, IfAddr};
use serde::{Deserialize, Serialize};

/// A detected network interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceInfo {
    /// Interface name, e.g. `eth0`.
    pub name: String,
    /// Primary MAC address in lower-case colon notation when known.
    pub mac: Option<String>,
    /// IPv4 addresses assigned to the interface, sorted and deduplicated.
    pub ipv4: Vec<String>,
    /// Whether the interface is administratively up (best effort).
    pub is_up: bool,
}

impl InterfaceInfo {
    /// Whether at least one IPv4 address is assigned.
    #[must_use]
    pub fn has_ipv4(&self) -> bool {
        !self.ipv4.is_empty()
    }

    /// First (canonical) IPv4 address when present.
    #[must_use]
    pub fn primary_ipv4(&self) -> Option<&str> {
        self.ipv4.first().map(String::as_str)
    }
}

/// Detect the non-loopback network interfaces of the host.
///
/// IPv4 addresses are grouped per interface; IPv6-only interfaces are omitted
/// because the appliance management plane is IPv4 only. Loopback addresses are
/// filtered even when the platform reports them under another name.
#[must_use]
pub fn detect() -> Vec<InterfaceInfo> {
    let addresses = match get_if_addrs() {
        Ok(addresses) => addresses,
        Err(error) => {
            tracing::warn!(%error, "failed to enumerate network interfaces");
            return Vec::new();
        }
    };

    let entries = addresses
        .into_iter()
        .filter_map(|interface| match interface.addr {
            IfAddr::V4(address) => Some((
                interface.name,
                address.ip.is_loopback(),
                address.ip.to_string(),
            )),
            IfAddr::V6(_) => None,
        });
    let grouped = group_ipv4(entries);

    grouped
        .into_iter()
        .map(|(name, ipv4)| InterfaceInfo {
            mac: read_mac(&name),
            is_up: interface_is_up(&name),
            name,
            ipv4,
        })
        .collect()
}

/// Group IPv4 addresses per interface name, dropping loopback entries.
fn group_ipv4(
    entries: impl IntoIterator<Item = (String, bool, String)>,
) -> BTreeMap<String, Vec<String>> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, is_loopback, ipv4) in entries {
        if is_loopback {
            continue;
        }
        grouped.entry(name).or_default().push(ipv4);
    }
    for addresses in grouped.values_mut() {
        addresses.sort();
        addresses.dedup();
    }
    grouped
}

/// Read the MAC address of an interface from sysfs.
#[cfg(target_os = "linux")]
fn read_mac(name: &str) -> Option<String> {
    let raw = std::fs::read_to_string(format!("/sys/class/net/{name}/address")).ok()?;
    normalize_mac(raw.trim())
}

/// MAC addresses are not exposed by `get_if_addrs`; degrade to `None`.
#[cfg(not(target_os = "linux"))]
fn read_mac(_name: &str) -> Option<String> {
    None
}

/// Read the operational state of an interface from sysfs.
#[cfg(target_os = "linux")]
fn interface_is_up(name: &str) -> bool {
    match std::fs::read_to_string(format!("/sys/class/net/{name}/operstate")) {
        Ok(state) => !state.trim().eq_ignore_ascii_case("down"),
        Err(_) => true,
    }
}

/// Operational state is not determinable without extra privileges; assume up.
#[cfg(not(target_os = "linux"))]
fn interface_is_up(_name: &str) -> bool {
    true
}

/// Normalize a MAC address to lower-case colon notation.
#[cfg(any(target_os = "linux", test))]
fn normalize_mac(value: &str) -> Option<String> {
    let hex: String = value
        .chars()
        .filter(char::is_ascii_hexdigit)
        .collect::<String>()
        .to_ascii_lowercase();
    if hex.len() != 12 {
        return None;
    }
    let groups: Vec<&str> = (0..6).map(|index| &hex[index * 2..index * 2 + 2]).collect();
    Some(groups.join(":"))
}

#[cfg(test)]
mod tests {
    use super::{detect, group_ipv4, normalize_mac, InterfaceInfo};

    #[test]
    fn groups_and_deduplicates_addresses() {
        let grouped = group_ipv4(vec![
            ("eth0".to_owned(), false, "192.168.1.10".to_owned()),
            ("eth0".to_owned(), false, "192.168.1.10".to_owned()),
            ("eth0".to_owned(), false, "10.0.0.2".to_owned()),
            ("eth1".to_owned(), false, "172.16.0.2".to_owned()),
            ("lo".to_owned(), true, "127.0.0.1".to_owned()),
        ]);
        assert_eq!(
            grouped.get("eth0"),
            Some(&vec!["10.0.0.2".to_owned(), "192.168.1.10".to_owned()])
        );
        assert_eq!(grouped.get("eth1"), Some(&vec!["172.16.0.2".to_owned()]));
        assert!(!grouped.contains_key("lo"));
    }

    #[test]
    fn normalizes_mac_addresses() {
        assert_eq!(
            normalize_mac("AA:BB:CC:DD:EE:FF"),
            Some("aa:bb:cc:dd:ee:ff".to_owned())
        );
        assert_eq!(
            normalize_mac("aa-bb-cc-dd-ee-ff"),
            Some("aa:bb:cc:dd:ee:ff".to_owned())
        );
        assert_eq!(normalize_mac("not-a-mac"), None);
        assert_eq!(normalize_mac("00:11:22"), None);
    }

    #[test]
    fn interface_helpers() {
        let info = InterfaceInfo {
            name: "eth0".to_owned(),
            mac: Some("aa:bb:cc:dd:ee:ff".to_owned()),
            ipv4: vec!["192.168.1.10".to_owned()],
            is_up: true,
        };
        assert!(info.has_ipv4());
        assert_eq!(info.primary_ipv4(), Some("192.168.1.10"));
        let empty = InterfaceInfo {
            ipv4: Vec::new(),
            ..info
        };
        assert!(!empty.has_ipv4());
        assert_eq!(empty.primary_ipv4(), None);
    }

    #[test]
    fn detect_never_returns_loopback_addresses() {
        // The test must not depend on the host interfaces, only on the fact
        // that loopback is filtered out when the platform reports it.
        let interfaces = detect();
        for interface in &interfaces {
            assert!(!interface.name.is_empty());
            for address in &interface.ipv4 {
                assert!(
                    !address
                        .parse::<std::net::Ipv4Addr>()
                        .expect("parsed IPv4")
                        .is_loopback(),
                    "loopback address leaked: {address}"
                );
            }
        }
    }
}
