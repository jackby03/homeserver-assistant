import { cn } from '@/lib/cn';

interface ProgressBarProps {
  value: number;
  label?: string;
  tone?: 'mint' | 'amber' | 'coral';
  showValue?: boolean;
  className?: string;
}

const toneClass: Record<'mint' | 'amber' | 'coral', string> = {
  mint: 'bg-mint-gradient',
  amber: 'bg-amber-gradient',
  coral: 'bg-coral-gradient',
};

export function ProgressBar({
  value,
  label,
  tone = 'mint',
  showValue = false,
  className,
}: ProgressBarProps): React.JSX.Element {
  const clamped = Math.min(100, Math.max(0, Number.isFinite(value) ? value : 0));
  return (
    <div className={cn('space-y-1.5', className)}>
      {label !== undefined || showValue ? (
        <div className="flex items-center justify-between text-xs text-ink-400">
          {label !== undefined ? <span>{label}</span> : <span />}
          {showValue ? <span className="tabular text-ink-300">{Math.round(clamped)}%</span> : null}
        </div>
      ) : null}
      <div
        className="h-1.5 w-full overflow-hidden rounded-full bg-white/[0.06]"
        role="progressbar"
        aria-valuenow={Math.round(clamped)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={label}
      >
        <div
          className={cn('h-full rounded-full transition-[width] duration-500 ease-smooth', toneClass[tone])}
          style={{ width: `${clamped}%` }}
        />
      </div>
    </div>
  );
}
