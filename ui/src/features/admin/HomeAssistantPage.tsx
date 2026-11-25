import { useEffect, useMemo, useState } from 'react';
import {
  CheckCircle2,
  CloudOff,
  House,
  Link2,
  Play,
  Power,
  Radio,
  RefreshCw,
  ScanLine,
  Sparkles,
} from 'lucide-react';
import { discoveryApi } from '@/api/discovery';
import { homeAssistantApi } from '@/api/homeassistant';
import type {
  DiscoveredDevice,
  HomeAssistantEntity,
  HomeAssistantStatus,
} from '@/api/types';
import { EmptyState } from '@/components/EmptyState';
import { Field } from '@/components/Field';
import { PageHeader } from '@/components/PageHeader';
import { Spinner } from '@/components/Spinner';
import { StatusPill } from '@/components/StatusPill';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import { cn } from '@/lib/cn';
import { formatRelative } from '@/lib/format';
import { useResource } from '@/lib/use-resource';
import { useToast } from '@/state/ToastProvider';

const TOGGLEABLE_DOMAINS = new Set(['light', 'switch', 'fan', 'input_boolean', 'media_player', 'automation']);

export function HomeAssistantPage(): React.JSX.Element {
  const { t, language } = useI18n();
  const toast = useToast();
  const [refreshToken, setRefreshToken] = useState(0);
  const [search, setSearch] = useState('');
  const [busyEntity, setBusyEntity] = useState<string | null>(null);
  const [busyScene, setBusyScene] = useState<string | null>(null);
  const [provisioning, setProvisioning] = useState(false);
  const [url, setUrl] = useState('');
  const [token, setToken] = useState('');
  const [savingConfig, setSavingConfig] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [devices, setDevices] = useState<DiscoveredDevice[]>([]);
  const [adoptingId, setAdoptingId] = useState<string | null>(null);

  const status = useResource(() => homeAssistantApi.status(), [refreshToken], { pollMs: 10_000 });
  const entities = useResource(() => homeAssistantApi.entities(), [refreshToken], { pollMs: 15_000 });
  const currentStatus: HomeAssistantStatus | null = status.data;

  useEffect(() => {
    if (currentStatus?.url != null) setUrl(currentStatus.url);
  }, [currentStatus?.url]);

  const refresh = (): void => setRefreshToken((value) => value + 1);

  const filteredEntities = useMemo(() => {
    const all = entities.data?.entities ?? [];
    const needle = search.trim().toLocaleLowerCase();
    if (needle === '') return all;
    return all.filter(
      (entity) =>
        entity.name.toLocaleLowerCase().includes(needle) ||
        entity.entity_id.toLocaleLowerCase().includes(needle) ||
        entity.domain.toLocaleLowerCase().includes(needle),
    );
  }, [entities.data, search]);

  const grouped = useMemo(() => {
    const map = new Map<string, HomeAssistantEntity[]>();
    for (const entity of filteredEntities) {
      const list = map.get(entity.domain) ?? [];
      list.push(entity);
      map.set(entity.domain, list);
    }
    return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [filteredEntities]);

  const scenes = useMemo(
    () => (entities.data?.entities ?? []).filter((entity) => entity.domain === 'scene'),
    [entities.data],
  );

  const provision = async (): Promise<void> => {
    setProvisioning(true);
    try {
      const result = await homeAssistantApi.provision();
      toast.push({ level: 'success', message: t('ha.toast.provisioned') });
      setUrl(result.url);
      refresh();
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setProvisioning(false);
    }
  };

  const saveConfig = async (): Promise<void> => {
    if (url.trim() === '' || token.trim() === '') return;
    setSavingConfig(true);
    try {
      await homeAssistantApi.configure({ url: url.trim(), token: token.trim() });
      setToken('');
      toast.push({ level: 'success', message: t('ha.toast.configSaved') });
      refresh();
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setSavingConfig(false);
    }
  };

  const toggleEntity = async (entity: HomeAssistantEntity): Promise<void> => {
    const service = entity.state === 'on' ? 'turn_off' : 'turn_on';
    setBusyEntity(entity.entity_id);
    try {
      await homeAssistantApi.callService(entity.domain, service, { entity_id: entity.entity_id });
      toast.push({ level: 'success', message: t('ha.toast.serviceCalled') });
      entities.reload();
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setBusyEntity(null);
    }
  };

  const activateScene = async (entity: HomeAssistantEntity): Promise<void> => {
    setBusyScene(entity.entity_id);
    try {
      await homeAssistantApi.callService('scene', 'turn_on', { entity_id: entity.entity_id });
      toast.push({ level: 'success', message: t('ha.toast.sceneActivated', { name: entity.name }) });
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setBusyScene(null);
    }
  };

  const scan = async (): Promise<void> => {
    setScanning(true);
    try {
      const result = await discoveryApi.scan();
      setDevices(result.devices);
      toast.push({ level: 'success', message: t('disc.scanSuccess', { count: result.devices.length }) });
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setScanning(false);
    }
  };

  const adopt = async (device: DiscoveredDevice): Promise<void> => {
    setAdoptingId(device.id);
    try {
      await discoveryApi.adopt(device.id);
      setDevices((current) =>
        current.map((item) => (item.id === device.id ? { ...item, adopted: true } : item)),
      );
      toast.push({ level: 'success', message: t('disc.adoptSuccess') });
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setAdoptingId(null);
    }
  };

  const configured = currentStatus?.configured === true;

  return (
    <div className="space-y-5">
      <PageHeader
        title={t('ha.title')}
        subtitle={t('ha.subtitle')}
        actions={
          <button type="button" className="btn btn-secondary btn-sm" onClick={refresh}>
            <RefreshCw size={14} className={status.refreshing || entities.refreshing ? 'animate-spin' : undefined} />
            {t('common.refresh')}
          </button>
        }
      />

      {/* Status + connection -------------------------------------------------- */}
      <div className="grid gap-4 xl:grid-cols-[1.2fr_1fr]">
        <section className="glass relative overflow-hidden p-6">
          <div
            aria-hidden="true"
            className="pointer-events-none absolute -right-16 -top-20 h-48 w-48 rounded-full bg-mint-400/[0.07] blur-3xl"
          />
          <div className="relative flex flex-col gap-5 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex items-center gap-4">
              <span className="grid h-14 w-14 place-items-center rounded-2xl border border-mint-400/20 bg-mint-400/[0.07] text-mint-300">
                <House size={24} />
              </span>
              <div className="space-y-1.5">
                <h2 className="font-display text-lg font-medium text-ink-50">Home Assistant</h2>
                <div className="flex flex-wrap items-center gap-2">
                  <StatusPill
                    status={configured ? (currentStatus.running ? 'running' : 'stopped') : 'unknown'}
                    label={
                      configured
                        ? currentStatus.running
                          ? t('ha.status.running')
                          : t('ha.status.stopped')
                        : t('ha.status.notConfigured')
                    }
                    tone={configured ? (currentStatus.running ? 'success' : 'warning') : 'neutral'}
                    pulse={configured && currentStatus.running}
                  />
                  {currentStatus?.version != null ? (
                    <span className="chip tabular">v{currentStatus.version}</span>
                  ) : null}
                  {currentStatus?.instance != null ? (
                    <span className="chip font-mono">{currentStatus.instance.name}</span>
                  ) : null}
                </div>
              </div>
            </div>
            {!configured ? (
              <button
                type="button"
                className="btn btn-primary btn-sm"
                onClick={() => void provision()}
                disabled={provisioning}
              >
                {provisioning ? <Spinner size={15} className="text-[#04121a]" /> : <Sparkles size={15} />}
                {provisioning ? t('ha.provisioning') : t('ha.provision')}
              </button>
            ) : null}
          </div>
          {currentStatus?.url != null ? (
            <p className="relative mt-4 flex items-center gap-2 text-[12px] text-ink-400">
              <Link2 size={13} className="text-ink-500" />
              <a
                href={currentStatus.url}
                target="_blank"
                rel="noreferrer"
                className="font-mono text-mint-300 transition hover:text-mint-200"
              >
                {currentStatus.url}
              </a>
            </p>
          ) : null}
          {!configured ? (
            <p className="relative mt-3 text-[12px] leading-relaxed text-ink-500">{t('ha.provisionHint')}</p>
          ) : currentStatus.running ? null : (
            <p className="relative mt-3 flex items-center gap-2 text-[12px] text-amber-300">
              <CloudOff size={13} />
              {t('ha.status.stopped')}
            </p>
          )}
        </section>

        <section className="glass p-6">
          <h2 className="font-display text-[15px] font-medium text-ink-100">{t('ha.configTitle')}</h2>
          <p className="mt-1 text-[12px] leading-relaxed text-ink-500">{t('ha.configHint')}</p>
          <div className="mt-4 space-y-3.5">
            <Field label={t('ha.url')} htmlFor="ha-url">
              <input
                id="ha-url"
                className="input input-mono"
                value={url}
                onChange={(event) => setUrl(event.target.value)}
                placeholder="http://127.0.0.1:8123"
                spellCheck={false}
              />
            </Field>
            <Field label={t('ha.token')} htmlFor="ha-token" hint={t('ha.tokenHint')}>
              <input
                id="ha-token"
                className="input"
                type="password"
                value={token}
                onChange={(event) => setToken(event.target.value)}
                autoComplete="off"
                placeholder="••••••••••••"
              />
            </Field>
            <button
              type="button"
              className="btn btn-secondary btn-sm w-full"
              onClick={() => void saveConfig()}
              disabled={savingConfig || url.trim() === '' || token.trim() === ''}
            >
              {savingConfig ? <Spinner size={14} /> : <Link2 size={14} />}
              {t('ha.saveConfig')}
            </button>
          </div>
        </section>
      </div>

      {/* Scenes --------------------------------------------------------------- */}
      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h2 className="font-display text-[17px] font-medium text-ink-100">{t('ha.scenes')}</h2>
          <span className="text-[11px] text-ink-500">{scenes.length}</span>
        </div>
        {!configured ? (
          <p className="rounded-2xl border border-white/[0.06] bg-white/[0.015] px-4 py-3 text-[12.5px] text-ink-500">
            {t('ha.connectFirst')}
          </p>
        ) : scenes.length === 0 ? (
          <EmptyState icon={<Radio size={20} />} title={t('ha.noScenes')} />
        ) : (
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
            {scenes.map((scene) => (
              <button
                key={scene.entity_id}
                type="button"
                onClick={() => void activateScene(scene)}
                disabled={busyScene !== null}
                className="glass glass-hover group flex min-h-[96px] flex-col items-start justify-between gap-3 p-4 text-left"
              >
                <span className="grid h-9 w-9 place-items-center rounded-xl border border-amber-400/20 bg-amber-400/[0.08] text-amber-300">
                  {busyScene === scene.entity_id ? <Spinner size={15} /> : <Play size={15} />}
                </span>
                <span className="w-full">
                  <span className="block truncate text-[13px] font-medium text-ink-100">{scene.name}</span>
                  <span className="mt-0.5 block truncate font-mono text-[10.5px] text-ink-500">
                    {scene.entity_id}
                  </span>
                </span>
              </button>
            ))}
          </div>
        )}
      </section>

      {/* Entities ------------------------------------------------------------- */}
      <section className="space-y-3">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex items-baseline gap-2">
            <h2 className="font-display text-[17px] font-medium text-ink-100">{t('ha.entities')}</h2>
            <span className="text-[11px] text-ink-500">
              {t('ha.entityCount', { count: filteredEntities.length })}
            </span>
          </div>
          <input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={t('ha.searchEntities')}
            aria-label={t('ha.searchEntities')}
            className="input min-h-[40px] sm:w-64"
          />
        </div>
        {!configured ? (
          <p className="rounded-2xl border border-white/[0.06] bg-white/[0.015] px-4 py-3 text-[12.5px] text-ink-500">
            {t('ha.connectFirst')}
          </p>
        ) : entities.loading && entities.data === null ? (
          <div className="space-y-2">
            <div className="skeleton h-12 w-full" />
            <div className="skeleton h-12 w-full" />
            <div className="skeleton h-12 w-full" />
          </div>
        ) : grouped.length === 0 ? (
          <EmptyState title={t('ha.entitiesEmpty')} />
        ) : (
          <div className="space-y-3">
            {grouped.map(([domain, list]) => (
              <div key={domain} className="overflow-hidden rounded-2xl border border-white/[0.06]">
                <div className="flex items-center justify-between border-b border-white/[0.05] bg-white/[0.025] px-4 py-2.5">
                  <p className="font-mono text-[11.5px] uppercase tracking-wider text-ink-400">{domain}</p>
                  <span className="text-[11px] tabular text-ink-500">{list.length}</span>
                </div>
                <ul className="divide-y divide-white/[0.04]">
                  {list.map((entity) => {
                    const toggleable = TOGGLEABLE_DOMAINS.has(entity.domain) && entity.domain !== 'automation';
                    return (
                      <li
                        key={entity.entity_id}
                        className="flex flex-wrap items-center justify-between gap-2 px-4 py-2.5"
                      >
                        <div className="min-w-0">
                          <p className="truncate text-[13px] text-ink-100">{entity.name}</p>
                          <p className="truncate font-mono text-[10.5px] text-ink-500">{entity.entity_id}</p>
                        </div>
                        <div className="flex shrink-0 items-center gap-2.5">
                          <span
                            className={cn(
                              'rounded-full border px-2 py-0.5 text-[10.5px] font-medium',
                              entity.state === 'on' || entity.state === 'home'
                                ? 'border-mint-500/25 bg-mint-500/[0.08] text-mint-300'
                                : 'border-white/10 bg-white/[0.03] text-ink-400',
                            )}
                          >
                            {entity.state}
                          </span>
                          {toggleable ? (
                            <button
                              type="button"
                              onClick={() => void toggleEntity(entity)}
                              disabled={busyEntity !== null}
                              aria-label={entity.state === 'on' ? t('ha.toggleOff') : t('ha.toggleOn')}
                              title={entity.state === 'on' ? t('ha.toggleOff') : t('ha.toggleOn')}
                              className={cn(
                                'grid h-9 w-9 place-items-center rounded-lg border transition',
                                entity.state === 'on'
                                  ? 'border-mint-400/25 bg-mint-400/[0.08] text-mint-300'
                                  : 'border-white/[0.08] text-ink-400 hover:text-ink-100',
                              )}
                            >
                              {busyEntity === entity.entity_id ? <Spinner size={14} /> : <Power size={14} />}
                            </button>
                          ) : null}
                        </div>
                      </li>
                    );
                  })}
                </ul>
              </div>
            ))}
          </div>
        )}
      </section>

      {/* Discovery ------------------------------------------------------------ */}
      <section className="space-y-3">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <h2 className="font-display text-[17px] font-medium text-ink-100">{t('disc.title')}</h2>
            <p className="text-[12px] text-ink-500">{t('disc.subtitle')}</p>
          </div>
          <button type="button" className="btn btn-secondary btn-sm" onClick={() => void scan()} disabled={scanning}>
            {scanning ? <Spinner size={14} /> : <ScanLine size={15} />}
            {scanning ? t('disc.scanning') : t('disc.scan')}
          </button>
        </div>
        {devices.length === 0 ? (
          <EmptyState icon={<ScanLine size={20} />} title={t('disc.empty')} body={t('disc.note')} />
        ) : (
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {devices.map((device) => (
              <article key={device.id} className="glass p-4">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <p className="truncate text-[13px] font-medium text-ink-100">
                      {device.hostname ?? device.ip}
                    </p>
                    <p className="mt-0.5 truncate font-mono text-[10.5px] text-ink-500">
                      {device.ip}
                      {device.mac !== null ? ` · ${device.mac}` : ''}
                    </p>
                  </div>
                  {device.adopted ? (
                    <StatusPill label={t('disc.adopted')} tone="success" />
                  ) : (
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm shrink-0"
                      onClick={() => void adopt(device)}
                      disabled={adoptingId !== null}
                    >
                      {adoptingId === device.id ? <Spinner size={13} /> : null}
                      {t('disc.adopt')}
                    </button>
                  )}
                </div>
                <div className="mt-3 flex flex-wrap items-center gap-1.5">
                  {device.vendor !== null ? <span className="chip">{device.vendor}</span> : null}
                  {device.protocols.map((protocol) => (
                    <span key={protocol} className="chip font-mono uppercase">
                      {protocol}
                    </span>
                  ))}
                </div>
                <p className="mt-2 text-[10.5px] text-ink-600">
                  {t('disc.lastSeen')}: {formatRelative(device.last_seen, language)}
                </p>
              </article>
            ))}
          </div>
        )}
      </section>

      <p className="flex items-center gap-1.5 text-[11px] text-ink-600">
        <CheckCircle2 size={12} />
        {t('disc.note')}
      </p>
    </div>
  );
}
