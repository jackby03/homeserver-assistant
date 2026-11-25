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

use hs_core::config::{DnsConfig, DnsProtocol, DnsProvider};

use crate::error::{NetworkError, NetworkResult};

/// Bootstrap resolvers used to resolve an encrypted endpoint hostname.
const CLOUDFLARE_BOOTSTRAP: [&str; 2] = ["1.1.1.1", "1.0.0.1"];
/// Quad9 bootstrap resolvers.
const QUAD9_BOOTSTRAP: [&str; 2] = ["9.9.9.9", "149.112.112.112"];
/// Control D bootstrap resolvers.
const CONTROLD_BOOTSTRAP: [&str; 2] = ["76.76.2.0", "76.76.10.0"];
/// Conservative fallback bootstrap resolvers for custom providers.
const DEFAULT_BOOTSTRAP: [&str; 2] = ["1.1.1.1", "1.0.0.1"];

/// A validated encrypted DNS upstream endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsUpstream {
    /// Endpoint address. For [`DnsProtocol::Doh`] this is an absolute
    /// `https://` URL; for [`DnsProtocol::Dot`] it is a hostname (or
    /// `hostname:853` when the configuration supplies an explicit port).
    pub address: String,
    /// Transport protocol used towards the endpoint.
    pub protocol: DnsProtocol,
    /// Provider that originated the endpoint.
    pub provider: DnsProvider,
}

impl DnsUpstream {
    /// Render the endpoint in the `upstream_dns` syntax used by AdGuard Home
    /// and Technitium (`https://...` for DoH, `tls://host[:port]` for DoT).
    #[must_use]
    pub fn as_engine_upstream(&self) -> String {
        match self.protocol {
            DnsProtocol::Doh => self.address.clone(),
            DnsProtocol::Dot => format!("tls://{}", self.address),
        }
    }

    /// Stable wire name of the transport (`doh` or `dot`).
    #[must_use]
    pub fn protocol_id(&self) -> &'static str {
        self.protocol.id()
    }
}

/// Resolve the configured provider into concrete encrypted upstreams.
///
/// Exactly one upstream is returned today (the provider selected by the
/// operator); the vector keeps the API future-proof for resolver sets.
///
/// # Errors
/// Returns [`NetworkError::Invalid`] when the provider is
/// [`DnsProvider::Custom`] but the custom endpoint is missing or unsafe (for
/// example plaintext `http://` for DoH).
pub fn upstreams(config: &DnsConfig) -> NetworkResult<Vec<DnsUpstream>> {
    let address = match config.provider {
        DnsProvider::Cloudflare => match config.protocol {
            DnsProtocol::Doh => "https://cloudflare-dns.com/dns-query".to_owned(),
            DnsProtocol::Dot => "dns.cloudflare.com".to_owned(),
        },
        DnsProvider::CloudflareMalware => match config.protocol {
            DnsProtocol::Doh => "https://security.cloudflare-dns.com/dns-query".to_owned(),
            DnsProtocol::Dot => "security.cloudflare-dns.com".to_owned(),
        },
        DnsProvider::CloudflareFamily => match config.protocol {
            DnsProtocol::Doh => "https://family.cloudflare-dns.com/dns-query".to_owned(),
            DnsProtocol::Dot => "family.cloudflare-dns.com".to_owned(),
        },
        DnsProvider::Quad9 => match config.protocol {
            DnsProtocol::Doh => "https://dns.quad9.net/dns-query".to_owned(),
            DnsProtocol::Dot => "dns.quad9.net".to_owned(),
        },
        DnsProvider::ControlD => match config.protocol {
            // Free, unfiltered Control D endpoint (profile p0).
            DnsProtocol::Doh => "https://freedns.controld.com/p0".to_owned(),
            DnsProtocol::Dot => "p0.freedns.controld.com".to_owned(),
        },
        DnsProvider::Custom => {
            let custom = config.custom_upstream.as_deref().ok_or_else(|| {
                NetworkError::Invalid(
                    "custom DNS provider requires `custom_upstream` to be set".to_owned(),
                )
            })?;
            validate_custom_upstream(config.protocol, custom)?;
            custom.trim().to_owned()
        }
    };

    Ok(vec![DnsUpstream {
        address,
        protocol: config.protocol,
        provider: config.provider,
    }])
}

