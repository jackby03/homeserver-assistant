import type { ReactNode } from 'react';
import { AlertTriangle } from 'lucide-react';
import { Modal } from './Modal';
import { Spinner } from './Spinner';
import { useI18n } from '@/i18n/I18nProvider';

interface ConfirmDialogProps {
  open: boolean;
  onClose: () => void;
  onConfirm: () => void;
  title: ReactNode;
  message: ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  busy?: boolean;
}

export function ConfirmDialog({
  open,
  onClose,
  onConfirm,
  title,
  message,
  confirmLabel,
  cancelLabel,
  danger = true,
  busy = false,
}: ConfirmDialogProps): React.JSX.Element {
  const { t } = useI18n();
  return (
    <Modal
      open={open}
      onClose={busy ? () => undefined : onClose}
      locked={busy}
      size="sm"
      title={title}
      footer={
        <>
          <button type="button" className="btn btn-secondary" onClick={onClose} disabled={busy}>
            {cancelLabel ?? t('common.cancel')}
          </button>
          <button
            type="button"
            className={danger ? 'btn btn-danger' : 'btn btn-primary'}
            onClick={onConfirm}
            disabled={busy}
          >
            {busy ? <Spinner size={16} /> : null}
            {confirmLabel ?? t('common.confirm')}
          </button>
        </>
      }
    >
      <div className="flex gap-3.5">
        <div
          className={
            danger
              ? 'grid h-10 w-10 shrink-0 place-items-center rounded-xl border border-coral-400/25 bg-coral-500/10 text-coral-300'
              : 'grid h-10 w-10 shrink-0 place-items-center rounded-xl border border-amber-400/25 bg-amber-400/10 text-amber-300'
          }
        >
          <AlertTriangle size={18} />
        </div>
        <p className="text-sm leading-relaxed text-ink-300">{message}</p>
      </div>
    </Modal>
  );
}
