import { useEffect, useMemo, useState } from 'react';
import { CheckCircle2, Globe2, ShieldCheck, Wifi, WifiOff } from 'lucide-react';
import { networkApi } from '@/api/network';
import type { DnsProtocol, DnsProvider, DnsProviderInfo } from '@/api/types';
import { Field, Segmented } from '@/components/Field';
import { Spinner } from '@/components/Spinner';
import { useI18n } from '@/i18n/I18nProvider';
import { cn } from '@/lib/cn';

interface DnsProviderPickerProps {
  provider: DnsProvider;
  protocol: DnsProtocol;
  customUpstream: string;
  disabled?: boolean;
  idPrefix?: string;
  onProviderChange: (provider: DnsProvider) => void;
  onProtocolChange: (protocol: DnsProtocol) => void;
  onCustomUpstreamChange: (value: string) => void;
}

/** Minimal fallback catalog for offline/failed fetches (the daemon is authoritative). */
const FALLBACK_CATALOG: DnsProviderInfo[] = [
  {
    id: 'cloudflare',
    label: 'Cloudflare',
    vendor: { id: 'cloudflare', name: 'Cloudflare' },
    ipv4: ['1.1.1.1', '1.0.0.1'],
    ipv6: ['2606:4700:4700::1111', '2606:4700:4700::1001'],
    doh: 'https://cloudflare-dns.com/dns-query',
    dot: 'one.one.one.one',
    plain: true,
    requires_config_id: false,
    note: null,
  },
  {
    id: 'quad9',
    label: 'Quad9',
    vendor: { id: 'quad9', name: 'Quad9' },
    ipv4: ['9.9.9.9', '149.112.112.112'],
    ipv6: ['2620:fe::fe', '2620:fe::9'],
    doh: 'https://dns.quad9.net/dns-query',
    dot: 'dns.quad9.net',
    plain: true,
    requires_config_id: false,
    note: null,
  },
  {
    id: 'adguard_family',
    label: 'AdGuard DNS Family',
    vendor: { id: 'adguard', name: 'AdGuard DNS' },
    ipv4: ['94.140.14.15', '94.140.15.16'],
    ipv6: ['2a10:50c0::bad1:ff', '2a10:50c0::bad2:ff'],
    doh: 'https://family.adguard-dns.com/dns-query',
    dot: 'family.adguard-dns.com',
    plain: true,
    requires_config_id: false,
    note: null,
  },
  {
    id: 'custom',
    label: 'Personalizado',
    vendor: { id: 'custom', name: 'Personalizado' },
    ipv4: [],
    ipv6: [],
    doh: null,
    dot: null,
    plain: false,
    requires_config_id: false,
    note: null,
  },
];

function supportedProtocols(provider: DnsProviderInfo | undefined): DnsProtocol[] {
  if (provider === undefined) return ['doh'];
  if (provider.id === 'custom') return ['doh', 'dot'];
  const protocols: DnsProtocol[] = [];
  if (provider.doh !== null) protocols.push('doh');
  if (provider.dot !== null) protocols.push('dot');
  if (provider.plain) protocols.push('plain');
  return protocols;
}

/**
 * Data-driven DNS provider selector: vendor filter, full free-resolver catalog,
 * transport picker filtered by capability and custom/NextDNS endpoint input.
 */