/// Plain IP addresses used only to bootstrap the encrypted endpoint hostname.
///
/// These values are written to the engine `bootstrap_dns` field, never to
/// `upstream_dns`.
#[must_use]
pub fn bootstrap_servers(config: &DnsConfig) -> Vec<String> {
    let servers: &[&str] = match config.provider {
        DnsProvider::Cloudflare
        | DnsProvider::CloudflareMalware
        | DnsProvider::CloudflareFamily => &CLOUDFLARE_BOOTSTRAP,
        DnsProvider::Quad9 => &QUAD9_BOOTSTRAP,
        DnsProvider::ControlD => &CONTROLD_BOOTSTRAP,
        DnsProvider::Custom => &DEFAULT_BOOTSTRAP,
    };
    servers.iter().map(|server| (*server).to_owned()).collect()
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

    let upstreams = upstreams(config)?;
    validate_blocklists(&config.blocklists)?;

    let upstream_values = upstreams
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
    let upstreams = upstreams(config)?;
    validate_blocklists(&config.blocklists)?;

    let forwarders: Vec<serde_json::Value> = upstreams
        .iter()
        .map(|upstream| {
            serde_json::json!({
                "address": upstream.address,
                "endpoint": upstream.as_engine_upstream(),
                "protocol": upstream.protocol_id(),
                "provider": upstream.provider.id(),
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
    use serde_yaml::Value;

    use hs_core::config::{DnsConfig, DnsProtocol, DnsProvider};

    use super::{bootstrap_servers, render_adguard_config, render_technitium_config, upstreams};
    use crate::error::NetworkError;

    fn config_with(provider: DnsProvider, protocol: DnsProtocol) -> DnsConfig {
        DnsConfig {
            provider,
            protocol,
            ..DnsConfig::default()
        }
    }

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

    fn lookup_u64(document: &Value, path: &[&str]) -> Option<u64> {
        lookup(document, path).as_u64()
    }

    #[test]
    fn resolves_every_provider_for_both_transports() {
        let cases = [
            (
                DnsProvider::Cloudflare,
                DnsProtocol::Doh,
                "https://cloudflare-dns.com/dns-query",
            ),
            (
                DnsProvider::Cloudflare,
                DnsProtocol::Dot,
                "dns.cloudflare.com",
            ),
            (
                DnsProvider::CloudflareMalware,
                DnsProtocol::Doh,
                "https://security.cloudflare-dns.com/dns-query",
            ),
            (
                DnsProvider::CloudflareMalware,
                DnsProtocol::Dot,
                "security.cloudflare-dns.com",
            ),
            (
                DnsProvider::CloudflareFamily,
                DnsProtocol::Doh,
                "https://family.cloudflare-dns.com/dns-query",
            ),
            (
                DnsProvider::CloudflareFamily,
                DnsProtocol::Dot,
                "family.cloudflare-dns.com",
            ),
            (
                DnsProvider::Quad9,
                DnsProtocol::Doh,
                "https://dns.quad9.net/dns-query",
            ),
            (DnsProvider::Quad9, DnsProtocol::Dot, "dns.quad9.net"),
            (
                DnsProvider::ControlD,
                DnsProtocol::Doh,
                "https://freedns.controld.com/p0",
            ),
            (
                DnsProvider::ControlD,
                DnsProtocol::Dot,
                "p0.freedns.controld.com",
            ),
        ];
        for (provider, protocol, expected) in cases {
            let resolved = upstreams(&config_with(provider, protocol)).expect("upstream");
            assert_eq!(resolved.len(), 1);
            assert_eq!(resolved[0].address, expected);
            assert_eq!(resolved[0].protocol, protocol);
            assert_eq!(resolved[0].provider, provider);
        }
    }

    #[test]
    fn resolves_custom_upstreams() {
        let mut config = config_with(DnsProvider::Custom, DnsProtocol::Doh);
        config.custom_upstream = Some("https://dns.example.com/dns-query".to_owned());
        let resolved = upstreams(&config).expect("custom DoH");
        assert_eq!(resolved[0].address, "https://dns.example.com/dns-query");
        assert_eq!(
            resolved[0].as_engine_upstream(),
            "https://dns.example.com/dns-query"
        );

        config.protocol = DnsProtocol::Dot;
        config.custom_upstream = Some("dot.example.com:853".to_owned());
        let resolved = upstreams(&config).expect("custom DoT");
        assert_eq!(
            resolved[0].as_engine_upstream(),
            "tls://dot.example.com:853"
        );
    }

    #[test]
    fn rejects_unsafe_custom_upstreams() {
        let mut config = config_with(DnsProvider::Custom, DnsProtocol::Doh);
        assert!(matches!(upstreams(&config), Err(NetworkError::Invalid(_))));

        config.custom_upstream = Some("http://dns.example.com/dns-query".to_owned());
        assert!(matches!(upstreams(&config), Err(NetworkError::Invalid(_))));

        config.custom_upstream = Some("https://".to_owned());
        assert!(matches!(upstreams(&config), Err(NetworkError::Invalid(_))));

        config.protocol = DnsProtocol::Dot;
        config.custom_upstream = Some("tls://dns.example.com".to_owned());
        assert!(matches!(upstreams(&config), Err(NetworkError::Invalid(_))));

        config.custom_upstream = Some("dns example.com".to_owned());
        assert!(matches!(upstreams(&config), Err(NetworkError::Invalid(_))));

        config.custom_upstream = Some("dns.example.com:0".to_owned());
        assert!(matches!(upstreams(&config), Err(NetworkError::Invalid(_))));
    }

    #[test]
    fn bootstrap_servers_follow_the_provider() {
        assert_eq!(
            bootstrap_servers(&config_with(DnsProvider::Cloudflare, DnsProtocol::Doh)),
            vec!["1.1.1.1".to_owned(), "1.0.0.1".to_owned()]
        );
        assert_eq!(
            bootstrap_servers(&config_with(DnsProvider::Quad9, DnsProtocol::Dot)),
            vec!["9.9.9.9".to_owned(), "149.112.112.112".to_owned()]
        );
        assert_eq!(
            bootstrap_servers(&config_with(DnsProvider::ControlD, DnsProtocol::Doh)),
            vec!["76.76.2.0".to_owned(), "76.76.10.0".to_owned()]
        );
    }

    #[test]
    fn renders_valid_adguard_yaml_with_encrypted_upstreams() {
        let config = DnsConfig::default();
        let yaml = render_adguard_config(&config, "admin", "$2y$hash", "10.20.30.1")
            .expect("render AdGuard config");
        let parsed: Value = serde_yaml::from_str(&yaml).expect("valid YAML");

        assert_eq!(lookup_u64(&parsed, &["schema_version"]), Some(27));
        assert_eq!(lookup_u64(&parsed, &["dns", "port"]), Some(53));
        assert_eq!(
            lookup(&parsed, &["dns", "bind_hosts", "0"]),
            &Value::String("10.20.30.1".to_owned())
        );
        assert_eq!(
            lookup(&parsed, &["dns", "upstream_dns", "0"]),
            &Value::String("https://cloudflare-dns.com/dns-query".to_owned())
        );
        assert_eq!(
            lookup(&parsed, &["dns", "protection_enabled"]),
            &Value::Bool(true)
        );
        assert_eq!(
            lookup(&parsed, &["dns", "filtering_enabled"]),
            &Value::Bool(true)
        );
        assert_eq!(
            lookup(&parsed, &["users", "0", "name"]),
            &Value::String("admin".to_owned())
        );
        assert_eq!(
            lookup(&parsed, &["users", "0", "password"]),
            &Value::String("$2y$hash".to_owned())
        );

        let filters = lookup(&parsed, &["filters"]);
        let filters = filters.as_sequence().expect("filters must be a list");
        assert_eq!(filters.len(), config.blocklists.len());
        for (filter, expected_url) in filters.iter().zip(config.blocklists.iter()) {
            assert_eq!(
                filter.get("url"),
                Some(&Value::String(expected_url.clone()))
            );
            assert_eq!(filter.get("enabled"), Some(&Value::Bool(true)));
            assert!(filter
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| !name.is_empty()));
        }
    }

    #[test]
    fn adguard_upstreams_are_never_plaintext() {
        let config = DnsConfig::default();
        let yaml = render_adguard_config(&config, "admin", "$2y$hash", "0.0.0.0")
            .expect("render AdGuard config");
        let parsed: Value = serde_yaml::from_str(&yaml).expect("valid YAML");
        let upstreams = lookup(&parsed, &["dns", "upstream_dns"])
            .as_sequence()
            .expect("upstream_dns must be a list");
        assert!(!upstreams.is_empty());
        for upstream in upstreams {
            let address = upstream.as_str().expect("upstream must be a string");
            assert!(
                address.starts_with("https://") || address.starts_with("tls://"),
                "plaintext upstream rendered: {address}"
            );
        }
        // Bootstrap resolvers are plain IPs by design; they are not upstreams.
        let bootstrap = lookup(&parsed, &["dns", "bootstrap_dns"])
            .as_sequence()
            .expect("bootstrap_dns must be a list");
        assert_eq!(bootstrap.len(), 2);
    }

    #[test]
    fn adguard_renders_dot_with_tls_scheme() {
        let config = config_with(DnsProvider::Quad9, DnsProtocol::Dot);
        let yaml = render_adguard_config(&config, "admin", "$2y$hash", "10.20.30.1")
            .expect("render AdGuard config");
        let parsed: Value = serde_yaml::from_str(&yaml).expect("valid YAML");
        assert_eq!(
            lookup(&parsed, &["dns", "upstream_dns", "0"]),
            &Value::String("tls://dns.quad9.net".to_owned())
        );
    }

    #[test]
    fn adguard_follows_disabled_filtering_flag() {
        let config = DnsConfig {
            enabled: false,
            ..DnsConfig::default()
        };
        let yaml = render_adguard_config(&config, "admin", "$2y$hash", "0.0.0.0")
            .expect("render AdGuard config");
        let parsed: Value = serde_yaml::from_str(&yaml).expect("valid YAML");
        assert_eq!(
            lookup(&parsed, &["dns", "protection_enabled"]),
            &Value::Bool(false)
        );
        assert_eq!(
            lookup(&parsed, &["dns", "filtering_enabled"]),
            &Value::Bool(false)
        );
    }

    #[test]
    fn adguard_rejects_bad_admin_inputs() {
        let config = DnsConfig::default();
        assert!(matches!(
            render_adguard_config(&config, "", "$2y$hash", "0.0.0.0"),
            Err(NetworkError::Invalid(_))
        ));
        assert!(matches!(
            render_adguard_config(&config, "admin", "", "0.0.0.0"),
            Err(NetworkError::Invalid(_))
        ));
        assert!(matches!(
            render_adguard_config(&config, "admin", "$2y$hash", "not-an-ip"),
            Err(NetworkError::Invalid(_))
        ));
    }

    #[test]
    fn adguard_rejects_invalid_blocklists() {
        let config = DnsConfig {
            blocklists: vec!["not a url".to_owned()],
            ..DnsConfig::default()
        };
        assert!(matches!(
            render_adguard_config(&config, "admin", "$2y$hash", "0.0.0.0"),
            Err(NetworkError::Invalid(_))
        ));
    }

    #[test]
    fn renders_valid_technitium_json() {
        let config = config_with(DnsProvider::CloudflareFamily, DnsProtocol::Dot);
        let json = render_technitium_config(&config).expect("render Technitium config");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

        assert_eq!(parsed["engine"], "technitium");
        assert_eq!(parsed["dns_port"], 53);
        assert_eq!(parsed["forwarder_protocol"], "dot");
        let forwarders = parsed["forwarders"].as_array().expect("forwarders");
        assert_eq!(forwarders.len(), 1);
        assert_eq!(forwarders[0]["address"], "family.cloudflare-dns.com");
        assert_eq!(forwarders[0]["endpoint"], "tls://family.cloudflare-dns.com");
        assert_eq!(forwarders[0]["provider"], "cloudflare_family");
        assert!(parsed["bootstrap_dns"]
            .as_array()
            .is_some_and(|list| !list.is_empty()));
        assert!(parsed["blocklists"]
            .as_array()
            .is_some_and(|list| !list.is_empty()));
        assert_eq!(parsed["protection_enabled"], true);
    }

    #[test]
    fn technitium_rejects_invalid_custom_upstream() {
        let mut config = config_with(DnsProvider::Custom, DnsProtocol::Doh);
        config.custom_upstream = Some("http://dns.example.com/dns-query".to_owned());
        assert!(matches!(
            render_technitium_config(&config),
            Err(NetworkError::Invalid(_))
        ));
    }

    #[test]
    fn technitium_json_uses_doh_urls() {
        let config = config_with(DnsProvider::Cloudflare, DnsProtocol::Doh);
        let json = render_technitium_config(&config).expect("render Technitium config");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(
            parsed["forwarders"][0]["address"],
            "https://cloudflare-dns.com/dns-query"
        );
        assert_eq!(parsed["forwarders"][0]["protocol"], "doh");
        assert_eq!(parsed["forwarder_protocol"], "doh");
    }
}
