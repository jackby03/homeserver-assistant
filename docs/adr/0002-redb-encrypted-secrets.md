# ADR 0002 — Embedded redb database and an AEAD secret vault

- **Status:** accepted
- **Date:** 2026-01-15

## Context

The appliance needs durable storage for users, sessions, profiles, deployed app
records, audit entries and encrypted secrets. The ISO must stay small and the
server must survive power loss on a wall-mounted laptop serving as a UPS-backed
machine.

## Decision

- Use **redb** (pure-Rust embedded transactional key/value store) for structured
  state. Values are JSON-encoded behind typed accessors.
- Store secrets separately in the same database as ChaCha20-Poly1305 envelopes.
  The 256-bit master key is a file with mode `0600` (`master.key`). Nonce is
  random per encryption; the envelope is versioned for future key rotation.
- Write configuration atomically (temp file + rename, `0600`).
- Keep audit entries append-only with a monotonic sequence allocated inside the
  same transaction.

## Consequences

- No native dependencies (SQLite/OpenSSL) in the runtime image, no migration
  tooling to operate; schema evolution is handled by version fields and
  additive changes.
- Backups are a file copy of `config.json`, `state.redb` and `master.key`
  (the last one encrypted offline).
- Losing `master.key` loses stored secrets; documented in the deployment guide.
- redb is a single-process store, which matches the appliance model (one
  daemon).

## Alternatives considered

- **SQLite via rusqlite:** rejected to avoid a C toolchain in the ISO and keep
  the dependency surface pure Rust.
- **JSON files for everything:** rejected; no transactions/atomicity for
  sessions and audit.
- **PostgreSQL:** rejected; overkill for a single appliance.
