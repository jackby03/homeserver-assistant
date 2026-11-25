//! SSDP / UPnP discovery adapter.
//!
//! Sends one `M-SEARCH` datagram for `ssdp:all` to the SSDP multicast group and
//! collects the unicast `HTTP/1.1 200 OK` responses until the timeout expires.
//! The device address is always taken from the UDP response source (never from
//! the untrusted payload), which is what keeps the observation trustworthy.
//!
//! Socket errors are logged and tolerated: a scan must never fail because one
//! device answered late or the network stack refused a multicast send.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use async_trait::async_trait;
use hs_core::models::DiscoveryProtocol;
use tokio::net::UdpSocket;

use crate::{DiscoveryResult, DiscoverySource, RawDevice};

/// SSDP multicast address and port.
const SSDP_MULTICAST_ADDR: &str = "239.255.255.250:1900";
/// Maximum search wait advertised in the `MX` header, in seconds.
const SSDP_MAX_MX_SECONDS: u64 = 3;
/// Response buffer size; SSDP replies are small.
const SSDP_BUFFER_SIZE: usize = 4096;

/// SSDP discovery source.
#[derive(Debug, Default)]
pub struct SsdpSource;

impl SsdpSource {
    /// Create an SSDP discovery source.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DiscoverySource for SsdpSource {
    fn protocol(&self) -> DiscoveryProtocol {
        DiscoveryProtocol::Ssdp
    }

    async fn discover(&self, timeout: Duration) -> DiscoveryResult<Vec<RawDevice>> {
        let bounded = timeout
            .max(Duration::from_millis(10))
            .min(Duration::from_secs(SSDP_MAX_MX_SECONDS));
        // Watchdog on top of the internal deadline: a misbehaving network stack
        // must never hang the whole scan.
        let devices =
            match tokio::time::timeout(bounded + Duration::from_millis(500), search(bounded)).await
            {
                Ok(devices) => devices,
                Err(_) => {
                    tracing::warn!("SSDP discovery watchdog expired");
                    Vec::new()
                }
            };
        Ok(devices)
    }
}

/// Send the M-SEARCH request and collect responses until the deadline.
async fn search(timeout: Duration) -> Vec<RawDevice> {
    let socket = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(socket) => socket,
        Err(error) => {
            tracing::warn!(%error, "SSDP socket could not be bound");
            return Vec::new();
        }
    };

    let mx_seconds = timeout.as_secs().clamp(1, SSDP_MAX_MX_SECONDS);
    let request = format!(
        "M-SEARCH * HTTP/1.1\r\n\
         HOST: {SSDP_MULTICAST_ADDR}\r\n\
         MAN: \"ssdp:all\"\r\n\
         MX: {mx_seconds}\r\n\
         ST: ssdp:all\r\n\
         USER-AGENT: HomeServerAssistant/0.1 UPnP/1.1\r\n\
         \r\n"
    );
    if let Err(error) = socket
        .send_to(request.as_bytes(), SSDP_MULTICAST_ADDR)
        .await
    {
        tracing::warn!(%error, "SSDP M-SEARCH could not be sent");
        return Vec::new();
    }

    let deadline = tokio::time::Instant::now() + timeout;
    let mut buffer = vec![0u8; SSDP_BUFFER_SIZE];
    let mut devices: BTreeMap<String, RawDevice> = BTreeMap::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, socket.recv_from(&mut buffer)).await {
            Ok(Ok((length, source))) => {
                let text = String::from_utf8_lossy(&buffer[..length]);
                if let Some(device) = parse_ssdp_response(&text, &source.ip().to_string()) {
                    devices.entry(device.ip.clone()).or_insert(device);
                }
            }
            Ok(Err(error)) => {
                tracing::debug!(%error, "SSDP receive failed");
                break;
            }
            Err(_) => break,
        }
    }
    devices.into_values().collect()
}

/// Parse one SSDP response into a raw device observation.
///
/// Returns `None` when the payload is not a well-formed `HTTP/1.1 200` response
/// or when the source address is not IPv4. The device address comes from
/// `source_ip` (the UDP packet source) and is never read from the headers.
#[must_use]
pub fn parse_ssdp_response(text: &str, source_ip: &str) -> Option<RawDevice> {
    let source_ip: Ipv4Addr = source_ip.trim().parse().ok()?;
    let mut lines = text.lines();
    let status = lines.next()?.trim();
    if !status.to_ascii_uppercase().starts_with("HTTP/1.1 200") {
        return None;
    }

    let mut location: Option<&str> = None;
    let mut server: Option<&str> = None;
    let mut usn: Option<&str> = None;
    let mut host: Option<&str> = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match name.trim().to_ascii_uppercase().as_str() {
            "LOCATION" => location = Some(value),
            "SERVER" => server = Some(value),
            "USN" => usn = Some(value),
            "HOST" => host = Some(value),
            _ => {}
        }
    }
    // USN and HOST are parsed for completeness/observability only; they cannot
    // be trusted as identity (USN is device controlled, HOST is the multicast
    // address we sent the search to).
    tracing::trace!(?usn, ?host, %source_ip, "SSDP response headers");

    Some(RawDevice {
        ip: source_ip.to_string(),
        mac: None,
        hostname: location.and_then(hostname_from_location),
        vendor: server.and_then(vendor_from_server),
        protocol: DiscoveryProtocol::Ssdp,
    })
}

