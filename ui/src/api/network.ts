import { http } from './client';
import type {
  DmzConfig,
  DnsConfig,
  DnsProviderInfo,
  DnsTestResponse,
  FirewallApplyResponse,
  FirewallStatusResponse,
  NetworkMode,
  NetworkViewResponse,
  StaticIpv4,
} from './types';

export interface UpdateNetworkRequest {
  interface: string;
  /** Advanced: LAN bridge managed by the appliance. */
  lan_bridge?: string;
  mode: NetworkMode;
  static_ipv4: StaticIpv4 | null;
}

/** Network, DNS, DMZ and firewall management. */
export const networkApi = {
  view: (): Promise<NetworkViewResponse> => http.get<NetworkViewResponse>('/network'),

  /**
   * Interface addressing is part of the network config. The daemon exposes the
   * DNS and DMZ updaters explicitly; this mirrors the same `SystemConfig.network`
   * shape at `PUT /network` for the addressing form.
   */
  updateNetwork: (payload: UpdateNetworkRequest): Promise<void> =>
    http.put<void>('/network', payload),

  updateDns: (payload: DnsConfig): Promise<void> => http.put<void>('/network/dns', payload),

  updateDmz: (payload: DmzConfig): Promise<void> => http.put<void>('/network/dmz', payload),

  firewall: (): Promise<FirewallStatusResponse> =>
    http.get<FirewallStatusResponse>('/network/firewall'),

  applyFirewall: (): Promise<FirewallApplyResponse> =>
    http.post<FirewallApplyResponse>('/network/firewall/apply'),

  testDns: (): Promise<DnsTestResponse> => http.post<DnsTestResponse>('/network/dns/test'),

  /** Complete free-DNS catalog (IPv4/IPv6/DoH/DoT), grouped by vendor in the UI. */
  dnsProviders: (): Promise<DnsProviderInfo[]> =>
    http.get<DnsProviderInfo[]>('/network/dns/providers'),
};
