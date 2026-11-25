# Security Model — HomeServer Assistant OS

This document describes the threat model, the security controls implemented in
the repository and the operational checklist for the appliance. Security issues
should be reported privately to the maintainers.

## 1. Assets

| Asset | Where it lives | Protection |
|---|---|---|
| Admin credentials | `state.redb` (Argon2id PHC) | memory-hard hashing, login throttling |
| Session tokens | HttpOnly `hs_session` cookie / Bearer | HS256 JWT + server-side session record |
| LLM API keys, HA token, tunnel tokens | encrypted secret store | ChaCha20-Poly1305, key in `master.key` (0600) |
| Face embeddings (biometrics) | secret store | encrypted at rest, never leaves the appliance |
| Camera frames | daemon memory only | never written to disk, never sent off-device |
| Runtime workloads | Incus (LXC/VM) | unprivileged containers, LAN/DMZ separation |
| Home network | nftables | DMZ isolation, no inbound port forwards |

## 2. Trust boundaries

```
[ Browser (LAN) ] --TLS--> [ Caddy ] --loopback--> [ hs-daemon (root) ]
[ Kiosk UI (loopback) ] --cookie--> [ kiosk listener 127.0.0.1:3000 ]
[ Vision service (unprivileged) ] --unix socket--> [ hs-daemon ]
[ DMZ workloads ] --X--> [ LAN ]  (denied)
[ Internet ] --outbound tunnel--> [ Caddy ] --> [ DMZ apps ]
[ hs-daemon ] --HTTPS--> [ LLM / DNS upstreams ]  (only configured endpoints)
```

- The **daemon is the only privileged component**. It holds the Incus socket,
  nftables, sysfs and the secret vault. Kiosk and vision run unprivileged.
- The **admin API is loopback-only**; only Caddy can reach it. Caddy is the only
  process exposing TLS on the LAN.
- The **kiosk listener is loopback-only** and requires a per-boot token set as a
  HttpOnly cookie by the kiosk page handler. The token is not exposed to the
  network.

## 3. Authentication

- **Passwords:** Argon2id, m=19 MiB, t=2, p=1 (OWASP), random salt per password,
  PHC strings stored verbatim so parameters can be upgraded.
- **Password policy:** minimum 12 characters and at least three of
  lowercase/uppercase/digits/symbols.
- **Login throttling:** 5 failures per username+IP in a sliding 5-minute window;
  exceeded attempts return `429` and are audited. Credentials failures never
  reveal whether the username exists.
- **Sessions:** 256-bit random HS256 secret per install; JWT contains `sub`,
  `sid`, `iat`, `exp`, `iss`; the `sid` maps to a server-side `SessionRecord`.
  Revocation is immediate server-side. JWT TTL defaults to 12 hours.
- **Cookies:** `HttpOnly`, `SameSite=Strict`, `Secure` when served through the
  TLS terminator, `Path=/`.
- **CSRF:** cookie-authenticated state-changing requests must send
  `X-HS-CSRF` matching the token derived from the session id. Bearer clients are
  exempt. Login itself requires no CSRF token but sets one.
- **RBAC:** `admin` (all), `operator` (operations, no network/users), `viewer`
  (read-only). Permissions are checked in the route layer and re-checked in
  services. See `hs-auth::rbac`.
- **First boot:** only `/api/v1/setup/*` routes are available while
  `initialized == false`; the first admin creation is refused if any user exists.

## 4. Secrets at rest

- A 256-bit master key is generated on first boot and stored at
  `<data_dir>/master.key` with mode `0600`.
- Values are encrypted with ChaCha20-Poly1305 using a fresh 96-bit nonce per
  encryption; the database stores `{version, nonce, ciphertext}` in base64.
- The API never returns secret values, only names and a configured/not
  configured flag.
- Logs never include secrets or full bodies; errors are typed and sanitized.

## 5. Network isolation (DMZ)

Invariants enforced by the nftables renderer (`hs-network::firewall`) and
covered by golden tests:

1. `default drop` on the forward chain.
2. Established/related traffic accepted first.
3. `LAN -> DMZ` allowed only if explicitly enabled.
4. `DMZ -> LAN` **dropped** (logged with prefix `dmz-to-lan`).
5. `DMZ -> host` **dropped**, including the admin port, except DNS/53 and
   DHCP if enabled.
6. `DMZ -> Internet` is explicitly allowed and masqueraded when DMZ egress is
   enabled, so workloads can update and the outbound tunnel client can connect;
   this never relaxes rules 4 and 5 (ordering is golden-tested).
7. The ruleset is validated with `nft -c -f` before being applied; the daemon
   never applies a partially rendered ruleset.

Apps deployed to the DMZ do not receive a LAN reverse-proxy hostname. External
publication happens only through outbound tunnels (Cloudflare Tunnel/WireGuard)
with no inbound port forwarding on the home router.

## 6. TLS

