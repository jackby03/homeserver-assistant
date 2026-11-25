import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import {
  Camera,
  CheckCircle2,
  Hand,
  MonitorSmartphone,
  Play,
  RotateCcw,
  Speaker,
  X,
} from 'lucide-react';
import { setupApi } from '@/api/setup';
import type { AudioSink, SetupPeripherals } from '@/api/types';
import { peripheralLabel } from '@/api/types';
import { Field } from '@/components/Field';
import { Spinner } from '@/components/Spinner';
import { StatusPill } from '@/components/StatusPill';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import { useResource } from '@/lib/use-resource';
import { useToast } from '@/state/ToastProvider';
import { StepError, StepFooter } from './StepChrome';
import type { SetupSummaryPatch } from './types';

interface StepDevicesProps {
  onNext: (patch?: SetupSummaryPatch) => void;
  onBack: () => void;
}

interface TouchPoint {
  id: number;
  x: number;
  y: number;
}

/** Step 3: audio sink test, camera/touch detection and a touch calibration canvas. */
export function StepDevices({ onNext, onBack }: StepDevicesProps): React.JSX.Element {
  const { t } = useI18n();
  const toast = useToast();
  const peripherals = useResource<SetupPeripherals>(() => setupApi.peripherals(), []);
  const [sinkId, setSinkId] = useState('');
  const [testing, setTesting] = useState(false);
  const [played, setPlayed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [touchOpen, setTouchOpen] = useState(false);

  const data = peripherals.data;
  const sinks: AudioSink[] = data?.audio_sinks ?? [];

  useEffect(() => {
    if (data === null || sinkId !== '') return;
    const preferred = data.audio_sinks.find((sink) => sink.is_default) ?? data.audio_sinks[0];
    if (preferred !== undefined) setSinkId(preferred.id);
  }, [data, sinkId]);

  const testAudio = async (): Promise<void> => {
    if (sinkId === '') return;
    setError(null);
    setTesting(true);
    try {
      const result = await setupApi.testAudio(sinkId);
      setPlayed(result.played);
      if (result.played) {
        toast.push({ level: 'success', message: t('setup.step3.testPlayed') });
      }
    } catch (cause) {
      setError(t(errorMessageKey(cause)));
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setTesting(false);
    }
  };

  const cameraCount = data?.camera_devices.length ?? 0;
  const touchCount = data?.touch_devices.length ?? 0;

  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-display text-[22px] font-medium tracking-tight text-ink-50">
          {t('setup.step3.title')}
        </h2>
        <p className="mt-1.5 max-w-2xl text-[13px] leading-relaxed text-ink-400">
          {t('setup.step3.description')}
        </p>
      </div>

      <StepError message={error} />

      <div className="grid gap-4 lg:grid-cols-2">
        {/* Audio ------------------------------------------------------------- */}
        <section className="space-y-3 rounded-2xl border border-white/[0.06] bg-white/[0.015] p-4">
          <div className="flex items-center gap-2">
            <Speaker size={15} className="text-mint-400" />
            <h3 className="text-[13px] font-medium text-ink-100">{t('setup.step3.audio')}</h3>
          </div>
          <p className="text-[11.5px] leading-relaxed text-ink-500">{t('setup.step3.audioHint')}</p>

          {peripherals.loading ? (
            <div className="skeleton h-20 w-full" />
          ) : sinks.length === 0 ? (
            <p className="rounded-xl border border-amber-400/20 bg-amber-400/[0.06] px-3.5 py-2.5 text-[12px] text-amber-200">
              {t('setup.step3.noSinks')}
            </p>
          ) : (
            <>
              <Field label={t('setup.step3.audio')} htmlFor="setup-sink">
                <select
                  id="setup-sink"
                  className="input"
                  value={sinkId}
                  onChange={(event) => {
                    setSinkId(event.target.value);
                    setPlayed(false);
                  }}
                >
                  {sinks.map((sink) => (
                    <option key={sink.id} value={sink.id}>
                      {sink.name}
                      {sink.is_default ? ' ★' : ''}
                    </option>
                  ))}
                </select>
              </Field>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={() => void testAudio()}
                  disabled={testing || sinkId === ''}
                >
                  {testing ? <Spinner size={14} /> : <Play size={14} />}
                  {testing ? t('setup.step3.testing') : t('setup.step3.test')}
                </button>
                {played ? (
                  <StatusPill label={t('setup.step3.testPlayed')} tone="success" />
                ) : null}
              </div>
              <p className="text-[10.5px] text-ink-600">
                {sinks.find((sink) => sink.id === sinkId)?.description ?? ''}
              </p>
            </>
          )}
        </section>

        {/* Camera + touch ----------------------------------------------------- */}
        <section className="space-y-3 rounded-2xl border border-white/[0.06] bg-white/[0.015] p-4">
          <div className="flex items-center gap-2">
            <MonitorSmartphone size={15} className="text-mint-400" />
            <h3 className="text-[13px] font-medium text-ink-100">
              {t('setup.step3.camera')} & {t('setup.step3.touch')}
            </h3>
          </div>

          <div className="flex items-center justify-between gap-3 rounded-xl border border-white/[0.06] bg-white/[0.02] px-3.5 py-2.5">
            <div className="flex items-center gap-2.5">
              <Camera size={15} className={cameraCount > 0 ? 'text-mint-400' : 'text-ink-500'} />
              <div>
                <p className="text-[12.5px] text-ink-100">{t('setup.step3.camera')}</p>
                <p className="text-[10.5px] text-ink-500">
                  {cameraCount > 0
                    ? t('setup.step3.cameraFound', { count: cameraCount })
                    : t('setup.step3.cameraNone')}
                </p>
              </div>
            </div>
            <StatusPill
              label={cameraCount > 0 ? t('common.connected') : t('common.offline')}
              tone={cameraCount > 0 ? 'success' : 'neutral'}
            />
          </div>

          <div className="flex items-center justify-between gap-3 rounded-xl border border-white/[0.06] bg-white/[0.02] px-3.5 py-2.5">
            <div className="flex items-center gap-2.5">
              <Hand size={15} className={touchCount > 0 ? 'text-mint-400' : 'text-ink-500'} />
              <div>
                <p className="text-[12.5px] text-ink-100">{t('setup.step3.touch')}</p>
                <p className="text-[10.5px] text-ink-500">
                  {touchCount > 0
                    ? t('setup.step3.touchFound', { count: touchCount })
                    : t('setup.step3.touchNone')}
                </p>
              </div>
            </div>
            <StatusPill
              label={touchCount > 0 ? t('common.connected') : t('common.offline')}
              tone={touchCount > 0 ? 'success' : 'neutral'}
            />
          </div>

          {cameraCount > 0 || touchCount > 0 ? (
            <ul className="space-y-1 text-[10.5px] text-ink-500">
              {(data?.camera_devices ?? []).slice(0, 2).map((device, index) => (
                <li key={`camera-${index}`} className="truncate">
                  📷 {peripheralLabel(device, t('common.unknown'))}
                </li>
              ))}
              {(data?.touch_devices ?? []).slice(0, 2).map((device, index) => (
                <li key={`touch-${index}`} className="truncate">
                  👆 {peripheralLabel(device, t('common.unknown'))}
                </li>
              ))}
            </ul>
          ) : null}

          <button type="button" className="btn btn-secondary btn-sm w-full" onClick={() => setTouchOpen(true)}>
            <Hand size={14} />
            {t('setup.step3.touchTest')}
          </button>
          <p className="text-[10.5px] leading-relaxed text-ink-600">{t('setup.step3.note')}</p>
        </section>
      </div>

      {touchOpen ? <TouchTestOverlay onClose={() => setTouchOpen(false)} /> : null}

      <StepFooter onNext={() => onNext()} onBack={onBack} hint={t('setup.step3.note')} />
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Full-screen touch test                                                     */
/* -------------------------------------------------------------------------- */

const MAX_POINTS = 250;

function TouchTestOverlay({ onClose }: { onClose: () => void }): React.JSX.Element {
  const { t } = useI18n();
  const [points, setPoints] = useState<TouchPoint[]>([]);
  const nextId = useRef(1);
  const containerRef = useRef<HTMLDivElement | null>(null);

  const addPoint = (clientX: number, clientY: number): void => {
    const node = containerRef.current;
    if (node === null) return;
    const rect = node.getBoundingClientRect();
    const id = nextId.current;
    nextId.current += 1;
    setPoints((current) => {
      const next = [...current, { id, x: clientX - rect.left, y: clientY - rect.top }];
      return next.length > MAX_POINTS ? next.slice(next.length - MAX_POINTS) : next;
    });
  };

  return createPortal(
    <div
      ref={containerRef}
      role="dialog"
      aria-modal="true"
      aria-label={t('setup.step3.touchTest')}
      className="fixed inset-0 z-[75] select-none overflow-hidden bg-night-950/95"
      onPointerDown={(event) => addPoint(event.clientX, event.clientY)}
      onPointerMove={(event) => {
        if (event.buttons > 0) addPoint(event.clientX, event.clientY);
      }}
    >
      <div aria-hidden="true" className="pointer-events-none absolute inset-0 opacity-40">
        <div className="absolute left-1/2 top-0 h-full w-px -translate-x-1/2 bg-mint-400/20" />
        <div className="absolute left-0 top-1/2 h-px w-full -translate-y-1/2 bg-mint-400/20" />
        <div className="absolute inset-10 rounded-3xl border border-dashed border-white/10" />
      </div>

      {points.map((point) => (
        <span
          key={point.id}
          aria-hidden="true"
          className="pointer-events-none absolute h-6 w-6 -translate-x-1/2 -translate-y-1/2 rounded-full border border-mint-300/60 bg-mint-400/20"
          style={{ left: point.x, top: point.y }}
        />
      ))}

      <div className="pointer-events-none absolute inset-x-0 top-0 flex items-start justify-between p-5">
        <div className="glass-strong pointer-events-auto rounded-2xl px-4 py-3">
          <p className="text-[13px] font-medium text-ink-50">{t('setup.step3.touchTest')}</p>
          <p className="mt-0.5 text-[11px] text-ink-400">{t('setup.step3.touchHint')}</p>
          <p className="mt-1 text-[11px] tabular text-mint-300">
            {t('setup.step3.touchProgress', { count: points.length })}
          </p>
        </div>
        <div className="pointer-events-auto flex gap-2">
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={() => setPoints([])}
          >
            <RotateCcw size={14} />
            {t('setup.step3.touchClear')}
          </button>
          <button type="button" className="btn btn-primary btn-sm" onClick={onClose}>
            <CheckCircle2 size={14} />
            {t('setup.step3.touchStop')}
          </button>
        </div>
      </div>

      <button
        type="button"
        onClick={onClose}
        aria-label={t('common.close')}
        className="absolute bottom-5 left-1/2 grid h-12 w-12 -translate-x-1/2 place-items-center rounded-full border border-white/10 bg-white/[0.06] text-ink-200 transition hover:bg-white/[0.12]"
      >
        <X size={20} />
      </button>

      {points.length === 0 ? (
        <div className="pointer-events-none absolute inset-0 grid place-items-center">
          <div className="flex flex-col items-center gap-3 text-center">
            <Hand size={34} className="text-mint-400/60" />
            <p className="text-[13px] text-ink-400">{t('setup.step3.touchHint')}</p>
          </div>
        </div>
      ) : null}
    </div>,
    document.body,
  );
}
