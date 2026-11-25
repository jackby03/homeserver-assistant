# HomeServer Assistant OS

**Un home server de código abierto, local-first, instalable desde una ISO y
pensado para vivir montado en la pared.** Combina un panel web de administración,
virtualización con Incus (contenedores LXC, contenedores OCI y VMs), DNS cifrado
con filtrado, firewall con DMZ aislada, Home Assistant, un agente de IA con
herramientas y una interfaz táctil tipo HomePod/Echo Show con avatar animado y
visión 100 % local.

> **Estado:** MVP funcional verificado en el entorno de desarrollo. Backend Rust
> con **400+ tests**, `clippy -D warnings` limpio, UI compilada y probada,
> infraestructura/ISO validada estáticamente (su ejecución real requiere Linux).
> Ver [Estado del proyecto](#estado-del-proyecto).

> **Idioma:** la documentación principal está en español; el código, los
> comentarios, los logs y los mensajes de consola están en inglés (regla de
> `AGENTS.md`). La UI está internacionalizada en `es` (por defecto) y `en`.

---

## Tabla de contenidos

1. [Qué incluye](#qué-incluye)
2. [Arquitectura en 30 segundos](#arquitectura-en-30-segundos)
3. [Stack técnico](#stack-técnico)
4. [Estructura del repositorio](#estructura-del-repositorio)
5. [Desarrollo rápido](#desarrollo-rápido)
6. [Primer arranque](#primer-arranque)
7. [Seguridad](#seguridad)
8. [Puertos y servicios](#puertos-y-servicios)
9. [Configuración y datos](#configuración-y-datos)
10. [Estado del proyecto](#estado-del-proyecto)
11. [Documentación](#documentación)
12. [Licencia](#licencia)

---

## Qué incluye

### Panel de administración (HTTPS + login)

- **Dashboard**: CPU, RAM, discos (ZFS/Btrfs), temperatura, batería/UPS,
  tráfico de red y salud de servicios, en tiempo real por WebSocket.
- **Cómputo**: listar/crear/iniciar/parar/reiniciar/eliminar instancias de
  Incus (LXC, OCI/Docker y VMs QEMU/KVM), límites de CPU/RAM, snapshots y
  restauración en un clic, logs y terminal web.
- **Apps**: catálogo de plantillas (Home Assistant, AdGuard Home, Music
  Assistant, Immich, Nextcloud, Vaultwarden, Uptime Kuma, n8n), despliegue en
  `LAN` o `DMZ`, variables de entorno, subdominio y proxy inverso.
- **Home Assistant + IoT**: aprovisionamiento, estado, entidades, escenas y
  adopción de dispositivos descubiertos por mDNS/SSDP/ARP.
- **Red y seguridad**: IP estática/DHCP, **catálogo de 40 variantes DNS gratuitas**
  (default **Cloudflare Family**; Cloudflare, Google, AdGuard, Quad9, CleanBrowsing, OpenDNS, NextDNS, Comodo,
  Control D con 13 listas comunitarias, DNS.WATCH o custom) con IPv4/IPv6,
  **DoH/DoT** (y DNS sin cifrar solo para proveedores heredados), listas de
  bloqueo, subred DMZ con `nftables` (aislada de la LAN y del puerto de
  administración pero con salida a Internet enmascarada para actualizaciones y
  túneles), y túneles salientes (Cloudflare Tunnel / WireGuard).
- **IA y perfiles**: proveedores OpenAI, Anthropic, DeepSeek, opencode-go y
  endpoints compatibles, claves cifradas, herramientas con permisos por perfil,
  workflows tipo nodo y perfiles familiares con avatar y biometría local.
- **Voz en el kiosko**: wake word continuo «Hola Aura» con Vosk, STT local
  (Vosk/faster-whisper), Piper local y micrófono de Chrome; sin depender de la
  interfaz PortAudio/WDM del host de desarrollo.
- **Asistente local-first**: router de comandos en español que resuelve sin IA
  la hora/fecha, el clima actual de Lima u otra ciudad vía Open-Meteo, el
  bienestar, volumen, música, luces y escenas de Home Assistant, cálculos,
  chistes, identidad y ayuda. Solo las preguntas que no cubre pasan al
  LLM configurado (con permisos por perfil también para las acciones locales).
  El clima es la única llamada saliente de este router y solo se hace cuando
  el usuario pregunta por el tiempo; no se envía audio ni datos personales.
- **Asistente de voz siempre activo**: se activa llamándolo por su nombre (nombre
  personalizable, por defecto `Aura`), con sensibilidad, ventana de escucha y
  modo nombre/botón configurables; el reconocimiento de voz y las transcripciones
  se procesan **100 % en local**.

### Kiosko de pared (pantalla completa)

- **Avatar SVG animado** con siluetas `circle`, `squircle`, `cloud`, `cone`,
  `droplet` y dos ojos en cápsula; color, color de ojos e inclinación
  configurables por perfil.
- **Estados reactivos**: `idle`, `tracking` (los ojos siguen a la persona con
  coordenadas normalizadas), `listening`, `thinking`, `speaking` y
  `profile_switch` al reconocer a un miembro de la familia (el helper local
  calcula el embedding y `POST /kiosk/recognize` hace el matching y activa el
  perfil).
- **Controles rápidos**: música al equipo de sonido, escenas/luces de Home
  Assistant, actividad del agente y estado de red.
- **Wizard de primer arranque de 6 pasos**, también accesible desde el móvil o
  el PC de la LAN.

### Visión y audio locales

- `hs-vision` (Rust) lee la cámara, detecta movimiento/persona y emite
  coordenadas `(x, y)` normalizadas por un socket Unix. **Los fotogramas nunca
  salen del dispositivo ni se guardan.**
- Reconocimiento facial opcional con InsightFace local para generar embeddings;
  la comparación y el umbral viven en Rust (`hs-profiles`) y los embeddings se
  guardan cifrados.
- Audio vía PipeWire/WirePlumber con salida al equipo de sonido externo (jack,
  USB DAC o Bluetooth).

---

## Arquitectura en 30 segundos

Cuatro planos acoplados por APIs locales, un solo demonio privilegiado:

```
┌─────────────────────────────────────────────────────────────────────┐
│ 4 · Inteligencia: agente LLM + herramientas + perfiles + visión     │
├─────────────────────────────────────────────────────────────────────┤
│ 3 · Red/seguridad/IoT: DNS DoH/DoT, nftables, DMZ, Home Assistant   │
├─────────────────────────────────────────────────────────────────────┤
│ 2 · Cómputo: RuntimeProvider + Incus (LXC/OCI/VMs) + despliegue     │
├─────────────────────────────────────────────────────────────────────┤
│ 1 · Host: métricas, energía, audio, cámara, persistencia, auth      │
└─────────────────────────────────────────────────────────────────────┘
             todo compuesto por hs-daemon (Rust, systemd)
```

- `hs-daemon` escucha **solo en loopback**: API admin `127.0.0.1:8080` y kiosko
  `127.0.0.1:3000`. **Caddy** termina TLS en `:8443` y sirve el portal con CA
  local.
- 13 crates Rust con fronteras estrictas y puertos/adaptadores; `hs-core` es
  hoja. Ver [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).
- UI React + TypeScript estricto + Tailwind, una sola SPA con tres
  experiencias: `/setup`, `/admin`, `/kiosk`.

---

## Stack técnico

| Capa | Tecnología |
|---|---|
| Backend | Rust 2021 (tokio, axum, tower, redb, serde, tracing) |
| Auth | Argon2id, JWT HS256 propio, sesiones revocables, RBAC, CSRF |
| Cómputo | Incus REST sobre socket Unix (LXC, OCI, QEMU/KVM, ZFS/Btrfs) |
| Red | nftables, AdGuard Home o Technitium, DoH/DoT, Caddy, Cloudflare Tunnel/WireGuard |
| Domótica | Home Assistant REST + descubrimiento mDNS/SSDP/ARP |
| IA | Adaptadores OpenAI-compatible + Anthropic, tool calling, workflows |
| Visión | Rust + V4L2; helper Python opcional (InsightFace) para embeddings |
| UI | React 19, TypeScript strict, Vite, Tailwind |
| Empaquetado | Ubuntu 24.04 autoinstall + xorriso + QEMU smoke test |

---

## Estructura del repositorio

```text
homeserver-assistant/
├── AGENTS.md                    # reglas de ingeniería y seguridad (obligatorias)
├── Cargo.toml                   # workspace Rust (13 crates)
├── Makefile                     # build-daemon, build-ui, build-vision, test, iso…
├── CHANGELOG.md / CONTRIBUTING.md
├── .github/workflows/ci.yml     # CI: fmt, clippy, tests, UI, scripts
├── crates/
│   ├── hs-core/                 # config, modelos, redb, vault, eventos
│   ├── hs-auth/                 # Argon2id, JWT, sesiones, RBAC, CSRF, auditoría
│   ├── hs-system/               # métricas, energía, audio, cámara/táctil
│   ├── hs-runtime/              # RuntimeProvider + Incus + MockRuntime
│   ├── hs-apps/                 # catálogo y despliegue de aplicaciones
│   ├── hs-network/              # DNS DoH/DoT, nftables, DMZ, netplan
│   ├── hs-discovery/            # mDNS, SSDP/UPnP, ARP + OUI
│   ├── hs-homeassistant/        # cliente REST y aprovisionamiento
│   ├── hs-agent/                # adaptadores LLM y bucle agéntico
│   ├── hs-tools/                # registro de herramientas, permisos, workflows
│   ├── hs-profiles/             # perfiles, avatares, biometría local
│   ├── hs-vision/               # servicio de seguimiento local (+ binario)
│   └── hs-daemon/               # API HTTP/WS, kiosko, adaptadores y composition root
├── ui/                          # SPA React (wizard, admin, kiosko)
├── voice/                       # listener local de wake word (STT local opcional)
├── vision/python/               # helper opcional InsightFace (local)
├── infra/                       # incus preseed, nftables, systemd, servicios
├── iso/                         # autoinstall Ubuntu + build-iso.sh + test QEMU
├── docs/                        # arquitectura, API, seguridad, despliegue, ADRs
└── scripts/                     # instalador y validadores de assets
```

---

## Desarrollo rápido

Requisitos: **Rust 1.85+**, **Node 22+**, **pnpm 9+**. Para Incus/nftables/ISO
hace falta Linux; en Windows/macOS el daemon corre con `--dev` usando fakes.

```bash
# 1) Dependencias (una vez; luego se puede compilar offline)
cargo fetch
cd ui && pnpm install && cd ..

# 2) Calidad del backend
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline --lib --tests

# 3) UI: type-check, tests y build de producción
cd ui && pnpm typecheck && pnpm test && pnpm build && cd ..

# 4) Assets de infraestructura/ISO
python3 scripts/check-assets.py

# 5) Daemon en modo desarrollo (runtime y hardware simulados)
cargo run -p hs-daemon --offline -- serve --dev \
  --data-dir ./.hs-data --static-dir ui/dist
#   API admin:  http://127.0.0.1:8080
#   Kiosko:     http://127.0.0.1:3000/kiosk

# 6) UI en modo desarrollo (opcional)
cd ui && pnpm dev     # Vite en :5173 con proxy a :8080

# 7) ISO instalable (solo Linux, requiere xorriso)
make iso && make test-vm
```

Más detalles en [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md).

---

## Verificación del repositorio

Resultados de la última ejecución en el entorno de desarrollo (Windows):

| Comprobación | Resultado |
|---|---|
| `cargo test --workspace --offline --lib --tests` | **442 tests, 0 fallos** (17 binarios de test) |
| `cargo clippy --workspace --all-targets -- -D warnings` | limpio |
| `cargo fmt --all -- --check` | limpio |
| `pnpm typecheck` / `pnpm test` / `pnpm build` | limpio / 20 tests / `ui/dist` 0,91 MB |
| `python3 scripts/check-assets.py` | 75 comprobaciones, 0 fallos |
| Smoke del daemon real (`serve --dev`) | `/api/v1/health`, `/setup/status`, UI admin y `/kiosk` con cookie responden 200 |
| ShellCheck de los scripts (por el agente de infraestructura) | 0 avisos en 7 scripts |
| ISO/QEMU, `nft -c`, `netplan apply`, Incus real | requieren un host Linux; comandos documentados en `docs/ISO.md` |

Los tests cubren seguridad (JWT manipulado, CSRF, RBAC, bloqueo de login,
re-vocación de sesiones, aislamiento DMZ, permisos de herramientas), persistencia
cifrada, degradación de servicios y los flujos HTTP completos del daemon.

---

## Primer arranque

1. Arranca la ISO en el portátil/pantalla de pared. El sistema instala,
   provisiona Incus y arranca `hs-daemon`, `hs-vision` y el kiosko.
2. Abre `https://<ip-del-servidor>:8443` desde el móvil o el PC y acepta la CA
   local (instrucciones en [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md)).
3. El **wizard de 6 pasos** configura: cuenta de administrador e idioma, red +
   DNS cifrado + DMZ, audio/táctil/cámara, descubrimiento IoT, proveedor de IA y
   perfiles familiares, y por último despliega los servicios con progreso en
   vivo.
4. Al terminar, `initialized = true` y el kiosko toma la pantalla completa.

---

## Seguridad

Resumen (detalle completo en [`docs/SECURITY.md`](docs/SECURITY.md)):

- Contraseñas **Argon2id**; sesiones JWT con registro servidor y revocación
  inmediata; cookies `HttpOnly` + `SameSite=Strict`; **CSRF** obligatorio para
  peticiones mutantes con cookie; bloqueo de fuerza bruta.
- **RBAC** `admin` / `operator` / `viewer` comprobado en rutas y servicios.
- Secretos (API keys, tokens, biometría) cifrados con **ChaCha20-Poly1305**;
  clave maestra en `master.key` con permisos `0600`.
- **DMZ aislada** por `nftables`: sin ruta a la LAN ni al puerto de
  administración; reglas con tests golden.
- **TLS terminado en Caddy** con CA local; el demonio nunca se expone a la LAN.
- Visión y biometría 100 % locales; el agente solo ejecuta herramientas
  permitidas explícitamente en el perfil activo.

---

## Puertos y servicios

| Puerto | Bind | Servicio |
|---|---|---|
| `8443/tcp` | LAN | Caddy: portal de administración (TLS, CA local) |
| `8080/tcp` | `127.0.0.1` | API REST/WS del daemon |
| `3000/tcp` | `127.0.0.1` | Kiosko (UI + API de kiosko) |
| `53/udp,tcp` | LAN | AdGuard Home (DNS filtrado) |
| `8123/tcp` | LAN | Home Assistant |
| `22/tcp` | LAN (opcional) | SSH (deshabilitado por defecto en la ISO) |
| socket Unix | — | `/run/homecore/vision.sock` |

---

## Configuración y datos

- Directorio de datos: `/var/lib/homecore` (`HS_DATA_DIR` para cambiarlo).
  - `config.json` — configuración validada y versionada.
  - `state.redb` — usuarios, sesiones, perfiles, apps, proveedores, workflows,
    auditoría y secretos cifrados.
  - `master.key` — clave maestra del vault (¡backup offline!).
- Variables de entorno: `HS_DATA_DIR`, `HS_ADMIN_BIND`, `HS_KIOSK_BIND`,
  `HS_STATIC_DIR`, `HS_VISION_SOCKET`, `HS_CADDY_APPS_DIR`, `HS_CADDY_ENV_FILE`,
  `RUST_LOG`.
- Reset de desarrollo: `make reset-dev` (borra `./.hs-data`).

---

## Estado del proyecto

| Área | Estado |
|---|---|
| Núcleo, configuración, persistencia cifrada | ✅ Implementado y probado (redb + ChaCha20-Poly1305) |
| Autenticación, RBAC, CSRF, auditoría | ✅ Implementado y probado (Argon2id, JWT con revocación) |
| API admin + kiosko, wizard, WebSockets | ✅ Implementado; 18 tests de integración HTTP |
| Motor Incus, snapshots, apps | ✅ Adaptador nativo + mock; validación real en host Linux |
| Despliegue de apps con Caddy | ✅ Snippets `apps.d` por app, recarga automática |
| DNS DoH/DoT, nftables, DMZ, netplan | ✅ Catálogo de 40 variantes, renderizado/golden tests; aplicación real en Linux |
| Home Assistant + descubrimiento IoT | ✅ Implementado con transportes simulados en tests |
| Asistente de voz local (wake word) | ✅ Nombre configurable, matcher Rust con tests, helper STT local opcional |
| Agente IA, herramientas, workflows, perfiles | ✅ 10 herramientas con permisos por perfil y trazas |
| Visión local | ✅ Seguimiento en Rust + helper Python opcional + matching en daemon |
| UI (admin, kiosko, wizard) | ✅ Type-check, 15 tests y build de producción (0,90 MB) |
| Infraestructura, systemd, ISO, QEMU | 🟡 Plantillas y scripts validados; build/prueba real en host Linux |
| TLS nativo en el daemon | ❌ Decisión de diseño: lo termina Caddy (ver ADR 0003) |
| Terminal PTY interactiva en Incus | 🟡 Ejecución de comandos uno-a-uno por WebSocket; PTY pendiente |
| Actualizaciones OTA firmadas | 🟡 Documentado como mantenimiento manual; canal OTA pendiente |

---

## Documentación

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — planos, crates, flujos, datos.
- [`docs/API.md`](docs/API.md) — referencia REST/WS.
- [`docs/SECURITY.md`](docs/SECURITY.md) — modelo de amenazas y controles.
- [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) — hardware, instalación, TLS, backups.
- [`docs/ISO.md`](docs/ISO.md) — construcción y prueba de la ISO.
- [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) — entorno de desarrollo y tests.
- [`voice/README.md`](voice/README.md) — wake word local y STT sin nube.
- [`docs/adr/`](docs/adr/) — decisiones de arquitectura.
- [`AGENTS.md`](AGENTS.md) — reglas para contribuir (humanos y agentes de IA).

---

## Licencia

MIT — ver [`LICENSE`](LICENSE).










