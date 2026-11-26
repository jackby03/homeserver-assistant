# Roadmap

This is a living roadmap, not a promise. Priorities are security, local-first
behavior and reproducibility; features are listed only when they have a clear
owner/implementation path. See `README.md` for the current status table.

## Current baseline (0.1.x)

- Rust workspace with admin API, kiosk API and typed React UI.
- Incus provider with a development mock; DNS/DoH/DoT and nftables/DMZ
  renderers with golden tests.
- Home Assistant, profiles, local vision, wake word and local STT/TTS helpers.
- Agent with per-profile tool permissions, risk-aware confirmation, persistent
  memory, absolute/relative reminders, RSS news and SOUL.md personality.
- Opt-in Composio gateway: auth configs, connected accounts, discovery and
  allow-listed calendar/email actions.

## Next

### 1. Usable external integrations

- OAuth connection UI for Composio (open redirect URL, poll connection status,
  show connected accounts per toolkit).
- Per-profile connected accounts and minimal Google scopes.
- Move execution from direct `/tools/execute` to the Composio Tool Router
  session flow when the upstream API stabilizes.
- Audit rows for every external action and rate limits/kill switch in the UI.
- Direct CalDAV/IMAP alternative for users who do not want a cloud gateway.

### 2. Firewall and network management

- Visual rule editor and dry-run preview on top of `hs-network::firewall`.
- Port forwarding with explicit warning and DMZ presets.
- WireGuard, guest VLAN isolation, reverse proxy/ACME and DHCP/DNS UI.
- IPv6 policy for LAN/DMZ.

### 3. Appliance operations

- Backup/restore with verification and off-appliance targets.
- Signed OTA update channel; rollback.
- SMART, RAID and snapshot automation; storage usage warnings.
- Monitoring, alerting and notification channels.

### 4. Home automation and media

- Matter/Thread/Zigbee onboarding (via Home Assistant and local bridges).
- Media library/photos/files, local playback and AirPlay/Chromecast where
  feasible.
- Energy monitoring and UPS integration.

### 5. Hardening and reach

- IDS/IPS and parental-control presets.
- Reproducible ISO builds on CI and a real-hardware test matrix.
- Accessibility and localization passes (`es`/`en` first).
- More tests for prompt injection, tool abuse and external integration failure
  modes.

## Stable release and ISO gate

The first stable ISO is not a date-driven milestone. It ships when the
appliance has been validated end to end on real hardware and the SourceForge
gate in `docs/PUBLISHING.md` is met: reproducible source tag, real Incus,
DNS/firewall/DMZ, backup/restore/upgrade, kiosk, voice and a soak period. Until
then GitHub publishes source only and SourceForge receives no ISO.

## How to help

- Pick an issue labelled `good first issue` or `area/*`.
- Read `CONTRIBUTING.md` and sign off commits with DCO.
- Security reports: see `SECURITY.md`; do not open a public issue.
- Design discussions are welcome before large changes; the architecture rules
  are in `AGENTS.md` and `docs/ARCHITECTURE.md`.

## Non-goals

- Selling user data or requiring a cloud account for core local features.
- Exposing the admin API or kiosk directly to the internet.
- Silent automatic actions that cannot be audited or undone.
