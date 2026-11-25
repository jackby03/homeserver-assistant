# HTTP / WebSocket API Reference

- **Admin API base path:** `/api/v1` on `127.0.0.1:8080`, exposed to the LAN by
  Caddy at `https://<appliance>:8443`.
- **Kiosk API base path:** `/api/v1` on `127.0.0.1:3000` only, protected by the
  per-install kiosk token.
- **Content type:** `application/json` for requests and responses.
- **Timestamps:** RFC 3339 UTC strings.
- **IDs:** UUID strings.
- **Errors:** every non-2xx response uses the envelope
  `{"error":{"code":"...","message":"...","details":?}}`.

## Authentication

Cookie sessions (browser) and `Authorization: Bearer <jwt>` (API clients) are
accepted. Cookies set by `POST /auth/login`:

| Cookie | Attributes | Purpose |
|---|---|---|
| `hs_session` | `HttpOnly`, `SameSite=Strict`, `Secure` behind TLS | session JWT |
| `hs_csrf` | readable by JS, `SameSite=Strict` | double-submit CSRF token |

Cookie-authenticated **mutating** requests (`POST`, `PUT`, `PATCH`, `DELETE`)
must send `X-HS-CSRF: <csrf_token>`; otherwise the API returns `403 forbidden`.
Bearer-token requests are exempt.

Roles: `admin` (all), `operator` (operations, no network/users), `viewer`
(read-only). Permission denials return `403` with `code: "forbidden"`.

Login throttling: 5 failed attempts per username+IP in 5 minutes → `429`.

---

## Public endpoints

### `GET /api/v1/health`
```json
{"status":"ok","version":"0.1.0","uptime_secs":42,"initialized":false,"fake_runtime":false}
```

### `GET /api/v1/setup/status`
```json
{"initialized":false,"device_name":"HomeServer Assistant","language":"es",
 "completed_steps":[1,2],"running":false,"error":null}
```

---

## Authentication

### `POST /api/v1/auth/login`
Request: `{"username":"admin","password":"..."}`  
Response `200` + `Set-Cookie`:
```json
{"user":{"id":"…","username":"admin","display_name":"Admin","role":"admin",
         "disabled":false,"created_at":"…","last_login_at":"…"},
 "csrf_token":"…"}
```
Errors: `401 invalid_credentials`, `429 too_many_attempts`.

### `POST /api/v1/auth/logout`
Clears cookies and revokes the server-side session. `200 {"ok":true}`.

### `GET /api/v1/auth/me`
Same body as login. `401` when the session is missing/expired/revoked.

### `POST /api/v1/auth/password`
`{"current_password":"…","new_password":"…"}` — revokes other sessions.
`200 {"ok":true}`.

### Users (admin only)

| Method | Path | Body |
|---|---|---|
| GET | `/users` | — |
| POST | `/users` | `{username,password,display_name,role}` |
| GET | `/users/{id}` | — |
| PATCH | `/users/{id}` | `{role?,disabled?}` |
| DELETE | `/users/{id}` | — |
| POST | `/users/{id}/password` | `{new_password}` |

Responses are `UserView` objects (never include the password hash).

---

## First-boot onboarding

Available only while `initialized == false`. Step 1 creates the administrator and
returns a session; steps 2–6 require that authenticated administrator.

### `POST /api/v1/setup/admin` (public, only when no users exist)
```json
{"username":"admin","password":"Sup3rSecret!!",
 "device":{"name":"Salón","language":"es","timezone":"America/Lima"}}
```
Response = login payload + cookies. Errors: `409 conflict` when users/initialized.

### `POST /api/v1/setup/network`
```json
{"interface":"eth0","lan_bridge":"br-lan","mode":"dhcp","static_ipv4":null,
 "dns":{"enabled":true,"engine":"adguard_home","provider":"cloudflare_family",
        "protocol":"doh","custom_upstream":null,"blocklists":["https://…"]},
 "dmz":{"enabled":true,"bridge":"br-dmz","subnet":"10.20.30.0/24",
        "gateway":"10.20.30.1","allow_dns":true}}
```
Response: the persisted `NetworkConfig`.

