import type { ReactNode } from 'react';
import { cn } from '@/lib/cn';

type Accent = 'mint' | 'amber' | 'coral' | 'neutral';

const accentTile: Record<Accent, string> = {
  mint: 'border-mint-400/20 bg-mint-400/[0.08] text-mint-300',
  amber: 'border-amber-400/20 bg-amber-400/[0.08] text-amber-300',
  coral: 'border-coral-400/20 bg-coral-500/[0.08] text-coral-300',
  neutral: 'border-white/[0.08] bg-white/[0.04] text-ink-300',
};

interface MetricCardProps {
  title: ReactNode;
  icon?: ReactNode;
  accent?: Accent;
  value?: ReactNode;
  caption?: ReactNode;
  action?: ReactNode;
  children?: ReactNode;
  className?: string;
}

/** Glass metric tile used across the dashboard and detail pages. */
export function MetricCard({
  title,
  icon,
  accent = 'mint',
  value,
  caption,
  action,
  children,
  className,
}: MetricCardProps): React.JSX.Element {
  return (
    <section className={cn('glass glass-hover flex flex-col gap-3 p-5', className)}>
      <header className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2.5">
          {icon !== undefined ? (
            <span
              className={cn(
                'grid h-8 w-8 shrink-0 place-items-center rounded-xl border',
                accentTile[accent],
              )}
            >
              {icon}
            </span>
          ) : null}
          <h2 className="truncate text-[13px] font-medium text-ink-300">{title}</h2>
        </div>
        {action !== undefined ? <div className="shrink-0">{action}</div> : null}
      </header>
      {value !== undefined ? (
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
          <span className="stat-value">{value}</span>
          {caption !== undefined ? (
            <span className="text-xs leading-relaxed text-ink-400">{caption}</span>
          ) : null}
        </div>
      ) : null}
      {children}
    </section>
  );
}
