//! Input validation helpers.
//!
//! Validation is deliberately centralized in the core crate so that every
//! entry point (HTTP handlers, onboarding wizard, CLI, tests) applies the exact
//! same rules. Functions return [`CoreError::Validation`] with the offending
//! field name.

use std::net::IpAddr;

use crate::error::{CoreError, CoreResult};

/// Usernames are lowercase-friendly identifiers used to log into the admin
/// portal. They are normalized to lowercase by the auth service.
pub fn username(value: &str) -> CoreResult<()> {
    let value = value.trim();
    if value.len() < 3 || value.len() > 32 {
        return Err(CoreError::validation(
            "username",
            "must be between 3 and 32 characters",
        ));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(CoreError::validation(
            "username",
            "only ASCII letters, digits, dot, underscore and hyphen are allowed",
        ));
    }
    if value.starts_with('.') || value.starts_with('-') || value.starts_with('_') {
        return Err(CoreError::validation(
            "username",
            "must start with an alphanumeric character",
        ));
    }
    Ok(())
}

/// Administrative passwords must be at least 12 characters and contain at
/// least three of the four character classes.
pub fn password(value: &str) -> CoreResult<()> {
    if value.chars().count() < 12 {
        return Err(CoreError::validation(
            "password",
            "must be at least 12 characters long",
        ));
    }
    if value.chars().count() > 256 {
        return Err(CoreError::validation(
            "password",
            "must be at most 256 characters long",
        ));
    }
    let classes = [
        value.chars().any(|c| c.is_ascii_lowercase()),
        value.chars().any(|c| c.is_ascii_uppercase()),
        value.chars().any(|c| c.is_ascii_digit()),
        value.chars().any(|c| !c.is_ascii_alphanumeric()),
    ]
    .iter()
    .filter(|present| **present)
    .count();
    if classes < 3 {
        return Err(CoreError::validation(
            "password",
            "must combine at least three of: lowercase, uppercase, digits, symbols",
        ));
    }
    Ok(())
}

/// Device display name shown in the kiosk and portal.
pub fn device_name(value: &str) -> CoreResult<()> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 48 {
        return Err(CoreError::validation(
            "device_name",
            "must be between 1 and 48 characters",
        ));
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(CoreError::validation(
            "device_name",
            "must not contain control characters",
        ));
    }
    Ok(())
}

/// RFC 1123 hostname: lowercase letters, digits and hyphens, no leading or
/// trailing hyphen, maximum 63 characters per label.
pub fn hostname(value: &str) -> CoreResult<()> {
    if value.is_empty() || value.len() > 63 {
        return Err(CoreError::validation(
            "hostname",
            "must be between 1 and 63 characters",
        ));
    }
    if value.starts_with('-') || value.ends_with('-') {
        return Err(CoreError::validation(
            "hostname",
            "must not start or end with a hyphen",
        ));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(CoreError::validation(
            "hostname",
            "only lowercase ASCII letters, digits and hyphen are allowed",
        ));
    }
    Ok(())
}

/// A DNS domain made of dot-separated labels (e.g. `home.local`).
pub fn domain(value: &str) -> CoreResult<()> {
    if value.is_empty() || value.len() > 253 {
        return Err(CoreError::validation(
            "domain",
            "must be between 1 and 253 characters",
        ));
    }
    for label in value.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(CoreError::validation(
                "domain",
                "labels must contain between 1 and 63 characters",
            ));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(CoreError::validation(
                "domain",
                "labels must not start or end with a hyphen",
            ));
        }
        if !label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(CoreError::validation(
                "domain",
                "only lowercase ASCII letters, digits, dots and hyphen are allowed",
            ));
        }
    }
    Ok(())
}

/// IANA timezone identifier (e.g. `America/Lima`). A full tzdata lookup is not
/// performed here; the value is validated structurally and applied on Linux.
pub fn timezone(value: &str) -> CoreResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
        && !value.starts_with('/')
        && !value.ends_with('/');
    if valid {
        Ok(())
    } else {
        Err(CoreError::validation(
            "timezone",
            "must be an IANA timezone identifier such as Europe/Madrid",
        ))
    }
}

/// Supported UI languages.
pub fn language(value: &str) -> CoreResult<()> {
    match value {
        "es" | "en" => Ok(()),
        _ => Err(CoreError::validation(
            "language",
            "supported languages are `es` and `en`",
        )),
    }
}

/// Subdomain labels used by the reverse proxy.
pub fn subdomain(value: &str) -> CoreResult<()> {
    hostname(value)
}

