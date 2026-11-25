# HomeServer Assistant OS — Web UI

Single-page application that ships the three experiences of the appliance:

| Experience | Route | Notes |
| --- | --- | --- |
| First-boot onboarding | `/setup` | 6-step wizard, public only while `SystemConfig.initialized == false` |
| Admin portal | `/login`, `/admin/*` | Management console, session cookie + CSRF |
| Wall kiosk | `/kiosk` | Full-screen avatar experience for the 1280×800 wall tablet |

Stack: **React 19 · TypeScript (strict) · Vite · Tailwind CSS 3.4 · lucide-react ·
@fontsource-variable** fonts (bundled locally). No component library, no runtime
CDN, no webfont requests to Google.

---

## Scripts

```bash
pnpm install        # install dependencies (navigator lockfile, pnpm >= 10)
pnpm dev            # dev server on :5173, proxies /api (HTTP + WS) to 127.0.0.1:8080
pnpm typecheck      # tsc --noEmit (strict, noUnusedLocals)
pnpm test           # vitest run (API client, ring buffer, WebSocket helper, avatar states)
pnpm build          # tsc --noEmit && vite build -> ui/dist
pnpm preview        # preview the production bundle
```

Node 22 is expected. The `node-linker=hoisted` setting in `.npmrc` keeps a flat
`node_modules` so the bundle can be verified inside restricted sandboxes where
symlinked package trees are not resolvable by the Node loader.

> **Sealed Windows sandbox tip.** If `pnpm` fails before touching the project
> with `ERR_PNPM_STORE_DIR_OPEN_OPERATION_LOCK` (pnpm 12 keeps its store lock in
> `%LOCALAPPDATA%`, outside the writable workspace), install a workspace-local
> pnpm once and use it for install only:
>
> ```powershell
> node "$env:ProgramFiles\nodejs\node_modules\npm\bin\npm-cli.js" install --prefix .tools --cache .npm-cache pnpm@10.28.2
> node .tools\node_modules\pnpm\bin\pnpm.cjs install --store-dir .pnpm-store
> ```
>
> Scripts (`typecheck`, `test`, `build`) then work with any pnpm because
> dependencies are already present.

### Running against the daemon

`pnpm dev` proxies `/api/*` and WebSocket upgrades to `http://127.0.0.1:8080`, so
the SPA can run against a local `hs-daemon` without CORS:

* API: `http://localhost:5173/api/v1/...` → `http://127.0.0.1:8080/api/v1/...`
* WS: `ws://localhost:5173/api/v1/kiosk/events` → daemon kiosk socket

In production the daemon serves the built `dist/`:

* admin portal on the configured admin port (TLS terminated upstream),
* kiosk on `127.0.0.1:3000`, authenticated by the HttpOnly `hs_kiosk` cookie.

### Kiosk in development

The kiosk API is loopback-only and requires the shared kiosk token. Create
`ui/.env.local` with the token from the daemon configuration:

```bash
VITE_KIOSK_TOKEN=<the kiosk token>
```

The token can also be overridden at runtime for debugging via
`localStorage.setItem('hs.kioskToken', '<token>')` (or
`setKioskTokenOverride()` from `src/api/kiosk.ts`). In production no token is
needed: the kiosk page is same-origin and uses the `hs_kiosk` cookie.

### Visual demo mode

Open `/kiosk?demo=1` (or start with `VITE_DEMO=1`) to render the kiosk with
clearly-labelled demo data and no backend calls. Demo mode also exposes avatar
state preview buttons for design review. Admin routes never fake data: every
mutation goes to the daemon and errors are surfaced.

---

## Environment variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `VITE_API_BASE` | *(empty = same origin)* | Absolute API origin, e.g. `https://homeserver.local:8443`. The client appends `/api/v1`. |
| `VITE_WS_BASE` | derived from API base/origin | Explicit WebSocket origin override. |
| `VITE_KIOSK_TOKEN` | *(empty)* | Kiosk shared secret for local development. |
| `VITE_DEMO` | `0` | `1` enables the kiosk visual demo mode globally. |

See [`.env.example`](./.env.example).

---

## Route map

```
/                     -> GET /api/v1/setup/status
                         initialized=false -> /setup
                         initialized=true  -> /admin/dashboard (session) | /login
/login                -> admin login (centered glass card)
/setup                -> 6-step first-boot wizard (redirects away once initialized)
/kiosk                -> wall kiosk (avatar, clock, music, scenes, talk, profiles)
/admin                -> redirect
/admin/dashboard      -> live host metrics, services, audit
/admin/instances      -> containers/VMs: actions, snapshots, logs, exec terminal
/admin/apps           -> catalog + deploy drawer + deployed apps
/admin/homeassistant  -> HA status/config, entities by domain, scenes, discovery
/admin/network        -> interfaces/addressing, DNS (DoH/DoT, blocklists), DMZ, firewall
/admin/agent          -> AI providers, workflows, chat playground, tool registry
/admin/profiles       -> family profiles, avatar editor, permissions, face status
/admin/audit          -> filterable tamper-evident audit log
*                     -> not found
```

`Cmd/Ctrl+K` opens the command palette (navigation + quick actions).

---

## API client

`src/api/` is a typed wrapper over the daemon REST + WebSocket contract.

* `client.ts` — base URL resolution, `credentials: 'include'`, CSRF
  (`X-HS-CSRF` from the `hs_csrf` cookie or the token returned by login/me,
  attached to every mutating request **except** `POST /auth/login`), uniform
  `ApiError` parsing of `{ error: { code, message, details } }`, and a global
  `hs:unauthorized` event that returns the SPA to `/login`.
