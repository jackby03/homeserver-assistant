import { useEffect, useRef, useState } from 'react';
import { Link } from 'react-router-dom';
import {
  Activity,
  BatteryCharging,
  CheckCircle2,
  Cpu,
  Database,
  HardDrive,
  ListChecks,
  MemoryStick,
  Network,
  Server,
  Thermometer,
  TriangleAlert,
} from 'lucide-react';
import { systemApi } from '@/api/system';
import { isRecord } from '@/api/types';
import type { AuditEntry, SystemOverview } from '@/api/types';
import { Gauge } from '@/components/Gauge';
import { MetricCard } from '@/components/MetricCard';
import { ProgressBar } from '@/components/ProgressBar';
import { Sparkline } from '@/components/Sparkline';
import { StatusPill } from '@/components/StatusPill';
import { SkeletonCard } from '@/components/Skeleton';
import { useI18n } from '@/i18n/I18nProvider';
import { cn } from '@/lib/cn';
import { useLiveEvents } from '@/lib/use-live-events';
import { useResource } from '@/lib/use-resource';
import { useNow } from '@/lib/useInterval';
import { RingBuffer } from '@/lib/ring-buffer';
import {
  formatBytes,
  formatDateTime,
  formatDuration,
  formatNumber,
  formatPercent,
  formatRate,
  formatRelative,
  toPercent,
} from '@/lib/format';
import { useAuth } from '@/state/AuthProvider';

const CPU_HISTORY = 48;

/** Extract an overview payload from loosely typed event frames. */
function extractOverview(message: unknown): SystemOverview | null {
  if (!isRecord(message)) return null;
  const candidates: unknown[] = [message, message.overview, message.data, message.payload];
  for (const candidate of candidates) {
    if (!isRecord(candidate)) continue;
    const cpu = candidate.cpu;
    if (typeof candidate.hostname === 'string' && isRecord(cpu) && typeof cpu.usage_percent === 'number') {
      return candidate as unknown as SystemOverview;
    }
  }
  return null;
}

interface NetworkRates {
  [name: string]: { sent: number; received: number };
}

function greetingKey(hour: number): 'dash.greeting.morning' | 'dash.greeting.afternoon' | 'dash.greeting.evening' | 'dash.greeting.night' {
  if (hour >= 5 && hour < 12) return 'dash.greeting.morning';
  if (hour >= 12 && hour < 19) return 'dash.greeting.afternoon';
  if (hour >= 19 && hour < 23) return 'dash.greeting.evening';
  return 'dash.greeting.night';
}

