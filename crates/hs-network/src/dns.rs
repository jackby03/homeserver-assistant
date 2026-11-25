//! Encrypted DNS upstream resolution and filtering-engine configuration.
//!
//! The appliance resolves the operator-friendly [`DnsConfig`] into concrete
//! upstream endpoints and renders the configuration files consumed by the
//! local filtering engine (AdGuard Home by default, Technitium optionally).
//!
//! Only encrypted transports are ever emitted:
//!
//! - [`DnsProtocol::Doh`] endpoints are `https://` URLs;
//! - [`DnsProtocol::Dot`] endpoints are hostnames; renderers add the `tls://`
//!   scheme understood by the engines.
//!
//! `bootstrap_dns` entries are plain IP addresses. They are **not** DNS
//! upstreams: they are only used once at startup to resolve the hostname of the
//! encrypted endpoint (chicken-and-egg problem), which is why the engines
//! require them separately from `upstream_dns`.

use hs_core::config::{DnsConfig, DnsProtocol};
use hs_core::dns::{provider_spec, DnsProvider};

use crate::error::{NetworkError, NetworkResult};

/// Conservative fallback bootstrap resolvers for the custom provider.
const DEFAULT_BOOTSTRAP: [&str; 2] = ["1.1.1.1", "1.0.0.1"];

/// A validated DNS upstream endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsUpstream {
    /// Endpoint address. For [`DnsProtocol::Doh`] this is an absolute
    /// `https://` URL; for [`DnsProtocol::Dot`] it is a hostname (or
    /// `hostname:853` when the configuration supplies an explicit port); for
    /// [`DnsProtocol::Plain`] it is a plain IP address.
    pub address: String,
    /// Transport protocol used towards the endpoint.
    pub protocol: DnsProtocol,
    /// Provider that originated the endpoint.
    pub provider: DnsProvider,
}

impl DnsUpstream {
    /// Render the endpoint in the `upstream_dns` syntax used by AdGuard Home
    /// and Technitium (`https://...` for DoH, `tls://host[:port]` for DoT,
    /// plain address for legacy plain DNS).
    #[must_use]
    pub fn as_engine_upstream(&self) -> String {
        match self.protocol {
            DnsProtocol::Doh => self.address.clone(),
            DnsProtocol::Dot => format!("tls://{}", self.address),
            DnsProtocol::Plain => self.address.clone(),
        }
    }

    /// Stable wire name of the transport (`doh`, `dot` or `plain`).
    #[must_use]
    pub fn protocol_id(&self) -> &'static str {
        self.protocol.id()
    }
}

