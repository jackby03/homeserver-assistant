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

## 6. After launch

- Triage issues against `ROADMAP.md`.
- Label `good first issue` and `area/*` issues.
- Keep Dependabot PRs green; review license changes before merging.
- Re-run the secret/history checks before every release.
- Tag hardware-specific claims with the tested platform, not as universal
  support.