export function DashboardPage(): React.JSX.Element {
  const { t, language } = useI18n();
  const { user } = useAuth();
  const now = useNow(30_000);

  const overview = useResource(() => systemApi.overview(), [], { pollMs: 5_000 });
  const audit = useResource(() => systemApi.audit(50), []);
  const data = overview.data;

  const socketStatus = useLiveEvents((event) => {
    const next = extractOverview(event);
    if (next !== null) {
      overview.update(next);
    }
  });

  const cpuHistory = useRef(new RingBuffer<number>(CPU_HISTORY));
  const memHistory = useRef(new RingBuffer<number>(CPU_HISTORY));
  const [history, setHistory] = useState<{ cpu: number[]; mem: number[] }>({ cpu: [], mem: [] });
  const networkSnapshot = useRef<{ at: number; bytes: NetworkRates } | null>(null);
  const [rates, setRates] = useState<NetworkRates>({});

  const generatedAt = data?.generated_at ?? null;
  useEffect(() => {
    if (data === null) return;
    cpuHistory.current.push(data.cpu.usage_percent);
    memHistory.current.push(toPercent(data.memory.used_mb, data.memory.total_mb));
    setHistory({ cpu: cpuHistory.current.values(), mem: memHistory.current.values() });
  }, [generatedAt, data]);

  useEffect(() => {
    if (data === null) return;
    const at = Date.now();
    const current: NetworkRates = {};
    for (const iface of data.network) {
      current[iface.name] = { sent: iface.bytes_sent, received: iface.bytes_received };
    }
    const previous = networkSnapshot.current;
    if (previous !== null) {
      const seconds = (at - previous.at) / 1000;
      if (seconds > 0.5) {
        const next: NetworkRates = {};
        for (const [name, stats] of Object.entries(current)) {
          const before = previous.bytes[name];
          if (before === undefined) continue;
          next[name] = {
            sent: Math.max(0, (stats.sent - before.sent) / seconds),
            received: Math.max(0, (stats.received - before.received) / seconds),
          };
        }
        setRates(next);
      }
    }
    networkSnapshot.current = { at, bytes: current };
  }, [generatedAt, data]);

  const attention = buildAttention(data, t);
  const deviceName = data?.hostname ?? t('app.name');
  const greeting = t(greetingKey(now.getHours()), { name: user?.display_name ?? '' });
  const liveLabel =
    socketStatus === 'open'
      ? t('dash.live')
      : socketStatus === 'connecting'
        ? t('dash.wsConnecting')
        : t('dash.pollingFallback');

  if (overview.loading && data === null) {
    return (
      <div className="space-y-4">
        <SkeletonCard className="h-40" />
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <SkeletonCard lines={3} />
          <SkeletonCard lines={3} />
          <SkeletonCard lines={3} />
          <SkeletonCard lines={3} />
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-5">
      {overview.error !== null && data === null ? (
        <div className="flex items-center justify-between gap-4 rounded-2xl border border-coral-400/25 bg-coral-500/[0.07] px-4 py-3">
          <p className="text-sm text-coral-200">{t('dash.error')}</p>
          <button type="button" className="btn btn-secondary btn-sm" onClick={overview.reload}>
            {t('common.retry')}
          </button>
        </div>
      ) : null}

      {/* Hero ---------------------------------------------------------------- */}
      <section className="glass relative overflow-hidden p-6 sm:p-8">
        <div
          aria-hidden="true"
          className="pointer-events-none absolute -right-24 -top-28 h-64 w-64 rounded-full bg-mint-400/[0.08] blur-3xl"
        />
        <div
          aria-hidden="true"
          className="pointer-events-none absolute -bottom-32 left-1/3 h-56 w-56 rounded-full bg-mint-500/[0.05] blur-3xl"
        />
        <div className="relative flex flex-col gap-8 lg:flex-row lg:items-center lg:justify-between">
          <div className="min-w-0 space-y-3">
            <p className="section-label">{formatDateTime(now.toISOString(), language, { weekday: 'long', day: 'numeric', month: 'long' })}</p>
            <h2 className="font-display text-[28px] font-medium leading-tight tracking-tight text-ink-50 sm:text-[34px]">
              {greeting}
            </h2>
            <p className="max-w-xl text-sm leading-relaxed text-ink-400">
              {attention.length === 0
                ? t('dash.heroHint', { device: deviceName })
                : t('dash.heroAttention', { count: attention.length })}
            </p>
            <div className="flex flex-wrap items-center gap-2 pt-1">
              <StatusPill label={liveLabel} tone={socketStatus === 'open' ? 'success' : 'neutral'} pulse={socketStatus === 'open'} />
              {data !== null ? (
                <>
                  <span className="chip tabular">
                    <Activity size={12} className="text-mint-400" />
                    {t('dash.uptime', { time: formatDuration(data.uptime_secs, language) })}
                  </span>
                  <span className="chip tabular">{t('dash.kernel', { version: data.kernel })}</span>
                  <span className="chip hidden sm:inline-flex">{data.os}</span>
                </>
              ) : null}
            </div>
          </div>

          <div className="shrink-0 lg:max-w-xs">
            {attention.length === 0 ? (
              <div className="flex items-center gap-3 rounded-2xl border border-mint-500/20 bg-mint-500/[0.06] px-4 py-3.5">
                <CheckCircle2 size={20} className="text-mint-400" />
                <p className="text-[13px] leading-snug text-mint-200">{t('dash.heroHint', { device: deviceName })}</p>
              </div>
            ) : (
              <ul className="space-y-2">
                {attention.slice(0, 3).map((item) => (
                  <li
                    key={item}
                    className="flex items-center gap-2.5 rounded-xl border border-amber-400/20 bg-amber-400/[0.06] px-3 py-2.5 text-[12px] leading-snug text-amber-200"
                  >
                    <TriangleAlert size={14} className="shrink-0 text-amber-300" />
                    <span className="truncate">{item}</span>
                  </li>
                ))}
                {attention.length > 3 ? (
                  <li className="px-1 text-[11px] text-ink-500">
                    +{attention.length - 3}
                  </li>
                ) : null}
              </ul>
            )}
          </div>
        </div>
      </section>

      {/* Primary metrics ----------------------------------------------------- */}
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        <MetricCard
          title={t('dash.cpu')}
          icon={<Cpu size={15} />}
          action={<span className="text-[11px] text-ink-500">{data !== null ? t('dash.cpuCores', { count: data.cpu.cores }) : ''}</span>}
        >
          <div className="flex items-center justify-between gap-4">
            <Gauge value={data?.cpu.usage_percent ?? 0} label="" size={104} />
            <div className="min-w-0 flex-1">
              <Sparkline values={history.cpu} tone="mint" ariaLabel={t('dash.cpu')} />
              <p className="mt-3 text-[11px] leading-relaxed text-ink-500">
                {data !== null
                  ? t('dash.cpuLoad', {
                      one: formatNumber(data.cpu.load_1, language, 2),
                      five: formatNumber(data.cpu.load_5, language, 2),
                      fifteen: formatNumber(data.cpu.load_15, language, 2),
                    })
                  : ''}
              </p>
            </div>
          </div>
        </MetricCard>

        <MetricCard
          title={t('dash.memory')}
          icon={<MemoryStick size={15} />}
          accent="amber"
          value={data !== null ? formatPercent(toPercent(data.memory.used_mb, data.memory.total_mb), language) : '—'}
          caption={data !== null ? t('dash.memoryUsed', {
            used: formatBytes(data.memory.used_mb * 1024 * 1024, language),
            total: formatBytes(data.memory.total_mb * 1024 * 1024, language),
          }) : undefined}
        >
          <Sparkline values={history.mem} tone="amber" ariaLabel={t('dash.memory')} className="mt-1" />
          {data !== null && data.memory.swap_total_mb > 0 ? (
            <p className="text-[11px] text-ink-500">
              {t('dash.swap', {
                used: formatBytes(data.memory.swap_used_mb * 1024 * 1024, language),
                total: formatBytes(data.memory.swap_total_mb * 1024 * 1024, language),
              })}
            </p>
          ) : null}
        </MetricCard>

        <MetricCard
          title={t('dash.temperature')}
          icon={<Thermometer size={15} />}
          accent="coral"
          value={data?.temperature_c !== null && data?.temperature_c !== undefined ? `${formatNumber(data.temperature_c, language, 0)} °C` : '—'}
          caption={t('dash.temperatureHint')}
        >
          <div className="mt-1 flex items-end gap-2">
            <div className="h-10 flex-1 overflow-hidden rounded-lg bg-white/[0.03]">
              <div
                className={cn(
                  'h-full rounded-lg transition-all duration-700 ease-smooth',
                  temperatureToneClass(data?.temperature_c ?? null),
                )}
                style={{ width: `${temperaturePercent(data?.temperature_c ?? null)}%`, opacity: 0.55 }}
              />
            </div>
          </div>
        </MetricCard>

        <MetricCard
          title={t('dash.battery')}
          icon={<BatteryCharging size={15} />}
          accent="mint"
          value={
            data?.battery !== null && data?.battery !== undefined
              ? formatPercent(data.battery.percent, language, 0)
              : '—'
          }
          caption={data?.battery == null ? t('dash.batteryNone') : undefined}
          action={
            data?.battery?.charging === true ? (
              <StatusPill label={t('dash.batteryCharging')} tone="warning" pulse />
            ) : undefined
          }
        >
          {data?.battery != null ? (
            <div className="space-y-2">
              <ProgressBar value={data.battery.percent} tone={data.battery.percent < 20 ? 'coral' : 'mint'} />
              <p className="text-[11px] text-ink-500">
                {data.battery.time_remaining_min !== null
                  ? t('dash.batteryRemaining', { time: formatDuration(data.battery.time_remaining_min * 60, language) })
                  : '—'}
              </p>
            </div>
          ) : null}
        </MetricCard>
      </div>

      {/* Storage + network --------------------------------------------------- */}
      <div className="grid gap-4 xl:grid-cols-2">
        <MetricCard
          title={t('dash.disks')}
          icon={<HardDrive size={15} />}
          action={<span className="text-[11px] text-ink-500">{data?.disks.length ?? 0}</span>}
        >
          {data === null || data.disks.length === 0 ? (
            <p className="text-sm text-ink-500">{t('dash.noDisks')}</p>
          ) : (
            <ul className="space-y-3.5">
              {data.disks.map((disk) => (
                <li key={disk.mount} className="space-y-1.5">
                  <div className="flex items-baseline justify-between gap-3">
                    <div className="min-w-0">
                      <span className="flex items-center gap-2 text-[13px] font-medium text-ink-100">
                        <Database size={13} className="shrink-0 text-ink-500" />
                        <span className="truncate font-mono text-[12px]">{disk.mount}</span>
                      </span>
                    </div>
                    <span className="shrink-0 text-[11px] tabular text-ink-400">
                      {formatPercent(disk.usage_percent, language)} · {disk.kind}
                    </span>
                  </div>
                  <ProgressBar
                    value={disk.usage_percent}
                    tone={disk.usage_percent >= 90 ? 'coral' : disk.usage_percent >= 75 ? 'amber' : 'mint'}
                  />
                  <p className="text-[11px] tabular text-ink-500">
                    {t('dash.diskFree', {
                      free: formatBytes(disk.available_gb * 1024 ** 3, language),
                      total: formatBytes(disk.total_gb * 1024 ** 3, language),
                    })}
                    <span className="text-ink-600"> · {disk.filesystem}</span>
                  </p>
                </li>
              ))}
            </ul>
          )}
        </MetricCard>

        <MetricCard title={t('dash.network')} icon={<Network size={15} />}>
          {data === null || data.network.length === 0 ? (
            <p className="text-sm text-ink-500">{t('dash.noNetwork')}</p>
          ) : (
            <ul className="space-y-3">
              {data.network.map((iface) => {
                const rate = rates[iface.name];
                return (
                  <li
                    key={iface.name}
                    className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-white/[0.05] bg-white/[0.02] px-3 py-2.5"
                  >
                    <div className="min-w-0">
                      <p className="flex items-center gap-2 font-mono text-[12px] text-ink-100">
                        {iface.name}
                        <span className="font-sans text-[11px] text-ink-500">{iface.ipv4.join(', ') || '—'}</span>
                      </p>
                      <p className="text-[10px] uppercase tracking-wider text-ink-600">{iface.mac}</p>
                    </div>
                    <div className="flex gap-4 text-right text-[11px] tabular">
                      <div>
                        <p className="text-ink-500">{t('dash.networkTx')}</p>
                        <p className="text-mint-300">{rate !== undefined ? formatRate(rate.sent, language) : '—'}</p>
                      </div>
                      <div>
                        <p className="text-ink-500">{t('dash.networkRx')}</p>
                        <p className="text-mint-300">{rate !== undefined ? formatRate(rate.received, language) : '—'}</p>
                      </div>
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </MetricCard>
      </div>

      {/* Services + audit ---------------------------------------------------- */}
      <div className="grid gap-4 xl:grid-cols-2">
        <MetricCard
          title={t('dash.services')}
          icon={<Server size={15} />}
          action={
            data !== null ? (
              <span className="text-[11px] text-ink-500">
                {t('dash.servicesSummary', {
                  active: data.services.filter((service) => service.active).length,
                  total: data.services.length,
                })}
              </span>
            ) : undefined
          }
        >
          {data === null || data.services.length === 0 ? (
            <p className="text-sm text-ink-500">{t('dash.noServices')}</p>
          ) : (
            <ul className="divide-y divide-white/[0.04]">
              {data.services.map((service) => (
                <li key={service.name} className="flex items-center justify-between gap-3 py-2 first:pt-0 last:pb-0">
                  <div className="flex min-w-0 items-center gap-2.5">
                    <span
                      aria-hidden="true"
                      className={cn(
                        'h-2 w-2 shrink-0 rounded-full',
                        service.active ? 'bg-mint-500' : 'bg-coral-400',
                      )}
                    />
                    <span className="truncate font-mono text-[12px] text-ink-200">{service.name}</span>
                  </div>
                  <div className="flex shrink-0 items-center gap-2.5">
                    {service.detail !== null ? (
                      <span className="hidden max-w-[180px] truncate text-[11px] text-ink-500 sm:inline">
                        {service.detail}
                      </span>
                    ) : null}
                    <StatusPill
                      label={service.active ? t('dash.serviceUp') : t('dash.serviceDown')}
                      tone={service.active ? 'success' : 'danger'}
                    />
                  </div>
                </li>
              ))}
            </ul>
          )}
        </MetricCard>

        <MetricCard
          title={t('dash.audit')}
          icon={<ListChecks size={15} />}
          action={
            <Link
              to="/admin/audit"
              className="text-[11px] font-medium text-mint-400 transition hover:text-mint-300"
            >
              {t('dash.auditViewAll')}
            </Link>
          }
        >
          {audit.loading && audit.data === null ? (
            <div className="space-y-2">
              <div className="skeleton h-4 w-full" />
              <div className="skeleton h-4 w-5/6" />
              <div className="skeleton h-4 w-2/3" />
            </div>
          ) : audit.data === null || audit.data.length === 0 ? (
            <p className="text-sm text-ink-500">{t('dash.auditEmpty')}</p>
          ) : (
            <ul className="divide-y divide-white/[0.04]">
              {audit.data.slice(0, 6).map((entry) => (
                <AuditRow key={entry.seq} entry={entry} />
              ))}
            </ul>
          )}
        </MetricCard>
      </div>
    </div>
  );
}

function AuditRow({ entry }: { entry: AuditEntry }): React.JSX.Element {
  const { language } = useI18n();
  return (
    <li className="flex items-center justify-between gap-3 py-2 first:pt-0 last:pb-0">
      <div className="min-w-0">
        <p className="truncate font-mono text-[12px] text-ink-200">{entry.action}</p>
        <p className="truncate text-[11px] text-ink-500">
          {entry.actor}
          {entry.target !== null ? ` · ${entry.target}` : ''} · {formatRelative(entry.timestamp, language)}
        </p>
      </div>
      <StatusPill status={entry.outcome} />
    </li>
  );
}

function buildAttention(
  data: SystemOverview | null,
  t: (key: 'dash.attentionBattery' | 'dash.attentionTemperature' | 'dash.attentionService' | 'dash.attentionDisk', vars?: Record<string, string | number>) => string,
): string[] {
  if (data === null) return [];
  const items: string[] = [];
  if (data.battery !== null && data.battery.present && data.battery.percent < 20 && !data.battery.charging) {
    items.push(t('dash.attentionBattery', { percent: Math.round(data.battery.percent) }));
  }
  if (data.temperature_c !== null && data.temperature_c >= 80) {
    items.push(t('dash.attentionTemperature', { value: Math.round(data.temperature_c) }));
  }
  for (const service of data.services) {
    if (!service.active) items.push(t('dash.attentionService', { name: service.name }));
  }
  for (const disk of data.disks) {
    if (disk.usage_percent >= 90) items.push(t('dash.attentionDisk', { mount: disk.mount }));
  }
  return items;
}

function temperaturePercent(value: number | null): number {
  if (value === null || !Number.isFinite(value)) return 0;
  return Math.min(100, Math.max(0, ((value - 20) / 70) * 100));
}

function temperatureToneClass(value: number | null): string {
  if (value === null) return 'bg-white/10';
  if (value >= 80) return 'bg-coral-gradient';
  if (value >= 65) return 'bg-amber-gradient';
  return 'bg-mint-gradient';
}
