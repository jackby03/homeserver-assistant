import type { KioskStateResponse, Profile } from '@/api/types';

const now = new Date().toISOString();

/** Visual-review data used when `?demo=1` or `VITE_DEMO=1`. Never mutated. */
export const DEMO_PROFILES: Profile[] = [
  {
    id: 'demo-ana',
    name: 'Ana',
    role: 'adult',
    avatar: { shape: 'squircle', color: '#6ee7ff', eye_color: '#0b1220', eye_tilt_deg: 0 },
    language: 'es',
    tone: 'friendly',
    allowed_tools: ['music.play', 'home.scene', 'agent.chat'],
    face_enrolled: true,
    created_at: now,
    updated_at: now,
  },
  {
    id: 'demo-leo',
    name: 'Leo',
    role: 'child',
    avatar: { shape: 'cloud', color: '#fbbf24', eye_color: '#20140a', eye_tilt_deg: 7 },
    language: 'es',
    tone: 'playful',
    allowed_tools: ['music.play', 'home.scene'],
    face_enrolled: true,
    created_at: now,
    updated_at: now,
  },
  {
    id: 'demo-visita',
    name: 'Visita',
    role: 'guest',
    avatar: { shape: 'droplet', color: '#a5b4fc', eye_color: '#0b1220', eye_tilt_deg: -5 },
    language: 'es',
    tone: 'calm',
    allowed_tools: [],
    face_enrolled: false,
    created_at: now,
    updated_at: now,
  },
];

export const DEMO_KIOSK_STATE: KioskStateResponse = {
  device_name: 'HomeServer Assistant',
  profile: DEMO_PROFILES[0] ?? null,
  music: {
    playing: true,
    track: 'Clair de Lune',
    artist: 'Debussy',
    volume: 42,
    sink: 'Salón · Home Theater',
  },
  scenes: [
    { entity_id: 'scene.cine', name: 'Cine' },
    { entity_id: 'scene.noche', name: 'Buenas noches' },
    { entity_id: 'scene.despertar', name: 'Despertar' },
    { entity_id: 'scene.lectura', name: 'Lectura' },
  ],
  network: { online: true, dns_ok: true, latency_ms: 14 },
  agent_activity: 'idle',
  assistant_name: 'Aura',
  voice: {
    wake_word_enabled: true,
    activation_mode: 'both',
    listening: false,
    listening_window_secs: 8,
  },
};
