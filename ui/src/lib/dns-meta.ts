import type { DmzConfig, DnsConfig } from '@/api/types';

/** Mirrors `DnsConfig::default()` in hs-core. */
export const DEFAULT_DNS_CONFIG: DnsConfig = {
  enabled: true,
  engine: 'adguard_home',
  provider: 'cloudflare_family',
  protocol: 'doh',
  custom_upstream: null,
  blocklists: [
    'https://adguardteam.github.io/HostlistsRegistry/assets/filter_1.txt',
    'https://adguardteam.github.io/HostlistsRegistry/assets/filter_2.txt',
  ],
};

/** Mirrors `DmzConfig::default()` in hs-core. */
export const DEFAULT_DMZ_CONFIG: DmzConfig = {
  enabled: false,
  bridge: 'br-dmz',
  subnet: '10.20.30.0/24',
  gateway: '10.20.30.1',
  allow_dns: true,
};