### `GET /api/v1/setup/peripherals`
`{"camera_devices":["/dev/video0"],"touch_devices":["/dev/input/event3"],
  "audio_sinks":[{"id":"…","name":"…","description":"…","is_default":true}]}`

### `POST /api/v1/setup/audio/test`
`{"sink_id":"…"}` → `{"applied":true,"sink_id":"…","sink_name":"…"}`. Applies the
sink/volume; there is no synthetic tone in v1.

### `POST /api/v1/setup/profiles`
`{"profiles":[CreateProfileRequest…]}` → array of created profiles.
`GET /api/v1/profiles/defaults` returns suggested household profiles.

### `POST /api/v1/setup/agent`
`{"provider":UpsertProviderRequest}` → creates, activates and persists the
provider. Optional: the wizard may skip AI configuration.

### `POST /api/v1/setup/finish`
Starts the deployment job → `{"job_id":"…","message":"deployment started"}`.
Progress streams on `WS /api/v1/ws/setup` as
`{"type":"setup_progress","step":6,"label":"…","percent":40,"done":false}` and
finally `{"type":"setup_finished"}`. Idempotent: `409` if already running.

---

## System

| Method | Path | Permission |
|---|---|---|
| GET | `/system/overview` | dashboard.view |
| GET | `/system/version` | dashboard.view |
| GET | `/system/peripherals` | dashboard.view |
| GET | `/system/audio/sinks` | dashboard.view |
| GET | `/system/power/policy` | dashboard.view |
| GET | `/audit?limit=50` | audit.view |

`/system/overview`:
```json
{"hostname":"homeserver","os":"Ubuntu 24.04","kernel":"6.8","uptime_secs":1000,
 "cpu":{"usage_percent":12.5,"cores":8,"load_1":0.3,"load_5":0.2,"load_15":0.2},
 "memory":{"total_mb":16000,"used_mb":4200,"available_mb":11800,
           "swap_total_mb":2048,"swap_used_mb":0},
 "disks":[{"mount":"/","filesystem":"ext4","total_gb":512.0,"used_gb":80.0,
           "available_gb":432.0,"usage_percent":15.6,"kind":"ext4"}],
 "temperature_c":48.0,
 "battery":{"present":true,"percent":80,"charging":false,"time_remaining_min":null},
 "network":[{"name":"eth0","ipv4":["192.168.1.10"],"mac":"…","bytes_sent":1,"bytes_received":2}],
 "services":[{"name":"hs-daemon","active":true,"detail":null}],
 "generated_at":"…"}
```

`/audit` returns a bare array of `AuditEntry` sorted newest first:
```json
[{"seq":42,"timestamp":"…","actor":"admin","action":"instance.start",
  "target":"home-assistant","outcome":"success","ip":"192.168.1.20","details":null}]
```

---

## Instances (Incus or mock provider)

| Method | Path | Permission |
|---|---|---|
| GET | `/instances` | instances.read |
| POST | `/instances` | instances.manage |
| GET | `/instances/{name}` | instances.read |
| DELETE | `/instances/{name}` | instances.manage |
| PATCH | `/instances/{name}/limits` | instances.manage |
| POST | `/instances/{name}/start` | instances.manage |
| POST | `/instances/{name}/stop` | instances.manage |
| POST | `/instances/{name}/restart` | instances.manage |
| GET | `/instances/{name}/snapshots` | instances.read |
| POST | `/instances/{name}/snapshots` | instances.manage |
| POST | `/instances/{name}/snapshots/{snapshot}/restore` | instances.manage |
| DELETE | `/instances/{name}/snapshots/{snapshot}` | instances.manage |
| GET | `/instances/{name}/logs?lines=200` | instances.read |
| WS | `/instances/{name}/exec` | instances.manage |

