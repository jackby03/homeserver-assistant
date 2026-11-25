import type {
  Avatar,
  DnsProvider,
  NetworkMode,
  ProfileRole,
  Tone,
} from '@/api/types';
import type { UiLanguage } from '@/i18n/dictionary';

/** Patch yielded by a wizard step when it advances. */
export interface SetupSummaryPatch {
  deviceName?: string;
  language?: UiLanguage;
  mode?: NetworkMode;
  dnsProvider?: DnsProvider;
  dnsEnabled?: boolean;
  profileCount?: number;
  agentLabel?: string | null;
}

export interface SetupSummary {
  deviceName: string;
  language: UiLanguage;
  mode: NetworkMode;
  dnsProvider: DnsProvider;
  dnsEnabled: boolean;
  profileCount: number;
  agentLabel: string | null;
}

export interface StepProps {
  onNext: (patch?: SetupSummaryPatch) => void;
  onBack: () => void;
}

export interface DraftProfile {
  id: number;
  name: string;
  role: ProfileRole;
  tone: Tone;
  avatar: Avatar;
}

let draftProfileId = 1;

export function nextProfileId(): number {
  const id = draftProfileId;
  draftProfileId += 1;
  return id;
}

export const DEFAULT_PROFILE_AVATARS: Array<{ role: ProfileRole; tone: Tone; avatar: Avatar }> = [
  {
    role: 'owner',
    tone: 'friendly',
    avatar: { shape: 'squircle', color: '#6ee7ff', eye_color: '#0b1220', eye_tilt_deg: 0 },
  },
  {
    role: 'adult',
    tone: 'friendly',
    avatar: { shape: 'circle', color: '#34d399', eye_color: '#0b1220', eye_tilt_deg: 0 },
  },
  {
    role: 'child',
    tone: 'playful',
    avatar: { shape: 'cloud', color: '#fbbf24', eye_color: '#0b1220', eye_tilt_deg: 6 },
  },
  {
    role: 'guest',
    tone: 'calm',
    avatar: { shape: 'droplet', color: '#a5b4fc', eye_color: '#0b1220', eye_tilt_deg: -4 },
  },
];

/** Profile templates used when the household skips the profile step. */
export function defaultProfiles(ownerName: string, guestName: string): DraftProfile[] {
  const [owner, , , guest] = DEFAULT_PROFILE_AVATARS;
  const make = (
    name: string,
    template: { role: ProfileRole; tone: Tone; avatar: Avatar },
  ): DraftProfile => ({
    id: nextProfileId(),
    name,
    role: template.role,
    tone: template.tone,
    avatar: template.avatar,
  });
  return [
    make(ownerName, owner ?? DEFAULT_PROFILE_AVATARS[0]!),
    make(guestName, guest ?? DEFAULT_PROFILE_AVATARS[3]!),
  ];
}
