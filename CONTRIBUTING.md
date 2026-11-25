# Contributing to HomeServer Assistant OS

Thanks for helping! This is a local-first appliance project; security and
reproducibility come before feature velocity.

## Before you start

1. Read [`AGENTS.md`](AGENTS.md) — it contains the mandatory engineering rules
   (crate boundaries, security invariants, testing, English source code).
2. Read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and
   [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md).
3. For a non-trivial change, open an issue or discussion describing the problem
   and the proposed approach.

## Development workflow

```bash
cargo fetch                          # once
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib --tests
cd ui && pnpm install && pnpm typecheck && pnpm build
```

For a local appliance run:

```bash
cargo run -p hs-daemon -- serve --dev --data-dir ./.hs-data --static-dir ui/dist
# admin: http://127.0.0.1:8080 · kiosk: http://127.0.0.1:3000/kiosk
```

## Pull requests

- Keep changes small and focused; one concern per PR.
- Add tests for security-critical logic (auth, permissions, firewall, tools).
- Update `docs/API.md` when the HTTP/WS contract changes.
- Do not include secrets, generated databases, built UI artifacts or ISO files.
- CI must be green (`rust`, `ui`, `scripts` jobs).

## Security reports

Please do not open a public issue for vulnerabilities. Contact the maintainers
privately with the affected component, a reproduction and impact assessment.
See [`docs/SECURITY.md`](docs/SECURITY.md) for the threat model and incident
response procedures.

## License

By contributing you agree that your contributions are licensed under the MIT
license (see [`LICENSE`](LICENSE)).
