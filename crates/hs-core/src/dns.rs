//! Free public DNS resolver catalog.
//!
//! The appliance ships a data-driven catalog of free DNS providers and their
//! filtering variants. Each entry lists the IPv4/IPv6 bootstrap addresses and
//! the encrypted endpoints (DoH/DoT) so the UI can show exactly what will be
//! configured and the resolver can build the engine configuration without a
//! giant match statement.
//!
//! Everything here is public, free-to-use resolver data published by each
//! vendor. `DnsProvider` is a validated identifier (a newtype over the catalog
//! id) so configuration files and API payloads stay stable and typo-proof.
//!
//! Security: encrypted transports are preferred. Plain DNS is only available
//! for providers that do not offer DoH/DoT (currently Comodo Secure) and must be
//! selected explicitly; the UI marks it as unencrypted.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::config::DnsProtocol;
use crate::error::{CoreError, CoreResult};

/// Vendor metadata used to group providers in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DnsVendorSpec {
    /// Stable vendor id.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
}

const CLOUDFLARE: DnsVendorSpec = DnsVendorSpec {
    id: "cloudflare",
    name: "Cloudflare",
};
const GOOGLE: DnsVendorSpec = DnsVendorSpec {
    id: "google",
    name: "Google",
};
const ADGUARD: DnsVendorSpec = DnsVendorSpec {
    id: "adguard",
    name: "AdGuard DNS",
};
const QUAD9: DnsVendorSpec = DnsVendorSpec {
    id: "quad9",
    name: "Quad9",
};
const CLEANBROWSING: DnsVendorSpec = DnsVendorSpec {
    id: "cleanbrowsing",
    name: "CleanBrowsing",
};
const OPENDNS: DnsVendorSpec = DnsVendorSpec {
    id: "opendns",
    name: "OpenDNS",
};
const NEXTDNS: DnsVendorSpec = DnsVendorSpec {
    id: "nextdns",
    name: "NextDNS",
};
const COMODO: DnsVendorSpec = DnsVendorSpec {
    id: "comodo",
    name: "Comodo Secure",
};
const CONTROLD: DnsVendorSpec = DnsVendorSpec {
    id: "controld",
    name: "Control D",
};
const DNSWATCH: DnsVendorSpec = DnsVendorSpec {
    id: "dnswatch",
    name: "DNS.WATCH",
};
const CUSTOM_VENDOR: DnsVendorSpec = DnsVendorSpec {
    id: "custom",
    name: "Personalizado",
};

/// One free DNS provider variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DnsProviderSpec {
    /// Stable identifier persisted in `SystemConfig` and used by the API.
    pub id: &'static str,
    /// Human readable name.
    pub label: &'static str,
    /// Vendor used for grouping.
    pub vendor: DnsVendorSpec,
    /// IPv4 bootstrap addresses.
    pub ipv4: &'static [&'static str],
    /// IPv6 bootstrap addresses.
    pub ipv6: &'static [&'static str],
    /// DoH endpoint (`https://...`, may contain `{id}` for NextDNS).
    pub doh: Option<&'static str>,
    /// DoT hostname (may contain `{id}` for NextDNS).
    pub dot: Option<&'static str>,
    /// Whether plain DNS (port 53, unencrypted) is available.
    pub plain: bool,
    /// Whether the provider needs a user-specific configuration id.
    pub requires_config_id: bool,
    /// Short note shown in the UI.
    pub note: Option<&'static str>,
}

/// Compact constructor for catalog entries: one call per provider keeps the
/// table readable, and a struct literal would repeat every field name 40 times.
#[allow(clippy::too_many_arguments)]
const fn provider(
    id: &'static str,
    label: &'static str,
    vendor: DnsVendorSpec,
    ipv4: &'static [&'static str],
    ipv6: &'static [&'static str],
    doh: Option<&'static str>,
    dot: Option<&'static str>,
    plain: bool,
    requires_config_id: bool,
    note: Option<&'static str>,
) -> DnsProviderSpec {
    DnsProviderSpec {
        id,
        label,
        vendor,
        ipv4,
        ipv6,
        doh,
        dot,
        plain,
        requires_config_id,
        note,
    }
}

