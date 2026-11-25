//! Discovery orchestration: run every source and merge the observations.
//!
//! [`DiscoveryService::scan`] waits for each configured [`crate::DiscoverySource`]
//! (bounded by [`DEFAULT_SCAN_TIMEOUT`]), tolerates per-source failures and
//! merges the raw observations into the normalized
//! [`hs_core::models::DiscoveredDevice`] list exposed by the API.
//!
//! Merging is deterministic:
//! - records are matched by MAC address first, then by IPv4 address, so an ARP
//!   observation enriches the hostname reported earlier by mDNS;
//! - protocols are unioned and sorted;
//! - the first non-empty hostname/vendor wins, with the offline OUI table as the
//!   vendor fallback;
//! - the id is `mac:<mac>` when a MAC is known, `ip:<ip>` otherwise.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::Utc;
use hs_core::models::DiscoveredDevice;

use crate::vendor::{normalize_mac, vendor_for_mac};
use crate::{DiscoverySource, RawDevice};

/// Default bound for a full discovery scan.
pub const DEFAULT_SCAN_TIMEOUT: Duration = Duration::from_secs(3);

/// Runs the configured discovery sources and merges their results.
pub struct DiscoveryService {
    sources: Vec<Box<dyn DiscoverySource>>,
}

impl DiscoveryService {
    /// Create a service from an explicit list of sources (composition root).
    #[must_use]
    pub fn new(sources: Vec<Box<dyn DiscoverySource>>) -> Self {
        Self { sources }
    }

    /// Create the service with the three built-in protocol adapters.
    #[must_use]
    pub fn default_sources() -> Self {
        Self::new(vec![
            Box::new(crate::mdns::MdnsSource::new()),
            Box::new(crate::ssdp::SsdpSource::new()),
            Box::new(crate::arp::ArpSource::new()),
        ])
    }

    /// Run a scan with the default timeout.
    pub async fn scan(&self) -> Vec<DiscoveredDevice> {
        self.scan_with_timeout(DEFAULT_SCAN_TIMEOUT).await
    }

    /// Run a scan bounded by `timeout` per source.
    ///
    /// A source that fails or exceeds the timeout is logged and skipped; the
    /// scan itself never returns an error.
    pub async fn scan_with_timeout(&self, timeout: Duration) -> Vec<DiscoveredDevice> {
        let mut observations: Vec<RawDevice> = Vec::new();
        for source in &self.sources {
            let protocol = source.protocol();
            match tokio::time::timeout(timeout, source.discover(timeout)).await {
                Ok(Ok(devices)) => observations.extend(devices),
                Ok(Err(error)) => {
                    tracing::warn!(?protocol, %error, "discovery source failed");
                }
                Err(_) => {
                    tracing::warn!(?protocol, "discovery source timed out");
                }
            }
        }
        merge_devices(observations)
    }
}

impl Default for DiscoveryService {
    fn default() -> Self {
        Self::default_sources()
    }
}

/// Merge raw observations into normalized devices.
///
/// Malformed IPv4 addresses are ignored. The output is sorted by device id so
/// repeated scans without network changes produce stable ordering.
#[must_use]
pub fn merge_devices(observations: Vec<RawDevice>) -> Vec<DiscoveredDevice> {
    let now = Utc::now();
    let mut devices: Vec<DiscoveredDevice> = Vec::new();
    let mut index_by_mac: BTreeMap<String, usize> = BTreeMap::new();
    let mut index_by_ip: BTreeMap<String, usize> = BTreeMap::new();

    for observation in observations {
        let Some(ip) = canonical_ipv4(&observation.ip) else {
            tracing::debug!(
                ip = %observation.ip,
                "ignoring discovery observation with a malformed IPv4 address"
            );
            continue;
        };
        let mac = observation.mac.as_deref().and_then(normalize_mac);
        let hostname = non_empty(observation.hostname);
        let vendor = non_empty(observation.vendor);

        let existing = mac
            .as_ref()
            .and_then(|mac| index_by_mac.get(mac).copied())
            .or_else(|| index_by_ip.get(&ip).copied());

        match existing {
            Some(index) => {
                let device = &mut devices[index];
                if device.mac.is_none() {
                    if let Some(mac) = mac.clone() {
                        device.mac = Some(mac.clone());
                        device.id = format!("mac:{mac}");
                        index_by_mac.insert(mac, index);
                    }
                }
                if device.hostname.is_none() {
                    device.hostname = hostname;
                }
                if device.vendor.is_none() {
                    device.vendor = vendor;
                }
                device.protocols.push(observation.protocol);
                device.protocols.sort();
                device.protocols.dedup();
                device.last_seen = now;
                index_by_ip.insert(ip, index);
            }
            None => {
                let id = match &mac {
                    Some(mac) => format!("mac:{mac}"),
                    None => format!("ip:{ip}"),
                };
                let device = DiscoveredDevice {
                    id,
                    ip: ip.clone(),
                    mac: mac.clone(),
                    hostname,
                    vendor: vendor
                        .or_else(|| mac.as_deref().and_then(vendor_for_mac).map(str::to_owned)),
                    protocols: vec![observation.protocol],
                    last_seen: now,
                    adopted: false,
                };
                let index = devices.len();
                devices.push(device);
                index_by_ip.insert(ip, index);
                if let Some(mac) = mac {
                    index_by_mac.insert(mac, index);
                }
            }
        }
    }

    // Last chance to label devices whose MAC was learned after the vendor was
    // already resolved by a different source.
    for device in &mut devices {
        if device.vendor.is_none() {
            if let Some(vendor) = device.mac.as_deref().and_then(vendor_for_mac) {
                device.vendor = Some(vendor.to_owned());
            }
        }
    }

    devices.sort_by(|left, right| left.id.cmp(&right.id));
    devices
}