Create body:
```json
{"name":"home-assistant","kind":"container","image":"ghcr.io/home-assistant/home-assistant:stable",
 "segment":"lan","limits":{"cpu_limit":2.0,"memory_mb":2048},"env":{"TZ":"America/Lima"},"start":true}
```
`kind` is `container` or `vm`; `segment` is `lan`, `dmz` or `null`; names must be
lowercase letters/digits/hyphens and at most 63 characters.

List response:
```json
{"instances":[{"name":"…","kind":"container","status":"running","image":"…",
  "ipv4_addresses":["10.0.0.5"],"limits":{"cpu_limit":2.0,"memory_mb":2048},
  "created_at":"…","snapshot_count":1}],
 "provider":{"name":"incus","available":true}}
```

Exec WebSocket (JSON messages):
- client → `{"type":"input","data":"ls -la /"}` or
  `{"type":"command","command":["ls","-la","/"]}`
- server → `{"type":"output","data":"…"}`, `{"type":"exit","code":0}`,
  `{"type":"error","message":"…"}`.

> The v1 exec transport runs one-shot commands (no PTY). Interactive shells are a
> roadmap item; the protocol is already message-based.

---

## Applications

| Method | Path | Permission |
|---|---|---|
| GET | `/apps/catalog` | apps.read |
| GET | `/apps` | apps.read |
| POST | `/apps` | apps.deploy |
| POST | `/apps/{id}/start` \| `/stop` | apps.deploy |
| DELETE | `/apps/{id}` | apps.deploy |

Catalog entries: `{id,name,description,image,default_port,category,segment,env[],featured}`.
Deploy body:
```json
{"name":"Nextcloud","template_id":"nextcloud","image":null,"segment":"dmz",
 "internal_port":80,"subdomain":"cloud","expose_externally":true,
 "env":{"TZ":"America/Lima"}}
```
`template_id` is optional; when omitted, `image` is required. LAN deployments get
`hostname` = `subdomain.base_domain` and the daemon writes a Caddy site snippet
(`/etc/caddy/apps.d/<id>.caddy`, `tls internal`, reverse proxy to the instance
IPv4) and reloads Caddy. DMZ deployments never get a LAN hostname; they are only
reachable through the outbound tunnel.

---

## Network, DNS and firewall

| Method | Path | Permission |
|---|---|---|
| GET | `/network` | network.read |
| PUT | `/network` | network.manage |
| PUT | `/network/dns` | network.manage |
| GET | `/network/dns/providers` | network.read |
| POST | `/network/dns/test` | network.read |
| PUT | `/network/dmz` | network.manage |
| GET | `/network/firewall` | network.read |
| POST | `/network/firewall/apply` | network.manage |

`GET /network` → `{"interfaces":[InterfaceInfo…],"config":NetworkConfig}`.

`PUT /network` updates the uplink/bridge/addressing configuration
(`{interface,lan_bridge?,mode,static_ipv4?}`) and validates the whole network
configuration. DNS/DMZ settings are updated by their own endpoints. The change
is persisted; the daemon applies the rendered netplan/firewall at first boot or
when the portal asks for it.

`PUT /network/dns` body is `DnsConfig`:
```json
{"enabled":true,"engine":"adguard_home","provider":"quad9","protocol":"dot",
 "custom_upstream":"dns.example.com","blocklists":["https://…"]}
```
Providers: `cloudflare`, `cloudflare_malware`, `cloudflare_family`, `quad9`,
`controld`, `custom`. Protocols: `doh`, `dot`. `custom_upstream` is required for
`custom` (http(s) URL for DoH, hostname for DoT).

`PUT /network/dmz` body is `DmzConfig`
`{enabled,bridge,subnet,gateway,allow_dns}` with subnet validation and gateway
inside the subnet.

`GET /network/firewall` → `{"ruleset":"#!/usr/sbin/nft …","last_applied_at":null}`.
`POST /network/firewall/apply` validates isolation, writes the ruleset and runs
`nft -f` (Linux). On non-Linux it returns `501 unsupported`.