const CLOUDFLARE_IPV4: &[&str] = &["1.1.1.1", "1.0.0.1"];
const CLOUDFLARE_IPV6: &[&str] = &["2606:4700:4700::1111", "2606:4700:4700::1001"];
const CLOUDFLARE_MALWARE_IPV4: &[&str] = &["1.1.1.2", "1.0.0.2"];
const CLOUDFLARE_MALWARE_IPV6: &[&str] = &["2606:4700:4700::1112", "2606:4700:4700::1002"];
const CLOUDFLARE_FAMILY_IPV4: &[&str] = &["1.1.1.3", "1.0.0.3"];
const CLOUDFLARE_FAMILY_IPV6: &[&str] = &["2606:4700:4700::1113", "2606:4700:4700::1003"];
const GOOGLE_IPV4: &[&str] = &["8.8.8.8", "8.8.4.4"];
const GOOGLE_IPV6: &[&str] = &["2001:4860:4860::8888", "2001:4860:4860::8844"];
const GOOGLE_DNS64_IPV6: &[&str] = &["2001:4860:4860::6464", "2001:4860:4860::64"];
const ADGUARD_IPV4: &[&str] = &["94.140.14.14", "94.140.15.15"];
const ADGUARD_IPV6: &[&str] = &["2a10:50c0::ad1:ff", "2a10:50c0::ad2:ff"];
const ADGUARD_UNFILTERED_IPV4: &[&str] = &["94.140.14.140", "94.140.14.141"];
const ADGUARD_UNFILTERED_IPV6: &[&str] = &["2a10:50c0::1:ff", "2a10:50c0::2:ff"];
const ADGUARD_FAMILY_IPV4: &[&str] = &["94.140.14.15", "94.140.15.16"];
const ADGUARD_FAMILY_IPV6: &[&str] = &["2a10:50c0::bad1:ff", "2a10:50c0::bad2:ff"];
const QUAD9_IPV4: &[&str] = &["9.9.9.9", "149.112.112.112"];
const QUAD9_IPV6: &[&str] = &["2620:fe::fe", "2620:fe::9"];
const QUAD9_ECS_IPV4: &[&str] = &["9.9.9.11", "149.112.112.11"];
const QUAD9_ECS_IPV6: &[&str] = &["2620:fe::11", "2620:fe::fe:11"];
const QUAD9_UNSECURED_IPV4: &[&str] = &["9.9.9.10", "149.112.112.10"];
const QUAD9_UNSECURED_IPV6: &[&str] = &["2620:fe::10", "2620:fe::fe:10"];
const CLEANBROWSING_SECURITY_IPV4: &[&str] = &["185.228.168.9", "185.228.169.9"];
const CLEANBROWSING_SECURITY_IPV6: &[&str] = &["2a0d:2a00:1::2", "2a0d:2a00:2::2"];
const CLEANBROWSING_ADULT_IPV4: &[&str] = &["185.228.168.10", "185.228.169.11"];
const CLEANBROWSING_ADULT_IPV6: &[&str] = &["2a0d:2a00:1::1", "2a0d:2a00:2::1"];
const CLEANBROWSING_FAMILY_IPV4: &[&str] = &["185.228.168.168", "185.228.169.168"];
const CLEANBROWSING_FAMILY_IPV6: &[&str] = &["2a0d:2a00:1::", "2a0d:2a00:2::"];
const OPENDNS_IPV4: &[&str] = &["208.67.222.222", "208.67.220.220"];
const OPENDNS_IPV6: &[&str] = &["2620:119:35::35", "2620:119:53::53"];
const OPENDNS_FAMILY_IPV4: &[&str] = &["208.67.222.123", "208.67.220.123"];
const OPENDNS_FAMILY_IPV6: &[&str] = &["2620:119:35::123", "2620:119:53::123"];
const OPENDNS_SANDBOX_IPV4: &[&str] = &["208.67.222.2", "208.67.220.2"];
const OPENDNS_SANDBOX_IPV6: &[&str] = &["2620:0:ccc::2", "2620:0:ccd::2"];
const NEXTDNS_IPV4: &[&str] = &["45.90.28.0", "45.90.30.0"];
const NEXTDNS_IPV6: &[&str] = &["2a07:a8c0::", "2a07:a8c1::"];
const COMODO_IPV4: &[&str] = &["8.26.56.26", "8.20.247.20"];
const CONTROLD_IPV4: &[&str] = &["76.76.2.0", "76.76.10.0"];
const CONTROLD_IPV6: &[&str] = &["2606:1a40::0", "2606:1a40:1::0"];
const DNSWATCH_IPV4: &[&str] = &["84.200.69.80", "84.200.70.40"];
const DNSWATCH_IPV6: &[&str] = &["2001:1608:10:25::1c04:b12f", "2001:1608:10:25::9249:d69b"];
const EMPTY_V4: &[&str] = &[];

