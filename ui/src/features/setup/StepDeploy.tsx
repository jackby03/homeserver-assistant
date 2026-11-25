import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';
import {
  CheckCircle2,
  MonitorPlay,
  Rocket,
  RotateCcw,
  ShieldCheck,
  Sparkles,
  Wifi,
  XCircle,
} from 'lucide-react';
import { authApi } from '@/api/auth';
import { setupApi } from '@/api/setup';
import type { SetupProgressMessage, SetupWsMessage } from '@/api/types';
import { isRecord } from '@/api/types';
import { ReconnectingSocket } from '@/api/ws';
import type { SocketStatus } from '@/api/ws';
import { ProgressBar } from '@/components/ProgressBar';
import { Spinner } from '@/components/Spinner';
import { StatusPill } from '@/components/StatusPill';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import { useInterval } from '@/lib/useInterval';
import { useToast } from '@/state/ToastProvider';
import type { SetupSummary } from './types';

type Phase = 'ready' | 'deploying' | 'done' | 'error';

interface LogLine {
  id: number;
  percent: number;
  label: string;
}

function parseSetupMessage(raw: unknown): SetupWsMessage | null {
  if (!isRecord(raw) || typeof raw.type !== 'string') return null;
  if (raw.type === 'finished') return { type: 'finished' };
  if (
    raw.type === 'progress' &&
    typeof raw.step === 'string' &&
    typeof raw.label === 'string' &&
    typeof raw.percent === 'number'
  ) {
    const message: SetupProgressMessage = {
      type: 'progress',
      step: raw.step,
      label: raw.label,
      percent: Math.min(100, Math.max(0, raw.percent)),
      done: raw.done === true,
    };
    return message;
  }
  return null;
}

interface StepDeployProps {
  summary: SetupSummary;
  onBack: () => void;
}

