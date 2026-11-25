import type { DmzConfig, DnsConfig, DnsProvider } from '@/api/types';
import type { DictKey } from '@/i18n/dictionary';

/** DNS provider ordering and copy shared by the admin page and the wizard. */
export const DNS_PROVIDERS: DnsProvider[] = [
  'cloudflare',
  'cloudflare_malware',
  'cloudflare_family',
  'quad9',
  'controld',
  'custom',
];

export const DNS_PROVIDER_LABEL_KEYS: Record<DnsProvider, DictKey> = {
  cloudflare: 'net.provider.cloudflare',
  cloudflare_malware: 'net.provider.cloudflare_malware',
  cloudflare_family: 'net.provider.cloudflare_family',
  quad9: 'net.provider.quad9',
  controld: 'net.provider.controld',
  custom: 'net.provider.custom',
};

export const DNS_PROVIDER_DESC_KEYS: Record<DnsProvider, DictKey> = {
  cloudflare: 'net.provider.desc.cloudflare',
  cloudflare_malware: 'net.provider.desc.cloudflare_malware',
  cloudflare_family: 'net.provider.desc.cloudflare_family',
  quad9: 'net.provider.desc.quad9',
  controld: 'net.provider.desc.controld',
  custom: 'net.provider.desc.custom',
};

/** Mirrors `DnsConfig::default()` in hs-core. */
export const DEFAULT_DNS_CONFIG: DnsConfig = {
  enabled: true,
  engine: 'adguard_home',
  provider: 'cloudflare',
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
