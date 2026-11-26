# Security Policy

HomeServer Assistant OS is a local-first appliance that holds credentials,
family profiles and control over the home network. We take security reports
seriously and prefer private disclosure.

## Reporting a vulnerability

**Do not open a public issue for security vulnerabilities.**

Use GitHub's private vulnerability reporting for this repository
("Security" → "Report a vulnerability"). If that is unavailable, contact the
maintainers through the repository owner's public profile and ask for a private
channel. Do not include exploit details in a public message.

Please include:

- affected version or commit;
- component (daemon, auth, network/firewall, tools/agent, UI, ISO, voice);
- reproduction steps or a proof of concept;
- impact assessment and any suggested fix;
- whether the issue is already public.

We will acknowledge the report, investigate it and coordinate disclosure. We do
not offer a bug bounty or a guaranteed response time; this is a volunteer
project.

## Supported versions

| Version | Supported |
|---|---|
| `0.1.x` | Yes, current development baseline |
| Older tags | No |

Security fixes land on `main` and are noted in `CHANGELOG.md`.

## Scope

In scope:

- authentication, sessions, CSRF, RBAC and secret handling;
- tool permissions and the agent confirmation/risk gate;
- external integrations (Composio credential handling, OAuth ids);
- nftables/DMZ isolation, DNS and network configuration;
- kiosk token isolation and the loopback boundary;
- ISO, systemd and Caddy hardening;
- supply-chain issues in tracked dependencies.

Out of scope:

- vulnerabilities in third-party services (Composio, Google, Home Assistant,
  LLM providers), which should be reported upstream;
- physical access to an unlocked appliance;
- misconfiguration documented as unsafe (for example exposing the admin port or
  running the kiosk outside loopback);
- issues that require root-level access already granted to the daemon.

See [`docs/SECURITY.md`](docs/SECURITY.md) for the full threat model and
[`docs/AGENT_CAPABILITIES.md`](docs/AGENT_CAPABILITIES.md) for the agent/tool
security model.

## Safe harbor

We will not pursue legal action against researchers who act in good faith,
avoid privacy violations and service disruption, and give us reasonable time to
fix the issue before public disclosure.

## Secrets

Never include real API keys, tokens, passwords, biometric data, `master.key` or
`state.redb` in a report. Use redacted values and, when possible, a minimal
reproduction against a local test instance.
