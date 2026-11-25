# Development Guide

## Prerequisites

| Tool | Version | Notes |
|---|---|---|
| Rust | 1.85+ (stable) | `rust-toolchain.toml` pins the channel and components |
| Node.js | 22+ | for the UI |
| pnpm | 9+ | `corepack enable pnpm` |
| Python | 3.11+ | optional, only for the face-embedding helper |
| Incus | 6.x | only to exercise the real runtime adapter on Linux |
| `make` | any | convenience targets; all commands can be run directly |

On Windows everything except Incus, nftables and the camera can be built and
tested. The daemon runs with `--dev` using in-memory fakes.

## Repository layout

```
crates/        Rust workspace (13 crates, see docs/ARCHITECTURE.md)
ui/            React + TypeScript SPA (wizard, admin portal, kiosk)
vision/python/ optional InsightFace embedding helper (local only)
infra/         Incus preseed, nftables, systemd units, service templates
iso/           Ubuntu autoinstall and reproducible ISO build
docs/          Architecture, API, security, deployment and ADRs
```

## Offline builds

Dependencies are fetched once with `cargo fetch`. CI and this repository are
expected to build with `--offline`:

```bash
cargo build --workspace --offline
cargo test --workspace --offline --lib --tests   # rustdoc may be unavailable in minimal toolchains
```

Add `--lib --tests` when the environment lacks `rustdoc`; doctests are still
part of CI where the component is installed.

## Running the daemon locally

```bash
# Fake runtime + fake hardware, data in ./.hs-data, UI served from ui/dist if present
cargo run -p hs-daemon --offline -- serve --dev --data-dir ./.hs-data --static-dir ui/dist
# Admin API:  http://127.0.0.1:8080
# Kiosk:      http://127.0.0.1:3000/kiosk
```

In dev mode the daemon uses `MockRuntime`, `FakeHardware` and in-memory vision
events, so it starts on any OS without root.

## Running the UI

```bash
cd ui
pnpm install
pnpm dev          # Vite on :5173, proxies /api to http://127.0.0.1:8080
pnpm typecheck
pnpm build        # outputs ui/dist, served by the daemon
```

For the kiosk during development open `http://localhost:5173/kiosk?demo=1` and
set `VITE_KIOSK_TOKEN`/`VITE_API_BASE` as documented in `ui/README.md`.

## First boot flow (development)

1. Start the daemon with an empty data directory.
2. Open `http://127.0.0.1:8080` (or Caddy `:8443` in production); the wizard
   appears because `initialized == false`.
3. Step 1 creates the admin account (Argon2id) and device settings.
4. Steps 2–5 persist network/DNS/DMZ, audio/peripherals, discovery, provider and
   profiles.
5. Step 6 runs the deployment job and streams progress over `/ws/setup`.
6. `initialized = true`, the kiosk opens.

To reset: stop the daemon, delete the data directory (this destroys secrets and
state), restart.

## Testing

```bash
# Everything that does not need root/Incus/camera
cargo test --workspace --offline --lib --tests

# A single crate
cargo test -p hs-core --offline --lib --tests
cargo test -p hs-auth --offline --lib --tests

# UI
cd ui && pnpm typecheck && pnpm build
```

Security-critical tests live next to the code they protect:

- `hs-core`: config validation, file permissions, vault tamper detection.
- `hs-auth`: JWT tampering (`alg:none`, wrong secret, expiry), RBAC, last-admin
  protection, login throttling, session revocation, CSRF.
- `hs-network`: golden nftables ruleset tests (DMZ→LAN and DMZ→host drops).
- `hs-tools`: denied tools never reach the port adapters.
- `hs-daemon/tests`: router-level integration tests (auth, setup gating, RBAC,
  kiosk token).

## Linting

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
```

## Conventions

Read `AGENTS.md`. In short: English identifiers/logs, no `unwrap` outside tests,
typed errors, doc comments on public items, tests for security logic, and no
secrets or machine-specific paths committed.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `cargo` cannot download crates | run `cargo fetch` once; then use `--offline` |
| doctests fail with "rustdoc binary not applicable" | minimal toolchain; run with `--lib --tests` |
| daemon exits with "configuration lock poisoned" | a thread panicked while holding a lock; check logs |
| port already in use | pass `--bind 127.0.0.1:8081` / `--kiosk-bind 127.0.0.1:3001` |
| Incus unavailable in dev | expected; use `--dev` or run on Linux with Incus installed |
