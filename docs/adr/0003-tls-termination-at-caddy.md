# ADR 0003 — TLS termination at Caddy, daemon bound to loopback

- **Status:** accepted
- **Date:** 2026-01-15

## Context

The admin portal must be served over HTTPS on the LAN (`https://<ip>:8443`) with
authentication, while the daemon itself should not implement a TLS stack. The
appliance also needs a reverse proxy for deployed apps and an internal CA that a
family can trust once.

## Decision

- `hs-daemon` binds **only to loopback**: admin API `127.0.0.1:8080`, kiosk
  `127.0.0.1:3000`.
- **Caddy** terminates TLS on `:8443` with its internal CA, serves security
  headers, and proxies to the daemon. App subdomains are additional Caddy sites
  generated from `AppRecord`s.
- The kiosk is not proxied; Chromium on the appliance loads
  `http://127.0.0.1:3000/kiosk` and receives an HttpOnly kiosk cookie from the
  daemon.
- Cookies are marked `Secure` when the request arrives through Caddy (detected
  via a trusted `X-Forwarded-Proto` header from loopback).

## Consequences

- One process owns certificates and HTTP security headers; the daemon's attack
  surface is not directly reachable from the LAN.
- No native TLS / certificate-generation dependencies in the Rust build.
- Deployments must install the local CA on client devices (documented).
- If Caddy is down, the portal is unavailable but the kiosk and local services
  keep working.

## Alternatives considered

- **TLS inside the daemon (rustls/openssl):** rejected; more code, certificate
  lifecycle duplicated, and native dependencies in the ISO.
- **Plain HTTP on the LAN:** rejected; credentials and sessions would be
  exposed.
