# AGENTS.md — Engineering Rules for HomeServer Assistant OS

This repository is the home-server appliance "HomeServer Assistant OS". It is a
Rust workspace (`crates/`), a React/TypeScript UI (`ui/`), a local vision
service (`crates/hs-vision` + optional Python helper in `vision/python/`), and
reproducible deployment assets (`infra/`, `iso/`, `docs/`).

These rules are **mandatory** for every human and AI contributor.

---

## 1. Golden rules

1. **English only in source code.** Identifiers, comments, doc comments, log
   messages, error messages and commit messages are written in English.
   User-facing UI copy is localized (default `es`, `en` available); the source
   keys are English.
2. **Never log or commit secrets.** API keys, tunnel tokens, passwords,
   biometric embeddings and private keys are stored encrypted through
   `hs_core::state::SecretVault`. `.env` files, `*.key`, `*.pem`, databases and
   build artifacts are git-ignored.
3. **No `unwrap()` / `expect()` / `panic!()` on request, config or I/O paths.**
   Use `thiserror` error enums and propagate. `expect()` is acceptable only in
   tests and in `main` for unrecoverable startup configuration.
4. **Local-first.** Vision, face matching, DNS filtering and automation must run
   on the appliance. Outbound network calls are limited to explicitly configured
   providers (LLM, DNS upstreams, tunnels).
5. **Least privilege.** The privileged daemon runs as root only where required
   (Incus, nftables, sysfs). The kiosk and the vision services run as
   unprivileged users. Every mutating API requires authentication + RBAC.

## 2. Architecture boundaries

| Crate | Responsibility | May depend on |
|---|---|---|
| `hs-core` | Config schema, domain models, storage (redb), secret vault, shared events | std crates only |
| `hs-auth` | Argon2id, JWT HS256, sessions, RBAC, CSRF, audit | `hs-core` |
| `hs-system` | Metrics, power policy, audio (`wpctl`), camera/touch detection | `hs-core` |
| `hs-runtime` | `RuntimeProvider` port + Incus adapter + mock adapter | `hs-core` |
| `hs-apps` | App catalog, deployment orchestration, proxy routes | `hs-core`, `hs-runtime` |
| `hs-network` | DNS/DoH/DoT profiles, nftables rendering, DMZ validation, netplan | `hs-core` |
| `hs-discovery` | mDNS / SSDP / ARP discovery ports and adapters | `hs-core` |
| `hs-homeassistant` | Home Assistant REST client, provisioning | `hs-core`, `hs-runtime` |
| `hs-agent` | LLM provider adapters, agent loop (tool calling) | `hs-core`, `hs-tools`, `hs-profiles` |
| `hs-tools` | Tool registry, permission enforcement, workflow engine | `hs-core`, `hs-profiles` (ports only) |
| `hs-profiles` | Family profiles, avatars, local biometric matching | `hs-core` |
| `hs-vision` | Local camera tracking service + event bridge | `hs-core` |
| `hs-daemon` | HTTP/WS composition root; wires all adapters | everything |

Rules:

- **Dependency inversion:** `hs-tools` must not depend on runtime/HA/audio
  crates directly. It defines small port traits; `hs-daemon` provides adapters.
- **No circular dependencies.** `hs-core` is a leaf.
- **Composition root:** only `hs-daemon` knows concrete adapters. Domain crates
  expose constructors that accept trait objects.
- **Errors:** each crate has one `thiserror` error enum. Convert at boundaries;
  never expose internal types raw from HTTP handlers (use `hs-core::api::ApiError`).

## 3. Security invariants (do not regress)

1. Admin portal routes are authenticated unless explicitly listed as public in
   `hs-daemon/src/routes/public.rs`. Setup routes exist **only** while
   `SystemConfig::initialized == false`.
2. Password hashing is Argon2id with parameters from `hs-auth::password`.
3. Session cookies are `HttpOnly`, `SameSite=Strict`, `Secure` when TLS is
   terminated upstream, and expire server-side.
4. State-changing cookie-authenticated requests require the CSRF header and
   cookie to match. Bearer-token clients are exempt.
5. Every tool call is checked against the active profile's `allowed_tools`
   **before** execution. There is no bypass flag.
6. DMZ isolation: containers on the `dmz` bridge must not reach the `lan`
   bridge, the host admin port, or host services other than the explicit allow
   list. `hs_network::firewall` has golden tests for these rules; changes to the
   rendered ruleset require updating tests.
7. The kiosk API is reachable only from loopback and requires the kiosk token.
8. Hashes for comparison use constant-time comparison (`subtle`).

## 4. Code style

- `cargo fmt --all` — formatting is not optional.
- `cargo clippy --workspace --all-targets -- -D warnings` must pass. If a lint
  is impossible to satisfy, use `#[allow(...)]` **with a reason comment**.
- Public items have doc comments (`///`). Modules have `//!` headers.
- Functions do one thing; handlers stay thin (validate → call service → map
  response). Business logic belongs in domain crates, not in route modules.
- Prefer strong newtypes (`ProfileId`, `InstanceName`) over bare strings when a
  value has validation rules.
- JSON field names are `snake_case`; IDs are UUID strings; timestamps are RFC
  3339 UTC (`chrono::DateTime<Utc>`).
- Logs are structured (`tracing`): `tracing::info!(instance = %name, "instance started")`.
  Never log full request bodies or secrets.
- Console/startup output is English.

## 5. Testing

- Every crate has unit tests. `hs-core`, `hs-auth`, `hs-network` and `hs-tools`
  require meaningful coverage of security-critical logic.
- Integration tests live in `crates/hs-daemon/tests/` and exercise the router
  through `tower::ServiceExt::oneshot` with an in-memory/test data directory.
- Do not mock what you can model: prefer fakes (`FakeRuntime`, `FakeClock`) over
  fragile process mocks.
- Run `cargo test --workspace --offline` before handing work back.
- Tests must not require root, systemd, Incus, network access or a camera.

## 6. Working agreements

- Keep each change focused; do not edit unrelated files just to reformat them.
- Do not change workspace dependencies or another crate's manifest without a
  maintainer decision in the issue/PR.
- Prefer reproducible offline builds when possible; document any first-time
  fetch or environment setup in the PR.
- Keep the build green. Never leave the workspace in a non-compiling state.
- Report what changed, how it was verified (`cargo test` output summary) and
  any known gaps. Be precise.

## 7. Definition of done (per feature)

- Code compiles with `-D warnings`.
- Unit/integration tests added and passing.
- Public API documented in `docs/API.md` (route) or rustdoc (library).
- Security implications considered and written down (see `docs/SECURITY.md`).
- No secrets, absolute developer paths or machine-specific data in the diff.
