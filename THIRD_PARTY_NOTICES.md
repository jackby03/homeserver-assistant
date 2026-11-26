# Third-party notices

HomeServer Assistant OS is licensed under **GPL-3.0-only** (see `LICENSE`).
Third-party components remain under their original licenses. This file
summarizes the components that are vendored in the repository, plus the main
runtime categories. It is not a replacement for the license text shipped with
each dependency.

## Vendored assets

| Component | Location | License |
|---|---|---|
| BoopFace lite 0.9 expressive avatar | `ui/src/vendor/boopface/` | MIT; see `ui/src/vendor/boopface/LICENSE` |
| Inter variable font | npm `@fontsource-variable/inter` | SIL Open Font License 1.1 |
| Outfit variable font | npm `@fontsource-variable/outfit` | SIL Open Font License 1.1 |

The BoopFace copyright notice is retained in its vendored license file. The
fonts are distributed by Fontsource under the SIL OFL; their copyright notices
are included in the npm packages.

## UI dependencies

| Component | License |
|---|---|
| React, React DOM | MIT |
| React Router | MIT |
| Lucide icons (`lucide-react`) | ISC |
| Vite, Vitest, Tailwind CSS, PostCSS, Autoprefixer | MIT (TypeScript is Apache-2.0) |

## Rust dependencies

The Rust workspace uses crates from crates.io, predominantly dual-licensed
MIT OR Apache-2.0. Their exact versions and source repositories are recorded in
`Cargo.lock`; each crate ships its license in the Cargo registry. Before
publishing binaries, run a license audit (for example `cargo about` or
`cargo deny`) over the lockfile and keep the license texts required by those
crates. Report any incompatible dependency so it can be replaced.

## Local voice and vision components

| Component | Typical license |
|---|---|
| faster-whisper | MIT |
| Vosk | Apache-2.0 |
| Piper | MIT |
| Wyoming protocol services | Apache-2.0 (see each upstream project) |
| OpenAI Whisper model weights | MIT (model card terms apply) |
| InsightFace / ONNX runtime, when installed optionally | MIT / MIT |

These components are optional local helpers and are not all vendored in this
repository. If you ship them in an image or ISO, include their upstream license
files and attribution notices.

## External services

Home Assistant, Composio, Google Calendar/Gmail, LLM providers, DNS upstreams
and tunnel providers are **external services**, not bundled code. Their terms,
privacy policies and data-processing agreements apply when you enable them. The
appliance never bundles their credentials.

## Reporting an attribution mistake

If a notice is missing or inaccurate, please open an issue or contact the
maintainers. We want redistributors to have a complete and correct notice file.
