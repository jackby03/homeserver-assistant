import { useState } from 'react';
import {
  CheckCircle2,
  Globe2,
  Network,
  Plus,
  Router,
  ShieldAlert,
  ShieldCheck,
  Trash2,
} from 'lucide-react';
import { setupApi } from '@/api/setup';
import type {
  DmzConfig,
  DnsConfig,
  DnsProvider,
  NetworkMode,
  StaticIpv4,
} from '@/api/types';
import { Field, Segmented, Toggle } from '@/components/Field';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import { cn } from '@/lib/cn';
import {
  DEFAULT_DMZ_CONFIG,
  DEFAULT_DNS_CONFIG,
  DNS_PROVIDERS,
  DNS_PROVIDER_DESC_KEYS,
  DNS_PROVIDER_LABEL_KEYS,
} from '@/lib/dns-meta';
import { useToast } from '@/state/ToastProvider';
import { StepError, StepFooter } from './StepChrome';
import type { SetupSummaryPatch } from './types';

const IPV4_PATTERN = /^(\d{1,3}\.){3}\d{1,3}$/;

interface StepNetworkProps {
  onNext: (patch?: SetupSummaryPatch) => void;
  onBack: () => void;
}

/** Step 2: LAN addressing, encrypted DNS filtering and DMZ. */
export function StepNetwork({ onNext, onBack }: StepNetworkProps): React.JSX.Element {
  const { t } = useI18n();
  const toast = useToast();
  const [interfaceName, setInterfaceName] = useState('eth0');
  const [mode, setMode] = useState<NetworkMode>('dhcp');
  const [address, setAddress] = useState('192.168.1.10');
  const [prefix, setPrefix] = useState('24');
  const [gateway, setGateway] = useState('192.168.1.1');
  const [dns, setDns] = useState<DnsConfig>(DEFAULT_DNS_CONFIG);
  const [customUpstream, setCustomUpstream] = useState('');
  const [blocklistInput, setBlocklistInput] = useState('');
  const [dmz, setDmz] = useState<DmzConfig>(DEFAULT_DMZ_CONFIG);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (): Promise<void> => {
    setError(null);
    let staticIpv4: StaticIpv4 | undefined;
    if (mode === 'static') {
      if (!IPV4_PATTERN.test(address) || !IPV4_PATTERN.test(gateway)) {
        setError(t('net.invalidIpv4'));
        return;
      }
      const prefixNumber = Number(prefix);
      if (!Number.isInteger(prefixNumber) || prefixNumber < 1 || prefixNumber > 32) {
        setError(t('net.invalidPrefix'));
        return;
      }
      staticIpv4 = { address, prefix_len: prefixNumber, gateway };
    }
    if (dns.provider === 'custom' && customUpstream.trim() === '') {
      setError(t('net.dns.customHint'));
      return;
    }
    for (const list of dns.blocklists) {
      if (!/^https?:\/\/\S+$/i.test(list)) {
        setError(t('net.dns.invalidUrl'));
        return;
      }
    }
    setBusy(true);
    try {
      await setupApi.configureNetwork({
        interface: interfaceName.trim() === '' ? 'eth0' : interfaceName.trim(),
        mode,
        ...(staticIpv4 !== undefined ? { static_ipv4: staticIpv4 } : {}),
        dns: {
          ...dns,
          custom_upstream: dns.provider === 'custom' ? customUpstream.trim() : null,
        },
        dmz,
      });
      toast.push({ level: 'success', message: t('setup.toast.networkSaved') });
      onNext({ mode, dnsProvider: dns.provider, dnsEnabled: dns.enabled });
    } catch (cause) {
      setError(t(errorMessageKey(cause)));
      setBusy(false);
    }
  };

  const addBlocklist = (): void => {
    const value = blocklistInput.trim();
    if (value === '' || dns.blocklists.includes(value)) return;
    setDns((current) => ({ ...current, blocklists: [...current.blocklists, value] }));
    setBlocklistInput('');
  };

  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-display text-[22px] font-medium tracking-tight text-ink-50">
          {t('setup.step2.title')}
        </h2>
        <p className="mt-1.5 max-w-2xl text-[13px] leading-relaxed text-ink-400">
          {t('setup.step2.description')}
        </p>
      </div>

      <StepError message={error} />

      {/* Local network ------------------------------------------------------ */}
      <section className="space-y-4 rounded-2xl border border-white/[0.06] bg-white/[0.015] p-4">
        <div className="flex items-center gap-2">
          <Network size={15} className="text-mint-400" />
          <h3 className="text-[13px] font-medium text-ink-100">{t('setup.step2.network')}</h3>
        </div>
        <div className="grid gap-4 sm:grid-cols-2">
          <Field label={t('net.interface')} htmlFor="setup-interface" hint={t('net.interfacesHint')}>
            <input
              id="setup-interface"
              className="input input-mono"
              value={interfaceName}
              onChange={(event) => setInterfaceName(event.target.value)}
              placeholder="eth0"
              spellCheck={false}
            />
          </Field>
          <Field label={t('net.mode')}>
            <Segmented
              value={mode}
              onChange={setMode}
              options={[
                { value: 'dhcp', label: t('net.mode.dhcp') },
                { value: 'static', label: t('net.mode.static') },
              ]}
            />
          </Field>
        </div>
        {mode === 'static' ? (
          <div className="grid gap-4 sm:grid-cols-3">
            <Field label={t('net.address')} htmlFor="setup-address">
              <input
                id="setup-address"
                className="input input-mono"
                value={address}
                onChange={(event) => setAddress(event.target.value)}
                spellCheck={false}
              />
            </Field>
            <Field label={t('net.prefix')} htmlFor="setup-prefix">
              <input
                id="setup-prefix"
                className="input input-mono tabular"
                type="number"
                min="1"
                max="32"
                value={prefix}
                onChange={(event) => setPrefix(event.target.value)}
              />
            </Field>
            <Field label={t('net.gateway')} htmlFor="setup-gateway">
              <input
                id="setup-gateway"
                className="input input-mono"
                value={gateway}
                onChange={(event) => setGateway(event.target.value)}
                spellCheck={false}
              />
            </Field>
          </div>
        ) : null}
      </section>

      {/* DNS ----------------------------------------------------------------- */}
      <section className="space-y-4 rounded-2xl border border-white/[0.06] bg-white/[0.015] p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <ShieldCheck size={15} className="text-mint-400" />
            <h3 className="text-[13px] font-medium text-ink-100">{t('setup.step2.dns')}</h3>
          </div>
          <Toggle
            checked={dns.enabled}
            onChange={(enabled) => setDns((current) => ({ ...current, enabled }))}
            label={t('net.dns.enabled')}
          />
        </div>

        <div className={cn('space-y-4', !dns.enabled && 'pointer-events-none opacity-50')}>
          <div className="grid gap-2.5 sm:grid-cols-2 xl:grid-cols-3">
            {DNS_PROVIDERS.map((provider: DnsProvider) => {
              const selected = dns.provider === provider;
              return (
                <button
                  key={provider}
                  type="button"
                  onClick={() => setDns((current) => ({ ...current, provider }))}
                  aria-pressed={selected}
                  className={cn(
                    'rounded-2xl border p-3 text-left transition duration-150',
                    selected
                      ? 'border-mint-400/35 bg-mint-400/[0.06]'
                      : 'border-white/[0.07] bg-white/[0.02] hover:border-white/[0.15]',
                  )}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-[12.5px] font-medium text-ink-100">
                      {t(DNS_PROVIDER_LABEL_KEYS[provider])}
                    </span>
                    {selected ? <CheckCircle2 size={13} className="text-mint-400" /> : null}
                  </div>
                  <p className="mt-1 text-[10.5px] leading-relaxed text-ink-500">
                    {t(DNS_PROVIDER_DESC_KEYS[provider])}
                  </p>
                </button>
              );
            })}
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <Field label={t('net.dns.protocol')}>
              <Segmented
                value={dns.protocol}
                onChange={(protocol) => setDns((current) => ({ ...current, protocol }))}
                options={[
                  { value: 'doh', label: 'DoH', icon: <Globe2 size={14} /> },
                  { value: 'dot', label: 'DoT', icon: <ShieldCheck size={14} /> },
                ]}
              />
            </Field>
            <Field label={t('net.dns.engine')} hint={t('net.dns.engineHint')}>
              <Segmented
                value={dns.engine}
                onChange={(engine) => setDns((current) => ({ ...current, engine }))}
                options={[
                  { value: 'adguard_home', label: t('net.dns.engine.adguard_home') },
                  { value: 'technitium', label: t('net.dns.engine.technitium') },
                ]}
              />
            </Field>
          </div>

          {dns.provider === 'custom' ? (
            <Field label={t('net.dns.customUpstream')} htmlFor="setup-custom-dns" hint={t('net.dns.customHint')}>
              <input
                id="setup-custom-dns"
                className="input input-mono"
                value={customUpstream}
                onChange={(event) => setCustomUpstream(event.target.value)}
                placeholder="https://dns.example.com/dns-query"
                spellCheck={false}
              />
            </Field>
          ) : null}

          <div className="space-y-2">
            <p className="field-label">{t('net.dns.blocklists')}</p>
            {dns.blocklists.length === 0 ? (
              <p className="text-xs text-ink-500">{t('common.none')}</p>
            ) : (
              <ul className="space-y-1.5">
                {dns.blocklists.map((list) => (
                  <li
                    key={list}
                    className="flex items-center gap-2 rounded-xl border border-white/[0.06] bg-white/[0.02] px-3 py-2"
                  >
                    <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-ink-400">{list}</span>
                    <button
                      type="button"
                      className="grid h-8 w-8 shrink-0 place-items-center rounded-lg text-ink-500 transition hover:bg-coral-500/10 hover:text-coral-300"
                      onClick={() =>
                        setDns((current) => ({
                          ...current,
                          blocklists: current.blocklists.filter((item) => item !== list),
                        }))
                      }
                      aria-label={t('common.remove')}
                    >
                      <Trash2 size={13} />
                    </button>
                  </li>
                ))}
              </ul>
            )}
            <div className="flex gap-2">
              <input
                className="input input-mono flex-1"
                value={blocklistInput}
                onChange={(event) => setBlocklistInput(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    event.preventDefault();
                    addBlocklist();
                  }
                }}
                placeholder={t('net.dns.blocklistPlaceholder')}
                aria-label={t('net.dns.addBlocklist')}
                spellCheck={false}
              />
              <button type="button" className="btn btn-secondary btn-sm" onClick={addBlocklist}>
                <Plus size={14} />
                {t('net.dns.addBlocklist')}
              </button>
            </div>
          </div>
        </div>
      </section>

      {/* DMZ ----------------------------------------------------------------- */}
      <section className="space-y-4 rounded-2xl border border-white/[0.06] bg-white/[0.015] p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <ShieldAlert size={15} className="text-amber-300" />
            <h3 className="text-[13px] font-medium text-ink-100">{t('setup.step2.dmz')}</h3>
          </div>
          <Toggle
            checked={dmz.enabled}
            onChange={(enabled) => setDmz((current) => ({ ...current, enabled }))}
            label={t('net.dmz.enabled')}
          />
        </div>
        <p className="text-[12px] leading-relaxed text-ink-500">{t('net.dmz.explainer')}</p>
        <div className={cn('space-y-4', !dmz.enabled && 'pointer-events-none opacity-50')}>
          <div className="grid gap-4 sm:grid-cols-3">
            <Field label={t('net.dmz.bridge')} htmlFor="setup-dmz-bridge">
              <input
                id="setup-dmz-bridge"
                className="input input-mono"
                value={dmz.bridge}
                onChange={(event) => setDmz((current) => ({ ...current, bridge: event.target.value }))}
                spellCheck={false}
              />
            </Field>
            <Field label={t('net.dmz.subnet')} htmlFor="setup-dmz-subnet">
              <input
                id="setup-dmz-subnet"
                className="input input-mono"
                value={dmz.subnet}
                onChange={(event) => setDmz((current) => ({ ...current, subnet: event.target.value }))}
                spellCheck={false}
              />
            </Field>
            <Field label={t('net.dmz.gateway')} htmlFor="setup-dmz-gateway">
              <input
                id="setup-dmz-gateway"
                className="input input-mono"
                value={dmz.gateway}
                onChange={(event) => setDmz((current) => ({ ...current, gateway: event.target.value }))}
                spellCheck={false}
              />
            </Field>
          </div>
          <Toggle
            checked={dmz.allow_dns}
            onChange={(allow_dns) => setDmz((current) => ({ ...current, allow_dns }))}
            label={t('net.dmz.allowDns')}
            description={t('net.dmz.explainer')}
          />
          <p className="flex items-center gap-2 text-[11px] text-ink-500">
            <Router size={13} className="text-mint-400" />
            {t('net.dmz.warning')}
          </p>
        </div>
      </section>

      <StepFooter onNext={() => void submit()} onBack={onBack} busy={busy} hint={t('setup.step2.skipHint')} />
    </div>
  );
}
