import { useEffect } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { X } from 'lucide-react';
import { cn } from '@/lib/cn';
import { useI18n } from '@/i18n/I18nProvider';

interface DrawerProps {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  subtitle?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  width?: 'md' | 'lg' | 'xl';
}

const widthClasses = {
  md: 'sm:max-w-md',
  lg: 'sm:max-w-xl',
  xl: 'sm:max-w-2xl',
} as const;

/** Right-side panel for details, deployment and editing flows. */
export function Drawer({
  open,
  onClose,
  title,
  subtitle,
  children,
  footer,
  width = 'lg',
}: DrawerProps): React.JSX.Element | null {
  const { t } = useI18n();

  useEffect(() => {
    if (!open) return undefined;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
      document.body.style.overflow = previousOverflow;
    };
  }, [open, onClose]);

  if (!open) return null;

  return createPortal(
    <div className="fixed inset-0 z-50 flex justify-end">
      <div
        aria-hidden="true"
        className="absolute inset-0 animate-[hs-fade-in_200ms_ease] bg-night-950/70 backdrop-blur-sm"
        onMouseDown={onClose}
      />
      <aside
        role="dialog"
        aria-modal="true"
        aria-label={typeof title === 'string' ? title : undefined}
        className={cn(
          'glass-strong relative z-10 flex h-full w-full flex-col animate-[hs-drawer-in_240ms_var(--hs-ease)] rounded-none border-y-0 border-r-0 sm:rounded-l-[24px]',
          widthClasses[width],
        )}
        style={{ animationName: 'hs-drawer-in' }}
      >
        <header className="flex items-start justify-between gap-4 border-b border-white/[0.06] px-5 py-4 sm:px-6">
          <div className="min-w-0 space-y-1">
            <h2 className="truncate font-display text-lg font-medium text-ink-50">{title}</h2>
            {subtitle !== undefined ? (
              <p className="text-sm leading-relaxed text-ink-400">{subtitle}</p>
            ) : null}
          </div>
          <button
            type="button"
            onClick={onClose}
            className="btn btn-ghost btn-icon -mr-1 shrink-0"
            aria-label={t('common.close')}
          >
            <X size={18} />
          </button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5 sm:px-6">{children}</div>
        {footer !== undefined ? (
          <footer className="border-t border-white/[0.06] px-5 py-4 sm:px-6">{footer}</footer>
        ) : null}
      </aside>
    </div>,
    document.body,
  );
}
