import { http } from './client';import type {
  AgentChatResponse,
  KioskStateResponse,
  MusicState,
} from './types';
import { readStored, writeStored } from '../lib/preferences';

/**
 * The kiosk API is loopback-only and authenticated with a shared token instead
 * of the admin session cookie. The token is injected by the kiosk launcher at
 * build time (`VITE_KIOSK_TOKEN`) or overridden in localStorage for local
 * development.
 */
export const KIOSK_TOKEN_KEY = 'hs.kioskToken';

export function kioskToken(): string {
  const injected = (import.meta.env.VITE_KIOSK_TOKEN ?? '').trim();
  if (injected !== '') return injected;
  return readStored(KIOSK_TOKEN_KEY) ?? '';
}

export function setKioskTokenOverride(token: string | null): void {
  if (token === null || token.trim() === '') {
    writeStored(KIOSK_TOKEN_KEY, '');
  } else {
    writeStored(KIOSK_TOKEN_KEY, token.trim());
  }
}

function authHeaders(): Record<string, string> {
  const token = kioskToken();
  return token === '' ? {} : { 'X-Kiosk-Token': token };
}

/** Kiosk-facing endpoints; all calls carry the kiosk token header. */
export interface KioskHistoryEntry {
  role: 'user' | 'assistant';
  content: string;
}

export const kioskApi = {
  state: (): Promise<KioskStateResponse> =>
    http.get<KioskStateResponse>('/kiosk/state', { headers: authHeaders() }),

  activateProfile: (profileId: string): Promise<void> =>
    http.post<void>('/kiosk/profile/activate', { profile_id: profileId }, { headers: authHeaders() }),

  setListening: (active: boolean): Promise<void> =>
    http.post<void>('/kiosk/listening', { active }, { headers: authHeaders() }),

  query: (text: string, history: KioskHistoryEntry[] = []): Promise<AgentChatResponse> =>
    http.post<AgentChatResponse>(
      '/kiosk/agent/query',
      { text, history },
      { headers: authHeaders() },
    ),

  musicPlay: (): Promise<MusicState | void> =>
    http.post<MusicState | void>('/kiosk/music/play', undefined, { headers: authHeaders() }),

  musicPause: (): Promise<MusicState | void> =>
    http.post<MusicState | void>('/kiosk/music/pause', undefined, { headers: authHeaders() }),

  setVolume: (volume: number): Promise<void> =>
    http.post<void>('/kiosk/music/volume', { volume }, { headers: authHeaders() }),

  activateScene: (entityId: string): Promise<void> =>
    http.post<void>(
      `/kiosk/scene/${encodeURIComponent(entityId)}/activate`,
      undefined,
      { headers: authHeaders() },
    ),
};



const STT_BASE_URL = (
  (import.meta.env as Record<string, string | undefined>).VITE_STT_URL ??
  'http://127.0.0.1:5055'
).replace(/\/$/, '');

/**
 * Send a browser microphone recording to the loopback STT server.
 *
 * Chrome/MediaRecorder uses the same Windows capture path as Audacity, so it
 * works with the G435 where PortAudio's DirectSound/MME streams were corrupt.
 */
export async function transcribeKioskAudio(blob: Blob): Promise<string> {
  const response = await fetch(`${STT_BASE_URL}/transcribe`, {
    method: 'POST',
    headers: { 'Content-Type': blob.type || 'application/octet-stream' },
    body: blob,
  });
  if (!response.ok) {
    throw new Error(`local STT failed (HTTP ${response.status})`);
  }
  const data = (await response.json()) as { text?: string; error?: string };
  if (data.error) throw new Error(data.error);
  return (data.text ?? '').trim();
}
