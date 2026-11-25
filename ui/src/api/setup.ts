import { http } from './client';
import type {
  AudioTestResponse,
  SetupAdminRequest,
  SetupAgentRequest,
  SetupFinishResponse,
  SetupNetworkRequest,
  SetupPeripherals,
  SetupProfilesRequest,
} from './types';

/**
 * First-boot endpoints. They are only available while the appliance is not
 * initialized; the UI gates the wizard accordingly.
 */
export const setupApi = {
  createAdmin: (payload: SetupAdminRequest): Promise<void> =>
    http.post<void>('/setup/admin', payload),

  configureNetwork: (payload: SetupNetworkRequest): Promise<void> =>
    http.post<void>('/setup/network', payload),

  peripherals: (): Promise<SetupPeripherals> => http.get<SetupPeripherals>('/setup/peripherals'),

  testAudio: (sinkId: string): Promise<AudioTestResponse> =>
    http.post<AudioTestResponse>('/setup/audio/test', { sink_id: sinkId }),

  createProfiles: (payload: SetupProfilesRequest): Promise<void> =>
    http.post<void>('/setup/profiles', payload),

  /** All fields are optional: the assistant step can be skipped. */
  configureAgent: (payload: SetupAgentRequest): Promise<void> =>
    http.post<void>('/setup/agent', payload),

  finish: (): Promise<SetupFinishResponse> => http.post<SetupFinishResponse>('/setup/finish'),
};
