import { useEffect, useState } from 'react';
import { Mic, Save, ShieldCheck } from 'lucide-react';
import { voiceApi } from '@/api/voice';
import type { VoiceConfig, VoiceStatus } from '@/api/types';
import { Field, Segmented, Toggle } from '@/components/Field';
import { Spinner } from '@/components/Spinner';
import { StatusPill } from '@/components/StatusPill';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import { cn } from '@/lib/cn';
import { useToast } from '@/state/ToastProvider';

/**
 * Always-on voice assistant settings: customizable wake name, activation mode,
 * sensitivity and listening window. Everything runs locally: the kiosk sends
 * only transcripts produced on the appliance.
 */
export function VoiceSection(): React.JSX.Element {
  const { t } = useI18n();
  const toast = useToast();
  const [config, setConfig] = useState<VoiceConfig | null>(null);
  const [status, setStatus] = useState<VoiceStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    voiceApi
      .config()
      .then((response) => {
        if (cancelled) return;
        setConfig(response.config);
        setStatus(response.status);
      })
      .catch(() => {
        if (!cancelled) setError(t('errors.generic'));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [t]);

  const update = (patch: Partial<VoiceConfig>): void => {
    setConfig((current) => (current === null ? current : { ...current, ...patch }));
  };

  const save = async (): Promise<void> => {
    if (config === null) return;
    setSaving(true);
    setError(null);
    try {
      const response = await voiceApi.update(config);
      setConfig(response.config);
      setStatus(response.status);
      toast.push({ level: 'success', message: t('agent.voice.saved') });
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setSaving(false);
    }
  };

  const statusLabel =
    status === null
      ? ''
      : !status.wake_word_enabled
        ? t('agent.voice.status.disabled')
        : status.listening
          ? t('agent.voice.status.listening')
          : t('agent.voice.status.idle');

  return (
    <section className="glass space-y-4 p-5 sm:p-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex items-center gap-2.5">
          <span className="grid h-8 w-8 place-items-center rounded-xl border border-cyan-400/20 bg-cyan-400/[0.07] text-cyan-300">
            <Mic size={16} />
          </span>
          <div>
            <h2 className="font-display text-[15px] font-medium text-ink-100">
              {t('agent.voice.title')}
            </h2>
            <p className="text-[11px] text-ink-500">{t('agent.voice.subtitle')}</p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {status !== null ? (
            <StatusPill
              label={statusLabel}
              tone={status.listening ? 'success' : status.wake_word_enabled ? 'neutral' : 'warning'}
              pulse={status.listening}
            />
          ) : null}
          <StatusPill label={String(status?.activations ?? 0)} tone="neutral" />
        </div>
      </div>

      {loading ? (
        <div className="flex items-center gap-2 text-[12px] text-ink-400">
          <Spinner size={14} /> {t('common.loading')}
        </div>
      ) : config === null ? (
        <p className="text-[12.5px] text-coral-300">{error}</p>
      ) : (
        <>
          <div className="grid gap-4 sm:grid-cols-2">
            <Field
              label={t('agent.voice.name')}
              htmlFor="voice-name"
              hint={t('agent.voice.nameHint', { name: config.assistant_name || '…' })}
            >
              <input
                id="voice-name"
                className="input"
                value={config.assistant_name}
                maxLength={32}
                onChange={(event) => update({ assistant_name: event.target.value })}
                spellCheck={false}
              />
            </Field>
            <div className="flex items-end justify-end gap-3 pb-1">
              <div className="flex flex-col items-end gap-2">
              <Toggle
                checked={config.wake_word_enabled}
                onChange={(wake_word_enabled) => update({ wake_word_enabled })}
                label={t('agent.voice.wakeEnabled')}
              />
            </div>
            </div>
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <Field label={t('agent.voice.mode')}>
              <Segmented
                value={config.activation_mode}
                onChange={(activation_mode) => update({ activation_mode })}
                options={[
                  { value: 'wake_word', label: t('agent.voice.mode.wake_word') },
                  { value: 'push_to_talk', label: t('agent.voice.mode.push_to_talk') },
                  { value: 'both', label: t('agent.voice.mode.both') },
                ]}
              />
            </Field>
            <Field label={t('agent.voice.language')}>
              <Segmented
                value={config.language}
                onChange={(language) => update({ language })}
                options={[
                  { value: 'es', label: 'Español' },
                  { value: 'en', label: 'English' },
                ]}
              />
            </Field>
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <Field label={t('agent.voice.sensitivity')} hint={t('agent.voice.sensitivityHint')}>
              <div className="flex items-center gap-3">
                <input
                  type="range"
                  min={0}
                  max={1}
                  step={0.05}
                  value={config.sensitivity}
                  onChange={(event) => update({ sensitivity: Number(event.target.value) })}
                  className="h-1.5 flex-1 accent-mint-400"
                  aria-label={t('agent.voice.sensitivity')}
                />
                <span className="w-10 text-right text-[12px] tabular text-ink-300">
                  {config.sensitivity.toFixed(2)}
                </span>
              </div>
            </Field>
            <Field label={t('agent.voice.window')} htmlFor="voice-window">
              <input
                id="voice-window"
                type="number"
                className="input"
                min={3}
                max={60}
                value={config.listening_window_secs}
                onChange={(event) =>
                  update({ listening_window_secs: Number(event.target.value) })
                }
              />
            </Field>
          </div>

          {status !== null && config.wake_word_enabled && !status.listener_connected ? (
            <p className="rounded-xl border border-amber-400/25 bg-amber-400/[0.06] px-3.5 py-2.5 text-[11.5px] leading-relaxed text-amber-200">
              {t('agent.voice.listenerDisconnected')}
            </p>
          ) : null}

          <div className="flex flex-wrap items-center justify-between gap-3 border-t border-white/[0.06] pt-4">
            <p className="flex items-center gap-1.5 text-[11px] text-ink-500">
              <ShieldCheck size={13} className="text-mint-400" />
              {status?.listener_connected
                ? t('agent.voice.listenerConnected')
                : t('agent.voice.localOnly')}
            </p>
            <button
              type="button"
              className="btn btn-primary btn-sm"
              onClick={() => void save()}
              disabled={saving}
            >
              {saving ? <Spinner size={14} className="text-[#04121a]" /> : <Save size={14} />}
              {t('agent.voice.save')}
            </button>
          </div>
          <p className={cn('text-[10.5px] text-ink-600', error === null && 'hidden')}>{error}</p>
        </>
      )}
    </section>
  );
}