/// Complete free-DNS catalog (ordered by vendor, then filtering level).
pub static DNS_PROVIDERS: &[DnsProviderSpec] = &[
    // --- Cloudflare -------------------------------------------------------
    provider(
        "cloudflare",
        "Cloudflare",
        CLOUDFLARE,
        CLOUDFLARE_IPV4,
        CLOUDFLARE_IPV6,
        Some("https://cloudflare-dns.com/dns-query"),
        Some("one.one.one.one"),
        true,
        false,
        Some("Rápido y sin filtrado"),
    ),
    provider(
        "cloudflare_malware",
        "Cloudflare Malware",
        CLOUDFLARE,
        CLOUDFLARE_MALWARE_IPV4,
        CLOUDFLARE_MALWARE_IPV6,
        Some("https://security.cloudflare-dns.com/dns-query"),
        Some("security.cloudflare-dns.com"),
        true,
        false,
        Some("Bloquea malware y phishing"),
    ),
    provider(
        "cloudflare_family",
        "Cloudflare Family",
        CLOUDFLARE,
        CLOUDFLARE_FAMILY_IPV4,
        CLOUDFLARE_FAMILY_IPV6,
        Some("https://family.cloudflare-dns.com/dns-query"),
        Some("family.cloudflare-dns.com"),
        true,
        false,
        Some("Bloquea malware y contenido adulto"),
    ),
    // --- Google -----------------------------------------------------------
    provider(
        "google",
        "Google Public DNS",
        GOOGLE,
        GOOGLE_IPV4,
        GOOGLE_IPV6,
        Some("https://dns.google/dns-query"),
        Some("dns.google"),
        true,
        false,
        Some("Estándar, sin filtrado"),
    ),
    provider(
        "google_dns64",
        "Google DNS64",
        GOOGLE,
        EMPTY_V4,
        GOOGLE_DNS64_IPV6,
        Some("https://dns64.dns.google/dns-query"),
        Some("dns64.dns.google"),
        false,
        false,
        Some("Solo IPv6 (NAT64)"),
    ),
    // --- AdGuard DNS ------------------------------------------------------
    provider(
        "adguard",
        "AdGuard DNS",
        ADGUARD,
        ADGUARD_IPV4,
        ADGUARD_IPV6,
        Some("https://dns.adguard-dns.com/dns-query"),
        Some("dns.adguard-dns.com"),
        true,
        false,
        Some("Bloqueo de anuncios y rastreadores"),
    ),
    provider(
        "adguard_unfiltered",
        "AdGuard DNS sin filtro",
        ADGUARD,
        ADGUARD_UNFILTERED_IPV4,
        ADGUARD_UNFILTERED_IPV6,
        Some("https://unfiltered.adguard-dns.com/dns-query"),
        Some("unfiltered.adguard-dns.com"),
        true,
        false,
        Some("Sin filtrado"),
    ),
    provider(
        "adguard_family",
        "AdGuard DNS Family",
        ADGUARD,
        ADGUARD_FAMILY_IPV4,
        ADGUARD_FAMILY_IPV6,
        Some("https://family.adguard-dns.com/dns-query"),
        Some("family.adguard-dns.com"),
        true,
        false,
        Some("Anuncios + contenido adulto"),
    ),
    // --- Quad9 ------------------------------------------------------------
    provider(
        "quad9",
        "Quad9",
        QUAD9,
        QUAD9_IPV4,
        QUAD9_IPV6,
        Some("https://dns.quad9.net/dns-query"),
        Some("dns.quad9.net"),
        true,
        false,
        Some("Recomendado: bloquea dominios maliciosos"),
    ),
    provider(
        "quad9_ecs",
        "Quad9 Secured (ECS)",
        QUAD9,
        QUAD9_ECS_IPV4,
        QUAD9_ECS_IPV6,
        Some("https://dns11.quad9.net/dns-query"),
        Some("dns11.quad9.net"),
        true,
        false,
        Some("Bloqueo con EDNS Client Subnet"),
    ),
    provider(
        "quad9_unsecured",
        "Quad9 Unsecured",
        QUAD9,
        QUAD9_UNSECURED_IPV4,
        QUAD9_UNSECURED_IPV6,
        Some("https://dns10.quad9.net/dns-query"),
        Some("dns10.quad9.net"),
        true,
        false,
        Some("Sin bloqueo"),
    ),
    // --- CleanBrowsing ----------------------------------------------------
    provider(
        "cleanbrowsing_security",
        "CleanBrowsing Security",
        CLEANBROWSING,
        CLEANBROWSING_SECURITY_IPV4,
        CLEANBROWSING_SECURITY_IPV6,
        Some("https://doh.cleanbrowsing.org/doh/security-filter/"),
        Some("security-filter-dns.cleanbrowsing.org"),
        true,
        false,
        Some("Bloquea phishing y malware"),
    ),
    provider(
        "cleanbrowsing_adult",
        "CleanBrowsing Adult",
        CLEANBROWSING,
        CLEANBROWSING_ADULT_IPV4,
        CLEANBROWSING_ADULT_IPV6,
        Some("https://doh.cleanbrowsing.org/doh/adult-filter/"),
        Some("adult-filter-dns.cleanbrowsing.org"),
        true,
        false,
        Some("Bloquea contenido adulto"),
    ),
    provider(
        "cleanbrowsing_family",
        "CleanBrowsing Family",
        CLEANBROWSING,
        CLEANBROWSING_FAMILY_IPV4,
        CLEANBROWSING_FAMILY_IPV6,
        Some("https://doh.cleanbrowsing.org/doh/family-filter/"),
        Some("family-filter-dns.cleanbrowsing.org"),
        true,
        false,
        Some("Filtro familiar completo"),
    ),
    // --- OpenDNS ----------------------------------------------------------
    provider(
        "opendns",
        "OpenDNS Home",
        OPENDNS,
        OPENDNS_IPV4,
        OPENDNS_IPV6,
        Some("https://doh.opendns.com/dns-query"),
        Some("dns.opendns.com"),
        true,
        false,
        Some("Filtrado opcional vía cuenta OpenDNS"),
    ),
    provider(
        "opendns_familyshield",
        "OpenDNS FamilyShield",
        OPENDNS,
        OPENDNS_FAMILY_IPV4,
        OPENDNS_FAMILY_IPV6,
        Some("https://doh.familyshield.opendns.com/dns-query"),
        Some("familyshield.opendns.com"),
        true,
        false,
        Some("Bloqueo familiar preconfigurado"),
    ),
    provider(
        "opendns_sandbox",
        "OpenDNS Sandbox",
        OPENDNS,
        OPENDNS_SANDBOX_IPV4,
        OPENDNS_SANDBOX_IPV6,
        Some("https://doh.sandbox.opendns.com/dns-query"),
        Some("sandbox.opendns.com"),
        true,
        false,
        Some("Sin filtrado"),
    ),
    // --- NextDNS ----------------------------------------------------------
    provider(
        "nextdns",
        "NextDNS",
        NEXTDNS,
        NEXTDNS_IPV4,
        NEXTDNS_IPV6,
        Some("https://dns.nextdns.io/{id}"),
        Some("{id}.dns.nextdns.io"),
        false,
        true,
        Some("Requiere tu ID de configuración de NextDNS"),
    ),
    // --- Comodo -----------------------------------------------------------
    provider(
        "comodo_secure",
        "Comodo Secure DNS",
        COMODO,
        COMODO_IPV4,
        EMPTY_V4,
        None,
        None,
        true,
        false,
        Some("Solo DNS sin cifrar (sin DoH/DoT)"),
    ),
    // --- Control D --------------------------------------------------------
    provider(
        "controld",
        "Control D Unfiltered",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/p0"),
        Some("p0.freedns.controld.com"),
        true,
        false,
        Some("Sin filtrado"),
    ),
    provider(
        "controld_malware",
        "Control D Malware",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/p1"),
        Some("p1.freedns.controld.com"),
        true,
        false,
        Some("Bloquea malware"),
    ),
    provider(
        "controld_ads_tracking",
        "Control D Ads & Tracking",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/p2"),
        Some("p2.freedns.controld.com"),
        true,
        false,
        Some("Bloquea anuncios y rastreadores"),
    ),
    provider(
        "controld_social",
        "Control D Social",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/p3"),
        Some("p3.freedns.controld.com"),
        true,
        false,
        Some("Bloquea redes sociales"),
    ),
    provider(
        "controld_family",
        "Control D Family",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/family"),
        Some("family.freedns.controld.com"),
        true,
        false,
        Some("Filtro familiar"),
    ),
    provider(
        "controld_uncensored",
        "Control D Uncensored",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/uncensored"),
        Some("uncensored.freedns.controld.com"),
        true,
        false,
        Some("Sin censura"),
    ),
    provider(
        "controld_oisd_full",
        "Control D OISD Full",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/x-oisd"),
        Some("x-oisd.freedns.controld.com"),
        true,
        false,
        Some("Lista comunitaria OISD Full"),
    ),
    provider(
        "controld_oisd_basic",
        "Control D OISD Basic",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/x-oisd-basic"),
        Some("x-oisd-basic.freedns.controld.com"),
        true,
        false,
        Some("Lista comunitaria OISD Basic"),
    ),
    provider(
        "controld_stevenblack",
        "Control D StevenBlack",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/x-stevenblack"),
        Some("x-stevenblack.freedns.controld.com"),
        true,
        false,
        Some("Lista StevenBlack Unified"),
    ),
    provider(
        "controld_1hosts_lite",
        "Control D 1Hosts Lite",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/x-1hosts-lite"),
        Some("x-1hosts-lite.freedns.controld.com"),
        true,
        false,
        Some("Lista 1Hosts Lite"),
    ),
    provider(
        "controld_1hosts_pro",
        "Control D 1Hosts Pro",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/x-1hosts-pro"),
        Some("x-1hosts-pro.freedns.controld.com"),
        true,
        false,
        Some("Lista 1Hosts Pro"),
    ),
    provider(
        "controld_1hosts_xtra",
        "Control D 1Hosts Xtra",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/x-1hosts-xtra"),
        Some("x-1hosts-xtra.freedns.controld.com"),
        true,
        false,
        Some("Lista 1Hosts Xtra"),
    ),
    provider(
        "controld_hagezi_light",
        "Control D Hagezi Light",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/x-hagezi-light"),
        Some("x-hagezi-light.freedns.controld.com"),
        true,
        false,
        Some("Lista Hagezi Light"),
    ),
    provider(
        "controld_hagezi_normal",
        "Control D Hagezi Normal",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/x-hagezi-normal"),
        Some("x-hagezi-normal.freedns.controld.com"),
        true,
        false,
        Some("Lista Hagezi Normal"),
    ),
    provider(
        "controld_hagezi_pro",
        "Control D Hagezi Pro",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/x-hagezi-pro"),
        Some("x-hagezi-pro.freedns.controld.com"),
        true,
        false,
        Some("Lista Hagezi Pro"),
    ),
    provider(
        "controld_hagezi_proplus",
        "Control D Hagezi Pro Plus",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/x-hagezi-proplus"),
        Some("x-hagezi-proplus.freedns.controld.com"),
        true,
        false,
        Some("Lista Hagezi Pro Plus"),
    ),
    provider(
        "controld_hagezi_ultimate",
        "Control D Hagezi Ultimate",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/x-hagezi-ultimate"),
        Some("x-hagezi-ultimate.freedns.controld.com"),
        true,
        false,
        Some("Lista Hagezi Ultimate"),
    ),
    provider(
        "controld_hagezi_tif",
        "Control D Hagezi TIF",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/x-hagezi-tif"),
        Some("x-hagezi-tif.freedns.controld.com"),
        true,
        false,
        Some("Lista Hagezi TIF"),
    ),
    provider(
        "controld_adguard_filter",
        "Control D AdGuard Filter",
        CONTROLD,
        CONTROLD_IPV4,
        CONTROLD_IPV6,
        Some("https://freedns.controld.com/x-adguard"),
        Some("x-adguard.freedns.controld.com"),
        true,
        false,
        Some("Lista AdGuard Filter"),
    ),
    // --- DNS.WATCH --------------------------------------------------------
    provider(
        "dnswatch",
        "DNS.WATCH",
        DNSWATCH,
        DNSWATCH_IPV4,
        DNSWATCH_IPV6,
        Some("https://resolver2.dns.watch/dns-query"),
        None,
        true,
        false,
        Some("DoT no disponible; usa DoH o sin cifrar"),
    ),
    // --- Custom -----------------------------------------------------------
    provider(
        "custom",
        "Personalizado",
        CUSTOM_VENDOR,
        EMPTY_V4,
        EMPTY_V4,
        None,
        None,
        false,
        false,
        Some("Tu propio resolver DoH/DoT"),
    ),
];

