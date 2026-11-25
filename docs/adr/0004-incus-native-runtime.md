# ADR 0004 — Native Incus adapter over its Unix socket

- **Status:** accepted
- **Date:** 2026-01-15

## Context

The compute plane must support unprivileged system containers, OCI/Docker
application containers and QEMU/KVM virtual machines with snapshots, using a
single, local, well-supported engine on Debian/Ubuntu.

## Decision

- Define a `RuntimeProvider` port (`hs-runtime`) that models lifecycle,
  snapshots, limits, logs, exec and OCI deployment with normalized types.
- Implement a **native Incus adapter** speaking the Incus REST API over
  `/var/lib/incus/unix.socket` (hyperlocal on Unix). On non-Unix development
  machines the adapter reports `Unavailable` and never panics.
- Keep REST access behind an internal transport trait so the API mapping,
  operation waiting and error handling are unit-tested without Incus.
- Provide `MockRuntime` for tests and `hs-daemon --dev` mode.
- Use the `incus exec` CLI for one-shot commands initially (no PTY); an
  interactive terminal over the Incus exec WebSocket remains a follow-up.

## Consequences

- One dependency (Incus) provides LXC, OCI and VMs with ZFS/Btrfs snapshots.
- The daemon needs root (Incus socket + nftables + sysfs); hardening is applied
  in the systemd unit.
- Tests run on any OS because the adapter is isolated behind a transport.

## Alternatives considered

- **Docker only:** rejected; no system containers/VMs, weaker isolation model.
- **Proxmox/HAOS-specific APIs:** rejected; appliance should stay a plain
  Debian/Ubuntu system.
- **Shelling out for everything:** rejected; error handling and JSON parsing
  are more reliable through the REST API.