`POST /network/dns/test` → `{"ok":true,"latency_ms":23,"provider":"quad9","error":null}`.

---

## IoT discovery

| Method | Path | Permission |
|---|---|---|
| GET | `/discovery/devices` | dashboard.view |
| POST | `/discovery/scan` | dashboard.view |
| POST | `/discovery/devices/{id}/adopt` | dashboard.view |

Devices: `{id,ip,mac,hostname,vendor,protocols:["mdns","ssdp","arp"],last_seen,adopted}`.
A scan may take a few seconds (mDNS/SSDP timeouts) and returns the merged list.

---

## Home Assistant

| Method | Path | Permission |
|---|---|---|
| GET | `/homeassistant/status` | dashboard.view |
| POST | `/homeassistant/provision` | apps.deploy |
| PUT | `/homeassistant/config` | agent.manage |
| GET | `/homeassistant/entities` | dashboard.view |
| GET | `/homeassistant/scenes` | dashboard.view |
| POST | `/homeassistant/services/{domain}/{service}` | dashboard.view |
| POST | `/homeassistant/scenes/{entity_id}/activate` | dashboard.view |

- Provision body `{name?,port?}` deploys the official HA container on the LAN and
  stores the URL. `409` on validation conflicts, `503` when the runtime is down.
- Config body `{url,token}` stores the long-lived token encrypted in the vault and
  configures the client live.
- Service call body `{"entity_id":"light.kitchen","brightness":128}` (the `data`
  wrapper is also accepted).
- When Home Assistant is not configured, read endpoints return `503`.

---

## Agent, tools and workflows

| Method | Path | Permission |
|---|---|---|
| GET | `/agent/providers` | agent.read |
| POST | `/agent/providers` | agent.manage |
| DELETE | `/agent/providers/{id}` | agent.manage |
| POST | `/agent/providers/{id}/activate` | agent.manage |
| GET | `/agent/tools` | agent.read |
| POST | `/agent/chat` | agent.manage |
| GET/POST | `/agent/workflows` | agent.read / agent.manage |
| PUT/DELETE | `/agent/workflows/{id}` | agent.manage |
| POST | `/agent/workflows/{id}/run` | agent.manage |

Provider upsert:
```json
{"kind":"deepseek","label":"DeepSeek","model":"deepseek-chat",
 "base_url":null,"api_key":"sk-…","enabled":true}
```
`kind`: `openai`, `anthropic`, `deepseek`, `opencode_go`, `custom` (custom and
opencode-go require `base_url`). The API key is encrypted in the vault and never
returned.

Chat:
```json
{"profile_id":"…","messages":[{"role":"user","content":"enciende la luz del salón"}],
 "use_tools":true}
```
Response:
```json
{"reply":"He encendido la luz.","model":"deepseek-chat",
 "tool_trace":[{"tool":"homeassistant.call_service","arguments":{…},
                "allowed":true,"output":{…},"error":null,"duration_ms":12}],
 "usage":{"prompt_tokens":120,"completion_tokens":30}}
```
Every tool is checked against the profile's `allowed_tools` **before** execution;
denied tools appear in the trace with `"allowed":false` and are never executed.

Workflow body: `{name,description,enabled,nodes[],edges[]}` where a node is one of
`{"id":"t","type":"trigger","trigger":{"type":"manual"}}`,
`{"id":"n1","type":"tool","tool":"music.play","arguments":{}}`,
`{"id":"n2","type":"condition","field":"state","equals":"on"}`,
`{"id":"n3","type":"delay","seconds":5}`. Run body: `{profile_id,input}`; response
`{workflow_id,success,steps[],output}`.

---

## Household profiles

| Method | Path | Permission |
|---|---|---|
| GET | `/profiles` | profiles.read |
| GET | `/profiles/defaults` | profiles.read |
| POST | `/profiles` | profiles.manage |
| GET/PUT/DELETE | `/profiles/{id}` | profiles.read / profiles.manage |
| PUT | `/profiles/{id}/tools` | profiles.manage |
| POST | `/profiles/{id}/activate` | kiosk.control |
| POST | `/profiles/{id}/enroll` | profiles.manage |
| DELETE | `/profiles/{id}/face` | profiles.manage |

