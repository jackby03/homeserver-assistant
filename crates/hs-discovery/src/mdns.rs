//! mDNS (Bonjour / Avahi) discovery adapter.
//!
//! Browses a small set of common service types for a bounded time and keeps the
//! IPv4 addresses of resolved services. The blocking `mdns-sd` daemon is driven
//! from a dedicated blocking task so it never stalls the async runtime.
//!
//! mDNS normally reveals the hostname but not the MAC address; ARP fills the gap
//! during the merge phase of [`crate::DiscoveryService`].

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use hs_core::models::DiscoveryProtocol;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::{DiscoveryError, DiscoveryResult, DiscoverySource, RawDevice};

/// Service types browsed by the appliance.
///
/// They cover the ecosystems the appliance integrates with (Home Assistant,
/// HomeKit, Matter, Chromecast, ESPHome) without brute-forcing the whole mDNS
/// namespace, which would be noisy on a home network.
pub const MDNS_SERVICE_TYPES: [&str; 6] = [
    "_http._tcp.local.",
    "_hap._tcp.local.",
    "_matter._tcp.local.",
    "_googlecast._tcp.local.",
    "_esphomelib._tcp.local.",
    "_home-assistant._tcp.local.",
];

/// mDNS discovery source.
#[derive(Debug, Default)]
pub struct MdnsSource;

impl MdnsSource {
    /// Create an mDNS discovery source.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DiscoverySource for MdnsSource {
    fn protocol(&self) -> DiscoveryProtocol {
        DiscoveryProtocol::Mdns
    }

    async fn discover(&self, timeout: Duration) -> DiscoveryResult<Vec<RawDevice>> {
        tokio::task::spawn_blocking(move || collect_devices(timeout))
            .await
            .map_err(|err| DiscoveryError::Transport(format!("mDNS collector failed: {err}")))
    }
}

/// Blocking mDNS collection loop bounded by `timeout`.
///
/// Daemon creation and browse errors are tolerated (they are logged and the
/// function returns whatever was collected so far) because discovery must never
/// fail the whole scan.
fn collect_devices(timeout: Duration) -> Vec<RawDevice> {
    let daemon = match ServiceDaemon::new() {
        Ok(daemon) => daemon,
        Err(error) => {
            tracing::warn!(%error, "mDNS daemon could not be started");
            return Vec::new();
        }
    };

    let mut receivers = Vec::new();
    for service_type in MDNS_SERVICE_TYPES {
        match daemon.browse(service_type) {
            Ok(receiver) => receivers.push(receiver),
            Err(error) => {
                tracing::warn!(%error, service_type, "mDNS browse failed");
            }
        }
    }

    let deadline = Instant::now() + timeout;
    let mut devices: BTreeMap<String, RawDevice> = BTreeMap::new();
    while Instant::now() < deadline {
        let mut progressed = false;
        for receiver in &receivers {
            while let Ok(event) = receiver.try_recv() {
                progressed = true;
                if let ServiceEvent::ServiceResolved(info) = event {
                    for device in devices_from_service(&info) {
                        devices.entry(device.ip.clone()).or_insert(device);
                    }
                }
            }
        }
        if !progressed {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    let _ = daemon.shutdown();
    devices.into_values().collect()
}

/// Convert a resolved mDNS service into one raw device per IPv4 address.
fn devices_from_service(info: &ServiceInfo) -> Vec<RawDevice> {
    let addresses = info.get_addresses_v4().into_iter().copied();
    raw_devices_for(normalize_hostname(info.get_hostname()), addresses)
}

/// Build raw mDNS observations from a hostname and a set of IPv4 addresses.
fn raw_devices_for(
    hostname: Option<String>,
    addresses: impl IntoIterator<Item = Ipv4Addr>,
) -> Vec<RawDevice> {
    let mut addresses: Vec<Ipv4Addr> = addresses.into_iter().collect();
    addresses.sort_unstable();
    addresses.dedup();
    addresses
        .into_iter()
        .map(|address| RawDevice {
            ip: address.to_string(),
            mac: None,
            hostname: hostname.clone(),
            vendor: None,
            protocol: DiscoveryProtocol::Mdns,
        })
        .collect()
}

/// Normalize an mDNS hostname, discarding empty values and bare IP addresses.
fn normalize_hostname(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if trimmed.is_empty() || trimmed.parse::<std::net::IpAddr>().is_ok() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::{normalize_hostname, raw_devices_for, MDNS_SERVICE_TYPES};
    use hs_core::models::DiscoveryProtocol;

    #[test]
    fn normalizes_service_hostnames() {
        assert_eq!(
            normalize_hostname("Living-Room-Lamp.local."),
            Some("living-room-lamp.local".to_owned())
        );
        assert_eq!(normalize_hostname("printer"), Some("printer".to_owned()));
        assert_eq!(normalize_hostname("192.168.1.10"), None);
        assert_eq!(normalize_hostname("   "), None);
        assert_eq!(normalize_hostname(""), None);
    }

    #[test]
    fn builds_one_observation_per_ipv4_address() {
        let addresses = [
            Ipv4Addr::new(192, 168, 1, 20),
            Ipv4Addr::new(192, 168, 1, 10),
            Ipv4Addr::new(192, 168, 1, 20),
        ];
        let devices = raw_devices_for(Some("lamp.local".to_owned()), addresses);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].ip, "192.168.1.10");
        assert_eq!(devices[1].ip, "192.168.1.20");
        for device in &devices {
            assert_eq!(device.hostname.as_deref(), Some("lamp.local"));
            assert_eq!(device.mac, None);
            assert_eq!(device.protocol, DiscoveryProtocol::Mdns);
        }
    }

    #[test]
    fn browses_a_focused_set_of_service_types() {
        assert!(MDNS_SERVICE_TYPES.len() >= 3);
        for service_type in MDNS_SERVICE_TYPES {
            assert!(service_type.ends_with(".local."));
            assert!(service_type.starts_with('_'));
        }
    }
}
