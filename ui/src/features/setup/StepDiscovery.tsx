import { useState } from 'react';
import { CheckCircle2, ScanLine, ShieldCheck, Wifi } from 'lucide-react';
import { discoveryApi } from '@/api/discovery';
import type { DiscoveredDevice } from '@/api/types';
import { EmptyState } from '@/components/EmptyState';
import { Spinner } from '@/components/Spinner';
import { StatusPill } from '@/components/StatusPill';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import { formatRelative } from '@/lib/format';
import { useResource } from '@/lib/use-resource';
import { useToast } from '@/state/ToastProvider';
import { StepError, StepFooter } from './StepChrome';
import type { SetupSummaryPatch } from './types';

interface StepDiscoveryProps {
  onNext: (patch?: SetupSummaryPatch) => void;
  onBack: () => void;
}

/** Step 4: local network discovery and Home Assistant adoption. */
export function StepDiscovery({ onNext, onBack }: StepDiscoveryProps): React.JSX.Element {
  const { t, language } = useI18n();
  const toast = useToast();
  const devices = useResource(() => discoveryApi.devices(), []);
  const [scanning, setScanning] = useState(false);
  const [adoptingId, setAdoptingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const list = devices.data?.devices ?? [];

  const scan = async (): Promise<void> => {
    setError(null);
    setScanning(true);
    try {
      const result = await discoveryApi.scan();
      devices.update(result);
      toast.push({
        level: 'success',
        message: t('setup.step4.found', { count: result.devices.length }),
      });
    } catch (cause) {
      setError(t(errorMessageKey(cause)));
    } finally {
      setScanning(false);
    }
  };

  const adopt = async (device: DiscoveredDevice): Promise<void> => {
    setAdoptingId(device.id);
    try {
      await discoveryApi.adopt(device.id);
      devices.update({
        devices: list.map((item) => (item.id === device.id ? { ...item, adopted: true } : item)),
      });
      toast.push({ level: 'success', message: t('disc.adoptSuccess') });
    } catch (cause) {
      setError(t(errorMessageKey(cause)));
    } finally {
      setAdoptingId(null);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-display text-[22px] font-medium tracking-tight text-ink-50">
          {t('setup.step4.title')}
        </h2>
        <p className="mt-1.5 max-w-2xl text-[13px] leading-relaxed text-ink-400">
          {t('setup.step4.description')}
        </p>
      </div>

      <StepError message={error ?? (devices.error !== null ? t(errorMessageKey(devices.error)) : null)} />

      <div className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-white/[0.06] bg-white/[0.015] px-4 py-3">
        <div className="flex items-center gap-2">
          <ScanLine size={15} className="text-mint-400" />
          <span className="text-[13px] font-medium text-ink-100">{t('disc.devices')}</span>
          <span className="text-[11px] tabular text-ink-500">
            {t('setup.step4.found', { count: list.length })}
          </span>
        </div>
        <button type="button" className="btn btn-secondary btn-sm" onClick={() => void scan()} disabled={scanning}>
          {scanning ? <Spinner size={14} /> : <ScanLine size={14} />}
          {scanning ? t('setup.step4.scanning') : t('setup.step4.scan')}
        </button>
      </div>

      {devices.loading && list.length === 0 ? (
        <div className="space-y-2">
          <div className="skeleton h-14 w-full rounded-xl" />
          <div className="skeleton h-14 w-full rounded-xl" />
        </div>
      ) : list.length === 0 ? (
        <EmptyState icon={<Wifi size={20} />} title={t('setup.step4.none')} body={t('disc.note')} />
      ) : (
        <ul className="space-y-2">
          {list.map((device) => (
            <li
              key={device.id}
              className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-white/[0.06] bg-white/[0.02] px-4 py-3"
            >
              <div className="min-w-0">
                <p className="truncate text-[13px] text-ink-100">{device.hostname ?? device.ip}</p>
                <p className="mt-0.5 truncate font-mono text-[10.5px] text-ink-500">
                  {device.ip}
                  {device.mac !== null ? ` · ${device.mac}` : ''}
                  {device.vendor !== null ? ` · ${device.vendor}` : ''}
                </p>
                <p className="mt-1 flex flex-wrap items-center gap-1.5">
                  {device.protocols.map((protocol) => (
                    <span key={protocol} className="chip font-mono uppercase">
                      {protocol}
                    </span>
                  ))}
                  <span className="text-[10px] text-ink-600">
                    {t('disc.lastSeen')}: {formatRelative(device.last_seen, language)}
                  </span>
                </p>
              </div>
              {device.adopted ? (
                <StatusPill label={t('setup.step4.adopted')} tone="success" />
              ) : (
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={() => void adopt(device)}
                  disabled={adoptingId !== null}
                >
                  {adoptingId === device.id ? <Spinner size={13} /> : <ShieldCheck size={13} />}
                  {adoptingId === device.id ? t('setup.step4.adopting') : t('setup.step4.adopt')}
                </button>
              )}
            </li>
          ))}
        </ul>
      )}

      <div className="flex items-start gap-2 rounded-xl border border-white/[0.05] bg-white/[0.015] px-3.5 py-2.5">
        <CheckCircle2 size={14} className="mt-0.5 shrink-0 text-mint-400" />
        <p className="text-[11px] leading-relaxed text-ink-500">{t('disc.note')}</p>
      </div>

      <StepFooter
        onNext={() => onNext()}
        onBack={onBack}
        hint={t('setup.step4.skipNote')}
      />
    </div>
  );
}