Profile:
```json
{"id":"…","name":"Mamá","role":"adult",
 "avatar":{"shape":"droplet","color":"#fbbf24","eye_color":"#0b1220","eye_tilt_deg":8.0},
 "language":"es","tone":"friendly",
 "allowed_tools":["music.play","homeassistant.call_service"],
 "face_enrolled":false,"created_at":"…","updated_at":"…"}
```
Create body omits `id`, timestamps and `face_enrolled`. Enrollment body:
`{"embedding":[0.12,-0.03,…]}` (produced locally by the optional InsightFace
helper); embeddings are encrypted and matched in Rust with cosine similarity.

---

## Kiosk API (`127.0.0.1:3000`, kiosk token required)

The kiosk page sets an HttpOnly `hs_kiosk` cookie. API clients may send
`X-Kiosk-Token: <token>` instead. All paths are under `/api/v1`.

| Method | Path | Purpose |
|---|---|---|
| GET | `/kiosk/state` | snapshot: device, profile, avatar state, tracking, music, network, scenes |
| POST | `/kiosk/profile/activate` | `{profile_id}` |
| POST | `/kiosk/listening` | `{active}` (avatar listening state) |
| POST | `/kiosk/agent/query` | `{text}` → agent response (uses the active profile) |
| POST | `/kiosk/music/play` \| `/pause` | first Home Assistant `media_player` |
| POST | `/kiosk/music/volume` | `{volume}` 0–100 (PipeWire) |
| POST | `/kiosk/scenes/{entity_id}/activate` | HA `scene.turn_on` |
| POST | `/kiosk/recognize` | `{embedding:[…],threshold?}` → local cosine match + profile activation |
| WS | `/kiosk/ws` | snapshot then `KioskEvent` stream |

`KioskEvent` (tagged by `type`):
```json
{"type":"avatar","state":"tracking","profile_id":"…"}
{"type":"tracking","point":{"x":0.42,"y":0.55,"confidence":0.87}}
{"type":"profile_activated","profile":{…}}
{"type":"music","state":{"playing":true,"track":"…","artist":"…","volume":60,"sink":"…"}}
{"type":"agent","activity":"thinking"}
{"type":"network","status":{"online":true,"dns_ok":true,"latency_ms":12}}
{"type":"notification","level":"warning","message":"…"}
```
`tracking.point` is `null` when nobody is detected. The kiosk only receives
kiosk-safe operations; it cannot manage users, network, firewall or delete
instances.

---

## Admin WebSockets

| Path | Purpose |
|---|---|
| `/api/v1/ws/events` | metrics every 3 s, instance changes, discovery count, audit, notifications |
| `/api/v1/ws/setup` | onboarding progress + `setup_finished` |

Both require a session (`dashboard.view`). Clients should fetch the current state
over REST first and use the socket for live deltas; slow consumers are dropped
and must reconnect (exponential backoff).

Message types: `metrics` (`overview`), `instance` (`name`,`status`),
`setup_progress` (`step`,`label`,`percent`,`done`), `setup_finished`,
`discovery` (`count`), `audit` (`entry`), `notification` (`level`,`message`).

---

## Status codes

| Code | Meaning |
|---|---|
| 200 | Success |
| 400 | Validation / malformed body |
| 401 | Missing, expired or revoked session |
| 403 | RBAC denial or missing/invalid CSRF |
| 404 | Entity not found |
| 409 | State conflict (already initialized/running, duplicate name) |
| 429 | Login throttled |
| 501 | Operation unsupported on this platform (nft/netplan on non-Linux) |
| 502 | Upstream failure (LLM, Home Assistant) |
| 503 | Provider unavailable (Incus, Home Assistant not configured) |
| 504 | Runtime operation timeout |
