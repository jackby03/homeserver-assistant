import { http } from './client';
import type { AdoptDeviceResponse, DiscoveryResponse } from './types';

/** LAN discovery and Home Assistant adoption. */
export const discoveryApi = {
  devices: (): Promise<DiscoveryResponse> => http.get<DiscoveryResponse>('/discovery/devices'),

  scan: (): Promise<DiscoveryResponse> => http.post<DiscoveryResponse>('/discovery/scan'),

  adopt: (id: string): Promise<AdoptDeviceResponse> =>
    http.post<AdoptDeviceResponse>(`/discovery/devices/${encodeURIComponent(id)}/adopt`),
};
