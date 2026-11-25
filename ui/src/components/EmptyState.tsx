import type { ReactNode } from 'react';
import { cn } from '@/lib/cn';

interface EmptyStateProps {
  icon?: ReactNode;
  title: ReactNode;
  body?: ReactNode;
  action?: ReactNode;
  className?: string;
}

/** Friendly empty/zero state with a deliberate next action. */
export function EmptyState({ icon, title, body, action, className }: EmptyStateProps): React.JSX.Element {
  return (
    <div
      className={cn(
        'flex flex-col items-center justify-center gap-4 rounded-2xl border border-dashed border-white/[0.09] bg-white/[0.015] px-6 py-12 text-center',
        className,
      )}
    >
      {icon !== undefined ? (
        <div className="grid h-12 w-12 place-items-center rounded-2xl border border-white/[0.08] bg-white/[0.03] text-mint-400">
          {icon}
        </div>
      ) : null}
      <div className="space-y-1.5">
        <p className="font-display text-base font-medium text-ink-100">{title}</p>
        {body !== undefined ? (
          <p className="mx-auto max-w-md text-sm leading-relaxed text-ink-400">{body}</p>
        ) : null}
      </div>
      {action !== undefined ? <div className="pt-1">{action}</div> : null}
    </div>
  );
}