/// A port that is bindable and meaningful (1..=65535).
pub fn port(value: u16) -> CoreResult<()> {
    if value == 0 {
        return Err(CoreError::validation("port", "must be between 1 and 65535"));
    }
    Ok(())
}

/// Secret names are lower-case dotted identifiers (`llm.openai.api_key`).
pub fn secret_name(value: &str) -> CoreResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 96
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(CoreError::validation(
            "name",
            "must contain only lowercase letters, digits, dot, underscore or hyphen",
        ))
    }
}

/// IPv4 address in dotted notation.
pub fn ipv4(value: &str) -> CoreResult<()> {
    match value.parse::<IpAddr>() {
        Ok(IpAddr::V4(_)) => Ok(()),
        _ => Err(CoreError::validation(
            "ipv4",
            "must be a valid IPv4 address",
        )),
    }
}

/// IPv4 CIDR network such as `10.20.30.0/24`.
pub fn ipv4_cidr(value: &str) -> CoreResult<()> {
    let Some((addr, prefix)) = value.split_once('/') else {
        return Err(CoreError::validation(
            "cidr",
            "must use CIDR notation such as 10.20.30.0/24",
        ));
    };
    let Ok(prefix) = prefix.parse::<u8>() else {
        return Err(CoreError::validation(
            "cidr",
            "prefix length must be a number between 0 and 32",
        ));
    };
    if prefix > 32 {
        return Err(CoreError::validation(
            "cidr",
            "prefix length must be between 0 and 32",
        ));
    }
    ipv4(addr)
}

/// Runtime instance names (Incus/LXC): lowercase letter followed by lowercase
/// letters, digits or hyphens, at most 63 characters.
pub fn instance_name(value: &str) -> CoreResult<()> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(CoreError::validation("name", "must not be empty"));
    };
    let valid = first.is_ascii_lowercase()
        && value.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if valid {
        Ok(())
    } else {
        Err(CoreError::validation(
            "name",
            "must start with a lowercase letter and contain only lowercase letters, digits or hyphens",
        ))
    }
}

/// An HTTP(S) URL using a host, and optionally a port and path.
pub fn http_url(value: &str) -> CoreResult<()> {
    let valid = (value.starts_with("https://") || value.starts_with("http://"))
        && value.len() > 8
        && !value.contains(char::is_whitespace);
    if valid {
        Ok(())
    } else {
        Err(CoreError::validation(
            "url",
            "must be an absolute http(s) URL",
        ))
    }
}

/// Tool identifiers are dotted lower-case names (`homeassistant.call_service`).
pub fn tool_name(value: &str) -> CoreResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_'));
    if valid {
        Ok(())
    } else {
        Err(CoreError::validation(
            "tool",
            "must contain only lowercase letters, digits, dot or underscore",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_usernames() {
        for name in ["admin", "maria.gomez", "home-server", "j_doe"] {
            assert!(username(name).is_ok(), "{name} should be valid");
        }
    }

    #[test]
    fn rejects_invalid_usernames() {
        for name in ["ab", "has space", "-leading", "üser", "a@b"] {
            assert!(username(name).is_err(), "{name} should be invalid");
        }
    }

    #[test]
    fn password_strength_rules() {
        assert!(password("Sup3rSecret!!").is_ok());
        assert!(password("short").is_err());
        assert!(password("alllowercaseonly").is_err());
        assert!(password("lowerUPPER1234").is_ok());
    }

    #[test]
    fn hostname_rules() {
        assert!(hostname("homeserver").is_ok());
        assert!(hostname("home-server-01").is_ok());
        assert!(hostname("-bad").is_err());
        assert!(hostname("Bad").is_err());
    }

    #[test]
    fn cidr_rules() {
        assert!(ipv4_cidr("10.20.30.0/24").is_ok());
        assert!(ipv4_cidr("10.20.30.0/33").is_err());
        assert!(ipv4_cidr("10.20.30.0").is_err());
        assert!(ipv4_cidr("not-an-ip/24").is_err());
    }

    #[test]
    fn timezone_rules() {
        assert!(timezone("America/Lima").is_ok());
        assert!(timezone("UTC").is_ok());
        assert!(timezone("bad timezone").is_err());
    }

    #[test]
    fn secret_name_rules() {
        assert!(secret_name("llm.openai.api_key").is_ok());
        assert!(secret_name("HA_TOKEN").is_err());
        assert!(secret_name("").is_err());
    }
}
