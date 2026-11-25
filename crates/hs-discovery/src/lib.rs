//! Local IoT discovery over mDNS, SSDP/UPnP and ARP.
//!
//! The crate exposes a small port ([`DiscoverySource`]) implemented by three
//! adapters ([`mdns::MdnsSource`], [`ssdp::SsdpSource`], [`arp::ArpSource`]) and
//! a merging service ([`DiscoveryService`]) that produces the normalized
//! [`hs_core::models::DiscoveredDevice`] list shown by the UI.
//!
//! # Design
//! - Every adapter returns raw observations ([`RawDevice`]) and tolerates
//!   network errors: a broken protocol never fails a whole scan.
//! - Merging is deterministic: records are deduplicated by MAC address first
//!   and then by IPv4 address, protocols are unioned and sorted, and the vendor
//!   is resolved through the offline OUI table in [`vendor`].
//! - Unit tests never touch the network: the protocols expose pure parsers and
//!   the service is exercised with fake sources.

#![deny(missing_docs)]

pub mod arp;
pub mod error;
pub mod mdns;
pub mod service;
pub mod ssdp;
pub mod vendor;

use std::time::Duration;

use async_trait::async_trait;
use hs_core::models::DiscoveryProtocol;

pub use error::{DiscoveryError, DiscoveryResult};
pub use service::DiscoveryService;

/// A single device observation reported by one discovery source.
///
/// The record is intentionally loose (all identifying fields optional except the
/// address) because mDNS, SSDP and ARP know different things about a device.
/// [`DiscoveryService`] merges the observations of all sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDevice {
    /// IPv4 address as reported by the source. Merging normalizes it.
    pub ip: String,
    /// MAC address when the source knows it (ARP does, mDNS/SSDP usually not).
    pub mac: Option<String>,
    /// Hostname advertised by the device.
    pub hostname: Option<String>,
    /// Vendor label derived from the discovery payload (OUI lookup happens at
    /// merge time when this is `None`).
    pub vendor: Option<String>,
    /// Protocol that produced this observation.
    pub protocol: DiscoveryProtocol,
}

/// Port implemented by every local discovery protocol adapter.
#[async_trait]
pub trait DiscoverySource: Send + Sync {
    /// Protocol implemented by this source.
    fn protocol(&self) -> DiscoveryProtocol;

    /// Run one discovery round bounded by `timeout`.
    ///
    /// Implementations should tolerate recoverable errors internally and only
    /// return an error when the whole protocol could not run; callers treat an
    /// error as "no devices from this source" and continue with the others.
    async fn discover(&self, timeout: Duration) -> DiscoveryResult<Vec<RawDevice>>;
}
