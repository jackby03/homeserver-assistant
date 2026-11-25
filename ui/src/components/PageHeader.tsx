import type { ReactNode } from 'react';
import { cn } from '@/lib/cn';

interface PageHeaderProps {
  title: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  eyebrow?: ReactNode;
  className?: string;
}

export function PageHeader({ title, subtitle, actions, eyebrow, className }: PageHeaderProps): React.JSX.Element {
  return (
    <header
      className={cn('flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between', className)}
    >
      <div className="space-y-1.5">
        {eyebrow !== undefined ? <div className="section-label">{eyebrow}</div> : null}
        <h1 className="font-display text-2xl font-medium tracking-tight text-ink-50 sm:text-[28px]">
          {title}
        </h1>
        {subtitle !== undefined ? (
          <p className="max-w-2xl text-sm leading-relaxed text-ink-400">{subtitle}</p>
        ) : null}
      </div>
      {actions !== undefined ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
    </header>
  );
}
