//! DNS (DoH/DoT) profiles, nftables firewall and DMZ isolation.
//!
//! The crate is split into small, mostly pure modules so that every artefact
//! can be unit tested without root privileges or network access:
//!
//! - [`dns`]: resolve the configured encrypted DNS provider into concrete
//!   upstream endpoints and render AdGuard Home / Technitium configuration.
//! - [`firewall`]: render, validate and apply the nftables ruleset that keeps
//!   the DMZ isolated from the LAN and the host.
//! - [`netplan`]: render and apply the netplan YAML for the LAN interface.
//! - [`interfaces`]: detect local network interfaces (best effort).
//!
//! # Security invariants
//! - Encrypted DNS only: no plaintext UDP/TCP upstream is ever rendered into a
//!   filtering engine configuration; bootstrap resolvers are only used to
//!   resolve the encrypted endpoint hostnames.
//! - DMZ isolation: [`firewall::render_nftables`] always emits an explicit
//!   DMZ -> LAN drop and an explicit DMZ -> host service drop. Opt-in DMZ
//!   internet egress (forward accept towards the WAN uplink plus a NAT
//!   masquerade table) never weakens those drops, and
//!   [`firewall::validate_isolation`] re-checks a ruleset before it is applied.
//! - Privileged operations (`apply_*`) never run in tests and return
//!   [`NetworkError::Unsupported`] outside Linux.

#![deny(missing_docs)]

pub mod dns;
pub mod error;
pub mod firewall;
pub mod interfaces;
pub mod netplan;

mod fsutil;

pub use error::{NetworkError, NetworkResult};

/// Build the canonical administrative portal URL for the appliance.
///
/// The portal is served by the local reverse proxy over TLS on
/// [`hs_core::config::SecurityConfig::admin_web_port`] and advertised with the
/// `.local` mDNS name of the appliance, e.g.
/// `https://homeserver.local:8443`.
///
/// The hostname is sanitized to the characters allowed in a DNS label; if the
/// configured hostname cannot be used the fallback `homeserver` is returned so
/// that a partially configured appliance still produces a safe URL.
#[must_use]
pub fn admin_url(config: &hs_core::config::SystemConfig) -> String {
    let hostname = config.device.hostname.trim().to_ascii_lowercase();
    let hostname = if hostname.is_empty()
        || hostname.len() > 63
        || !hostname
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || hostname.starts_with('-')
        || hostname.ends_with('-')
    {
        "homeserver"
    } else {
        hostname.as_str()
    };
    format!(
        "https://{hostname}.local:{}",
        config.security.admin_web_port
    )
}

#[cfg(test)]
mod tests {
    use hs_core::config::SystemConfig;

    use super::admin_url;

    #[test]
    fn admin_url_uses_hostname_and_port() {
        let mut config = SystemConfig::bootstrap();
        config.device.hostname = "homeserver".to_owned();
        config.security.admin_web_port = 9443;
        assert_eq!(admin_url(&config), "https://homeserver.local:9443");
    }

    #[test]
    fn admin_url_sanitizes_invalid_hostname() {
        let mut config = SystemConfig::bootstrap();
        config.device.hostname = "Bad Host!".to_owned();
        assert_eq!(admin_url(&config), "https://homeserver.local:8443");
        config.device.hostname = String::new();
        assert_eq!(admin_url(&config), "https://homeserver.local:8443");
    }
}
