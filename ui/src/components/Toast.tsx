import { AlertTriangle, CheckCircle2, Info, X, XCircle } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { cn } from '@/lib/cn';
import { useToast } from '@/state/ToastProvider';
import type { ToastLevel } from '@/state/ToastProvider';
import { useI18n } from '@/i18n/I18nProvider';

const levelStyles: Record<ToastLevel, { icon: LucideIcon; tone: string; iconTone: string }> = {
  info: {
    icon: Info,
    tone: 'border-mint-400/20',
    iconTone: 'text-mint-300',
  },
  success: {
    icon: CheckCircle2,
    tone: 'border-mint-500/25',
    iconTone: 'text-mint-400',
  },
  warning: {
    icon: AlertTriangle,
    tone: 'border-amber-400/25',
    iconTone: 'text-amber-300',
  },
  error: {
    icon: XCircle,
    tone: 'border-coral-400/30',
    iconTone: 'text-coral-300',
  },
};

/** Fixed toast viewport, rendered once by the app shell. */
export function ToastViewport(): React.JSX.Element {
  const { toasts, dismiss } = useToast();
  const { t } = useI18n();

  return (
    <div
      className="pointer-events-none fixed bottom-4 right-4 z-[80] flex w-[min(380px,calc(100vw-2rem))] flex-col gap-2"
      aria-live="polite"
      aria-relevant="additions"
    >
      {toasts.map((toast) => {
        const style = levelStyles[toast.level];
        const Icon = style.icon;
        return (
          <div
            key={toast.id}
            role={toast.level === 'error' ? 'alert' : 'status'}
            className={cn(
              'glass-strong pointer-events-auto flex items-start gap-3 rounded-2xl border px-4 py-3.5 animate-[hs-toast-in_220ms_var(--hs-ease)]',
              style.tone,
            )}
          >
            <Icon size={18} className={cn('mt-0.5 shrink-0', style.iconTone)} aria-hidden="true" />
            <div className="min-w-0 flex-1">
              {toast.title !== undefined ? (
                <p className="text-[13px] font-semibold text-ink-50">{toast.title}</p>
              ) : null}
              <p className="text-[13px] leading-relaxed text-ink-200">{toast.message}</p>
            </div>
            <button
              type="button"
              onClick={() => dismiss(toast.id)}
              className="-mr-1 -mt-0.5 rounded-lg p-1.5 text-ink-500 transition hover:bg-white/[0.06] hover:text-ink-200"
              aria-label={t('common.dismiss')}
            >
              <X size={14} />
            </button>
          </div>
        );
      })}
    </div>
  );
}