/// Look up a provider spec by id.
#[must_use]
pub fn provider_spec(id: &str) -> Option<&'static DnsProviderSpec> {
    DNS_PROVIDERS.iter().find(|spec| spec.id == id)
}

/// All vendors in catalog order (deduplicated).
#[must_use]
pub fn vendors() -> Vec<&'static DnsVendorSpec> {
    let mut vendors: Vec<&'static DnsVendorSpec> = Vec::new();
    for spec in DNS_PROVIDERS {
        if !vendors.iter().any(|vendor| vendor.id == spec.vendor.id) {
            vendors.push(&spec.vendor);
        }
    }
    vendors
}

/// Validated DNS provider identifier.
///
/// Serialized as the catalog id (`"quad9"`, `"cloudflare_family"`, ...). Any
/// unknown id is rejected at deserialization/validation time, so a typo can
/// never silently disable DNS filtering.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DnsProvider(String);

impl DnsProvider {
    /// Create a provider from a catalog id.
    ///
    /// # Errors
    /// Returns [`CoreError::Validation`] when the id is not in the catalog.
    pub fn new(id: impl Into<String>) -> CoreResult<Self> {
        let id = id.into();
        let id = id.trim();
        if provider_spec(id).is_none() {
            return Err(CoreError::validation(
                "provider",
                format!("unknown DNS provider `{id}`"),
            ));
        }
        Ok(Self(id.to_owned()))
    }