/// Canonical dotted IPv4 address, `None` when the value is malformed.
fn canonical_ipv4(value: &str) -> Option<String> {
    value
        .trim()
        .parse::<std::net::Ipv4Addr>()
        .ok()
        .map(|ip| ip.to_string())
}

/// Trim a text field and return `None` when it is empty.
fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use async_trait::async_trait;
    use hs_core::models::{DiscoveredDevice, DiscoveryProtocol};

    use super::{merge_devices, DiscoveryService};
    use crate::{DiscoveryError, DiscoveryResult, DiscoverySource, RawDevice};

    fn raw(
        ip: &str,
        mac: Option<&str>,
        hostname: Option<&str>,
        protocol: DiscoveryProtocol,
    ) -> RawDevice {
        RawDevice {
            ip: ip.to_owned(),
            mac: mac.map(str::to_owned),
            hostname: hostname.map(str::to_owned),
            vendor: None,
            protocol,
        }
    }

    struct FakeSource {
        protocol: DiscoveryProtocol,
        devices: Vec<RawDevice>,
        failure: Option<&'static str>,
        delay: Option<Duration>,
    }

    impl FakeSource {
        fn ok(protocol: DiscoveryProtocol, devices: Vec<RawDevice>) -> Self {
            Self {
                protocol,
                devices,
                failure: None,
                delay: None,
            }
        }

        fn failing(protocol: DiscoveryProtocol) -> Self {
            Self {
                protocol,
                devices: Vec::new(),
                failure: Some("simulated transport failure"),
                delay: None,
            }
        }

        fn slow(protocol: DiscoveryProtocol, delay: Duration) -> Self {
            Self {
                protocol,
                devices: Vec::new(),
                failure: None,
                delay: Some(delay),
            }
        }
    }

    #[async_trait]
    impl DiscoverySource for FakeSource {
        fn protocol(&self) -> DiscoveryProtocol {
            self.protocol
        }

        async fn discover(&self, _timeout: Duration) -> DiscoveryResult<Vec<RawDevice>> {
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            match self.failure {
                Some(message) => Err(DiscoveryError::Transport(message.to_owned())),
                None => Ok(self.devices.clone()),
            }
        }
    }

    #[tokio::test]
    async fn scan_merges_sources_by_ip_and_unions_protocols() {
        let service = DiscoveryService::new(vec![
            Box::new(FakeSource::ok(
                DiscoveryProtocol::Mdns,
                vec![raw(
                    "192.168.1.50",
                    None,
                    Some("lamp.local"),
                    DiscoveryProtocol::Mdns,
                )],
            )),
            Box::new(FakeSource::ok(
                DiscoveryProtocol::Arp,
                vec![raw(
                    "192.168.1.50",
                    Some("AC:67:B2:00:00:01"),
                    None,
                    DiscoveryProtocol::Arp,
                )],
            )),
            Box::new(FakeSource::ok(
                DiscoveryProtocol::Ssdp,
                vec![raw("192.168.1.60", None, None, DiscoveryProtocol::Ssdp)],
            )),
        ]);

        let devices = service.scan_with_timeout(Duration::from_secs(1)).await;
        assert_eq!(devices.len(), 2);
        // Sorted by id: "ip:..." before "mac:...".
        assert_eq!(devices[0].id, "ip:192.168.1.60");
        assert_eq!(devices[0].protocols, vec![DiscoveryProtocol::Ssdp]);
        let lamp = &devices[1];
        assert_eq!(lamp.id, "mac:ac:67:b2:00:00:01");
        assert_eq!(lamp.ip, "192.168.1.50");
        assert_eq!(lamp.hostname.as_deref(), Some("lamp.local"));
        assert_eq!(lamp.vendor.as_deref(), Some("Espressif"));
        assert_eq!(
            lamp.protocols,
            vec![DiscoveryProtocol::Mdns, DiscoveryProtocol::Arp]
        );
        assert!(lamp.last_seen <= chrono::Utc::now());
        assert!(!lamp.adopted);
    }

    #[tokio::test]
    async fn scan_ignores_malformed_addresses() {
        let service = DiscoveryService::new(vec![Box::new(FakeSource::ok(
            DiscoveryProtocol::Ssdp,
            vec![
                raw("not-an-ip", None, None, DiscoveryProtocol::Ssdp),
                raw("192.168.1.20", None, None, DiscoveryProtocol::Ssdp),
            ],
        ))]);
        let devices = service.scan_with_timeout(Duration::from_secs(1)).await;
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].ip, "192.168.1.20");
    }

    #[tokio::test]
    async fn scan_tolerates_failures_and_timeouts() {
        let service = DiscoveryService::new(vec![
            Box::new(FakeSource::failing(DiscoveryProtocol::Ssdp)),
            Box::new(FakeSource::slow(
                DiscoveryProtocol::Mdns,
                Duration::from_millis(200),
            )),
            Box::new(FakeSource::ok(
                DiscoveryProtocol::Arp,
                vec![raw(
                    "10.0.0.9",
                    Some("b8:27:eb:00:00:01"),
                    None,
                    DiscoveryProtocol::Arp,
                )],
            )),
        ]);
        let devices = service.scan_with_timeout(Duration::from_millis(20)).await;
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].ip, "10.0.0.9");
        assert_eq!(devices[0].vendor.as_deref(), Some("Raspberry Pi"));
    }

    #[test]
    fn merge_prefers_mac_identity_over_ip() {
        let devices = merge_devices(vec![
            raw(
                "192.168.1.70",
                None,
                Some("printer"),
                DiscoveryProtocol::Mdns,
            ),
            raw(
                "192.168.1.71",
                Some("00:0e:58:00:00:01"),
                None,
                DiscoveryProtocol::Arp,
            ),
            raw(
                "192.168.1.71",
                Some("00:0e:58:00:00:01"),
                None,
                DiscoveryProtocol::Ssdp,
            ),
        ]);
        assert_eq!(devices.len(), 2);
        let sonos = devices
            .iter()
            .find(|device| device.mac.as_deref() == Some("00:0e:58:00:00:01"))
            .expect("sonos device");
        assert_eq!(sonos.id, "mac:00:0e:58:00:00:01");
        assert_eq!(sonos.ip, "192.168.1.71");
        assert_eq!(sonos.vendor.as_deref(), Some("Sonos"));
        assert_eq!(
            sonos.protocols,
            vec![DiscoveryProtocol::Ssdp, DiscoveryProtocol::Arp]
        );
    }

    #[test]
    fn merge_dedupes_duplicate_observations() {
        let devices = merge_devices(vec![
            raw("10.0.0.2", None, Some("nas"), DiscoveryProtocol::Mdns),
            raw("10.0.0.2", None, Some("nas"), DiscoveryProtocol::Mdns),
            raw("10.0.0.3", None, None, DiscoveryProtocol::Ssdp),
            raw("10.0.0.3", None, None, DiscoveryProtocol::Ssdp),
        ]);
        assert_eq!(devices.len(), 2);
        for device in &devices {
            assert_eq!(device.protocols.len(), 1);
        }
    }

    #[test]
    fn merge_handles_empty_input() {
        assert!(merge_devices(Vec::new()).is_empty());
    }

    #[test]
    fn merged_devices_are_normalized() {
        let devices: Vec<DiscoveredDevice> = merge_devices(vec![raw(
            " 192.168.1.80 ",
            Some("AA-BB-CC-DD-EE-FF"),
            Some("  "),
            DiscoveryProtocol::Arp,
        )]);
        assert_eq!(devices.len(), 1);
        let device = &devices[0];
        assert_eq!(device.ip, "192.168.1.80");
        assert_eq!(device.mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
        assert_eq!(device.hostname, None);
        assert_eq!(device.id, "mac:aa:bb:cc:dd:ee:ff");
        assert_eq!(device.protocols, vec![DiscoveryProtocol::Arp]);
    }
}