/** Step 6: apply the configuration and watch the deployment over WebSocket. */
export function StepDeploy({ summary, onBack }: StepDeployProps): React.JSX.Element {
  const { t } = useI18n();
  const toast = useToast();
  const [phase, setPhase] = useState<Phase>('ready');
  const [percent, setPercent] = useState(0);
  const [label, setLabel] = useState('');
  const [logs, setLogs] = useState<LogLine[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [socketStatus, setSocketStatus] = useState<SocketStatus>('closed');
  const socketRef = useRef<ReconnectingSocket<unknown> | null>(null);
  const nextLogId = useRef(1);

  const finishDeployment = (): void => {
    socketRef.current?.close();
    socketRef.current = null;
    setPercent(100);
    setPhase('done');
  };

  useEffect(() => {
    return () => {
      socketRef.current?.close();
      socketRef.current = null;
    };
  }, []);

  // Fallback: if the WS never reports `finished`, poll the public setup status.
  useInterval(
    () => {
      void authApi
        .setupStatus()
        .then((status) => {
          if (status.initialized) finishDeployment();
        })
        .catch(() => undefined);
    },
    phase === 'deploying' ? 2_000 : null,
  );

  const start = async (): Promise<void> => {
    setError(null);
    setPhase('deploying');
    setPercent(0);
    setLogs([]);
    setLabel('');
    try {
      await setupApi.finish();
    } catch (cause) {
      setError(t(errorMessageKey(cause)));
      setPhase('error');
      return;
    }

    const socket = new ReconnectingSocket<unknown>('/ws/setup', {
      onStatus: setSocketStatus,
      onMessage: (raw) => {
        const message = parseSetupMessage(raw);
        if (message === null) return;
        if (message.type === 'progress') {
          setPercent(message.percent);
          setLabel(message.label);
          const id = nextLogId.current;
          nextLogId.current += 1;
          setLogs((current) => [...current.slice(-40), { id, percent: Math.round(message.percent), label: message.label }]);
          if (message.done) finishDeployment();
        } else {
          finishDeployment();
          toast.push({ level: 'success', message: t('setup.step6.success.title') });
        }
      },
    });
    socketRef.current = socket;
    socket.connect();
  };

  if (phase === 'done') {
    return (
      <div className="flex flex-col items-center gap-6 py-8 text-center">
        <div className="relative grid h-20 w-20 place-items-center">
          <span
            aria-hidden="true"
            className="absolute inset-0 rounded-full border border-mint-400/25 animate-[hs-ring-pulse_3s_ease-out_infinite]"
          />
          <span className="grid h-16 w-16 place-items-center rounded-full border border-mint-500/30 bg-mint-500/[0.1] text-mint-300">
            <CheckCircle2 size={30} />
          </span>
        </div>
        <div className="space-y-2">
          <h2 className="font-display text-[26px] font-medium tracking-tight text-ink-50">
            {t('setup.step6.success.title')}
          </h2>
          <p className="mx-auto max-w-md text-[13px] leading-relaxed text-ink-400">
            {t('setup.step6.success.description')}
          </p>
        </div>
        <div className="flex flex-col gap-2 sm:flex-row">
          <Link to="/login" replace className="btn btn-primary">
            <ShieldCheck size={16} />
            {t('setup.step6.openAdmin')}
          </Link>
          <a href="/kiosk" target="_blank" rel="noreferrer" className="btn btn-secondary">
            <MonitorPlay size={16} />
            {t('setup.step6.openKiosk')}
          </a>
        </div>
        <p className="text-[11px] text-ink-500">{t('app.localFirst')}</p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-display text-[22px] font-medium tracking-tight text-ink-50">
          {t('setup.step6.title')}
        </h2>
        <p className="mt-1.5 max-w-2xl text-[13px] leading-relaxed text-ink-400">
          {t('setup.step6.description')}
        </p>
      </div>

      {/* Summary ------------------------------------------------------------- */}
      <section className="grid gap-3 sm:grid-cols-2">
        <SummaryCard
          icon={<Wifi size={15} />}
          label={t('setup.step6.device')}
          value={summary.deviceName}
          detail={`${summary.language.toUpperCase()} · ${t('app.os')}`}
        />
        <SummaryCard
          icon={<ShieldCheck size={15} />}
          label={t('setup.step6.network')}
          value={summary.mode === 'static' ? t('net.mode.static') : t('net.mode.dhcp')}
          detail={t('net.dmz.title') + ' · ' + t('net.firewall.title')}
        />
        <SummaryCard
          icon={<Sparkles size={15} />}
          label={t('setup.step6.dns')}
          value={summary.dnsEnabled ? t('common.enabled') : t('common.disabled')}
          detail={summary.dnsProvider.toUpperCase()}
        />
        <SummaryCard
          icon={<Rocket size={15} />}
          label={t('setup.step6.profiles')}
          value={String(summary.profileCount)}
          detail={summary.agentLabel ?? t('setup.step6.aiSkipped')}
        />
      </section>

      {error !== null ? (
        <p
          role="alert"
          className="flex items-start gap-2 rounded-xl border border-coral-400/25 bg-coral-500/[0.08] px-3.5 py-2.5 text-[13px] text-coral-300"
        >
          <XCircle size={15} className="mt-0.5 shrink-0" />
          {error}
        </p>
      ) : null}

      {phase === 'deploying' || phase === 'error' ? (
        <section className="space-y-3 rounded-2xl border border-white/[0.06] bg-white/[0.015] p-4">
          <div className="flex items-center justify-between gap-3">
            <p className="text-[13px] font-medium text-ink-100">{t('setup.step6.progress')}</p>
            <div className="flex items-center gap-2">
              {phase === 'deploying' ? (
                <StatusPill
                  label={socketStatus === 'open' ? t('common.live') : t('common.reconnecting')}
                  tone={socketStatus === 'open' ? 'success' : 'warning'}
                  pulse={socketStatus === 'open'}
                />
              ) : (
                <StatusPill label={t('setup.step6.failed')} tone="danger" />
              )}
              <span className="text-[12px] tabular text-ink-300">{Math.round(percent)}%</span>
            </div>
          </div>
          <ProgressBar value={percent} tone={phase === 'error' ? 'coral' : 'mint'} />
          <p className="text-[12px] text-ink-400">
            {phase === 'deploying'
              ? label !== ''
                ? label
                : t('setup.step6.waiting')
              : t('setup.step6.failed')}
          </p>
          <div className="max-h-52 overflow-y-auto rounded-xl border border-white/[0.06] bg-[#040810] p-3 font-mono text-[10.5px] leading-relaxed text-ink-400">
            {logs.length === 0 ? (
              <p className="text-ink-600">{t('setup.step6.waiting')}</p>
            ) : (
              logs.map((line) => (
                <p key={line.id} className="flex gap-3">
                  <span className="w-9 shrink-0 text-right tabular text-ink-600">{line.percent}%</span>
                  <span className="min-w-0 truncate">{line.label}</span>
                </p>
              ))
            )}
          </div>
        </section>
      ) : null}

      <div className="flex flex-col-reverse gap-3 border-t border-white/[0.06] pt-5 sm:flex-row sm:items-center sm:justify-between">
        <button
          type="button"
          className="btn btn-secondary"
          onClick={onBack}
          disabled={phase === 'deploying'}
        >
          <RotateCcw size={15} />
          {t('setup.back')}
        </button>

        {phase === 'ready' || phase === 'error' ? (
          <button type="button" className="btn btn-primary" onClick={() => void start()}>
            {phase === 'error' ? <RotateCcw size={16} /> : <Rocket size={16} />}
            {phase === 'error' ? t('setup.step6.retry') : t('setup.step6.deploy')}
          </button>
        ) : (
          <div className="flex items-center gap-2 text-[12px] text-ink-400">
            <Spinner size={15} />
            {t('setup.step6.deploying')}
          </div>
        )}
      </div>
    </div>
  );
}

function SummaryCard({
  icon,
  label,
  value,
  detail,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  detail: string;
}): React.JSX.Element {
  return (
    <div className="rounded-2xl border border-white/[0.06] bg-white/[0.015] p-3.5">
      <div className="flex items-center gap-2 text-ink-500">
        {icon}
        <span className="text-[10px] font-semibold uppercase tracking-[0.14em]">{label}</span>
      </div>
      <p className="mt-1.5 truncate font-display text-[16px] font-medium text-ink-50">{value}</p>
      <p className="mt-0.5 truncate text-[11px] text-ink-500">{detail}</p>
    </div>
  );
}