    /// Cloudflare standard.
    #[must_use]
    pub fn cloudflare() -> Self {
        Self::catalog("cloudflare")
    }

    /// Cloudflare malware blocking.
    #[must_use]
    pub fn cloudflare_malware() -> Self {
        Self::catalog("cloudflare_malware")
    }

    /// Cloudflare family protection.
    #[must_use]
    pub fn cloudflare_family() -> Self {
        Self::catalog("cloudflare_family")
    }

    /// Quad9 recommended.
    #[must_use]
    pub fn quad9() -> Self {
        Self::catalog("quad9")
    }

    /// Control D free unfiltered.
    #[must_use]
    pub fn control_d() -> Self {
        Self::catalog("controld")
    }

    /// Custom provider (operator supplied endpoints).
    #[must_use]
    pub fn custom() -> Self {
        Self::catalog("custom")
    }

    /// Get a provider that is guaranteed to exist in the catalog.
    #[must_use]
    pub fn catalog(id: &'static str) -> Self {
        debug_assert!(provider_spec(id).is_some(), "catalog id `{id}`");
        Self(id.to_owned())
    }

    /// Stable catalog id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Provider spec, if the id is known (always true for validated values).
    #[must_use]
    pub fn spec(&self) -> Option<&'static DnsProviderSpec> {
        provider_spec(&self.0)
    }

    /// Display label (falls back to the id).
    #[must_use]
    pub fn label(&self) -> &str {
        self.spec().map_or(self.0.as_str(), |spec| spec.label)
    }

    /// Vendor metadata.
    #[must_use]
    pub fn vendor(&self) -> Option<&'static DnsVendorSpec> {
        self.spec().map(|spec| &spec.vendor)
    }

    /// Whether the provider supports the given transport.
    #[must_use]
    pub fn supports(&self, protocol: DnsProtocol) -> bool {
        let Some(spec) = self.spec() else {
            return false;
        };
        if self.0 == "custom" {
            return matches!(protocol, DnsProtocol::Doh | DnsProtocol::Dot);
        }
        match protocol {
            DnsProtocol::Doh => spec.doh.is_some(),
            DnsProtocol::Dot => spec.dot.is_some(),
            DnsProtocol::Plain => spec.plain,
        }
    }

    /// Whether the provider offers at least one encrypted transport.
    #[must_use]
    pub fn has_encrypted_transport(&self) -> bool {
        self.supports(DnsProtocol::Doh) || self.supports(DnsProtocol::Dot)
    }

    /// Preferred encrypted transport: DoH first, then DoT.
    ///
    /// The appliance always prioritizes encryption; plain DNS is only used for
    /// legacy providers without DoH/DoT. Returns `None` when the provider has no
    /// encrypted endpoint (currently only Comodo Secure).
    #[must_use]
    pub fn preferred_encrypted_protocol(&self) -> Option<DnsProtocol> {
        if self.supports(DnsProtocol::Doh) {
            Some(DnsProtocol::Doh)
        } else if self.supports(DnsProtocol::Dot) {
            Some(DnsProtocol::Dot)
        } else {
            None
        }
    }

    /// Whether the provider needs an operator-supplied configuration id.
    #[must_use]
    pub fn requires_config_id(&self) -> bool {
        self.spec().is_some_and(|spec| spec.requires_config_id)
    }

    /// Whether this is the operator-defined custom provider.
    #[must_use]
    pub fn is_custom(&self) -> bool {
        self.0 == "custom"
    }

    /// Whether this is NextDNS (needs a configuration id).
    #[must_use]
    pub fn is_next_dns(&self) -> bool {
        self.0 == "nextdns"
    }
}