/// Extract a hostname from the `LOCATION` header when it is not a bare IP.
fn hostname_from_location(location: &str) -> Option<String> {
    let without_scheme = location
        .trim()
        .split_once("://")
        .map_or(location.trim(), |(_scheme, rest)| rest);
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if authority.starts_with('[') {
        // IPv6 literal; the appliance discovery output is IPv4 only.
        return None;
    }
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => authority,
    };
    if host.is_empty() || host.parse::<IpAddr>().is_ok() {
        return None;
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return None;
    }
    Some(host)
}

/// Derive a vendor label from the `SERVER` header of popular UPnP devices.
fn vendor_from_server(server: &str) -> Option<String> {
    let lowercase = server.to_ascii_lowercase();
    const BRANDS: [(&str, &str); 18] = [
        ("sonos", "Sonos"),
        ("roku", "Roku"),
        ("google", "Google"),
        ("chromecast", "Google"),
        ("amazon", "Amazon"),
        ("samsung", "Samsung"),
        ("philips", "Philips Hue"),
        ("signify", "Philips Hue"),
        ("tuya", "Tuya"),
        ("shelly", "Shelly"),
        ("espressif", "Espressif"),
        ("broadlink", "Broadlink"),
        ("aqara", "Aqara"),
        ("lumi", "Aqara"),
        ("xiaomi", "Xiaomi"),
        ("tp-link", "TP-Link"),
        ("huawei", "Huawei"),
        ("ubiquiti", "Ubiquiti"),
    ];
    BRANDS
        .iter()
        .find(|(needle, _vendor)| lowercase.contains(needle))
        .map(|(_needle, vendor)| (*vendor).to_owned())
}

#[cfg(test)]
mod tests {
    use super::{hostname_from_location, parse_ssdp_response, vendor_from_server};

    const SONOS_RESPONSE: &str = "HTTP/1.1 200 OK\r\n\
        CACHE-CONTROL: max-age=1800\r\n\
        EXT:\r\n\
        LOCATION: http://192.168.1.40:1400/xml/device_description.xml\r\n\
        SERVER: Linux/3.14.0 UPnP/1.1 Sonos/70.0\r\n\
        ST: urn:schemas-upnp-org:device:ZonePlayer:1\r\n\
        USN: uuid:RINCON_000E58AABBCC01400::urn:schemas-upnp-org:device:ZonePlayer:1\r\n\
        \r\n";

    #[test]
    fn parses_ssdp_response_from_source_address() {
        let device =
            parse_ssdp_response(SONOS_RESPONSE, "192.168.1.40").expect("parsed SSDP response");
        assert_eq!(device.ip, "192.168.1.40");
        assert_eq!(device.mac, None);
        // LOCATION carries an IP, so no hostname is exposed.
        assert_eq!(device.hostname, None);
        assert_eq!(device.vendor.as_deref(), Some("Sonos"));
    }

    #[test]
    fn parses_hostname_from_location_header() {
        let response = "HTTP/1.1 200 OK\r\n\
            LOCATION: http://living-room-tv.local:8009/description.xml\r\n\
            SERVER: Roku/9.4 UPnP/1.0 Roku/9.4\r\n\
            \r\n";
        let device = parse_ssdp_response(response, "192.168.1.55").expect("parsed SSDP response");
        assert_eq!(device.ip, "192.168.1.55");
        assert_eq!(device.hostname.as_deref(), Some("living-room-tv.local"));
        assert_eq!(device.vendor.as_deref(), Some("Roku"));
    }

    #[test]
    fn ignores_non_200_and_malformed_responses() {
        assert!(parse_ssdp_response("NOTIFY * HTTP/1.1\r\n\r\n", "192.168.1.1").is_none());
        assert!(parse_ssdp_response("", "192.168.1.1").is_none());
        assert!(parse_ssdp_response(SONOS_RESPONSE, "not-an-ip").is_none());
    }

    #[test]
    fn header_helpers_handle_edge_cases() {
        assert_eq!(
            hostname_from_location("http://printer.local:8080/desc.xml"),
            Some("printer.local".to_owned())
        );
        assert_eq!(
            hostname_from_location("https://user@nas.local/"),
            Some("nas.local".to_owned())
        );
        assert_eq!(hostname_from_location("http://192.168.1.1/desc.xml"), None);
        assert_eq!(hostname_from_location("http://[fe80::1]/desc.xml"), None);
        assert_eq!(hostname_from_location(""), None);

        assert_eq!(
            vendor_from_server("Linux/4.1 UPnP/1.0 Google/2.0"),
            Some("Google".to_owned())
        );
        assert_eq!(vendor_from_server("Some/1.0"), None);
    }
}
