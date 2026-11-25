//! netplan YAML rendering and application.
//!
//! The appliance owns two Linux bridges:
//!
//! - `br-lan` (trusted): enslaves the physical uplink and carries the appliance
//!   LAN address (DHCP or static). LAN workloads (Home Assistant, DNS filter,
//!   media apps) attach to it through Incus profiles.
//! - `br-dmz` (isolated): `10.20.30.1/24`, no default route. DMZ workloads live
//!   here; nftables masquerades their outbound internet access without ever
//!   allowing them to reach the LAN or the host admin port.
//!
//! The physical interface (e.g. `eth0`) is only a bridge member: it must not
//! carry an address of its own, otherwise ARP/DHCP break because two interfaces
//! own the same address.
//!
//! [`render_netplan`] is pure and unit tested; [`apply_netplan`] writes the file
//! atomically and runs `netplan apply` on Linux only.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::Path;

use hs_core::config::{NetworkConfig, NetworkMode};
use serde::Serialize;

use crate::error::{NetworkError, NetworkResult};
use crate::fsutil::atomic_write;

/// Default search domain written into `nameservers.search`.
pub const NETPLAN_SEARCH_DOMAIN: &str = "home.local";

/// Address of the local filtering resolver used before it is provisioned.
///
/// Once the AdGuard Home container has an address, the daemon re-renders the
/// document with the container IP (see
/// [`render_netplan_with_options`]); `127.0.0.1` is the safe placeholder for a
/// native/host install.
pub const DEFAULT_DNS_SERVER: &str = "127.0.0.1";

/// Render a complete netplan YAML document with the default search domain and
/// DNS placeholder.
///
/// # Errors
/// Returns [`NetworkError::Invalid`] when the interface, bridges, static
/// address or DMZ subnet are invalid.
pub fn render_netplan(config: &NetworkConfig) -> NetworkResult<String> {
    render_netplan_with_options(config, NETPLAN_SEARCH_DOMAIN, DEFAULT_DNS_SERVER)
}

/// Render the netplan document with a custom DNS search domain.
///
/// # Errors
/// Returns [`NetworkError::Invalid`] when a field is invalid.
pub fn render_netplan_with_search_domain(
    config: &NetworkConfig,
    search_domain: &str,
) -> NetworkResult<String> {
    render_netplan_with_options(config, search_domain, DEFAULT_DNS_SERVER)
}

/// Render the netplan document with a custom search domain and resolver.
///
/// # Errors
/// Returns [`NetworkError::Invalid`] when a field is invalid.
pub fn render_netplan_with_options(
    config: &NetworkConfig,
    search_domain: &str,
    dns_server: &str,
) -> NetworkResult<String> {
    let uplink = validate_interface_name(&config.interface, "interface")?;
    let lan_bridge = validate_interface_name(&config.lan_bridge, "lan_bridge")?;
    let dmz_bridge = validate_interface_name(&config.dmz.bridge, "dmz.bridge")?;
    if lan_bridge == dmz_bridge {
        return Err(NetworkError::Invalid(
            "the LAN and DMZ bridges must be different interfaces".to_owned(),
        ));
    }
    if lan_bridge == uplink || dmz_bridge == uplink {
        return Err(NetworkError::Invalid(
            "bridges must be different from the physical uplink".to_owned(),
        ));
    }

    let search_domain = search_domain.trim();
    hs_core::validation::domain(search_domain)
        .map_err(|_| NetworkError::Invalid(format!("invalid search domain `{search_domain}`")))?;
    let dns_server = dns_server.trim();
    validate_ipv4(dns_server, "DNS server")?;

    let (lan_dhcp, lan_addresses, lan_routes) = lan_addressing(config)?;

    let dmz_gateway: Ipv4Addr = validate_ipv4(&config.dmz.gateway, "DMZ gateway")?;
    let dmz_network = config
        .dmz
        .subnet
        .parse::<ipnet::Ipv4Net>()
        .map_err(|err| NetworkError::Invalid(format!("invalid DMZ subnet: {err}")))?;
    if !dmz_network.contains(&dmz_gateway) {
        return Err(NetworkError::Invalid(
            "the DMZ gateway must belong to the DMZ subnet".to_owned(),
        ));
    }
    let dmz_prefix = dmz_network.prefix_len();

    let mut ethernets = BTreeMap::new();
    ethernets.insert(
        uplink.clone(),
        Ethernet {
            dhcp4: false,
            dhcp6: false,
        },
    );

    let mut bridges = BTreeMap::new();
    bridges.insert(
        lan_bridge,
        Bridge {
            interfaces: vec![uplink],
            dhcp4: lan_dhcp,
            addresses: lan_addresses,
            routes: lan_routes,
            nameservers: Some(Nameservers {
                addresses: vec![dns_server.to_owned()],
                search: vec![search_domain.to_owned()],
            }),
            parameters: BridgeParameters::default(),
        },
    );
    bridges.insert(
        dmz_bridge,
        Bridge {
            interfaces: Vec::new(),
            dhcp4: false,
            addresses: vec![format!("{dmz_gateway}/{dmz_prefix}")],
            routes: Vec::new(),
            nameservers: None,
            parameters: BridgeParameters::default(),
        },
    );

    let document = NetplanDocument {
        network: NetworkSection {
            version: 2,
            renderer: "networkd",
            ethernets,
            bridges,
        },
    };
    serde_yaml::to_string(&document)
        .map_err(|err| NetworkError::Apply(format!("failed to render netplan YAML: {err}")))
}