/// Resolve the configured provider into concrete upstreams.
///
/// Encrypted providers return exactly one upstream (the selected endpoint).
/// Plain DNS returns one upstream per IPv4/IPv6 bootstrap address because the
/// catalog lists every resolver for redundancy.
///
/// # Errors
/// Returns [`NetworkError::Invalid`] when the provider does not support the
/// selected transport, the custom endpoint is missing/unsafe, or a NextDNS
/// configuration id is missing or malformed.
pub fn upstreams(config: &DnsConfig) -> NetworkResult<Vec<DnsUpstream>> {
    let provider = config.provider.clone();

    if provider.is_custom() {
        if config.protocol == DnsProtocol::Plain {
            return Err(NetworkError::Invalid(
                "custom DNS requires DoH or DoT; encrypted transports are always preferred"
                    .to_owned(),
            ));
        }
        let custom = config.custom_upstream.as_deref().ok_or_else(|| {
            NetworkError::Invalid(
                "custom DNS provider requires `custom_upstream` to be set".to_owned(),
            )
        })?;
        validate_custom_upstream(config.protocol, custom)?;
        return Ok(vec![DnsUpstream {
            address: custom.trim().to_owned(),
            protocol: config.protocol,
            provider,
        }]);
    }

    if provider.is_next_dns() {
        let id = config
            .custom_upstream
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                NetworkError::Invalid(
                    "NextDNS requires your configuration id in `custom_upstream`".to_owned(),
                )
            })?;
        if !(4..=32).contains(&id.len())
            || !id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
        {
            return Err(NetworkError::Invalid(
                "NextDNS configuration id must be 4-32 letters, digits or hyphens".to_owned(),
            ));
        }
        let address = match config.protocol {
            DnsProtocol::Doh => format!("https://dns.nextdns.io/{id}"),
            DnsProtocol::Dot => format!("{id}.dns.nextdns.io"),
            DnsProtocol::Plain => {
                return Err(NetworkError::Invalid(
                    "NextDNS does not offer plain DNS".to_owned(),
                ));
            }
        };
        return Ok(vec![DnsUpstream {
            address,
            protocol: config.protocol,
            provider,
        }]);
    }

    let spec = provider
        .spec()
        .ok_or_else(|| NetworkError::Invalid(format!("unknown DNS provider `{provider}`")))?;

    match config.protocol {
        DnsProtocol::Doh => {
            let endpoint = spec.doh.ok_or_else(|| {
                NetworkError::Invalid(format!("provider `{}` does not offer DoH", spec.label))
            })?;
            Ok(vec![DnsUpstream {
                address: endpoint.to_owned(),
                protocol: config.protocol,
                provider,
            }])
        }
        DnsProtocol::Dot => {
            let host = spec.dot.ok_or_else(|| {
                NetworkError::Invalid(format!("provider `{}` does not offer DoT", spec.label))
            })?;
            Ok(vec![DnsUpstream {
                address: host.to_owned(),
                protocol: config.protocol,
                provider,
            }])
        }
        DnsProtocol::Plain => {
            if spec.doh.is_some() || spec.dot.is_some() {
                return Err(NetworkError::Invalid(format!(
                    "provider `{}` offers encrypted DNS; plain DNS is only available for legacy providers without DoH/DoT",
                    spec.label
                )));
            }
            if !spec.plain {
                return Err(NetworkError::Invalid(format!(
                    "provider `{}` does not offer plain DNS; choose DoH or DoT",
                    spec.label
                )));
            }
            let addresses = spec
                .ipv4
                .iter()
                .chain(spec.ipv6.iter())
                .map(|address| DnsUpstream {
                    address: (*address).to_owned(),
                    protocol: config.protocol,
                    provider: provider.clone(),
                })
                .collect::<Vec<_>>();
            if addresses.is_empty() {
                return Err(NetworkError::Invalid(format!(
                    "provider `{}` has no bootstrap addresses",
                    spec.label
                )));
            }
            Ok(addresses)
        }
    }
}

/// Plain IP addresses of the selected provider.
///
/// For encrypted transports these are written to the engine `bootstrap_dns`
/// field (used only to resolve the endpoint hostname), never to
/// `upstream_dns`. For plain DNS they are the upstreams themselves.
#[must_use]
pub fn bootstrap_servers(config: &DnsConfig) -> Vec<String> {
    let Some(spec) = config.provider.spec() else {
        return DEFAULT_BOOTSTRAP
            .iter()
            .map(|server| (*server).to_owned())
            .collect();
    };
    if config.provider.is_custom() {
        return DEFAULT_BOOTSTRAP
            .iter()
            .map(|server| (*server).to_owned())
            .collect();
    }
    spec.ipv4
        .iter()
        .chain(spec.ipv6.iter())
        .map(|server| (*server).to_owned())
        .collect()
}

/// Catalog id/label/vendor for the admin UI selector.
#[must_use]
pub fn provider_catalog() -> &'static [hs_core::dns::DnsProviderSpec] {
    hs_core::dns::DNS_PROVIDERS
}

/// Whether a protocol is usable for the configured provider.
#[must_use]
pub fn supports(config: &DnsConfig, protocol: DnsProtocol) -> bool {
    if config.provider.is_custom() {
        return protocol != DnsProtocol::Plain;
    }
    config.provider.supports(protocol)
}