TLS for the admin portal is terminated by **Caddy** with an internal CA. The
daemon listens on loopback plain HTTP. Rationale:

- one place for certificate generation, rotation, HSTS and security headers;
- no hand-rolled TLS stack in the application;
- the daemon's attack surface is not reachable from the LAN directly.

Caddy config sets HSTS, `X-Content-Type-Options`, `X-Frame-Options: DENY` for the
portal, and a strict CSP. Clients must trust the local CA once (documented in
`docs/DEPLOYMENT.md`).

## 7. Local vision and biometrics

- Frames are captured by `hs-vision` on the appliance and reduced to normalized
  `(x, y)` coordinates before leaving the process. Frames are never persisted.
- Face embeddings are extracted locally by an optional Python helper
  (InsightFace) and stored **encrypted** in the secret store. Matching happens in
  Rust (`hs-profiles`), with cosine similarity and a configurable threshold.
- A profile without enrolled biometrics can always be selected manually.
- Killing `hs-vision` degrades the kiosk to manual mode; no other feature depends
  on the camera.

## 7.1 Always-on voice assistant

- Microphone audio is processed **only on the appliance**. The optional
  `voice/wakeword.py` helper runs local speech-to-text (faster-whisper) and sends
  only the transcript to the loopback kiosk API; audio buffers are discarded
  after transcription and transcripts are not logged or persisted.
- The wake-word decision runs in Rust (`hs-core::voice` + `hs-daemon::voice`):
  the configured assistant name is normalized (case/accent-insensitive), matched
  token-wise with optional typo tolerance, and gated by a cooldown and a bounded
  listening window.
- Renaming the assistant only changes configuration; no model is trained and no
  cloud service is contacted. The kiosk token authenticates the helper, and the
  kiosk listener is loopback-only.
- The helper runs unprivileged (audio group); disabling `wake_word_enabled` in
  the portal stops activation and leaves push-to-talk/agent text as the only
  paths.

## 8. Agent and LLM data flow

- The API key is read from the vault only when calling the provider and dropped
  right after the request.
- The prompt contains the profile name, tone/language instructions, the
  conversation and the tool definitions **filtered by the profile's
  `allowed_tools`**. Tool results are appended only after execution.
- No tool can run without passing `ToolRegistry::invoke`, which checks
  `Profile::may_use_tool` before touching any port. Denials are traced and
  audited; the model receives an error tool message instead of retrying around
  the permission system.
- There is no "auto-approve" switch. Workflows enforce the same check per node.

## 9. Kiosk security

- Listener bound to `127.0.0.1:3000`; not proxied by Caddy.
- Kiosk API requires a per-install token (32 bytes, hex) delivered as an
  HttpOnly cookie by the page handler; third-party pages cannot read it.
- `SameSite=Strict` cookies plus loopback binding prevent remote CSRF.
- The kiosk cannot manage users, network/firewall or destructive runtime
  operations; its API surface is limited to avatar/profile activation, music,
  scenes and agent queries (subject to the active profile's tool permissions).

## 10. Audit

Every authentication event, RBAC denial, instance/app mutation, network change,
profile change, agent provider change and setup step is appended to the audit
table with actor, action, target, outcome, IP and timestamp. The UI exposes the
last N entries; entries are never edited or deleted through the API.

## 11. Hardening checklist (deployment)

- [ ] BIOS: disable USB boot after install, enable power-on-AC, set supervisor
      password.
- [ ] Full-disk encryption at install if the device is physically accessible.
- [ ] Change/verify the admin password and store it in a password manager.
- [ ] Trust the Caddy local CA only on devices that need the portal.
- [ ] Keep the daemon on loopback; never expose `:8080` or `:3000` on the LAN.
- [ ] Enable the firewall ruleset and verify DMZ isolation from a test container
      (`nft list ruleset`, `curl` from DMZ to LAN must time out).
- [ ] Configure the outbound tunnel; do not add router port forwards.
- [ ] Restrict LLM provider API keys to the minimum scope/usage budget.
- [ ] Back up `config.json`, `state.redb`, `master.key` (offline, encrypted) and
      Incus snapshots. Losing `master.key` makes stored secrets unrecoverable.
- [ ] Review the audit trail periodically.
- [ ] Keep the base OS, Incus, Caddy and containers updated; pin image tags.

## 12. Incident response

- **Lost password:** on the appliance console run
  `sudo hs-daemon reset-admin --username admin` (requires root; revokes all
  sessions and prints a one-time password to change at next login).
- **Suspected secret leak:** rotate the provider key at the provider, update it
  in the portal, then `systemctl restart hs-daemon`; audit entries show usage
  timestamps. Rotate `master.key` only by re-entering all secrets.
- **Compromised DMZ app:** stop/delete it in the portal; the DMZ has no path to
  the LAN by design. Review `nft list ruleset` and the audit log.
- **Stolen device:** full-disk encryption and BIOS password are the only
  protections for the data at rest; rotate all credentials.
