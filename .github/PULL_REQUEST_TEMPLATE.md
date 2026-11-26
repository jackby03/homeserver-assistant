## Summary

<!-- What does this change and why? Link the issue: Fixes #123 -->

## Scope and risks

- Affected crates/UI/voice/ISO:
- Security implications (auth, permissions, tools, network, secrets):
- Migration or compatibility notes:

## Verification

<!-- Paste the commands you ran and their result. -->

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib --tests
cd ui && pnpm typecheck && pnpm test && pnpm build
```

## Checklist

- [ ] I read `CONTRIBUTING.md`, `AGENTS.md` and the security model.
- [ ] Changes are focused and documented.
- [ ] Tests cover the new behavior or security-critical path.
- [ ] No secrets, databases, generated UI bundles or model files are included.
- [ ] Commits are signed off with DCO (`git commit -s`).
- [ ] Public API changes are reflected in `docs/API.md`.
