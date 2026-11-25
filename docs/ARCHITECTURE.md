# HomeServer Assistant OS — Architecture

> Status: living document. It describes the intended architecture of the
> appliance and the boundaries every crate must respect. See
> [`AGENTS.md`](../AGENTS.md) for the mandatory engineering rules.

## 1. Goals and constraints

HomeServer Assistant OS is a **local-first home server appliance** installed from
a reproducible ISO on a laptop mounted on a wall. It provides:

1. A management plane (admin portal) for compute, apps, DNS/firewall, Home
   Assistant, the AI agent and family profiles.
2. A wall kiosk (touch UI) with an animated avatar that follows people with a
   local camera, plays music through the home stereo and controls scenes.
3. Strong network isolation: a trusted `LAN` segment and an isolated `DMZ` for
   applications published through outbound tunnels only.
4. No mandatory cloud dependency: vision, face matching, DNS filtering and all
   state are local. LLM calls go only to explicitly configured providers.

**Non-goals (v1):** multi-tenant hosting, remote multi-user collaboration,
Windows/macOS server support, hot-plug HA clustering.

## 2. Four planes

```
┌──────────────────────────────────────────────────────────────────────────┐
│ 4. INTELLIGENCE: agent loop + tools + profiles + local vision            │
│    hs-agent ── hs-tools ── hs-profiles        hs-vision (Unix socket)    │
├──────────────────────────────────────────────────────────────────────────┤
│ 3. NETWORK / SECURITY / IOT: DNS (DoH/DoT), nftables, DMZ, discovery, HA │
│    hs-network ── hs-discovery ── hs-homeassistant                        │
├──────────────────────────────────────────────────────────────────────────┤
│ 2. COMPUTE: runtime-agnostic provider port, native Incus adapter         │
│    hs-runtime (trait) ── incus adapter (LXC / OCI / QEMU, snapshots)     │
│    hs-apps (catalog + deploy orchestration + proxy routes)               │
├──────────────────────────────────────────────────────────────────────────┤
│ 1. HOST: metrics, power policy, audio routing, camera/touch, persistence │
│    hs-system ── hs-auth ── hs-core (config, models, redb, secret vault)  │
└──────────────────────────────────────────────────────────────────────────┘
```

All planes are wired by **one privileged process**, `hs-daemon`, which exposes:

- the admin REST/WS API on `127.0.0.1:8080` (TLS terminated by Caddy on `:8443`);
- the kiosk UI + API on `127.0.0.1:3000` only;
- a Unix socket `/run/homecore/vision.sock` for tracking events.

The frontend (`ui/`) is a single React SPA with three route trees
(`/setup`, `/admin`, `/kiosk`) served from the daemon.

## 3. Crate map and dependency rules

```
hs-core  (leaf: config, models, redb, vault, events, validation, api errors)
 ├── hs-auth      Argon2id + HS256 JWT + sessions + RBAC + audit
 ├── hs-system    sysinfo metrics + sysfs/wpctl/systemd adapters
 ├── hs-runtime   RuntimeProvider port + Incus adapter + MockRuntime
 │    ├── hs-apps            catalog + deployment orchestration
 │    └── hs-homeassistant   HA REST client + provisioner
 ├── hs-network   DNS upstreams/config renderers + nftables + netplan
 ├── hs-discovery mDNS/SSDP/ARP discovery
 ├── hs-profiles  profiles, avatars, biometric matching
 ├── hs-tools     tool registry + permission enforcement + workflows
 │    └── hs-agent  LLM adapters + tool-calling loop
 ├── hs-vision    tracking service + event sink (binary + lib)
 └── hs-daemon    composition root: routers, adapters, background tasks
```

Rules:

- **`hs-core` is a leaf.** No domain crate may depend on another domain crate
  except along the arrows above.
- **Ports and adapters.** `hs-tools` defines small port traits
  (`RuntimeControl`, `DomoticsControl`, `AudioControl`, `SystemControl`,
  `WebhookClient`); `hs-daemon` implements them with real adapters. That keeps
  the security-critical permission check independent from infrastructure.
- **Composition root.** Only `hs-daemon` knows concrete adapters and their
  configuration. Domain crates accept injected trait objects.
- **Fail closed.** Missing providers, unavailable runtimes and unconfigured
  services return typed errors; no silent fallbacks on security paths.

## 4. Data flow

### 4.1 Admin request

```
Browser ── HTTPS :8443 ──> Caddy (internal CA, HSTS) ── HTTP 127.0.0.1:8080
   └─> axum router ──> AdminAuth extractor (cookie/bearer + CSRF) ──> RBAC check
        └─> domain service ──> redb / Incus / nftables / LLM provider
             └─> audit entry + ApiError on failure
```

### 4.2 Kiosk interaction

```
Camera ──> hs-vision (Rust, local) ── Unix socket JSON lines ──> daemon
   └─> KioskManager ──> broadcast KioskEvent ── WS :3000 ──> Avatar UI
Microphone ──> browser VAD ──> POST /kiosk/agent/query ──> agent loop
   └─> tools (permission-checked) ──> Home Assistant / music / runtime
```

### 4.3 Agent tool call

```
POST /agent/chat {profile_id, messages}
  └─> AgentService: load profile + enabled provider (API key from vault)
       └─> LLM (OpenAI-compatible or Anthropic)
            └─> tool_calls? ──> ToolRegistry::invoke(profile, tool, args)
                 ├─ profile.may_use_tool?  (denied => trace.allowed=false)
                 ├─ port call (runtime / HA / audio / webhook)
                 └─ result appended as tool message
            └─> final answer + tool trace + token usage
```