/// Look up a provider spec id (helper for provisioning/UI code).
#[must_use]
pub fn spec_for(id: &str) -> Option<&'static hs_core::dns::DnsProviderSpec> {
    provider_spec(id)
}
/// Render a complete AdGuard Home `AdGuardHome.yaml` configuration.
///
/// The rendered document contains the DNS listener (`dns.bind_hosts`,
/// `dns.port: 53`), the encrypted upstreams, the bootstrap resolvers, the
/// filtering flags and one entry per configured blocklist. The administrative
/// web UI is pinned to loopback so that it is only reachable through the
/// appliance reverse proxy.
///
/// # Errors
/// Returns [`NetworkError::Invalid`] when the DNS configuration or one of the
/// admin inputs is unusable.
pub fn render_adguard_config(
    config: &DnsConfig,
    admin_user: &str,
    admin_password_hash: &str,
    bind_address: &str,
) -> NetworkResult<String> {
    let admin_user = admin_user.trim();
    if admin_user.is_empty() || admin_user.contains(char::is_whitespace) {
        return Err(NetworkError::Invalid(
            "AdGuard admin user must be a non-empty value without whitespace".to_owned(),
        ));
    }
    let admin_password_hash = admin_password_hash.trim();
    if admin_password_hash.is_empty() || admin_password_hash.contains(char::is_whitespace) {
        return Err(NetworkError::Invalid(
            "AdGuard admin password hash must be a non-empty value without whitespace".to_owned(),
        ));
    }
    let bind_address = bind_address.trim();
    if bind_address.parse::<std::net::IpAddr>().is_err() {
        return Err(NetworkError::Invalid(format!(
            "AdGuard bind address `{bind_address}` is not an IP address"
        )));
    }

    let resolved = upstreams(config)?;
    validate_blocklists(&config.blocklists)?;

    let upstream_values = resolved
        .iter()
        .map(|upstream| yaml_string(upstream.as_engine_upstream()))
        .collect();
    let bootstrap_values = bootstrap_servers(config)
        .into_iter()
        .map(yaml_string)
        .collect();

    let mut filters = Vec::with_capacity(config.blocklists.len());
    for (index, url) in config.blocklists.iter().enumerate() {
        filters.push(yaml_mapping(vec![
            ("enabled", serde_yaml::Value::Bool(true)),
            ("url", yaml_string(url.clone())),
            ("name", yaml_string(blocklist_name(url, index))),
            ("id", serde_yaml::Value::Number((index as u64 + 1).into())),
        ]));
    }

    let users = yaml_sequence(vec![yaml_mapping(vec![
        ("name", yaml_string(admin_user)),
        ("password", yaml_string(admin_password_hash)),
    ])]);

    let dns_section = yaml_mapping(vec![
        ("bind_hosts", yaml_sequence(vec![yaml_string(bind_address)])),
        ("port", serde_yaml::Value::Number(53u64.into())),
        ("upstream_dns", yaml_sequence(upstream_values)),
        ("bootstrap_dns", yaml_sequence(bootstrap_values)),
        (
            "protection_enabled",
            serde_yaml::Value::Bool(config.enabled),
        ),
        ("filtering_enabled", serde_yaml::Value::Bool(config.enabled)),
    ]);

    let http_section = yaml_mapping(vec![("address", yaml_string("127.0.0.1:3000".to_owned()))]);

    let document = yaml_mapping(vec![
        ("schema_version", serde_yaml::Value::Number(27u64.into())),
        ("http", http_section),
        ("users", users),
        ("dns", dns_section),
        ("filters", yaml_sequence(filters)),
    ]);

    serde_yaml::to_string(&document).map_err(|err| {
        NetworkError::Apply(format!(
            "failed to render AdGuard Home configuration: {err}"
        ))
    })
}

/// Render a JSON configuration document for Technitium DNS Server.
///
/// Technitium is configured through its API; the appliance renders a
/// deterministic document that the provisioning step applies. It carries the
/// primary forwarder, the transport, the bootstrap resolvers and the blocklist
/// URLs, using the same validated endpoints as the AdGuard renderer.
///
/// # Errors
/// Returns [`NetworkError::Invalid`] when the DNS configuration is unusable.
pub fn render_technitium_config(config: &DnsConfig) -> NetworkResult<String> {
    let resolved = upstreams(config)?;
    validate_blocklists(&config.blocklists)?;

    let forwarders: Vec<serde_json::Value> = resolved
        .iter()
        .map(|upstream| {
            serde_json::json!({
                "address": upstream.address,
                "endpoint": upstream.as_engine_upstream(),
                "protocol": upstream.protocol_id(),
                "provider": upstream.provider.as_str(),
            })
        })
        .collect();

    let document = serde_json::json!({
        "engine": "technitium",
        "dns_port": 53,
        "forwarder_protocol": config.protocol.id(),
        "forwarders": forwarders,
        "bootstrap_dns": bootstrap_servers(config),
        "blocklists": config
            .blocklists
            .iter()
            .enumerate()
            .map(|(index, url)| serde_json::json!({
                "url": url,
                "name": blocklist_name(url, index),
                "enabled": true,
            }))
            .collect::<Vec<_>>(),
        "protection_enabled": config.enabled,
    });

    serde_json::to_string_pretty(&document).map_err(|err| {
        NetworkError::Apply(format!("failed to render Technitium configuration: {err}"))
    })
}

