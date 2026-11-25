//! Session cookie helpers.
//!
//! Cookies are built here instead of using a framework-specific jar so the
//! attributes are explicit and testable. TLS is terminated by Caddy, so the
//! `Secure` flag is enabled when the request arrived through the trusted
//! `X-Forwarded-Proto: https` proxy header (only Caddy on loopback sets it).
//!
//! Session cookies intentionally have no `Max-Age`: they live for the browser
//! session, while the authoritative expiry and revocation are server-side
//! (`SessionRecord`).

use axum::http::{HeaderMap, HeaderValue};
use axum_extra::extract::cookie::{Cookie, SameSite};

/// Session cookie name (must match `AuthConfig`).
pub const SESSION_COOKIE: &str = "hs_session";
/// CSRF cookie name (readable by the SPA).
pub const CSRF_COOKIE: &str = "hs_csrf";
/// Kiosk cookie name (loopback only).
pub const KIOSK_COOKIE: &str = "hs_kiosk";

/// Whether the original client request used HTTPS (through Caddy).
#[must_use]
pub fn is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|proto| proto.eq_ignore_ascii_case("https"))
}

/// Build the session cookie.
#[must_use]
pub fn session_cookie(token: &str, secure: bool) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token.to_owned()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(secure)
        .build()
}

/// Build the CSRF cookie (readable by JavaScript, so the SPA can echo it in
/// the `X-HS-CSRF` header).
#[must_use]
pub fn csrf_cookie(token: &str, secure: bool) -> Cookie<'static> {
    Cookie::build((CSRF_COOKIE, token.to_owned()))
        .path("/")
        .http_only(false)
        .same_site(SameSite::Strict)
        .secure(secure)
        .build()
}

/// Cookie used by the loopback kiosk listener.
#[must_use]
pub fn kiosk_cookie(token: &str) -> Cookie<'static> {
    Cookie::build((KIOSK_COOKIE, token.to_owned()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .build()
}

/// Clear a cookie by setting an empty value that expires immediately.
#[must_use]
pub fn cleared_cookie(name: &'static str, secure: bool) -> HeaderValue {
    let mut value = format!("{name}=; Path=/; SameSite=Strict; Max-Age=0");
    if matches!(name, SESSION_COOKIE | KIOSK_COOKIE) {
        value.push_str("; HttpOnly");
    }
    if secure {
        value.push_str("; Secure");
    }
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Build a `Set-Cookie` header value.
#[must_use]
pub fn header_value(cookie: Cookie<'static>) -> HeaderValue {
    HeaderValue::from_str(&cookie.to_string()).unwrap_or_else(|_| HeaderValue::from_static(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_cookie_is_hardened() {
        let rendered = session_cookie("token", true).to_string();
        assert!(rendered.contains("HttpOnly"));
        assert!(rendered.contains("SameSite=Strict"));
        assert!(rendered.contains("Secure"));
        assert!(rendered.contains("Path=/"));
    }

    #[test]
    fn csrf_cookie_is_readable() {
        let rendered = csrf_cookie("csrf", false).to_string();
        assert!(!rendered.contains("HttpOnly"));
        assert!(rendered.contains("SameSite=Strict"));
    }

    #[test]
    fn detects_https_proxy_header() {
        let mut headers = HeaderMap::new();
        assert!(!is_https(&headers));
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert!(is_https(&headers));
    }
}
