import { useState } from 'react';
import { Bot, KeyRound, Plus, Sparkles, Trash2, UsersRound } from 'lucide-react';
import { setupApi } from '@/api/setup';
import type { LlmProviderKind, ProfileRole, Tone } from '@/api/types';
import { AvatarFigure } from '@/components/Avatar';
import { Field } from '@/components/Field';
import { StatusPill } from '@/components/StatusPill';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import type { DictKey } from '@/i18n/dictionary';
import { cn } from '@/lib/cn';
import { useToast } from '@/state/ToastProvider';
import {
  BASE_URL_PLACEHOLDERS,
  MODEL_PLACEHOLDERS,
  PROVIDER_HINT_KEYS,
  PROVIDER_KINDS,
  PROVIDER_LABEL_KEYS,
} from '@/features/admin/agent/constants';
import { StepError, StepFooter } from './StepChrome';
import {
  DEFAULT_PROFILE_AVATARS,
  defaultProfiles,
  nextProfileId,
  type DraftProfile,
  type SetupSummaryPatch,
} from './types';

const ROLE_KEYS: Record<ProfileRole, DictKey> = {
  owner: 'prof.role.owner',
  adult: 'prof.role.adult',
  child: 'prof.role.child',
  guest: 'prof.role.guest',
};

const TONE_KEYS: Record<Tone, DictKey> = {
  friendly: 'prof.tone.friendly',
  calm: 'prof.tone.calm',
  playful: 'prof.tone.playful',
  formal: 'prof.tone.formal',
};

interface StepAssistantProps {
  onNext: (patch?: SetupSummaryPatch) => void;
  onBack: () => void;
}

