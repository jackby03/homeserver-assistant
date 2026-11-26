# Support

HomeServer Assistant OS is a volunteer open-source project. There is no SLA and
no paid support commitment.

## Before asking

1. Read [`README.md`](README.md), [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) and
   [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md).
2. Search existing issues and discussions.
3. Try the development mode:
   ```bash
   cargo run -p hs-daemon -- serve --dev --data-dir ./.hs-data --static-dir ui/dist
   ```
4. Collect logs with secrets removed (`config.json`, `master.key`, `state.redb`
   and `.env` files must never be uploaded).

## Where to ask

- **Bug reports and feature requests:** GitHub issues using the templates.
- **Usage questions and ideas:** GitHub Discussions, if enabled; otherwise open
  a question issue.
- **Security vulnerabilities:** private disclosure only, see
  [`SECURITY.md`](SECURITY.md).
- **Code contributions:** [`CONTRIBUTING.md`](CONTRIBUTING.md) and
  [`ROADMAP.md`](ROADMAP.md).

## What to include in a bug report

- version/commit and platform (Linux distro or development OS);
- whether it is development mode or a real appliance;
- steps to reproduce, expected and actual behavior;
- relevant logs with secrets redacted;
- screenshots for UI issues, never showing credentials or family data.

## Support boundaries

We cannot debug third-party services (Composio, Google, Home Assistant, LLM
providers, DNS resolvers) or custom modifications. We may ask you to reproduce
on a clean checkout first.

## Commercial use

GPLv3 allows commercial use and redistribution. If you build a product or an
appliance image on top of this project, you must comply with GPLv3 (including
source availability for covered modifications) and with the licenses in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md). We do not provide commercial
certification.
