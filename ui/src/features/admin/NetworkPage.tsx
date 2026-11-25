import { useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import {
  Ban,
  CheckCircle2,
  Globe2,
  History,
  Link2,
  Network,
  Plus,
  RefreshCw,
  RotateCcw,
  Router,
  ShieldAlert,
  ShieldCheck,
  Trash2,
  Wifi,
  XCircle,
  Zap,
} from 'lucide-react';
import { networkApi } from '@/api/network';
import type {
  DmzConfig,
  DnsConfig,
  DnsTestResponse,
  FirewallBackup,
  FirewallStatusResponse,
  NetworkInterface,
  NetworkMode,
  StaticIpv4,
} from '@/api/types';
import { EmptyState } from '@/components/EmptyState';
import { DnsProviderPicker } from '@/components/DnsProviderPicker';
import { Field, Segmented, Toggle } from '@/components/Field';
import { PageHeader } from '@/components/PageHeader';
import { Spinner } from '@/components/Spinner';
import { StatusPill } from '@/components/StatusPill';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import { cn } from '@/lib/cn';
import { formatDateTime } from '@/lib/format';
import { useResource } from '@/lib/use-resource';
import { useToast } from '@/state/ToastProvider';

const DEFAULT_DNS: DnsConfig = {
  enabled: true,
  engine: 'adguard_home',
  provider: 'cloudflare_family',
  protocol: 'doh',
  custom_upstream: null,
  blocklists: [],
};

const DEFAULT_DMZ: DmzConfig = {
  enabled: false,
  bridge: 'br-dmz',
  subnet: '10.20.30.0/24',
  gateway: '10.20.30.1',
  allow_dns: true,
};

const IPV4_PATTERN = /^(\d{1,3}\.){3}\d{1,3}$/;

export function NetworkPage(): React.JSX.Element {
  const { t, language } = useI18n();
  const toast = useToast();
  const [refreshToken, setRefreshToken] = useState(0);
  const view = useResource(() => networkApi.view(), [refreshToken]);
  const firewall = useResource<FirewallStatusResponse>(() => networkApi.firewall(), [refreshToken]);
  const firewallHistory = useResource<FirewallBackup[]>(
    () => networkApi.firewallHistory(),
    [refreshToken],
  );

  const [hydrated, setHydrated] = useState(false);
  const [interfaceName, setInterfaceName] = useState('eth0');
  const [lanBridge, setLanBridge] = useState('br-lan');
  const [mode, setMode] = useState<NetworkMode>('dhcp');
  const [address, setAddress] = useState('');
  const [prefix, setPrefix] = useState('24');
  const [gateway, setGateway] = useState('');
  const [dns, setDns] = useState<DnsConfig>(DEFAULT_DNS);
  const [blocklistInput, setBlocklistInput] = useState('');
  const [customUpstream, setCustomUpstream] = useState('');
  const [dmz, setDmz] = useState<DmzConfig>(DEFAULT_DMZ);

  const [savingNetwork, setSavingNetwork] = useState(false);
  const [savingDns, setSavingDns] = useState(false);
  const [savingDmz, setSavingDmz] = useState(false);
  const [applyingFirewall, setApplyingFirewall] = useState(false);
  const [rollingBackFirewall, setRollingBackFirewall] = useState(false);
  const [testingDns, setTestingDns] = useState(false);
  const [dnsResult, setDnsResult] = useState<DnsTestResponse | null>(null);
  const [formError, setFormError] = useState<string | null>(null);

  useEffect(() => {
    const config = view.data?.config;
    if (config === undefined || hydrated) return;
    setInterfaceName(config.interface);
    setLanBridge(config.lan_bridge ?? 'br-lan');
    setMode(config.mode);
    setAddress(config.static_ipv4?.address ?? '');
    setPrefix(config.static_ipv4 !== null ? String(config.static_ipv4.prefix_len) : '24');
    setGateway(config.static_ipv4?.gateway ?? '');
    setDns(config.dns);
    setCustomUpstream(config.dns.custom_upstream ?? '');
    setDmz(config.dmz);
    setHydrated(true);
  }, [view.data, hydrated]);

  const interfaces = view.data?.interfaces ?? [];
  const selectedInterface = interfaces.find((item) => item.name === interfaceName);
  const configDrifted =
    view.data !== null &&
    (interfaceName !== view.data.config.interface ||
      mode !== view.data.config.mode ||
      (lanBridge || 'br-lan') !== (view.data.config.lan_bridge ?? 'br-lan'));

  const saveNetwork = async (): Promise<void> => {
    setFormError(null);
    let staticIpv4: StaticIpv4 | null = null;
    if (mode === 'static') {
      if (!IPV4_PATTERN.test(address) || !IPV4_PATTERN.test(gateway)) {
        setFormError(t('net.invalidIpv4'));
        return;
      }
      const prefixNumber = Number(prefix);
      if (!Number.isInteger(prefixNumber) || prefixNumber < 1 || prefixNumber > 32) {
        setFormError(t('net.invalidPrefix'));
        return;
      }
      staticIpv4 = { address, prefix_len: prefixNumber, gateway };
    }
    setSavingNetwork(true);
    try {
      await networkApi.updateNetwork({
        interface: interfaceName,
        lan_bridge: lanBridge.trim() === '' ? undefined : lanBridge.trim(),
        mode,
        static_ipv4: staticIpv4,
      });
      toast.push({ level: 'success', message: t('common.saved') });
      setHydrated(false);
      setRefreshToken((value) => value + 1);
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setSavingNetwork(false);
    }
  };

  const dnsValidation = useMemo((): string | null => {
    if ((dns.provider === 'custom' || dns.provider === 'nextdns') && customUpstream.trim() === '')
      return t('net.dns.customHint');
    for (const list of dns.blocklists) {
      if (!/^https?:\/\/\S+$/i.test(list)) return t('net.dns.invalidUrl');
    }
    return null;
  }, [dns.provider, dns.blocklists, customUpstream, t]);

  const saveDns = async (): Promise<void> => {
    const validation = dnsValidation;
    if (validation !== null) {
      setFormError(validation);
      return;
    }
    setFormError(null);
    setSavingDns(true);
    try {
      await networkApi.updateDns({
        ...dns,
        custom_upstream:
          dns.provider === 'custom' || dns.provider === 'nextdns'
            ? customUpstream.trim()
            : null,
      });
      toast.push({ level: 'success', message: t('net.toast.dnsSaved') });
      setDnsResult(null);
      setRefreshToken((value) => value + 1);
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setSavingDns(false);
    }
  };

  const saveDmz = async (): Promise<void> => {
    setFormError(null);
    setSavingDmz(true);
    try {
      await networkApi.updateDmz(dmz);
      toast.push({ level: 'success', message: t('net.toast.dmzSaved') });
      setHydrated(false);
      setRefreshToken((value) => value + 1);
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setSavingDmz(false);
    }
  };

  const applyFirewall = async (): Promise<void> => {
    setApplyingFirewall(true);
    try {
      const result = await networkApi.applyFirewall();
      toast.push({
        level: 'success',
        message: t('net.toast.firewallApplied', { count: result.rules_count }),
      });
      firewall.reload();
      firewallHistory.reload();
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setApplyingFirewall(false);
    }
  };

  const rollbackFirewall = async (): Promise<void> => {
    setRollingBackFirewall(true);
    try {
      const result = await networkApi.rollbackFirewall();
      toast.push({
        level: 'success',
        message: t('net.toast.firewallRolledBack', { name: result.rolled_back_to }),
      });
      firewall.reload();
      firewallHistory.reload();
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setRollingBackFirewall(false);
    }
  };

  const testDns = async (): Promise<void> => {
    setTestingDns(true);
    setDnsResult(null);
    try {
      const result = await networkApi.testDns();
      setDnsResult(result);
    } catch (cause) {
      setDnsResult({ ok: false, latency_ms: null, provider: dns.provider });
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setTestingDns(false);
    }
  };

  const addBlocklist = (): void => {
    const value = blocklistInput.trim();
    if (value === '' || dns.blocklists.includes(value)) return;
    setDns((current) => ({ ...current, blocklists: [...current.blocklists, value] }));
    setBlocklistInput('');
  };

  return (
    <div className="space-y-5">
      <PageHeader
        title={t('net.title')}
        subtitle={t('net.subtitle')}
        actions={
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={() => {
              setHydrated(false);
              setRefreshToken((value) => value + 1);
            }}
            disabled={view.refreshing}
          >
            <RefreshCw size={14} className={view.refreshing ? 'animate-spin' : undefined} />
            {t('common.refresh')}
          </button>
        }
      />

      {view.error !== null && view.data === null ? (
        <div className="rounded-2xl border border-coral-400/25 bg-coral-500/[0.07] px-4 py-3 text-sm text-coral-200">
          {t(errorMessageKey(view.error))}
        </div>
      ) : null}

      {formError !== null ? (
        <p role="alert" className="rounded-2xl border border-coral-400/25 bg-coral-500/[0.07] px-4 py-2.5 text-[13px] text-coral-300">
          {formError}
        </p>
      ) : null}

      {/* Addressing ---------------------------------------------------------- */}
      <section className="glass space-y-5 p-5 sm:p-6">
        <div className="flex items-center gap-2.5">
          <span className="grid h-8 w-8 place-items-center rounded-xl border border-mint-400/20 bg-mint-400/[0.07] text-mint-300">
            <Network size={16} />
          </span>
          <div>
            <h2 className="font-display text-[15px] font-medium text-ink-100">{t('net.interfaces')}</h2>
            <p className="text-[11px] text-ink-500">{t('net.interfacesHint')}</p>
          </div>
        </div>

        {interfaces.length === 0 ? (
          <EmptyState title={t('net.noInterfaces')} />
        ) : (
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {interfaces.map((iface: NetworkInterface) => (
              <button
                key={iface.name}
                type="button"
                onClick={() => setInterfaceName(iface.name)}
                aria-pressed={interfaceName === iface.name}
                className={cn(
                  'rounded-2xl border p-4 text-left transition duration-150',
                  interfaceName === iface.name
                    ? 'border-mint-400/35 bg-mint-400/[0.06]'
                    : 'border-white/[0.07] bg-white/[0.02] hover:border-white/[0.15]',
                )}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="flex items-center gap-2 font-mono text-[13px] text-ink-100">
                    <Wifi size={14} className={iface.is_up ? 'text-mint-400' : 'text-ink-600'} />
                    {iface.name}
                  </span>
                  <StatusPill
                    label={iface.is_up ? t('net.up') : t('net.down')}
                    tone={iface.is_up ? 'success' : 'neutral'}
                  />
                </div>
                <p className="mt-2 font-mono text-[11px] text-ink-400">{iface.ipv4 || '—'}</p>
                <p className="mt-0.5 font-mono text-[10.5px] text-ink-600">{iface.mac}</p>
              </button>
            ))}
          </div>
        )}

        <div className="grid gap-4 lg:grid-cols-[1fr_1.4fr]">
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
          <p className="self-end pb-2.5 text-[12px] leading-relaxed text-ink-500">
            {mode === 'dhcp' ? t('net.dhcpHint') : t('net.staticHint')}
          </p>
        </div>

        {mode === 'static' ? (
          <div className="grid gap-4 sm:grid-cols-3">
            <Field label={t('net.address')} htmlFor="net-address">
              <input
                id="net-address"
                className="input input-mono"
                value={address}
                onChange={(event) => setAddress(event.target.value)}
                placeholder="192.168.1.10"
                spellCheck={false}
              />
            </Field>
            <Field label={t('net.prefix')} htmlFor="net-prefix">
              <input
                id="net-prefix"
                className="input input-mono tabular"
                type="number"
                min="1"
                max="32"
                value={prefix}
                onChange={(event) => setPrefix(event.target.value)}
              />
            </Field>
            <Field label={t('net.gateway')} htmlFor="net-gateway">
              <input
                id="net-gateway"
                className="input input-mono"
                value={gateway}
                onChange={(event) => setGateway(event.target.value)}
                placeholder="192.168.1.1"
                spellCheck={false}
              />
            </Field>
          </div>
        ) : null}

        <details className="group rounded-xl border border-white/[0.06] bg-white/[0.015] px-3.5 py-2.5">
          <summary className="cursor-pointer list-none text-[12px] font-medium text-ink-400 transition group-open:text-ink-100">
            {t('net.lanBridge')}
          </summary>
          <div className="mt-3">
            <Field label={t('net.lanBridge')} htmlFor="net-lan-bridge" hint={t('net.lanBridgeHint')}>
              <input
                id="net-lan-bridge"
                className="input input-mono"
                value={lanBridge}
                onChange={(event) => setLanBridge(event.target.value)}
                placeholder="br-lan"
                spellCheck={false}
              />
            </Field>
          </div>
        </details>

        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-[11px] text-ink-500">
            {selectedInterface !== undefined
              ? `${selectedInterface.name} · ${selectedInterface.mac}`
              : ''}
            {configDrifted ? ' · *' : ''}
          </p>
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={() => void saveNetwork()}
            disabled={savingNetwork}
          >
            {savingNetwork ? <Spinner size={14} /> : <Link2 size={14} />}
            {t('net.saveNetwork')}
          </button>
        </div>
      </section>

      {/* DNS ----------------------------------------------------------------- */}
      <section className="glass space-y-5 p-5 sm:p-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex items-center gap-2.5">
            <span className="grid h-8 w-8 place-items-center rounded-xl border border-mint-400/20 bg-mint-400/[0.07] text-mint-300">
              <ShieldCheck size={16} />
            </span>
            <div>
              <h2 className="font-display text-[15px] font-medium text-ink-100">{t('net.dns.title')}</h2>
              <p className="text-[11px] text-ink-500">{t('net.dns.subtitle')}</p>
            </div>
          </div>
          <div className="flex items-center gap-3">
            <Toggle
              checked={dns.enabled}
              onChange={(enabled) => setDns((current) => ({ ...current, enabled }))}
              label={t('net.dns.enabled')}
            />
          </div>
        </div>

        <div className={cn('space-y-5', !dns.enabled && 'pointer-events-none opacity-50')}>
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

          <DnsProviderPicker
            provider={dns.provider}
            protocol={dns.protocol}
            customUpstream={customUpstream}
            disabled={!dns.enabled}
            onProviderChange={(provider) => setDns((current) => ({ ...current, provider }))}
            onProtocolChange={(protocol) => setDns((current) => ({ ...current, protocol }))}
            onCustomUpstreamChange={setCustomUpstream}
          />

          <div className="flex flex-wrap items-end justify-end gap-2">
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              onClick={() => void testDns()}
              disabled={testingDns}
            >
              {testingDns ? <Spinner size={14} /> : <Zap size={14} />}
              {testingDns ? t('net.dns.testing') : t('net.dns.test')}
            </button>
            <button
              type="button"
              className="btn btn-primary btn-sm"
              onClick={() => void saveDns()}
              disabled={savingDns}
            >
              {savingDns ? <Spinner size={14} className="text-[#04121a]" /> : <ShieldCheck size={14} />}
              {t('net.saveDns')}
            </button>
          </div>

          {dnsResult !== null ? (
            <div
              className={cn(
                'flex flex-wrap items-center gap-3 rounded-xl border px-3.5 py-2.5 text-[12.5px]',
                dnsResult.ok
                  ? 'border-mint-500/25 bg-mint-500/[0.06] text-mint-200'
                  : 'border-coral-400/25 bg-coral-500/[0.06] text-coral-200',
              )}
            >
              {dnsResult.ok ? <CheckCircle2 size={15} /> : <XCircle size={15} />}
              {dnsResult.ok
                ? t('net.dns.testOk', { ms: dnsResult.latency_ms ?? 0 })
                : t('net.dns.testFail')}
              <span className="ml-auto font-mono text-[11px] text-ink-500">{dnsResult.provider}</span>
            </div>
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
                    <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-ink-300">{list}</span>
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
                      <Trash2 size={14} />
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

      {/* DMZ + firewall ------------------------------------------------------ */}
      <div className="grid gap-4 xl:grid-cols-2">
        <section className="glass space-y-4 p-5 sm:p-6">
          <div className="flex items-start justify-between gap-3">
            <div className="flex items-center gap-2.5">
              <span className="grid h-8 w-8 place-items-center rounded-xl border border-amber-400/20 bg-amber-400/[0.08] text-amber-300">
                <ShieldAlert size={16} />
              </span>
              <div>
                <h2 className="font-display text-[15px] font-medium text-ink-100">{t('net.dmz.title')}</h2>
                <p className="text-[11px] text-ink-500">{t('net.dmz.subtitle')}</p>
              </div>
            </div>
            <Toggle
              checked={dmz.enabled}
              onChange={(enabled) => setDmz((current) => ({ ...current, enabled }))}
              label={t('net.dmz.enabled')}
            />
          </div>

          <div className="rounded-2xl border border-white/[0.06] bg-white/[0.02] p-3.5">
            <div className="grid grid-cols-3 items-center gap-2 text-center">
              <DiagramNode icon={<Router size={16} />} label={t('net.dmz.diagram.lan')} tone="mint" />
              <DiagramNode icon={<Ban size={16} />} label={t('net.dmz.diagram.blocked')} tone="neutral" />
              <DiagramNode icon={<ShieldAlert size={16} />} label={t('net.dmz.diagram.dmz')} tone="amber" />
            </div>
            <div className="mt-3 grid grid-cols-3 items-center gap-2 text-center">
              <DiagramNode icon={<ShieldCheck size={16} />} label={t('net.dmz.diagram.host')} tone="mint" />
              <DiagramNode icon={<CheckCircle2 size={16} />} label={t('net.dmz.diagram.allowed')} tone="neutral" />
              <DiagramNode icon={<Globe2 size={16} />} label={t('net.dmz.diagram.internet')} tone="amber" />
            </div>
          </div>

          <p className="text-[12px] leading-relaxed text-ink-400">{t('net.dmz.explainer')}</p>

          <div className={cn('space-y-4', !dmz.enabled && 'pointer-events-none opacity-50')}>
            <div className="grid gap-4 sm:grid-cols-3">
              <Field label={t('net.dmz.bridge')} htmlFor="dmz-bridge">
                <input
                  id="dmz-bridge"
                  className="input input-mono"
                  value={dmz.bridge}
                  onChange={(event) => setDmz((current) => ({ ...current, bridge: event.target.value }))}
                  spellCheck={false}
                />
              </Field>
              <Field label={t('net.dmz.subnet')} htmlFor="dmz-subnet">
                <input
                  id="dmz-subnet"
                  className="input input-mono"
                  value={dmz.subnet}
                  onChange={(event) => setDmz((current) => ({ ...current, subnet: event.target.value }))}
                  spellCheck={false}
                />
              </Field>
              <Field label={t('net.dmz.gateway')} htmlFor="dmz-gateway">
                <input
                  id="dmz-gateway"
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
            <div className="flex justify-end">
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={() => void saveDmz()}
                disabled={savingDmz}
              >
                {savingDmz ? <Spinner size={14} /> : <ShieldAlert size={14} />}
                {t('net.saveDmz')}
              </button>
            </div>
          </div>
        </section>

        <section className="glass space-y-4 p-5 sm:p-6">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h2 className="font-display text-[15px] font-medium text-ink-100">{t('net.firewall.title')}</h2>
              <p className="text-[11px] text-ink-500">{t('net.firewall.subtitle')}</p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                className="btn btn-secondary btn-sm shrink-0"
                onClick={() => void rollbackFirewall()}
                disabled={rollingBackFirewall || (firewallHistory.data?.length ?? 0) === 0}
              >
                {rollingBackFirewall ? <Spinner size={14} /> : <RotateCcw size={14} />}
                {t('net.firewall.rollback')}
              </button>
              <button
                type="button"
                className="btn btn-primary btn-sm shrink-0"
                onClick={() => void applyFirewall()}
                disabled={applyingFirewall}
              >
                {applyingFirewall ? <Spinner size={14} className="text-[#04121a]" /> : <Zap size={14} />}
                {applyingFirewall ? t('net.firewall.applying') : t('net.firewall.apply')}
              </button>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2 text-[11px] text-ink-500">
            <StatusPill
              label={t('net.firewall.rulesCount', { count: countRules(firewall.data) })}
              tone={countRules(firewall.data) > 0 ? 'success' : 'neutral'}
            />
            <span>
              {t('net.firewall.lastApplied')}:{' '}
              {firewall.data?.last_applied_at != null
                ? formatDateTime(firewall.data.last_applied_at, language)
                : t('net.firewall.never')}
            </span>
          </div>

          <div className="grid gap-2 sm:grid-cols-2">
            {[
              ['inputDrop', t('net.firewall.check.inputDrop')],
              ['forwardDrop', t('net.firewall.check.forwardDrop')],
              ['dmzIsolation', t('net.firewall.check.dmzIsolation')],
              ['adminLan', t('net.firewall.check.adminLan')],
            ].map(([id, label]) => (
              <div
                key={String(id)}
                className="flex items-start gap-2 rounded-xl border border-white/[0.06] bg-white/[0.02] px-3 py-2.5"
              >
                <CheckCircle2 size={14} className="mt-0.5 shrink-0 text-mint-300" />
                <span className="text-[11.5px] leading-snug text-ink-300">{label}</span>
              </div>
            ))}
          </div>

          <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-3">
            <div className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-[0.12em] text-ink-400">
              <History size={13} className="text-mint-400" />
              {t('net.firewall.history')}
            </div>
            {(firewallHistory.data?.length ?? 0) === 0 ? (
              <p className="mt-2 text-[11.5px] text-ink-500">{t('net.firewall.noBackups')}</p>
            ) : (
              <ul className="mt-2 space-y-1.5">
                {firewallHistory.data?.slice(0, 4).map((backup) => (
                  <li
                    key={backup.name}
                    className="flex items-center justify-between gap-2 text-[11px] text-ink-400"
                  >
                    <span className="truncate font-mono">{backup.name}</span>
                    <span className="shrink-0 text-ink-600">
                      {formatDateTime(backup.modified_at, language)}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </div>

          <details className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-3">
            <summary className="cursor-pointer text-[11px] font-semibold uppercase tracking-[0.12em] text-ink-400">
              {t('net.firewall.viewRules')}
            </summary>
            <pre className="mt-2 max-h-[260px] overflow-auto rounded-lg border border-white/[0.05] bg-[#040810] p-3 font-mono text-[10.5px] leading-relaxed text-ink-300">
              {firewall.data !== null && firewall.data.ruleset.trim() !== ''
                ? firewall.data.ruleset
                : t('net.firewall.empty')}
            </pre>
          </details>
        </section>
      </div>
    </div>
  );
}

function DiagramNode({
  icon,
  label,
  tone,
}: {
  icon: ReactNode;
  label: string;
  tone: 'mint' | 'amber' | 'neutral';
}): React.JSX.Element {
  const toneClass =
    tone === 'mint'
      ? 'border-mint-400/20 bg-mint-400/[0.06] text-mint-300'
      : tone === 'amber'
        ? 'border-amber-400/20 bg-amber-400/[0.06] text-amber-300'
        : 'border-white/[0.08] bg-white/[0.03] text-ink-400';
  return (
    <div className={cn('flex flex-col items-center gap-1.5 rounded-xl border px-2 py-2.5', toneClass)}>
      {icon}
      <span className="text-[10px] leading-tight">{label}</span>
    </div>
  );
}

function countRules(status: FirewallStatusResponse | null): number {
  if (status === null || status.ruleset.trim() === '') return 0;
  return status.ruleset
    .split('\n')
    .filter((line) => line.trim() !== '' && !line.trim().startsWith('#')).length;
}
