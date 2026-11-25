import { lazy, Suspense, useCallback, useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { Navigate, useLocation, useNavigate } from 'react-router-dom';
import { WifiOff } from 'lucide-react';
import { authApi } from '@/api/auth';
import type { SetupStatus } from '@/api/types';
import { LoadingScreen } from '@/components/LoadingScreen';
import { BrandMark } from '@/components/BrandMark';
import { Spinner } from '@/components/Spinner';
import { useI18n } from '@/i18n/I18nProvider';
import { useAuth } from '@/state/AuthProvider';

const SetupWizard = lazy(() =>
  import('@/features/setup/SetupWizard').then((module) => ({ default: module.SetupWizard })),
);

/** Gate for the admin portal: waiting → boot screen, anonymous → login. */
export function RequireAuth({ children }: { children: ReactNode }): React.JSX.Element {
  const { t } = useI18n();
  const { status } = useAuth();
  const location = useLocation();

  if (status === 'loading') {
    return <LoadingScreen message={t('boot.loading')} hint={t('app.localFirst')} />;
  }
  if (status === 'anonymous') {
    return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  }
  return <>{children}</>;
}

/** `/` decides between setup, portal and login from the public setup status. */
export function RootRedirect(): React.JSX.Element {
  const { t } = useI18n();
  const navigate = useNavigate();
  const { status } = useAuth();
  const [initialized, setInitialized] = useState<boolean | null>(null);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let active = true;
    authApi
      .setupStatus()
      .then((setup) => {
        if (active) setInitialized(setup.initialized);
      })
      .catch(() => {
        if (active) setFailed(true);
      });
    return () => {
      active = false;
    };
  }, [attempt]);

  useEffect(() => {
    if (initialized === null) return;
    if (!initialized) {
      navigate('/setup', { replace: true });
      return;
    }
    if (status === 'loading') return;
    navigate(status === 'authenticated' ? '/admin/dashboard' : '/login', { replace: true });
  }, [initialized, status, navigate]);

  if (failed) {
    return (
      <ErrorPanel
        title={t('boot.error')}
        hint={t('boot.offlineHint')}
        retryLabel={t('common.retry')}
        onRetry={() => {
          setFailed(false);
          setAttempt((value) => value + 1);
        }}
      />
    );
  }

  return <LoadingScreen message={t('boot.checking')} hint={t('app.localFirst')} />;
}

/** `/setup` gate: only reachable before initialization. */
export function SetupRoute(): React.JSX.Element {
  const { t } = useI18n();
  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let active = true;
    authApi
      .setupStatus()
      .then((setup) => {
        if (active) setStatus(setup);
      })
      .catch(() => {
        if (active) setFailed(true);
      });
    return () => {
      active = false;
    };
  }, [attempt]);

  if (failed) {
    return (
      <ErrorPanel
        title={t('boot.error')}
        hint={t('boot.offlineHint')}
        retryLabel={t('common.retry')}
        onRetry={() => {
          setFailed(false);
          setAttempt((value) => value + 1);
        }}
      />
    );
  }
  if (status === null) {
    return <LoadingScreen message={t('boot.checking')} hint={t('app.localFirst')} />;
  }
  if (status.initialized) {
    return <Navigate to="/" replace />;
  }
  return (
    <Suspense
      fallback={
        <div className="grid min-h-screen place-items-center">
          <Spinner size={24} />
        </div>
      }
    >
      <SetupWizard initialStatus={status} />
    </Suspense>
  );
}

interface ErrorPanelProps {
  title: string;
  hint: string;
  retryLabel: string;
  onRetry: () => void;
}

export function ErrorPanel({ title, hint, retryLabel, onRetry }: ErrorPanelProps): React.JSX.Element {
  return (
    <div className="grid min-h-screen place-items-center px-6">
      <div className="glass flex max-w-md flex-col items-center gap-5 px-8 py-10 text-center">
        <BrandMark size={42} />
        <div className="grid h-12 w-12 place-items-center rounded-2xl border border-coral-400/25 bg-coral-500/10 text-coral-300">
          <WifiOff size={22} />
        </div>
        <div className="space-y-2">
          <h1 className="font-display text-xl font-medium text-ink-50">{title}</h1>
          <p className="text-sm leading-relaxed text-ink-400">{hint}</p>
        </div>
        <button type="button" className="btn btn-primary" onClick={onRetry}>
          {retryLabel}
        </button>
      </div>
    </div>
  );
}

/** Small hook used by pages that refresh data after a route action. */
export function useRefreshToken(): [number, () => void] {
  const [token, setToken] = useState(0);
  const refresh = useCallback(() => setToken((value) => value + 1), []);
  return [token, refresh];
}
