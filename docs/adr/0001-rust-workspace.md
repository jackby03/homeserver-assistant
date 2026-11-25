# ADR 0001 — One Rust workspace with ports and adapters

- **Status:** accepted
- **Date:** 2026-01-15

## Context

The appliance spans authentication, host hardware, container/VM orchestration,
networking, IoT discovery, an LLM agent and a local vision service. A single
process must wire them without becoming a monolith that is impossible to test.

## Decision

Use one Cargo workspace with small crates and a strict dependency direction:

- `hs-core` is a leaf (domain types, config, persistence, events).
- Infrastructure-heavy concerns are adapters behind small traits
  (`RuntimeProvider`, `HaTransport`, `DiscoverySource`, tool ports).
- `hs-daemon` is the composition root and the only crate that knows concrete
  adapters.

No crate may depend "upward"; `hs-tools` for example defines its own ports so it
does not depend on runtime/HA/audio crates.

## Consequences

- Security-critical logic (permissions, firewall rendering, config validation)
  is unit-testable without root, Incus, a camera or the network.
- The daemon is a wiring layer; handlers stay thin.
- More crates to compile, mitigated by a single workspace and shared
  dependencies.

## Alternatives considered

- **Single binary crate:** rejected; tests would require system services and
  the permission checks would drift into handlers.
- **Microservices:** rejected; an appliance should run one privileged daemon to
  keep privilege and failure domains small.
