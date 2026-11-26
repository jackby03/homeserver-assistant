# Changelog

All notable changes to HomeServer Assistant OS are documented here. The format
is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- Project relicensed from MIT to **GPL-3.0-only**. Added `NOTICE`, third-party
  notices, code of conduct, security policy, support, roadmap, issue/PR
  templates and Dependabot.
- Agent personality is now maintained in `SOUL.md` and loaded from
  `data/SOUL.md`.
- Added persistent per-profile memory, absolute/relative reminders, RSS news,
  and opt-in Composio calendar/email capabilities with risk-aware confirmation.

### Security

- Tool calls carry an explicit `ToolRisk`; writes require an explicit user
  confirmation.
- Composio uses one encrypted project API key, per-toolkit auth configs and
  connected accounts; OAuth tokens never enter the appliance.
- External action endpoints are allow-listed and config-validated.

## [0.1.0] - 2026-01-15

First end-to-end appliance baseline: Rust backend, React UI, local vision,
infrastructure and ISO pipeline.

### Added

- **Core (`hs-core`)** — versioned `SystemConfig` with validation, redb embedded
  database, ChaCha20-Poly1305 secret vault, atomic config writes, audit trail,
  domain models and kiosk event types.
- **Authentication (`hs-auth`)** — Argon2id passwords, HS256 JWT sessions with
  server-side revocation, RBAC (`admin`/`operator`/`viewer`), login throttling,
  CSRF tokens and audit entries.
- **Host (`hs-system`)** — CPU/RAM/disk/temperature/battery/network/service
  metrics, power policy (lid switch + charge limit), PipeWire audio sink/volume
  control, camera and touchscreen detection.
- **Compute (`hs-runtime`)** — `RuntimeProvider` port with a native Incus
  adapter (LXC, OCI, QEMU/KVM, snapshots, limits, logs, exec) and an in-memory
  mock with demo instances for development.
- **Apps (`hs-apps`)** — catalog of eight real applications, LAN/DMZ deployment
  through the runtime, reverse-proxy route derivation and app records.
- **Network (`hs-network`)** — data-driven free-DNS catalog with **40 variants**
  (Cloudflare, Google, AdGuard, Quad9, CleanBrowsing, OpenDNS, NextDNS, Comodo,
  Control D incl. 13 community lists, DNS.WATCH and custom) with IPv4/IPv6,
  DoH/DoT/plain capabilities; AdGuard Home and Technitium config rendering,
  nftables ruleset with DMZ isolation and NAT egress, netplan bridge rendering,
  interface detection.
- **Provider diagnostics** — OpenCode Zen Go base/model normalization, required
  `x-opencode-session` routing header and a provider connectivity test endpoint
  with real error details in the portal.
- **Local-first assistant router** — deterministic Spanish intents (time, date, arithmetic, volume, music, lights/scenes, identity, help, jokes) answered without LLM/network, with profile tool permissions; unknown questions fall back to the configured LLM.\n- **Voice assistant** — always-on local wake word with customizable name,
  sensitivity and listening window; local STT helper (`voice/wakeword.py`),
  transcript-only wake endpoint, kiosk wake event and portal configuration.
- **Discovery (`hs-discovery`)** — mDNS, SSDP/UPnP and ARP sources with an
  80-entry OUI vendor table and device merging.
- **Home Assistant (`hs-homeassistant`)** — testable REST client (status,
  entities, services, config), HA container provisioner and a reconfigurable
  service handle.
- **Profiles (`hs-profiles`)** — family profiles, avatars, tool permissions,
  encrypted biometric enrollment and cosine face matching.
- **Agent (`hs-agent`, `hs-tools`)** — OpenAI/Anthropic/DeepSeek/opencode-go
  adapters, tool-calling loop, ten built-in tools with permission enforcement
  before any adapter call, and a node-based workflow engine.
- **Vision (`hs-vision`)** — pure-Rust motion/person tracker with V4L2 capture,
  normalized gaze events over a Unix socket, and an optional local InsightFace
  embedding helper.
- **Daemon (`hs-daemon`)** — admin REST/WS API, loopback kiosk API with token
  auth, first-boot wizard with live deployment progress, background metrics,
  vision bridge, CLI (`serve`, `doctor`, `reset-admin`).
- **UI (`ui`)** — React 19 + TypeScript + Tailwind SPA with admin portal,
  six-step onboarding wizard, animated SVG avatar kiosk, i18n (`es`/`en`) and
  typed API client.
- **Infrastructure (`infra`, `iso`)** — Incus preseed with LAN/DMZ bridges,
  nftables baseline, systemd units with hardening, Caddy internal TLS, service
  templates, Ubuntu 24.04 autoinstall and reproducible ISO/QEMU scripts.
- **Documentation** — architecture, security model, API reference, deployment,
  ISO build guide, ADRs and contributor rules.

### Known limitations

- Incus `exec` uses one-shot commands; a full PTY terminal is planned.
- V4L2 capture supports read(2)-style cameras; MMAP-only UVC devices are
  rejected with a clear error.
- Real nftables/netplan/ISO application is validated on Linux hosts; the
  development machine only exercises renderers and fakes.
- No OTA update channel yet; updates are applied through the package manager and
  documented backups.



