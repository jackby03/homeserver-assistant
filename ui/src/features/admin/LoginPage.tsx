import { useState } from 'react';
import type { FormEvent } from 'react';
import { Navigate, useLocation, useNavigate } from 'react-router-dom';
import { ArrowRight, Eye, EyeOff, Lock, ShieldCheck, UserRound } from 'lucide-react';
import { BrandMark } from '@/components/BrandMark';
import { Spinner } from '@/components/Spinner';
import { useI18n } from '@/i18n/I18nProvider';
import { isApiError } from '@/api/client';
import { cn } from '@/lib/cn';
import { useAuth } from '@/state/AuthProvider';

interface LocationState {
  from?: string;
}

function readFrom(state: unknown): string {
  if (typeof state !== 'object' || state === null) return '/admin/dashboard';
  const from = (state as LocationState).from;
  return typeof from === 'string' && from.startsWith('/admin') ? from : '/admin/dashboard';
}

export function LoginPage(): React.JSX.Element {
  const { t, language, setLanguage } = useI18n();
  const { status, login } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [showPassword, setShowPassword] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (status === 'authenticated') {
    return <Navigate to="/admin/dashboard" replace />;
  }

  const onSubmit = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await login(username.trim(), password);
      navigate(readFrom(location.state), { replace: true });
    } catch (cause) {
      if (isApiError(cause) && cause.status === 401) {
        setError(t('login.error'));
      } else if (isApiError(cause) && cause.status === 0) {
        setError(t('errors.network'));
      } else {
        setError(t('errors.generic'));
      }
      setBusy(false);
    }
  };

  return (
    <div className="relative flex min-h-screen flex-col overflow-hidden">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute -left-40 top-[-12rem] h-[560px] w-[560px] rounded-full bg-mint-500/[0.07] blur-3xl"
      />
      <div
        aria-hidden="true"
        className="pointer-events-none absolute -right-32 bottom-[-14rem] h-[520px] w-[520px] rounded-full bg-mint-400/[0.06] blur-3xl"
      />

      <header className="relative z-10 flex items-center justify-between px-6 py-5 sm:px-10">
        <BrandMark size={34} withWordmark />
        <div className="flex items-center gap-2">
          <span className="hidden items-center gap-1.5 rounded-full border border-white/[0.08] bg-white/[0.03] px-3 py-1.5 text-[11px] text-ink-400 sm:inline-flex">
            <ShieldCheck size={13} className="text-mint-400" />
            {t('login.secure')}
          </span>
          <button
            type="button"
            onClick={() => setLanguage(language === 'es' ? 'en' : 'es')}
            className="btn btn-ghost min-h-[40px] px-2.5 text-[12px] uppercase text-ink-300"
            aria-label={t('common.language')}
          >
            {language}
          </button>
        </div>
      </header>

      <main className="relative z-10 flex flex-1 items-center justify-center px-4 pb-16 pt-4">
        <div className="w-full max-w-[420px] animate-fade-up">
          <div className="mb-7 flex flex-col items-center text-center">
            <div className="relative mb-5 grid h-16 w-16 place-items-center">
              <span
                aria-hidden="true"
                className="absolute inset-0 rounded-full border border-mint-400/20 animate-[hs-ring-pulse_4s_ease-out_infinite]"
              />
              <span
                aria-hidden="true"
                className="absolute inset-0 rounded-full border border-mint-500/10"
              />
              <BrandMark size={46} />
            </div>
            <h1 className="font-display text-[26px] font-medium tracking-tight text-ink-50">
              {t('login.title')}
            </h1>
            <p className="mt-1.5 text-sm text-ink-400">{t('login.subtitle')}</p>
          </div>

          <form onSubmit={(event) => void onSubmit(event)} className="glass-strong space-y-4 p-6">
            <div>
              <label className="field-label" htmlFor="login-username">
                {t('login.username')}
              </label>
              <div className="relative">
                <UserRound
                  size={16}
                  className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-500"
                />
                <input
                  id="login-username"
                  name="username"
                  type="text"
                  autoComplete="username"
                  autoCapitalize="none"
                  spellCheck={false}
                  required
                  value={username}
                  onChange={(event) => setUsername(event.target.value)}
                  className="input pl-10"
                  placeholder={t('setup.step1.usernamePlaceholder')}
                />
              </div>
            </div>

            <div>
              <label className="field-label" htmlFor="login-password">
                {t('login.password')}
              </label>
              <div className="relative">
                <Lock
                  size={16}
                  className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-500"
                />
                <input
                  id="login-password"
                  name="password"
                  type={showPassword ? 'text' : 'password'}
                  autoComplete="current-password"
                  required
                  value={password}
                  onChange={(event) => setPassword(event.target.value)}
                  className="input pl-10 pr-12"
                  placeholder="••••••••••"
                />
                <button
                  type="button"
                  onClick={() => setShowPassword((show) => !show)}
                  className="absolute right-1.5 top-1/2 grid h-9 w-9 -translate-y-1/2 place-items-center rounded-lg text-ink-500 transition hover:bg-white/[0.06] hover:text-ink-200"
                  aria-label={showPassword ? t('login.hidePassword') : t('login.showPassword')}
                  title={showPassword ? t('login.hidePassword') : t('login.showPassword')}
                >
                  {showPassword ? <EyeOff size={16} /> : <Eye size={16} />}
                </button>
              </div>
            </div>

            {error !== null ? (
              <div
                role="alert"
                className="rounded-xl border border-coral-400/25 bg-coral-500/[0.08] px-3.5 py-2.5 text-[13px] text-coral-300"
              >
                {error}
              </div>
            ) : null}

            <button
              type="submit"
              disabled={busy || username.trim() === '' || password === ''}
              className={cn('btn btn-primary w-full')}
            >
              {busy ? (
                <>
                  <Spinner size={16} className="text-[#04121a]" />
                  {t('login.submitting')}
                </>
              ) : (
                <>
                  {t('login.submit')}
                  <ArrowRight size={16} />
                </>
              )}
            </button>
          </form>

          <p className="mt-6 text-center text-[11px] leading-relaxed text-ink-500">
            {t('login.footnote')}
          </p>
        </div>
      </main>
    </div>
  );
}
