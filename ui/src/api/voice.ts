import { http } from './client';
import type { VoiceConfig, VoiceConfigResponse, VoiceStatus } from './types';

/** Always-on voice assistant configuration and runtime status. */
export const voiceApi = {
  config: (): Promise<VoiceConfigResponse> => http.get<VoiceConfigResponse>('/voice/config'),

  status: (): Promise<VoiceStatus> => http.get<VoiceStatus>('/voice/status'),

  update: (config: VoiceConfig): Promise<VoiceConfigResponse> =>
    http.put<VoiceConfigResponse>('/voice/config', config),
};
