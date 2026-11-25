import type { ReactNode } from 'react';
import { cn } from '@/lib/cn';
import { statusKey, statusTone } from '@/lib/status';
import type { Tone } from '@/lib/status';
import { useI18n } from '@/i18n/I18nProvider';

const toneClasses: Record<Tone, string> = {
  success: 'border-mint-500/25 bg-mint-500/[0.1] text-mint-300',
  warning: 'border-amber-400/25 bg-amber-400/[0.09] text-amber-300',
  danger: 'border-coral-400/25 bg-coral-500/[0.1] text-coral-300',
  neutral: 'border-white/10 bg-white/[0.04] text-ink-300',
  info: 'border-mint-400/20 bg-mint-400/[0.07] text-mint-300',
  accent: 'border-mint-400/30 bg-mint-400/[0.09] text-mint-200',
};

const dotClasses: Record<Tone, string> = {
  success: 'bg-mint-500',
  warning: 'bg-amber-400',
  danger: 'bg-coral-400',
  neutral: 'bg-ink-400',
  info: 'bg-mint-400',
  accent: 'bg-mint-400',
};

interface StatusPillProps {
  status?: string;
  label?: ReactNode;
  tone?: Tone;
  pulse?: boolean;
  className?: string;
}

/** Compact status chip with an accessible label and optional live pulse. */
export function StatusPill({
  status,
  label,
  tone,
  pulse = false,
  className,
}: StatusPillProps): React.JSX.Element {
  const { t } = useI18n();
  const resolvedTone = tone ?? (status !== undefined ? statusTone(status) : 'neutral');
  const key = status !== undefined ? statusKey(status) : null;
  const text = label ?? (key !== null ? t(key) : (status ?? ''));
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] font-medium leading-none',
        toneClasses[resolvedTone],
        className,
      )}
    >
      <span
        aria-hidden="true"
        className={cn(
          'h-1.5 w-1.5 shrink-0 rounded-full',
          dotClasses[resolvedTone],
          pulse && 'animate-[hs-pulse-dot_2s_ease-in-out_infinite]',
        )}
      />
      {text}
    </span>
  );
}