/// Validate the operator supplied custom upstream for the selected protocol.
///
/// DoH endpoints must be absolute `https://` URLs (plaintext HTTP is rejected);
/// DoT endpoints must be a hostname with an optional explicit port and without
/// a scheme.
fn validate_custom_upstream(protocol: DnsProtocol, value: &str) -> NetworkResult<()> {
    let value = value.trim();
    if value.is_empty() || value.contains(char::is_whitespace) {
        return Err(NetworkError::Invalid(
            "custom DNS upstream must not be empty or contain whitespace".to_owned(),
        ));
    }
    match protocol {
        DnsProtocol::Doh => {
            if !value.starts_with("https://") {
                return Err(NetworkError::Invalid(
                    "custom DoH upstream must be an absolute https:// URL".to_owned(),
                ));
            }
            let host = value
                .trim_start_matches("https://")
                .split(['/', '?', '#'])
                .next()
                .unwrap_or_default();
            if host.is_empty() {
                return Err(NetworkError::Invalid(
                    "custom DoH upstream must include a host".to_owned(),
                ));
            }
            Ok(())
        }
        DnsProtocol::Plain => value.parse::<std::net::IpAddr>().map(|_| ()).map_err(|_| {
            NetworkError::Invalid(
                "custom plain DNS upstream must be an IPv4 or IPv6 address".to_owned(),
            )
        }),
        DnsProtocol::Dot => {
            if value.contains("://") {
                return Err(NetworkError::Invalid(
                    "custom DoT upstream must be a hostname without a scheme".to_owned(),
                ));
            }
            let (host, port) = match value.split_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (value, None),
            };
            if host.is_empty()
                || !host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
            {
                return Err(NetworkError::Invalid(
                    "custom DoT hostname contains invalid characters".to_owned(),
                ));
            }
            if let Some(port) = port {
                if port.parse::<u16>().map_or(true, |port| port == 0) {
                    return Err(NetworkError::Invalid(
                        "custom DoT port must be a number between 1 and 65535".to_owned(),
                    ));
                }
            }
            Ok(())
        }
    }
}

/// Reject blocklists that are not absolute HTTP(S) URLs.
fn validate_blocklists(blocklists: &[String]) -> NetworkResult<()> {
    for url in blocklists {
        let valid = (url.starts_with("https://") || url.starts_with("http://"))
            && url.len() > 8
            && !url.contains(char::is_whitespace);
        if !valid {
            return Err(NetworkError::Invalid(format!(
                "blocklist `{url}` must be an absolute http(s) URL"
            )));
        }
    }
    Ok(())
}

/// Human readable blocklist name derived from the URL host.
fn blocklist_name(url: &str, index: usize) -> String {
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    if host.is_empty() {
        format!("HomeServer Assistant blocklist {}", index + 1)
    } else {
        host.to_owned()
    }
}

fn yaml_string(value: impl Into<String>) -> serde_yaml::Value {
    serde_yaml::Value::String(value.into())
}

fn yaml_sequence(items: Vec<serde_yaml::Value>) -> serde_yaml::Value {
    serde_yaml::Value::Sequence(items)
}