* `ws.ts` — `ReconnectingSocket`: exponential backoff 1 s → 15 s with jitter, a
  20 s heartbeat, silent-socket recycling, JSON guard at the call site.
* `types.ts` — mirrors the Rust serde models exactly (snake_case fields, RFC
  3339 timestamps). Responses that the daemon returns as bare arrays (`/audit`,
  `/agent/tools`, `/apps/catalog`, `/profiles`) are normalized defensively in
  their modules.
* Feature modules: `auth`, `system`, `setup`, `instances`, `apps`, `network`,
  `discovery`, `homeassistant`, `agent`, `profiles`, `kiosk`.

The typed `wsUrl()` helper builds `/api/v1/...` WebSocket URLs, so the dev proxy,
the admin origin and the loopback kiosk origin all resolve the same.

---

## Design system

Ambient nighttime home OS: near-black blue base, layered radial gradients, glass
panels with a 1 px inner highlight, generous spacing (16–20 px radii), soft glow
accents and a very subtle film grain.

### Tokens (`tailwind.config.js`, `src/index.css`)

| Token | Value | Use |
| --- | --- | --- |
| `night.900` | `#060a12` | app background |
| `night.800/750` | `#0b1220` / `#0e1726` | inputs, wells, panels |
| `mint.400 → mint.500` | `#6ee7ff → #34d399` | primary accent, gradients, focus |
| `amber.400` | `#fbbf24` | warnings, charging, featured |
| `coral.400/500` | `#fb7185` / `#e11d48` | destructive, errors |
| `ink.50 … ink.600` | cool blue-grey ramp | text hierarchy |

* **Display**: Outfit Variable (headings, kiosk clock, avatar greeting).
* **Body/UI**: Inter Variable.
* **Numeric metrics**: `.tabular` / `font-variant-numeric: tabular-nums`.
* **Radii**: cards 20 px, controls 12 px, modals 22–24 px.
* **Elevation**: `shadow-glass` = inset top highlight + deep ambient shadow.
* **Motion**: CSS keyframes only (`hs-*`), 150–250 ms `--hs-ease`
  (`cubic-bezier(.22,.61,.36,1)`); everything collapses under
  `prefers-reduced-motion: reduce`.

Accessibility: 44 px minimum touch targets (kiosk controls 56 px+), visible
`:focus-visible` rings, `aria-*` on dialogs/toggles/sliders/live regions, AA
contrast on the dark palette, no layout shift for skeleton→content transitions.

### Avatar engine

`src/components/Avatar.tsx` renders the SVG avatar entirely from props:

* shapes `circle | squircle | cloud | cone | droplet` (distinct paths),
* radial-gradient body from `avatar.color`, capsule eyes with `avatar.eye_color`
  and `eye_tilt_deg`,
* gaze offset from a normalized `TrackingPoint` with clamping and CSS transitions,
* states `idle | tracking | listening | thinking | speaking | profile_switch`,
  driven by modifier classes (`is-idle`, …) and CSS choreography.

`avatarStateClass()` is exported and unit-tested.

---

## i18n

A single typed, flat dictionary lives in `src/i18n/dictionary.ts`
(`es` is the source of truth, `en` is compile-checked against it). Components
only pass keys — no user-facing literal strings in JSX. Language is persisted in
`localStorage` (`hs.language`) and switched from the login screen, the wizard and
the admin topbar; the kiosk follows the same preference.

---

## Project structure

```
src/
  api/            typed REST + WS client and domain modules
  app/            route guards (/ redirect, setup gate, auth guard)
  components/     Avatar, Gauge, Sparkline, MetricCard, Table, Modal, Drawer,
                  Terminal, Toast, StatusPill, Field/Toggle/Segmented, …
  features/
    admin/        Login, Dashboard, Instances, Apps, HomeAssistant, Network,
                  Agent (+ agent/ Providers, Workflows, ChatTester, Tools),
                  Profiles, Audit
    setup/        6-step wizard (language, network, devices, discovery,
                  assistant, deploy) + step chrome
    kiosk/        full-screen kiosk experience + demo data
  i18n/           typed es/en dictionaries and provider
  layout/         admin shell: sidebar, topbar, command palette
  lib/            formatting, colors, ring buffer, hooks, live events, DNS meta
  pages/          not found
  state/          auth and toast providers
```

---

## Testing & quality gates

* `pnpm typecheck` — strict TypeScript, no `any`, no unused locals/params.
* `pnpm build` — production bundle in `ui/dist` (git-ignored).
* `pnpm test` — Vitest (node environment, worker threads):
  * CSRF header policy and API error envelope parsing,
  * `RingBuffer` bounds,
  * WebSocket backoff/jitter and URL building,
  * avatar state → CSS class mapping.

Unit tests run without a DOM or the daemon.

---

## API notes / assumptions

* `PUT /network` is used by the Addressing form to persist
  `{ interface, lan_bridge?, mode, static_ipv4 }`. The daemon exposes explicit
  DNS/DMZ updaters; this mirrors the same config section for addressing.
  `lan_bridge` is optional (server default `br-lan`).
* Provider management uses the documented `POST/activate/DELETE` endpoints; the
  contract has no provider update route, so the UI offers add + activate +
  delete instead of in-place edit.
* `GET /system/overview` bodies are treated defensively; WebSocket frames on
  `/ws/events` are parsed with a guard and ignored if they are not an overview
  payload, in which case the dashboard keeps its 5 s polling fallback.
* Kiosk profile switching prefers `GET /profiles`; if the kiosk session cannot
  read it (loopback cookie only, no admin session), the switcher falls back to
  the currently active profile from `/kiosk/state`.
* Nothing is optimistically claimed: every mutation awaits the daemon response
  and surfaces errors; offline kiosk/admin views never render fake success.