/** Step 5: optional AI provider and family profiles with avatar previews. */
export function StepAssistant({ onNext, onBack }: StepAssistantProps): React.JSX.Element {
  const { t, language } = useI18n();
  const toast = useToast();
  const [kind, setKind] = useState<LlmProviderKind>('openai');
  const [label, setLabel] = useState('');
  const [model, setModel] = useState('');
  const [baseUrl, setBaseUrl] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [profiles, setProfiles] = useState<DraftProfile[]>(() => [
    {
      id: nextProfileId(),
      name: t('setup.defaultProfile.owner'),
      role: 'owner',
      tone: 'friendly',
      avatar: DEFAULT_PROFILE_AVATARS[0]?.avatar ?? {
        shape: 'squircle',
        color: '#6ee7ff',
        eye_color: '#0b1220',
        eye_tilt_deg: 0,
      },
    },
  ]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const requiresBaseUrl = kind === 'custom' || kind === 'opencode_go';
  const aiFilled = model.trim() !== '';

  const addProfile = (): void => {
    const index = profiles.length;
    const template = DEFAULT_PROFILE_AVATARS[index % DEFAULT_PROFILE_AVATARS.length];
    if (template === undefined) return;
    setProfiles((current) => [
      ...current,
      {
        id: nextProfileId(),
        name: t('setup.step5.profileDefaultName', { count: current.length + 1 }),
        role: template.role,
        tone: template.tone,
        avatar: template.avatar,
      },
    ]);
  };

  const updateProfile = (id: number, patch: Partial<DraftProfile>): void => {
    setProfiles((current) => current.map((profile) => (profile.id === id ? { ...profile, ...patch } : profile)));
  };

  const useSuggestedProfiles = (): void => {
    setProfiles(defaultProfiles(t('setup.defaultProfile.owner'), t('setup.defaultProfile.guest')));
  };

  const saveProfiles = async (snapshot: DraftProfile[]): Promise<void> => {
    const finalProfiles =
      snapshot.length > 0
        ? snapshot
        : defaultProfiles(t('setup.defaultProfile.owner'), t('setup.defaultProfile.guest'));
    await setupApi.createProfiles({
      profiles: finalProfiles.map((profile) => ({
        name: profile.name.trim(),
        role: profile.role,
        avatar: profile.avatar,
        language,
        tone: profile.tone,
        allowed_tools: [],
      })),
    });
    return void finalProfiles;
  };

  const submit = async (skipAi: boolean): Promise<void> => {
    setError(null);
    for (const profile of profiles) {
      if (profile.name.trim() === '') {
        setError(t('errors.validation'));
        return;
      }
    }
    if (!skipAi && requiresBaseUrl && baseUrl.trim() === '') {
      setError(t('agent.baseUrl'));
      return;
    }
    setBusy(true);
    try {
      if (!skipAi && aiFilled) {
        await setupApi.configureAgent({
          kind,
          label: label.trim() === '' ? t(PROVIDER_LABEL_KEYS[kind]) : label.trim(),
          model: model.trim(),
          ...(baseUrl.trim() !== '' ? { base_url: baseUrl.trim() } : {}),
          ...(apiKey.trim() !== '' ? { api_key: apiKey.trim() } : {}),
        });
        toast.push({ level: 'success', message: t('setup.toast.agentSaved') });
      }
      const finalProfiles =
        profiles.length > 0
          ? profiles
          : defaultProfiles(t('setup.defaultProfile.owner'), t('setup.defaultProfile.guest'));
      await saveProfiles(finalProfiles);
      toast.push({ level: 'success', message: t('setup.toast.profilesSaved') });
      onNext({
        profileCount: finalProfiles.length,
        agentLabel: !skipAi && aiFilled ? model.trim() : null,
      });
    } catch (cause) {
      setError(t(errorMessageKey(cause)));
      setBusy(false);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-display text-[22px] font-medium tracking-tight text-ink-50">
          {t('setup.step5.title')}
        </h2>
        <p className="mt-1.5 max-w-2xl text-[13px] leading-relaxed text-ink-400">
          {t('setup.step5.description')}
        </p>
      </div>

      <StepError message={error} />

      {/* AI provider --------------------------------------------------------- */}
      <section className="space-y-4 rounded-2xl border border-white/[0.06] bg-white/[0.015] p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <Bot size={15} className="text-mint-400" />
            <h3 className="text-[13px] font-medium text-ink-100">{t('setup.step5.ai')}</h3>
            <StatusPill label={t('setup.optional')} tone="neutral" />
          </div>
          <span className="text-[11px] text-ink-500">{t('setup.step5.aiHint')}</span>
        </div>

        <div className="flex flex-wrap gap-1.5">
          {PROVIDER_KINDS.map((value) => (
            <button
              key={value}
              type="button"
              onClick={() => setKind(value)}
              aria-pressed={kind === value}
              className={cn(
                'min-h-[36px] rounded-full border px-3 text-[12px] transition',
                kind === value
                  ? 'border-mint-400/35 bg-mint-400/[0.08] text-mint-200'
                  : 'border-white/[0.08] text-ink-400 hover:border-white/[0.16] hover:text-ink-200',
              )}
            >
              {t(PROVIDER_LABEL_KEYS[value])}
            </button>
          ))}
        </div>
        <p className="text-[11px] text-ink-500">{t(PROVIDER_HINT_KEYS[kind])}</p>

        <div className="grid gap-4 sm:grid-cols-2">
          <Field label={t('setup.step5.provider')} htmlFor="setup-provider-label">
            <input
              id="setup-provider-label"
              className="input"
              value={label}
              onChange={(event) => setLabel(event.target.value)}
              placeholder={t(PROVIDER_LABEL_KEYS[kind])}
            />
          </Field>
          <Field label={t('setup.step5.model')} htmlFor="setup-model" hint={t('setup.step5.aiHint')}>
            <input
              id="setup-model"
              className="input input-mono"
              value={model}
              onChange={(event) => setModel(event.target.value)}
              placeholder={MODEL_PLACEHOLDERS[kind]}
              spellCheck={false}
            />
          </Field>
          <Field
            label={t('setup.step5.baseUrl')}
            htmlFor="setup-base-url"
            hint={requiresBaseUrl ? t(PROVIDER_HINT_KEYS[kind]) : t('common.optional')}
          >
            <input
              id="setup-base-url"
              className="input input-mono"
              value={baseUrl}
              onChange={(event) => setBaseUrl(event.target.value)}
              placeholder={BASE_URL_PLACEHOLDERS[kind]}
              spellCheck={false}
            />
          </Field>
          <Field label={t('setup.step5.apiKey')} htmlFor="setup-api-key" hint={t('agent.apiKeyHint')}>
            <div className="relative">
              <KeyRound
                size={15}
                className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-500"
              />
              <input
                id="setup-api-key"
                type="password"
                className="input pl-10"
                value={apiKey}
                onChange={(event) => setApiKey(event.target.value)}
                autoComplete="off"
                placeholder="sk-…"
              />
            </div>
          </Field>
        </div>
      </section>

      {/* Profiles ------------------------------------------------------------ */}
      <section className="space-y-3 rounded-2xl border border-white/[0.06] bg-white/[0.015] p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <UsersRound size={15} className="text-mint-400" />
            <h3 className="text-[13px] font-medium text-ink-100">{t('setup.step5.profiles')}</h3>
            <StatusPill label={t('setup.optional')} tone="neutral" />
          </div>
          <div className="flex gap-1.5">
            <button type="button" className="btn btn-ghost btn-sm" onClick={useSuggestedProfiles}>
              <Sparkles size={13} />
              {t('setup.step5.defaultProfiles')}
            </button>
            <button type="button" className="btn btn-secondary btn-sm" onClick={addProfile}>
              <Plus size={13} />
              {t('setup.step5.addProfile')}
            </button>
          </div>
        </div>
        <p className="text-[11px] leading-relaxed text-ink-500">{t('setup.step5.profilesHint')}</p>

        {profiles.length === 0 ? (
          <p className="rounded-xl border border-white/[0.06] bg-white/[0.02] px-3.5 py-2.5 text-[12px] text-ink-500">
            {t('setup.step5.defaultNote')}
          </p>
        ) : (
          <ul className="space-y-2">
            {profiles.map((profile, index) => (
              <li
                key={profile.id}
                className="flex flex-wrap items-center gap-3 rounded-xl border border-white/[0.06] bg-white/[0.02] px-3 py-2.5"
              >
                <AvatarFigure avatar={profile.avatar} state="idle" size={54} decorative />
                <div className="grid min-w-0 flex-1 gap-2 sm:grid-cols-3">
                  <Field label={index === 0 ? t('setup.step5.profileName') : ''} htmlFor={`profile-name-${profile.id}`}>
                    <input
                      id={`profile-name-${profile.id}`}
                      className="input min-h-[38px]"
                      value={profile.name}
                      onChange={(event) => updateProfile(profile.id, { name: event.target.value })}
                    />
                  </Field>
                  <Field label={index === 0 ? t('setup.step5.profileRole') : ''} htmlFor={`profile-role-${profile.id}`}>
                    <select
                      id={`profile-role-${profile.id}`}
                      className="input min-h-[38px]"
                      value={profile.role}
                      onChange={(event) =>
                        updateProfile(profile.id, { role: event.target.value as ProfileRole })
                      }
                    >
                      {(Object.keys(ROLE_KEYS) as ProfileRole[]).map((role) => (
                        <option key={role} value={role}>
                          {t(ROLE_KEYS[role])}
                        </option>
                      ))}
                    </select>
                  </Field>
                  <Field label={index === 0 ? t('setup.step5.tone') : ''} htmlFor={`profile-tone-${profile.id}`}>
                    <select
                      id={`profile-tone-${profile.id}`}
                      className="input min-h-[38px]"
                      value={profile.tone}
                      onChange={(event) => updateProfile(profile.id, { tone: event.target.value as Tone })}
                    >
                      {(Object.keys(TONE_KEYS) as Tone[]).map((tone) => (
                        <option key={tone} value={tone}>
                          {t(TONE_KEYS[tone])}
                        </option>
                      ))}
                    </select>
                  </Field>
                </div>
                <button
                  type="button"
                  className="btn btn-ghost btn-icon text-ink-500 hover:bg-coral-500/10 hover:text-coral-300"
                  onClick={() => setProfiles((current) => current.filter((item) => item.id !== profile.id))}
                  aria-label={t('common.remove')}
                >
                  <Trash2 size={15} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      <StepFooter
        onNext={() => void submit(false)}
        onBack={onBack}
        busy={busy}
        disabled={!aiFilled && profiles.length === 0}
        hint={t('setup.step5.profilesHint')}
        secondary={
          <button
            type="button"
            className="btn btn-ghost btn-sm text-ink-400"
            onClick={() => void submit(true)}
            disabled={busy}
          >
            {t('setup.step5.skipAi')}
          </button>
        }
      />
    </div>
  );
}
