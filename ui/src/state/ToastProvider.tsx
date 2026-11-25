import { createContext, useCallback, useContext, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';

export type ToastLevel = 'info' | 'success' | 'warning' | 'error';

export interface ToastMessage {
  id: number;
  level: ToastLevel;
  /** Short localized headline. */
  title?: string;
  /** Localized detail line. */
  message: string;
}

export interface ToastInput {
  level?: ToastLevel;
  title?: string;
  message: string;
  /** Milliseconds before auto-dismiss; 0 keeps the toast until dismissed. */
  durationMs?: number;
}

interface ToastContextValue {
  toasts: ToastMessage[];
  push: (input: ToastInput) => number;
  dismiss: (id: number) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

export function ToastProvider({ children }: { children: ReactNode }): React.JSX.Element {
  const [toasts, setToasts] = useState<ToastMessage[]>([]);
  const nextId = useRef(1);
  const timers = useRef(new Map<number, number>());

  const dismiss = useCallback((id: number) => {
    setToasts((current) => current.filter((toast) => toast.id !== id));
    const timer = timers.current.get(id);
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timers.current.delete(id);
    }
  }, []);

  const push = useCallback(
    (input: ToastInput): number => {
      const id = nextId.current;
      nextId.current += 1;
      const toast: ToastMessage = {
        id,
        level: input.level ?? 'info',
        message: input.message,
        ...(input.title !== undefined ? { title: input.title } : {}),
      };
      setToasts((current) => [...current.slice(-4), toast]);
      const duration = input.durationMs ?? 5_000;
      if (duration > 0) {
        const timer = window.setTimeout(() => dismiss(id), duration);
        timers.current.set(id, timer);
      }
      return id;
    },
    [dismiss],
  );

  const value = useMemo<ToastContextValue>(
    () => ({ toasts, push, dismiss }),
    [toasts, push, dismiss],
  );

  return <ToastContext.Provider value={value}>{children}</ToastContext.Provider>;
}

export function useToast(): ToastContextValue {
  const context = useContext(ToastContext);
  if (context === null) throw new Error('useToast must be used inside <ToastProvider>');
  return context;
}
