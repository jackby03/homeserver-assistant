import { useState } from 'react';
import { Globe2, Lock, UserRound } from 'lucide-react';
import { setupApi } from '@/api/setup';
import { Field, Segmented } from '@/components/Field';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import type { UiLanguage } from '@/i18n/dictionary';
import { useToast } from '@/state/ToastProvider';
import { StepError, StepFooter } from './StepChrome';
import type { SetupSummaryPatch } from './types';

interface StepLanguageProps {
  initialName: string;
  onNext: (patch?: SetupSummaryPatch) => void;
}

const TIMEZONE_SUGGESTIONS = [
  'UTC',
  'America/Lima',
  'America/Bogota',
  'America/Mexico_City',
  'America/Argentina/Buenos_Aires',
  'America/New_York',
  'Europe/Madrid',
  'Europe/London',
];

function browserTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
}

/** Step 1: system language, device identity and the admin account. */
export function StepLanguage({ initialName, onNext }: StepLanguageProps): React.JSX.Element {
  const { t, language, setLanguage } = useI18n();
  const toast = useToast();
  const [deviceName, setDeviceName] = useState(initialName);
  const [timezone, setTimezone] = useState(browserTimezone);
  const [username, setUsername] = useState('admin');
  const [password, setPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (): Promise<void> => {
    setError(null);
    if (deviceName.trim() === '') {
      setError(t('errors.validation'));
      return;
    }
    if (!/^[a-z0-9][a-z0-9_-]{1,31}$/.test(username.trim())) {
      setError(t('setup.step1.usernameInvalid'));
      return;
    }
    if (password.length < 10) {
      setError(t('setup.step1.passwordTooShort'));
      return;
    }
    if (password !== confirmPassword) {
      setError(t('setup.step1.passwordMismatch'));
      return;
    }
    setBusy(true);
    try {
      await setupApi.createAdmin({
        username: username.trim(),
        password,
        device: {
          name: deviceName.trim(),
          language,
          timezone: timezone.trim() === '' ? 'UTC' : timezone.trim(),
        },
      });
      toast.push({ level: 'success', message: t('setup.toast.adminCreated') });
      onNext({ deviceName: deviceName.trim(), language });
    } catch (cause) {
      setError(t(errorMessageKey(cause)));
      setBusy(false);
    }
  };

  const languageOptions: Array<{ value: UiLanguage; label: string }> = [
    { value: 'es', label: 'Español' },
    { value: 'en', label: 'English' },
  ];

  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-display text-[22px] font-medium tracking-tight text-ink-50">
          {t('setup.step1.title')}
        </h2>
        <p className="mt-1.5 max-w-2xl text-[13px] leading-relaxed text-ink-400">
          {t('setup.step1.description')}
        </p>
      </div>

      <StepError message={error} />

      <div className="grid gap-5 md:grid-cols-2">
        <div className="space-y-4">
          <Field label={t('setup.step1.language')}>
            <Segmented
              value={language}
              onChange={(value) => setLanguage(value)}
              options={languageOptions.map((option) => ({ value: option.value, label: option.label }))}
            />
          </Field>
          <Field label={t('setup.step1.deviceName')} htmlFor="setup-device-name" required>
            <input
              id="setup-device-name"
              className="input"
              value={deviceName}
              onChange={(event) => setDeviceName(event.target.value)}
              placeholder={t('setup.step1.devicePlaceholder')}
            />
          </Field>
          <Field
            label={t('setup.step1.timezone')}
            htmlFor="setup-timezone"
            hint={t('setup.step1.timezoneHint')}
          >
            <div className="relative">
              <Globe2
                size={15}
                className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-500"
              />
              <input
                id="setup-timezone"
                className="input input-mono pl-10"
                value={timezone}
                onChange={(event) => setTimezone(event.target.value)}
                list="setup-timezones"
                spellCheck={false}
              />
              <datalist id="setup-timezones">
                {TIMEZONE_SUGGESTIONS.map((zone) => (
                  <option key={zone} value={zone} />
                ))}
              </datalist>
            </div>
          </Field>
        </div>

        <div className="space-y-4 rounded-2xl border border-white/[0.06] bg-white/[0.015] p-4">
          <p className="text-[13px] font-medium text-ink-100">{t('setup.step1.account')}</p>
          <Field
            label={t('setup.step1.username')}
            htmlFor="setup-username"
            required
            hint={t('setup.step1.usernameHint')}
          >
            <div className="relative">
              <UserRound
                size={15}
                className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-500"
              />
              <input
                id="setup-username"
                className="input pl-10"
                value={username}
                onChange={(event) => setUsername(event.target.value)}
                autoCapitalize="none"
                spellCheck={false}
                placeholder={t('setup.step1.usernamePlaceholder')}
              />
            </div>
          </Field>
          <Field
            label={t('setup.step1.password')}
            htmlFor="setup-password"
            required
            hint={t('setup.step1.passwordHint')}
          >
            <div className="relative">
              <Lock
                size={15}
                className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-500"
              />
              <input
                id="setup-password"
                type="password"
                className="input pl-10"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                autoComplete="new-password"
              />
            </div>
          </Field>
          <Field label={t('setup.step1.confirmPassword')} htmlFor="setup-password-confirm" required>
            <div className="relative">
              <Lock
                size={15}
                className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-500"
              />
              <input
                id="setup-password-confirm"
                type="password"
                className="input pl-10"
                value={confirmPassword}
                onChange={(event) => setConfirmPassword(event.target.value)}
                autoComplete="new-password"
              />
            </div>
          </Field>
        </div>
      </div>

      <StepFooter onNext={() => void submit()} busy={busy} hint={t('app.localFirst')} />
    </div>
  );
}
