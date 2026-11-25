import { useEffect, useState } from 'react';
import { Check } from 'lucide-react';
import type { SetupStatus } from '@/api/types';
import { BrandMark } from '@/components/BrandMark';
import { useI18n } from '@/i18n/I18nProvider';
import type { DictKey, UiLanguage } from '@/i18n/dictionary';
import { cn } from '@/lib/cn';
import type { SetupSummary, SetupSummaryPatch } from './types';
import { StepLanguage } from './StepLanguage';
import { StepNetwork } from './StepNetwork';
import { StepDevices } from './StepDevices';
import { StepDiscovery } from './StepDiscovery';
import { StepAssistant } from './StepAssistant';
import { StepDeploy } from './StepDeploy';

const STEP_KEYS: DictKey[] = [
  'setup.steps.1',
  'setup.steps.2',
  'setup.steps.3',
  'setup.steps.4',
  'setup.steps.5',
  'setup.steps.6',
];

interface SetupWizardProps {
  initialStatus: SetupStatus;
}

/** Six-step first-boot onboarding wizard. */
export function SetupWizard({ initialStatus }: SetupWizardProps): React.JSX.Element {
  const { t, language, setLanguage } = useI18n();
  const initialLanguage: UiLanguage = initialStatus.language === 'en' ? 'en' : 'es';
  const [step, setStep] = useState(1);
  const [summary, setSummary] = useState<SetupSummary>({
    deviceName: initialStatus.device_name,
    language: initialLanguage,
    mode: 'dhcp',
    dnsProvider: 'cloudflare',
    dnsEnabled: true,
    profileCount: 2,
    agentLabel: null,
  });

  // The daemon language is the default for the first run; it also drives the
  // rest of the wizard immediately.
  useEffect(() => {
    setLanguage(initialLanguage);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const advance = (patch?: SetupSummaryPatch): void => {
    if (patch !== undefined) {
      setSummary((current) => ({ ...current, ...patch }));
    }
    setStep((current) => Math.min(STEP_KEYS.length, current + 1));
  };

  const back = (): void => setStep((current) => Math.max(1, current - 1));
  const currentKey = STEP_KEYS[step - 1] ?? 'setup.steps.1';

  return (
    <div className="relative min-h-screen overflow-hidden">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute -left-40 top-[-14rem] h-[620px] w-[620px] rounded-full bg-mint-500/[0.06] blur-3xl"
      />
      <div
        aria-hidden="true"
        className="pointer-events-none absolute -right-40 bottom-[-16rem] h-[560px] w-[560px] rounded-full bg-mint-400/[0.05] blur-3xl"
      />

      <div className="relative mx-auto flex min-h-screen w-full max-w-5xl flex-col px-4 py-6 sm:px-6 lg:py-8">
        <header className="flex items-center justify-between gap-4">
          <BrandMark size={36} withWordmark />
          <div className="flex items-center gap-2.5">
            <span className="hidden text-[11px] text-ink-500 sm:inline">{t('app.localFirst')}</span>
            <div className="flex items-center gap-0.5 rounded-xl border border-white/[0.08] bg-white/[0.03] p-0.5">
              {(['es', 'en'] as UiLanguage[]).map((value) => (
                <button
                  key={value}
                  type="button"
                  onClick={() => setLanguage(value)}
                  aria-pressed={language === value}
                  className={cn(
                    'min-h-[34px] rounded-lg px-2.5 text-[11px] font-semibold uppercase transition',
                    language === value ? 'bg-white/[0.09] text-ink-50' : 'text-ink-500 hover:text-ink-200',
                  )}
                >
                  {value}
                </button>
              ))}
            </div>
          </div>
        </header>

        {/* Stepper ---------------------------------------------------------- */}
        <div className="mt-6 space-y-3">
          <div className="flex items-baseline justify-between gap-3">
            <p className="font-display text-[20px] font-medium tracking-tight text-ink-50">
              {t(currentKey)}
            </p>
            <p className="shrink-0 text-[11px] tabular text-ink-500">
              {t('setup.stepLabel', { current: step, total: STEP_KEYS.length })}
            </p>
          </div>
          <ol className="flex items-center">
            {STEP_KEYS.map((key, index) => {
              const number = index + 1;
              const done = number < step;
              const current = number === step;
              return (
                <li key={key} className="flex min-w-0 flex-1 items-center gap-2">
                  {index > 0 ? (
                    <span
                      aria-hidden="true"
                      className={cn(
                        'h-px min-w-[10px] flex-1 transition-colors duration-300',
                        done || current ? 'bg-mint-500/50' : 'bg-white/[0.09]',
                      )}
                    />
                  ) : null}
                  <span
                    className={cn(
                      'grid h-7 w-7 shrink-0 place-items-center rounded-full border text-[11px] font-semibold transition duration-300',
                      done
                        ? 'border-mint-500/40 bg-mint-500/20 text-mint-300'
                        : current
                          ? 'border-mint-400/60 bg-mint-gradient text-[#04121a] shadow-glow'
                          : 'border-white/10 bg-white/[0.03] text-ink-500',
                    )}
                    aria-current={current ? 'step' : undefined}
                  >
                    {done ? <Check size={13} /> : number}
                  </span>
                  <span
                    className={cn(
                      'hidden truncate text-[11px] transition-colors lg:block',
                      current ? 'font-medium text-ink-100' : 'text-ink-500',
                    )}
                  >
                    {t(key)}
                  </span>
                </li>
              );
            })}
          </ol>
        </div>

        {/* Step content ----------------------------------------------------- */}
        <main className="mt-5 flex-1">
          <div className="glass-strong animate-[hs-fade-up_300ms_var(--hs-ease)] p-5 sm:p-7">
            {step === 1 ? (
              <StepLanguage
                initialName={initialStatus.device_name}
                onNext={advance}
              />
            ) : null}
            {step === 2 ? <StepNetwork onNext={advance} onBack={back} /> : null}
            {step === 3 ? <StepDevices onNext={advance} onBack={back} /> : null}
            {step === 4 ? <StepDiscovery onNext={advance} onBack={back} /> : null}
            {step === 5 ? <StepAssistant onNext={advance} onBack={back} /> : null}
            {step === 6 ? (
              <StepDeploy summary={summary} onBack={back} />
            ) : null}
          </div>
        </main>

        <footer className="mt-6 flex flex-wrap items-center justify-between gap-2 text-[11px] text-ink-600">
          <span>{t('app.os')}</span>
          <span>{t('app.footer')}</span>
        </footer>
      </div>
    </div>
  );
}
