# Publishing HomeServer Assistant OS

This is the maintainer checklist for the first public release. It assumes the
repository already exists locally and that you have a GitHub account/org.

## 1. Create the public repository

Create an **empty** public repository (do not add a README, license or
`.gitignore` from the GitHub wizard; this repository already has them).

```bash
git remote add origin git@github.com:<owner>/<repo>.git
git push -u origin main
```

Set the default branch to `main`.

## 2. Repository settings

- Description: `Local-first home server appliance with Incus, DNS/firewall,
  Home Assistant, local vision and an agentic voice assistant.`
- Topics: `home-server`, `self-hosted`, `local-first`, `rust`, `react`,
  `home-assistant`, `incus`, `gplv3`.
- Enable **Discussions** for usage questions.
- Enable **Private vulnerability reporting** (Settings → Security).
- Enable Issues and the issue templates in `.github/ISSUE_TEMPLATE/`.
- Protect `main`: require the `rust`, `ui` and `scripts` CI jobs, require a
  pull request, and dismiss stale approvals.
- Add a social preview image if available.

## 3. GPLv3 compliance checklist

- [ ] `LICENSE` is the verbatim GNU GPLv3 text.
- [ ] `Cargo.toml` and `ui/package.json` declare `GPL-3.0-only`.
- [ ] `NOTICE` contains the copyright and warranty notice.
- [ ] `THIRD_PARTY_NOTICES.md` covers vendored assets and the main dependency
      categories; keep it updated when adding a bundled component.
- [ ] The source corresponding to every published binary/ISO is available from
      this repository (tag the release).
- [ ] Build scripts are published; release artifacts include the LICENSE,
      NOTICE and third-party notices.
- [ ] The ISO does **not** ship a default console password; the build requires
      `HS_CONSOLE_PASSWORD_HASH`.
- [ ] No secrets or demo databases are committed (see below).

## 4. Secret and history checks

Run these before the first push:

```bash
git log --all --oneline -S '<known-old-demo-password>' -- .
git log --all --oneline -S 'PRIVATE KEY' -- .
git grep -n -I -E 'sk-[A-Za-z0-9]{20,}|BEGIN (RSA|OPENSSH|PRIVATE)' -- .
git status --porcelain
git diff --check
```

The repository was scrubbed so the old ISO console-password hash is not
reachable from any ref. If you restore old refs or import a bundle, re-run the
checks.

Never publish `.demo/`, `target/`, `ui/node_modules/`, `data/`, `.env*`,
`*.redb`, `master.key` or model binaries. Publish with GitHub or `git archive`;
do not zip the working tree.

## 5. First release

1. Update `CHANGELOG.md` and `CITATION.cff` version/date.
2. Run the full check suite:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace --lib --tests
   python3 scripts/check-assets.py
   cd ui && pnpm install --frozen-lockfile && pnpm typecheck && pnpm test && pnpm build
   ```
3. Build the UI and (on Linux) the ISO with a unique
   `HS_CONSOLE_PASSWORD_HASH`.
4. Create a signed/annotated tag `v0.1.0` and publish the changelog.
5. Upload artifacts (ISO, manifest, SHA256, signatures) only if you can publish
   the corresponding source tag and license notices. Prefer a release page that
   links the source, checksums and known limitations.
6. Announce the security model and the local-first scope; ask users to report
   vulnerabilities privately, not in public issues.

## 6. ISO distribution (SourceForge)

GitHub hosts the source and source releases. **SourceForge is the only channel
for published ISO images**, and only for a stable, functional release.

### 6.1 Stability gate

Do not upload an ISO until all of these are true:

- [ ] The source tag is frozen and reproducible (same commit + toolchain).
- [ ] Clean install from the ISO on at least one real x86-64 machine.
- [ ] First boot, setup wizard, admin login and kiosk verified.
- [ ] Real Incus LXC/OCI/VM lifecycle and snapshots verified.
- [ ] DNS filtering, nftables, DMZ isolation and admin-only exposure verified.
- [ ] Backup/restore and upgrade path verified on a second machine or VM.
- [ ] Home Assistant, profiles, local vision, wake word and agent tools verified
      without cloud credentials.
- [ ] No P0/P1 security or data-loss issues open.
- [ ] A soak period (at least 7 days on a household-like workload) completed.
- [ ] ISO checksum, manifest and signature generated and independently verified.
- [ ] `LICENSE`, `NOTICE` and `THIRD_PARTY_NOTICES.md` are inside the image.

### 6.2 SourceForge upload

1. Create the SourceForge project and release folder when the gate passes.
2. Upload the ISO, `.manifest.json`, `SHA256SUMS`, signature and release notes.
3. Publish the corresponding source tag on GitHub and link both directions.
4. Never replace an existing ISO in place; publish a new versioned folder.
5. Keep at least the previous stable ISO available for rollback.
6. Announce the exact source commit and known hardware matrix.

### 6.3 What not to publish

- Development/nightly ISOs.
- Images built from uncommitted changes or without `HS_CONSOLE_PASSWORD_HASH`.
- ISOs without checksums/signatures or with missing third-party notices.
- GitHub releases that silently include binary artefacts; keep them source-only
  unless the same source and notices are published.

## 6. After launch

- Triage issues against `ROADMAP.md`.
- Label `good first issue` and `area/*` issues.
- Keep Dependabot PRs green; review license changes before merging.
- Re-run the secret/history checks before every release.
- Tag hardware-specific claims with the tested platform, not as universal
  support.