## 5. Persistence

Data directory (default `/var/lib/homecore`, `0600`/`0700`):

| File | Content |
|---|---|
| `config.json` | `SystemConfig`: device, network/DNS/DMZ, audio, security, agent, kiosk, services |
| `state.redb` | users, sessions, profiles, providers, workflows, apps, audit, secrets |
| `master.key` | 32-byte ChaCha20-Poly1305 key (created on first boot) |
| `backups/` | configuration backups created before destructive changes |

- **Config** is validated on every read/write and written atomically
  (temp file + rename, `0600`).
- **Secrets** (LLM API keys, Home Assistant token, tunnel tokens, biometric
  embeddings) are encrypted with ChaCha20-Poly1305 and never serialized in
  plaintext.
- **Audit** entries are append-only with a monotonic sequence number.
- **Sessions** are server-side records: revoking a session immediately invalidates
  the JWT for state-changing requests (JWT alone would not).

## 6. Networking and security boundaries

```
Internet ──(outbound tunnel: Cloudflare/WireGuard)──> Caddy ──> DMZ apps
LAN br-lan (trusted) ──> admin :8443, DNS :53, HA :8123
DMZ br-dmz 10.20.30.0/24 ──X──> LAN                 (nftables drop)
DMZ                       ──X──> host admin port    (nftables drop)
DMZ                       ──> Internet (NAT)        (explicit allow)
```

- The host has one physical uplink (`network.interface`, usually `eth0`) and two
  bridges: `br-lan` (trusted workloads and LAN clients) and `br-dmz` (isolated
  workloads). Both names are part of `NetworkConfig` and are validated to differ.
- The DMZ bridge has no route to the LAN; the host forwards only what the
  explicit ruleset allows. DMZ workloads **may** reach the internet through
  masquerading on the uplink (needed for updates and for the outbound tunnel
  client), but they can never initiate traffic to the LAN, the host admin port or
  other host services.
- Apps deployed to `DMZ` get no LAN reverse-proxy hostname. External exposure is
  implemented by the outbound tunnel, never by opening router ports.
- TLS for the admin portal is terminated by Caddy with a local CA. The daemon
  binds loopback only; this avoids hand-rolling TLS and gives a single place for
  certificate rotation and security headers. See `docs/adr/0003`.

## 7. Frontend architecture

- **One SPA, three experiences.** React 19 + TypeScript strict + Tailwind.
- **Typed API client** mirrors the Rust serde models; errors use the shared
  `{error:{code,message,details}}` envelope.
- **Realtime:** the admin dashboard consumes `/ws/events`; the kiosk consumes
  `/kiosk/ws`; both reconnect with exponential backoff and refetch snapshots.
- **Avatar engine:** pure SVG + CSS. Shapes (`circle`, `squircle`, `cloud`,
  `cone`, `droplet`), capsule eyes with configurable color/tilt, and states
  driven by `KioskEvent` messages. No canvas/WebGL requirement, so it runs on
  low-power wall laptops.
- **i18n:** `es` (default) and `en`; no external CDNs, fonts are bundled for
  offline operation.

## 8. Deployment topology

- Ubuntu 24.04 LTS autoinstall ISO (`iso/`), systemd units (`infra/systemd`).
- Netplan creates `br-lan` (uplink `eth0` and LAN clients) and `br-dmz`
  (10.20.30.1/24, no default route). Incus preseed reuses those bridges and
  defines `lan`/`dmz` profiles, a project and the storage pool.
- Caddy serves the admin portal with internal TLS; Cloudflare Tunnel or
  WireGuard provides outbound-only publication from the DMZ.
- The daemon writes one Caddy site snippet per LAN app
  (`/etc/caddy/apps.d/<id>.caddy`, `tls internal`) and reloads Caddy, and keeps
  `/etc/homecore/caddy.env` in sync with the device hostname, base domain and
  admin port. Caddy is the only process listening on the LAN.
- Kiosk: `cage` (Wayland) + Chromium `--kiosk` pointing at
  `http://127.0.0.1:3000/kiosk`; the daemon sets the kiosk cookie.
- Power policy: lid switch ignored, battery charge limit 60–80 %, BIOS
  power-on-AC for 24/7 operation.

## 9. Failure modes

| Failure | Behavior |
|---|---|
| Incus unavailable | Runtime endpoints return `503`; daemon stays up; dashboard shows service down |
| Vision service down | Kiosk remains usable (idle avatar); daemon logs and retries the socket |
| LLM provider error | Agent returns typed error; no tool is executed |
| DNS upstream unreachable | AdGuard keeps serving cached/blocked answers; network page shows error |
| Power loss | redb is crash-safe; config writes are atomic; Incus snapshots are the backup unit |
| Lost admin password | `hs-daemon reset-admin` (root, local console) revokes sessions and sets a new password |

## 10. Decisions

Architecture decisions are recorded in [`docs/adr/`](adr/):

- `0001-rust-workspace.md` — one Rust workspace with ports/adapters.
- `0002-redb-encrypted-secrets.md` — embedded DB + AEAD vault.
- `0003-tls-termination-at-caddy.md` — Caddy terminates TLS, daemon is loopback.
- `0004-incus-native-runtime.md` — Incus REST over its Unix socket.