/// Write the netplan document to `target` and run `netplan apply`.
///
/// Uses the default DNS placeholder; prefer [`apply_netplan_with_dns`] once the
/// filtering resolver has an address.
///
/// On platforms other than Linux this returns [`NetworkError::Unsupported`]
/// without touching the target file.
///
/// # Errors
/// Returns [`NetworkError::Invalid`] for an invalid configuration,
/// [`NetworkError::Io`] when the file cannot be written and
/// [`NetworkError::Apply`] when `netplan apply` fails.
pub async fn apply_netplan(config: &NetworkConfig, target: &Path) -> NetworkResult<()> {
    apply_netplan_with_dns(config, target, DEFAULT_DNS_SERVER).await
}

/// Write the netplan document with a custom resolver and run `netplan apply`.
///
/// # Errors
/// Same as [`apply_netplan`].
pub async fn apply_netplan_with_dns(
    config: &NetworkConfig,
    target: &Path,
    dns_server: &str,
) -> NetworkResult<()> {
    let yaml = render_netplan_with_options(config, NETPLAN_SEARCH_DOMAIN, dns_server)?;

    if !cfg!(target_os = "linux") {
        return Err(NetworkError::Unsupported(
            "netplan can only be applied on Linux".to_owned(),
        ));
    }

    atomic_write(target, &yaml)?;
    let output = tokio::process::Command::new("netplan")
        .arg("apply")
        .output()
        .await
        .map_err(|err| NetworkError::Apply(format!("failed to run `netplan apply`: {err}")))?;
    if !output.status.success() {
        return Err(NetworkError::Apply(format!(
            "`netplan apply` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn lan_addressing(config: &NetworkConfig) -> NetworkResult<(bool, Vec<String>, Vec<Route>)> {
    match config.mode {
        NetworkMode::Dhcp => Ok((true, Vec::new(), Vec::new())),
        NetworkMode::Static => {
            let static_ipv4 = config.static_ipv4.as_ref().ok_or_else(|| {
                NetworkError::Invalid(
                    "static network mode requires `static_ipv4` to be configured".to_owned(),
                )
            })?;
            validate_ipv4(&static_ipv4.address, "static IPv4 address")?;
            if static_ipv4.prefix_len == 0 || static_ipv4.prefix_len > 32 {
                return Err(NetworkError::Invalid(
                    "static IPv4 prefix length must be between 1 and 32".to_owned(),
                ));
            }
            validate_ipv4(&static_ipv4.gateway, "static IPv4 gateway")?;
            Ok((
                false,
                vec![format!(
                    "{}/{}",
                    static_ipv4.address.trim(),
                    static_ipv4.prefix_len
                )],
                vec![Route {
                    to: "default",
                    via: static_ipv4.gateway.trim().to_owned(),
                }],
            ))
        }
    }
}

fn validate_ipv4(value: &str, field: &str) -> NetworkResult<Ipv4Addr> {
    let value = value.trim();
    value
        .parse::<Ipv4Addr>()
        .map_err(|_| NetworkError::Invalid(format!("invalid {field} `{value}`")))
}

/// Validate an interface or bridge name (Linux IFNAMSIZ limits it to 15 bytes).
fn validate_interface_name(name: &str, field: &str) -> NetworkResult<String> {
    let name = name.trim();
    let valid = !name.is_empty()
        && name.len() <= 15
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'));
    if valid {
        Ok(name.to_owned())
    } else {
        Err(NetworkError::Invalid(format!(
            "`{name}` is not a valid {field} name"
        )))
    }
}

/// Serialized netplan document root.
#[derive(Debug, Serialize)]
struct NetplanDocument<'a> {
    network: NetworkSection<'a>,
}

/// Serialized `network:` section.
#[derive(Debug, Serialize)]
struct NetworkSection<'a> {
    version: u32,
    renderer: &'a str,
    ethernets: BTreeMap<String, Ethernet>,
    bridges: BTreeMap<String, Bridge>,
}

/// Physical interface: only a bridge member, never addressed.
#[derive(Debug, Serialize)]
struct Ethernet {
    dhcp4: bool,
    dhcp6: bool,
}

/// Serialized bridge section.
#[derive(Debug, Serialize)]
struct Bridge {
    interfaces: Vec<String>,
    dhcp4: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    addresses: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    routes: Vec<Route>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nameservers: Option<Nameservers>,
    parameters: BridgeParameters,
}

/// Static default route.
#[derive(Debug, Serialize)]
struct Route {
    to: &'static str,
    via: String,
}

/// DNS resolvers advertised to LAN clients.
#[derive(Debug, Serialize)]
struct Nameservers {
    addresses: Vec<String>,
    search: Vec<String>,
}

/// Bridge parameters: no STP, no forwarding delay for an appliance.
#[derive(Debug, Default, Serialize)]
struct BridgeParameters {
    stp: bool,
    #[serde(rename = "forward-delay")]
    forward_delay: u32,
}

#[cfg(test)]
mod tests {
    use hs_core::config::{NetworkConfig, NetworkMode, StaticIpv4};
    use serde_yaml::Value;

    use super::{
        apply_netplan, render_netplan, render_netplan_with_options,
        render_netplan_with_search_domain, NETPLAN_SEARCH_DOMAIN,
    };
    use crate::error::NetworkError;

    fn lookup<'a>(document: &'a Value, path: &[&str]) -> &'a Value {
        let mut current = document;
        for segment in path {
            let next = match segment.parse::<usize>() {
                Ok(index) => current.get(index),
                Err(_) => current.get(*segment),
            };
            current = next.unwrap_or_else(|| {
                panic!("missing YAML segment `{segment}` in {document:?}");
            });
        }
        current
    }

    fn static_config() -> NetworkConfig {
        NetworkConfig {
            mode: NetworkMode::Static,
            static_ipv4: Some(StaticIpv4 {
                address: "192.168.1.10".to_owned(),
                prefix_len: 24,
                gateway: "192.168.1.1".to_owned(),
            }),
            ..NetworkConfig::default()
        }
    }

    #[test]
    fn renders_bridge_topology_and_valid_yaml() {
        let yaml = render_netplan(&NetworkConfig::default()).expect("render netplan");
        let parsed: Value = serde_yaml::from_str(&yaml).expect("valid YAML");

        assert_eq!(lookup(&parsed, &["network", "version"]).as_u64(), Some(2));
        assert_eq!(
            lookup(&parsed, &["network", "renderer"]).as_str(),
            Some("networkd")
        );
        // The uplink is only a bridge member.
        assert_eq!(
            lookup(&parsed, &["network", "ethernets", "eth0", "dhcp4"]).as_bool(),
            Some(false)
        );
        assert!(lookup(&parsed, &["network", "ethernets", "eth0"])
            .get("addresses")
            .is_none());
        assert_eq!(
            lookup(&parsed, &["network", "bridges", "br-lan", "dhcp4"]).as_bool(),
            Some(true)
        );
        assert_eq!(
            lookup(
                &parsed,
                &["network", "bridges", "br-lan", "interfaces", "0"]
            )
            .as_str(),
            Some("eth0")
        );
        assert_eq!(
            lookup(
                &parsed,
                &[
                    "network",
                    "bridges",
                    "br-lan",
                    "nameservers",
                    "addresses",
                    "0"
                ]
            )
            .as_str(),
            Some("127.0.0.1")
        );
        assert_eq!(
            lookup(
                &parsed,
                &["network", "bridges", "br-lan", "nameservers", "search", "0"]
            )
            .as_str(),
            Some(NETPLAN_SEARCH_DOMAIN)
        );
        assert_eq!(
            lookup(&parsed, &["network", "bridges", "br-dmz", "addresses", "0"]).as_str(),
            Some("10.20.30.1/24")
        );
        assert!(
            lookup(&parsed, &["network", "bridges", "br-dmz"])
                .get("routes")
                .is_none(),
            "the DMZ bridge must not have a default route"
        );
        assert!(
            lookup(&parsed, &["network", "bridges", "br-dmz"])
                .get("nameservers")
                .is_none(),
            "the DMZ bridge must not advertise a resolver"
        );
    }

    #[test]
    fn renders_static_lan_addressing() {
        let yaml = render_netplan(&static_config()).expect("render static netplan");
        let parsed: Value = serde_yaml::from_str(&yaml).expect("valid YAML");
        let lan = lookup(&parsed, &["network", "bridges", "br-lan"]);

        assert_eq!(lan.get("dhcp4").and_then(Value::as_bool), Some(false));
        assert_eq!(
            lan.get("addresses").and_then(Value::as_sequence),
            Some(&vec![Value::String("192.168.1.10/24".to_owned())])
        );
        assert_eq!(
            lookup(
                &parsed,
                &["network", "bridges", "br-lan", "routes", "0", "to"]
            )
            .as_str(),
            Some("default")
        );
        assert_eq!(
            lookup(
                &parsed,
                &["network", "bridges", "br-lan", "routes", "0", "via"]
            )
            .as_str(),
            Some("192.168.1.1")
        );
    }

    #[test]
    fn renders_custom_search_domain_and_dns_server() {
        let yaml = render_netplan_with_search_domain(&NetworkConfig::default(), "lan.example")
            .expect("render netplan");
        let parsed: Value = serde_yaml::from_str(&yaml).expect("valid YAML");
        assert_eq!(
            lookup(
                &parsed,
                &["network", "bridges", "br-lan", "nameservers", "search", "0"]
            )
            .as_str(),
            Some("lan.example")
        );

        let yaml =
            render_netplan_with_options(&NetworkConfig::default(), "home.local", "10.20.30.10")
                .expect("render netplan");
        let parsed: Value = serde_yaml::from_str(&yaml).expect("valid YAML");
        assert_eq!(
            lookup(
                &parsed,
                &[
                    "network",
                    "bridges",
                    "br-lan",
                    "nameservers",
                    "addresses",
                    "0"
                ]
            )
            .as_str(),
            Some("10.20.30.10")
        );
    }

    #[test]
    fn rejects_invalid_configurations() {
        let config = NetworkConfig {
            interface: "eth 0; rm -rf /".to_owned(),
            ..NetworkConfig::default()
        };
        assert!(matches!(
            render_netplan(&config),
            Err(NetworkError::Invalid(_))
        ));

        let mut config = NetworkConfig::default();
        config.dmz.bridge = "br-lan".to_owned();
        assert!(matches!(
            render_netplan(&config),
            Err(NetworkError::Invalid(_))
        ));

        let config = NetworkConfig {
            lan_bridge: "eth0".to_owned(),
            ..NetworkConfig::default()
        };
        assert!(matches!(
            render_netplan(&config),
            Err(NetworkError::Invalid(_))
        ));

        let config = NetworkConfig {
            mode: NetworkMode::Static,
            ..NetworkConfig::default()
        };
        assert!(matches!(
            render_netplan(&config),
            Err(NetworkError::Invalid(_))
        ));

        let mut config = static_config();
        if let Some(static_ipv4) = config.static_ipv4.as_mut() {
            static_ipv4.gateway = "999.1.1.1".to_owned();
        }
        assert!(matches!(
            render_netplan(&config),
            Err(NetworkError::Invalid(_))
        ));

        let mut config = NetworkConfig::default();
        config.dmz.gateway = "10.99.0.1".to_owned();
        assert!(matches!(
            render_netplan(&config),
            Err(NetworkError::Invalid(_))
        ));

        assert!(matches!(
            render_netplan_with_search_domain(&NetworkConfig::default(), "bad domain!"),
            Err(NetworkError::Invalid(_))
        ));
        assert!(matches!(
            render_netplan_with_options(&NetworkConfig::default(), "home.local", "not-an-ip"),
            Err(NetworkError::Invalid(_))
        ));
    }

    #[cfg(not(target_os = "linux"))]
    #[tokio::test]
    async fn apply_is_unsupported_outside_linux() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("90-homeserver.yaml");
        let error = apply_netplan(&NetworkConfig::default(), &target)
            .await
            .expect_err("netplan apply must be unsupported here");
        assert!(matches!(error, NetworkError::Unsupported(_)));
        assert!(!target.exists(), "unsupported apply must not write a file");
    }
}