impl Default for DnsProvider {
    fn default() -> Self {
        Self::cloudflare()
    }
}

impl fmt::Display for DnsProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for DnsProvider {
    type Err = CoreError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for DnsProvider {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value).map_err(|error| error.to_string())
    }
}

impl From<DnsProvider> for String {
    fn from(provider: DnsProvider) -> Self {
        provider.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_unique_ids_and_complete_data() {
        let mut ids: Vec<&str> = DNS_PROVIDERS.iter().map(|spec| spec.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "provider ids must be unique");
        assert!(
            count >= 40,
            "catalog should include every researched variant"
        );
        for spec in DNS_PROVIDERS {
            assert!(!spec.label.is_empty(), "{} needs a label", spec.id);
            if spec.id == "custom" {
                // The operator supplies the endpoint for the custom provider.
                continue;
            }
            assert!(
                !spec.ipv4.is_empty() || !spec.ipv6.is_empty(),
                "{} needs at least one bootstrap address",
                spec.id
            );
            assert!(
                spec.doh.is_some() || spec.dot.is_some() || spec.plain,
                "{} needs at least one transport",
                spec.id
            );
            if spec.id == "comodo_secure" {
                assert!(spec.plain && spec.doh.is_none() && spec.dot.is_none());
            }
            if spec.id == "dnswatch" {
                assert!(spec.doh.is_some() && spec.dot.is_none());
            }
            if spec.id == "nextdns" {
                assert!(spec.requires_config_id);
                assert!(spec.doh.unwrap_or_default().contains("{id}"));
            }
        }
    }

    #[test]
    fn every_vendor_is_grouped() {
        let vendors = vendors();
        assert!(vendors.len() >= 10);
        assert!(vendors
            .iter()
            .any(|vendor| vendor.id == "controld" && vendor.name == "Control D"));
    }

    #[test]
    fn provider_ids_are_validated() {
        assert_eq!(
            DnsProvider::new("cloudflare").unwrap().label(),
            "Cloudflare"
        );
        assert!(DnsProvider::new("does-not-exist").is_err());
        assert!(DnsProvider::new("").is_err());
        assert_eq!(DnsProvider::default(), DnsProvider::cloudflare());
    }

    #[test]
    fn protocol_support_matches_catalog() {
        let comodo = DnsProvider::new("comodo_secure").unwrap();
        assert!(comodo.supports(DnsProtocol::Plain));
        assert!(!comodo.supports(DnsProtocol::Doh));
        assert!(!comodo.supports(DnsProtocol::Dot));

        let dnswatch = DnsProvider::new("dnswatch").unwrap();
        assert!(dnswatch.supports(DnsProtocol::Doh));
        assert!(!dnswatch.supports(DnsProtocol::Dot));
        assert!(dnswatch.supports(DnsProtocol::Plain));

        let cloudflare = DnsProvider::cloudflare();
        assert!(cloudflare.supports(DnsProtocol::Doh));
        assert!(cloudflare.supports(DnsProtocol::Dot));
        assert!(cloudflare.supports(DnsProtocol::Plain));

        let custom = DnsProvider::custom();
        assert!(custom.supports(DnsProtocol::Doh));
        assert!(custom.supports(DnsProtocol::Dot));
        assert!(!custom.supports(DnsProtocol::Plain));
    }

    #[test]
    fn nextdns_requires_id_and_custom_is_flagged() {
        assert!(DnsProvider::new("nextdns").unwrap().requires_config_id());
        assert!(DnsProvider::custom().is_custom());
        assert!(!DnsProvider::cloudflare().is_custom());
    }

    #[test]
    fn serde_roundtrip_uses_catalog_id() {
        let provider = DnsProvider::new("controld_family").unwrap();
        let json = serde_json::to_string(&provider).unwrap();
        assert_eq!(json, "\"controld_family\"");
        let parsed: DnsProvider = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, provider);
        assert!(serde_json::from_str::<DnsProvider>("\"nope\"").is_err());
    }
}