export function DnsProviderPicker({
  provider,
  protocol,
  customUpstream,
  disabled = false,
  idPrefix = 'dns',
  onProviderChange,
  onProtocolChange,
  onCustomUpstreamChange,
}: DnsProviderPickerProps): React.JSX.Element {
  const { t } = useI18n();
  const [catalog, setCatalog] = useState<DnsProviderInfo[]>(FALLBACK_CATALOG);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [vendor, setVendor] = useState<string>('all');

  useEffect(() => {
    let cancelled = false;
    void networkApi
      .dnsProviders()
      .then((entries) => {
        if (!cancelled && entries.length > 0) setCatalog(entries);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const vendors = useMemo(() => {
    const unique = new Map<string, string>();
    for (const entry of catalog) unique.set(entry.vendor.id, entry.vendor.name);
    return [...unique.entries()].map(([id, name]) => ({ id, name }));
  }, [catalog]);

  const selected = useMemo(
    () => catalog.find((entry) => entry.id === provider),
    [catalog, provider],
  );
  const protocols = useMemo(() => supportedProtocols(selected), [selected]);

  useEffect(() => {
    if (protocols.length > 0 && !protocols.includes(protocol)) {
      onProtocolChange(protocols[0]);
    }
  }, [protocols, protocol, onProtocolChange]);

  const visible = vendor === 'all' ? catalog : catalog.filter((entry) => entry.vendor.id === vendor);
  const needsEndpoint = provider === 'custom' || (selected?.requires_config_id ?? false);

  return (
    <div className={cn('space-y-4', disabled && 'pointer-events-none opacity-50')}>
      <div>
        <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
          <p className="field-label">{t('net.dns.provider')}</p>
          {loading ? (
            <span className="flex items-center gap-1.5 text-[10.5px] text-ink-500">
              <Spinner size={12} /> {t('net.dns.loadingCatalog')}
            </span>
          ) : failed ? (
            <span className="flex items-center gap-1.5 text-[10.5px] text-amber-300">
              <WifiOff size={12} /> {t('net.dns.catalogFallback')}
            </span>
          ) : (
            <span className="flex items-center gap-1.5 text-[10.5px] text-mint-300">
              <Wifi size={12} /> {t('net.dns.catalogCount', { count: catalog.length })}
            </span>
          )}
        </div>

        <div className="mb-2.5 flex flex-wrap gap-1.5">
          <button
            type="button"
            onClick={() => setVendor('all')}
            className={cn('chip transition', vendor === 'all' && 'border-mint-400/40 text-mint-200')}
            aria-pressed={vendor === 'all'}
          >
            {t('net.dns.vendorAll')}
          </button>
          {vendors.map((entry) => (
            <button
              key={entry.id}
              type="button"
              onClick={() => setVendor(entry.id)}
              className={cn('chip transition', vendor === entry.id && 'border-mint-400/40 text-mint-200')}
              aria-pressed={vendor === entry.id}
            >
              {entry.name}
            </button>
          ))}
        </div>

        <div className="grid max-h-[19rem] gap-2 overflow-y-auto pr-1 sm:grid-cols-2 xl:grid-cols-3">
          {visible.map((entry) => {
            const isSelected = entry.id === provider;
            const entryProtocols = supportedProtocols(entry);
            return (
              <button
                key={entry.id}
                type="button"
                onClick={() => onProviderChange(entry.id)}
                aria-pressed={isSelected}
                className={cn(
                  'rounded-2xl border p-3 text-left transition duration-150',
                  isSelected
                    ? 'border-mint-400/35 bg-mint-400/[0.06] shadow-glow'
                    : 'border-white/[0.07] bg-white/[0.02] hover:border-white/[0.15]',
                )}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="text-[12.5px] font-medium text-ink-100">{entry.label}</span>
                  {isSelected ? <CheckCircle2 size={13} className="text-mint-400" /> : null}
                </div>
                <p className="mt-1 line-clamp-2 text-[10.5px] leading-relaxed text-ink-500">
                  {entry.note ?? entry.vendor.name}
                </p>
                <div className="mt-1.5 flex flex-wrap gap-1">
                  {entryProtocols.map((item) => (
                    <span key={item} className="chip chip-xs">
                      {item === 'doh' ? 'DoH' : item === 'dot' ? 'DoT' : t('net.dns.plain')}
                    </span>
                  ))}
                </div>
              </button>
            );
          })}
        </div>
      </div>

      <div className="grid gap-4 sm:grid-cols-2">
        <Field label={t('net.dns.protocol')}>
          <Segmented
            value={protocol}
            onChange={onProtocolChange}
            options={protocols.map((item) => ({
              value: item,
              label: item === 'doh' ? 'DoH' : item === 'dot' ? 'DoT' : t('net.dns.plain'),
              icon:
                item === 'plain' ? (
                  <WifiOff size={14} />
                ) : item === 'dot' ? (
                  <ShieldCheck size={14} />
                ) : (
                  <Globe2 size={14} />
                ),
            }))}
          />
        </Field>
        {selected !== undefined ? (
          <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] px-3 py-2 text-[10.5px] leading-relaxed text-ink-400">
            <p className="text-ink-200">{selected.vendor.name}</p>
            {selected.ipv4.length > 0 ? (
              <p className="truncate font-mono">IPv4: {selected.ipv4.join(', ')}</p>
            ) : null}
            {selected.ipv6.length > 0 ? (
              <p className="truncate font-mono">IPv6: {selected.ipv6.join(', ')}</p>
            ) : null}
            {selected.doh !== null ? <p className="truncate font-mono">DoH: {selected.doh}</p> : null}
            {selected.dot !== null ? <p className="truncate font-mono">DoT: {selected.dot}</p> : null}
          </div>
        ) : null}
      </div>

      {protocol === 'plain' ? (
        <p className="rounded-xl border border-amber-400/25 bg-amber-400/[0.06] px-3.5 py-2.5 text-[11.5px] leading-relaxed text-amber-200">
          {t('net.dns.plainWarning')}
        </p>
      ) : null}

      {needsEndpoint ? (
        <Field
          label={
            selected?.requires_config_id
              ? t('net.dns.nextdnsId')
              : t('net.dns.customUpstream')
          }
          htmlFor={`${idPrefix}-custom`}
          hint={selected?.requires_config_id ? t('net.dns.nextdnsHint') : t('net.dns.customHint')}
        >
          <input
            id={`${idPrefix}-custom`}
            className="input input-mono"
            value={customUpstream}
            onChange={(event) => onCustomUpstreamChange(event.target.value)}
            placeholder={
              selected?.requires_config_id
                ? 'abc123'
                : protocol === 'doh'
                  ? 'https://dns.example.com/dns-query'
                  : 'dns.example.com'
            }
            spellCheck={false}
          />
        </Field>
      ) : null}
    </div>
  );
}