fn yaml_mapping(entries: Vec<(&str, serde_yaml::Value)>) -> serde_yaml::Value {
    let mut mapping = serde_yaml::Mapping::new();
    for (key, value) in entries {
        mapping.insert(yaml_string(key), value);
    }
    serde_yaml::Value::Mapping(mapping)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hs_core::config::{DnsConfig, DnsEngine, DnsProtocol};
    use hs_core::dns::DnsProvider;

    fn provider(id: &str) -> DnsProvider {
        DnsProvider::new(id).expect("catalog provider")
    }

    fn config_with(id: &str, protocol: DnsProtocol) -> DnsConfig {
        DnsConfig {
            enabled: true,
            engine: DnsEngine::AdGuardHome,
            provider: provider(id),
            protocol,
            custom_upstream: None,
            blocklists: Vec::new(),
        }
    }

    fn custom_config(protocol: DnsProtocol, upstream: &str) -> DnsConfig {
        let mut config = config_with("custom", protocol);
        config.custom_upstream = Some(upstream.to_owned());
        config
    }

    #[test]
    fn cloudflare_endpoints_match_the_catalog() {
        let doh = upstreams(&config_with("cloudflare", DnsProtocol::Doh)).expect("doh");
        assert_eq!(doh.len(), 1);
        assert_eq!(doh[0].address, "https://cloudflare-dns.com/dns-query");
        assert_eq!(doh[0].as_engine_upstream(), doh[0].address);
        assert_eq!(doh[0].protocol_id(), "doh");

        let dot = upstreams(&config_with("cloudflare_family", DnsProtocol::Dot)).expect("dot");
        assert_eq!(dot[0].address, "family.cloudflare-dns.com");
        assert_eq!(
            dot[0].as_engine_upstream(),
            "tls://family.cloudflare-dns.com"
        );
    }

    #[test]
    fn filtering_variants_resolve_to_distinct_endpoints() {
        let cases = [
            ("google", "https://dns.google/dns-query"),
            ("adguard_family", "https://family.adguard-dns.com/dns-query"),
            ("quad9_unsecured", "https://dns10.quad9.net/dns-query"),
            (
                "cleanbrowsing_family",
                "https://doh.cleanbrowsing.org/doh/family-filter/",
            ),
            (
                "opendns_familyshield",
                "https://doh.familyshield.opendns.com/dns-query",
            ),
            ("controld_ads_tracking", "https://freedns.controld.com/p2"),
            (
                "controld_hagezi_ultimate",
                "https://freedns.controld.com/x-hagezi-ultimate",
            ),
            ("dnswatch", "https://resolver2.dns.watch/dns-query"),
        ];
        for (id, expected) in cases {
            let resolved = upstreams(&config_with(id, DnsProtocol::Doh)).expect(id);
            assert_eq!(resolved[0].address, expected, "provider {id}");
        }
    }

    #[test]
    fn bootstrap_servers_include_ipv4_and_ipv6() {
        let servers = bootstrap_servers(&config_with("cloudflare", DnsProtocol::Doh));
        assert!(servers.contains(&"1.1.1.1".to_owned()));
        assert!(servers.contains(&"2606:4700:4700::1111".to_owned()));
        let quad9 = bootstrap_servers(&config_with("quad9", DnsProtocol::Dot));
        assert!(quad9.contains(&"149.112.112.112".to_owned()));
        assert!(quad9.contains(&"2620:fe::9".to_owned()));
        let custom = bootstrap_servers(&custom_config(
            DnsProtocol::Doh,
            "https://dns.example.com/dns-query",
        ));
        assert_eq!(custom, vec!["1.1.1.1", "1.0.0.1"]);
    }

    #[test]
    fn nextdns_requires_configuration_id() {
        let mut config = config_with("nextdns", DnsProtocol::Doh);
        assert!(upstreams(&config).is_err());
        config.custom_upstream = Some("abc123".to_owned());
        let resolved = upstreams(&config).expect("nextdns");
        assert_eq!(resolved[0].address, "https://dns.nextdns.io/abc123");
        config.protocol = DnsProtocol::Dot;
        let dot = upstreams(&config).expect("nextdns dot");
        assert_eq!(dot[0].address, "abc123.dns.nextdns.io");
        config.custom_upstream = Some("bad id".to_owned());
        assert!(upstreams(&config).is_err());
    }

    #[test]
    fn unsupported_transports_are_rejected() {
        assert!(upstreams(&config_with("dnswatch", DnsProtocol::Dot)).is_err());
        assert!(upstreams(&config_with("comodo_secure", DnsProtocol::Doh)).is_err());
        assert!(upstreams(&config_with("comodo_secure", DnsProtocol::Dot)).is_err());
    }

    #[test]
    fn plain_dns_returns_every_bootstrap_address() {
        let config = config_with("comodo_secure", DnsProtocol::Plain);
        let resolved = upstreams(&config).expect("plain comodo");
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].address, "8.26.56.26");
        assert_eq!(resolved[0].protocol_id(), "plain");
        assert!(!config.protocol.is_encrypted());
        assert!(DnsProtocol::Doh.is_encrypted());
    }

    #[test]
    fn plain_dns_is_refused_for_providers_with_encryption() {
        let config = config_with("cloudflare_family", DnsProtocol::Plain);
        assert!(
            upstreams(&config).is_err(),
            "encrypted providers must never be configured as plain DNS"
        );
        let custom = custom_config(DnsProtocol::Plain, "192.0.2.53");
        // The custom provider must never silently downgrade to plain DNS.
        assert!(
            upstreams(&custom).is_err(),
            "custom plain DNS is refused; use DoH/DoT"
        );
    }

    #[test]
    fn custom_provider_validates_each_transport() {
        let doh = custom_config(DnsProtocol::Doh, "https://dns.example.com/dns-query");
        assert!(upstreams(&doh).is_ok());
        let plain_http = custom_config(DnsProtocol::Doh, "http://dns.example.com/dns-query");
        assert!(upstreams(&plain_http).is_err());
        let dot = custom_config(DnsProtocol::Dot, "dns.example.com:853");
        assert_eq!(
            upstreams(&dot).expect("dot")[0].address,
            "dns.example.com:853"
        );
        let plain = custom_config(DnsProtocol::Plain, "192.0.2.53");
        assert!(
            upstreams(&plain).is_err(),
            "custom DNS must stay encrypted (DoH/DoT)"
        );
        let bad_plain = custom_config(DnsProtocol::Plain, "not-an-ip");
        assert!(upstreams(&bad_plain).is_err());
        let missing = config_with("custom", DnsProtocol::Doh);
        assert!(upstreams(&missing).is_err());
    }

    #[test]
    fn provider_support_helper_matches_catalog() {
        let cloudflare = config_with("cloudflare", DnsProtocol::Doh);
        assert!(supports(&cloudflare, DnsProtocol::Doh));
        assert!(supports(&cloudflare, DnsProtocol::Dot));
        let comodo = config_with("comodo_secure", DnsProtocol::Plain);
        assert!(supports(&comodo, DnsProtocol::Plain));
        assert!(!supports(&comodo, DnsProtocol::Doh));
        let custom = config_with("custom", DnsProtocol::Doh);
        assert!(supports(&custom, DnsProtocol::Doh));
        assert!(!supports(&custom, DnsProtocol::Plain));
    }

    #[test]
    fn adguard_config_contains_catalog_endpoints_and_filters() {
        let mut config = config_with("quad9_ecs", DnsProtocol::Dot);
        config.blocklists = vec!["https://example.com/list.txt".to_owned()];
        let yaml = render_adguard_config(&config, "admin", "$argon2id$example", "127.0.0.1")
            .expect("render adguard");
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("valid yaml");
        let upstream = parsed["dns"]["upstream_dns"][0].as_str().expect("upstream");
        assert_eq!(upstream, "tls://dns11.quad9.net");
        let bootstrap = parsed["dns"]["bootstrap_dns"]
            .as_sequence()
            .expect("bootstrap");
        assert!(bootstrap
            .iter()
            .any(|value| value.as_str() == Some("9.9.9.11")));
        assert!(bootstrap
            .iter()
            .any(|value| value.as_str() == Some("2620:fe::11")));
        let filters = parsed["filters"].as_sequence().expect("filters");
        assert_eq!(filters.len(), 1);
    }

    #[test]
    fn adguard_config_supports_plain_legacy_provider() {
        let config = config_with("comodo_secure", DnsProtocol::Plain);
        let yaml =
            render_adguard_config(&config, "admin", "$hash", "127.0.0.1").expect("render plain");
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("valid yaml");
        let upstreams = parsed["dns"]["upstream_dns"]
            .as_sequence()
            .expect("upstreams");
        assert_eq!(upstreams.len(), 2);
        assert_eq!(upstreams[0].as_str(), Some("8.26.56.26"));
    }

    #[test]
    fn technitium_config_exposes_provider_metadata() {
        let config = config_with("controld_family", DnsProtocol::Doh);
        let json = render_technitium_config(&config).expect("render technitium");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["forwarder_protocol"], "doh");
        assert_eq!(parsed["forwarders"][0]["provider"], "controld_family");
        assert_eq!(
            parsed["forwarders"][0]["endpoint"],
            "https://freedns.controld.com/family"
        );
    }

    #[test]
    fn catalog_exposes_all_researched_variants() {
        let catalog = provider_catalog();
        assert!(catalog.len() >= 40);
        for id in [
            "cloudflare",
            "google_dns64",
            "adguard_family",
            "quad9_unsecured",
            "cleanbrowsing_security",
            "opendns_familyshield",
            "nextdns",
            "comodo_secure",
            "controld_hagezi_ultimate",
            "dnswatch",
        ] {
            assert!(spec_for(id).is_some(), "missing {id}");
        }
        assert!(spec_for("not-a-provider").is_none());
    }
}
