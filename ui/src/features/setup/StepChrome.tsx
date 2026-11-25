import type { ReactNode } from 'react';
import { ArrowLeft, ArrowRight } from 'lucide-react';
import { Spinner } from '@/components/Spinner';
import { useI18n } from '@/i18n/I18nProvider';

interface StepFooterProps {
  onNext: () => void;
  onBack?: () => void;
  nextLabel?: string;
  backLabel?: string;
  busy?: boolean;
  disabled?: boolean;
  hint?: ReactNode;
  /** Extra secondary actions rendered before the primary button. */
  secondary?: ReactNode;
}

/** Shared footer for wizard steps: back, hint and a primary continue action. */
export function StepFooter({
  onNext,
  onBack,
  nextLabel,
  backLabel,
  busy = false,
  disabled = false,
  hint,
  secondary,
}: StepFooterProps): React.JSX.Element {
  const { t } = useI18n();
  return (
    <div className="mt-7 flex flex-col-reverse gap-3 border-t border-white/[0.06] pt-5 sm:flex-row sm:items-center sm:justify-between">
      <div className="flex items-center gap-2">
        {onBack !== undefined ? (
          <button type="button" className="btn btn-secondary" onClick={onBack} disabled={busy}>
            <ArrowLeft size={16} />
            {backLabel ?? t('setup.back')}
          </button>
        ) : null}
        {hint !== undefined ? <span className="text-[11px] leading-relaxed text-ink-500">{hint}</span> : null}
      </div>
      <div className="flex flex-col-reverse gap-2 sm:flex-row sm:items-center">
        {secondary}
        <button type="button" className="btn btn-primary" onClick={onNext} disabled={disabled || busy}>
          {busy ? <Spinner size={16} className="text-[#04121a]" /> : <ArrowRight size={16} />}
          {busy ? t('common.loading') : (nextLabel ?? t('setup.continue'))}
        </button>
      </div>
    </div>
  );
}

export function StepError({ message }: { message: string | null }): React.JSX.Element | null {
  if (message === null) return null;
  return (
    <p
      role="alert"
      className="rounded-xl border border-coral-400/25 bg-coral-500/[0.08] px-3.5 py-2.5 text-[13px] leading-relaxed text-coral-300"
    >
      {message}
    </p>
  );
}
