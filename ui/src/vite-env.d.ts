/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Base URL of the daemon API. Empty string means same origin (dev proxy). */
  readonly VITE_API_BASE?: string;
  /** Optional explicit WebSocket base URL. */
  readonly VITE_WS_BASE?: string;
  /** Kiosk shared secret injected by the kiosk launcher. */
  readonly VITE_KIOSK_TOKEN?: string;
  /** `1` enables the visual demo mode (kiosk only). */
  readonly VITE_DEMO?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
