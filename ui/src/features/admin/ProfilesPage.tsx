import { useEffect, useMemo, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import {
  Bot,
  Check,
  Pencil,
  Plus,
  RefreshCw,
  ScanFace,
  Trash2,
  UsersRound,
} from 'lucide-react';
import { agentApi } from '@/api/agent';
import { profilesApi } from '@/api/profiles';
import type {
  Avatar,
  AvatarShape,
  AvatarState,
  Profile,
  ProfileRole,
  Tone,
  ToolDefinition,
} from '@/api/types';
import { AvatarFigure } from '@/components/Avatar';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { EmptyState } from '@/components/EmptyState';
import { Field, Segmented, Toggle } from '@/components/Field';
import { Modal } from '@/components/Modal';
import { PageHeader } from '@/components/PageHeader';
import { Spinner } from '@/components/Spinner';
import { StatusPill } from '@/components/StatusPill';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import type { DictKey } from '@/i18n/dictionary';
import { cn } from '@/lib/cn';
import { isValidHexColor } from '@/lib/color';
import { useResource } from '@/lib/use-resource';
import { useToast } from '@/state/ToastProvider';

const SHAPES: AvatarShape[] = ['circle', 'squircle', 'cloud', 'cone', 'droplet'];

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

const PREVIEW_STATES: AvatarState[] = ['idle', 'tracking', 'listening', 'thinking', 'speaking'];

const DEFAULT_AVATAR: Avatar = {
  shape: 'squircle',
  color: '#6ee7ff',
  eye_color: '#0b1220',
  eye_tilt_deg: 0,
};

const PROFILE_COLORS = [
  '#6ee7ff',
  '#34d399',
  '#fbbf24',
  '#fb7185',
  '#a5b4fc',
  '#67e8f9',
  '#f9a8d4',
  '#bef264',
];

export function ProfilesPage(): React.JSX.Element {
  const { t } = useI18n();
  const toast = useToast();
  const [params] = useSearchParams();
  const [refreshToken, setRefreshToken] = useState(0);
  const [editorOpen, setEditorOpen] = useState(params.get('new') === '1');
  const [editing, setEditing] = useState<Profile | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Profile | null>(null);
  const [deleting, setDeleting] = useState(false);

  const profiles = useResource(() => profilesApi.list(), [refreshToken]);
  const tools = useResource(() => agentApi.tools(), [refreshToken]);

  const refresh = (): void => setRefreshToken((value) => value + 1);

  const list = profiles.data ?? [];
  const toolList: ToolDefinition[] = tools.data ?? [];

  const openCreate = (): void => {
    setEditing(null);
    setEditorOpen(true);
  };

  const openEdit = (profile: Profile): void => {
    setEditing(profile);
    setEditorOpen(true);
  };

  const confirmDelete = async (): Promise<void> => {
    if (deleteTarget === null) return;
    setDeleting(true);
    try {
      await profilesApi.remove(deleteTarget.id);
      toast.push({ level: 'success', message: t('prof.toast.deleted') });
      setDeleteTarget(null);
      refresh();
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setDeleting(false);
    }
  };

  return (
    <div className="space-y-5">
      <PageHeader
        title={t('prof.title')}
        subtitle={t('prof.subtitle')}
        actions={
          <>
            <button type="button" className="btn btn-secondary btn-sm" onClick={refresh} disabled={profiles.refreshing}>
              <RefreshCw size={14} className={profiles.refreshing ? 'animate-spin' : undefined} />
              {t('common.refresh')}
            </button>
            <button type="button" className="btn btn-primary btn-sm" onClick={openCreate}>
              <Plus size={15} />
              {t('prof.new')}
            </button>
          </>
        }
      />

      {profiles.error !== null && list.length === 0 ? (
        <div className="rounded-2xl border border-coral-400/25 bg-coral-500/[0.07] px-4 py-3 text-sm text-coral-200">
          {t(errorMessageKey(profiles.error))}
        </div>
      ) : null}

      {profiles.loading && list.length === 0 ? (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
          {Array.from({ length: 3 }).map((_, index) => (
            <div key={index} className="skeleton h-64 w-full rounded-2xl" />
          ))}
        </div>
      ) : list.length === 0 ? (
        <EmptyState
          icon={<UsersRound size={20} />}
          title={t('prof.empty.title')}
          body={t('prof.empty.body')}
          action={
            <button type="button" className="btn btn-primary btn-sm" onClick={openCreate}>
              <Plus size={15} />
              {t('prof.new')}
            </button>
          }
        />
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
          {list.map((profile) => (
            <article key={profile.id} className="glass glass-hover relative overflow-hidden p-5">
              <div
                aria-hidden="true"
                className="pointer-events-none absolute -right-14 -top-14 h-40 w-40 rounded-full blur-3xl"
                style={{ backgroundColor: `${profile.avatar.color}18` }}
              />
              <div className="relative flex items-start gap-4">
                <div className="shrink-0">
                  <AvatarFigure avatar={profile.avatar} state="idle" size={92} />
                </div>
                <div className="min-w-0 flex-1">
                  <h2 className="truncate font-display text-[17px] font-medium text-ink-50">{profile.name}</h2>
                  <div className="mt-1.5 flex flex-wrap gap-1.5">
                    <StatusPill label={t(ROLE_KEYS[profile.role])} tone={profile.role === 'owner' ? 'accent' : 'neutral'} />
                    <StatusPill label={t(TONE_KEYS[profile.tone])} tone="neutral" />
                  </div>
                  <div className="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-ink-500">
                    <span className="inline-flex items-center gap-1">
                      <Bot size={12} />
                      {t('prof.toolsCount', { count: profile.allowed_tools.length })}
                    </span>
                    <span aria-hidden="true">·</span>
                    <span className="uppercase">{profile.language}</span>
                  </div>
                </div>
              </div>

              <div className="relative mt-4 flex items-center justify-between gap-3 border-t border-white/[0.05] pt-3">
                <span
                  className={cn(
                    'inline-flex items-center gap-1.5 text-[11px]',
                    profile.face_enrolled ? 'text-mint-300' : 'text-ink-500',
                  )}
                >
                  <ScanFace size={13} />
                  {profile.face_enrolled ? t('prof.face.enrolled') : t('prof.face.notEnrolled')}
                </span>
                <div className="flex gap-1">
                  <button
                    type="button"
                    className="btn btn-ghost btn-sm"
                    onClick={() => openEdit(profile)}
                    aria-label={t('prof.edit')}
                  >
                    <Pencil size={14} />
                    {t('common.edit')}
                  </button>
                  <button
                    type="button"
                    className="btn btn-ghost btn-icon text-ink-500 hover:bg-coral-500/10 hover:text-coral-300"
                    onClick={() => setDeleteTarget(profile)}
                    aria-label={t('prof.delete')}
                  >
                    <Trash2 size={15} />
                  </button>
                </div>
              </div>
            </article>
          ))}
        </div>
      )}

      {editorOpen ? (
        <ProfileEditor
          key={editing?.id ?? 'new'}
          profile={editing}
          tools={toolList}
          onClose={() => setEditorOpen(false)}
          onSaved={() => {
            setEditorOpen(false);
            refresh();
          }}
        />
      ) : null}

      <ConfirmDialog
        open={deleteTarget !== null}
        onClose={() => setDeleteTarget(null)}
        onConfirm={() => void confirmDelete()}
        busy={deleting}
        title={t('prof.confirmDelete.title', { name: deleteTarget?.name ?? '' })}
        message={t('prof.confirmDelete.body')}
        confirmLabel={t('common.delete')}
      />
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Editor                                                                     */
/* -------------------------------------------------------------------------- */

interface ProfileEditorProps {
  profile: Profile | null;
  tools: ToolDefinition[];
  onClose: () => void;
  onSaved: () => void;
}

function ProfileEditor({ profile, tools, onClose, onSaved }: ProfileEditorProps): React.JSX.Element {
  const { t } = useI18n();
  const toast = useToast();
  const [name, setName] = useState(profile?.name ?? '');
  const [role, setRole] = useState<ProfileRole>(profile?.role ?? 'adult');
  const [tone, setTone] = useState<Tone>(profile?.tone ?? 'friendly');
  const [language, setLanguage] = useState(profile?.language === 'en' ? 'en' : 'es');
  const [avatar, setAvatar] = useState<Avatar>(profile?.avatar ?? DEFAULT_AVATAR);
  const [allowedTools, setAllowedTools] = useState<string[]>(profile?.allowed_tools ?? []);
  const [previewState, setPreviewState] = useState<AvatarState>('idle');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [faceBusy, setFaceBusy] = useState(false);
  const [faceEnrolled, setFaceEnrolled] = useState(profile?.face_enrolled ?? false);

  useEffect(() => {
    if (profile === null) return;
    setFaceEnrolled(profile.face_enrolled);
  }, [profile]);

  const toggleTool = (tool: string, enabled: boolean): void => {
    setAllowedTools((current) =>
      enabled ? [...new Set([...current, tool])] : current.filter((item) => item !== tool),
    );
  };

  const submit = async (): Promise<void> => {
    setError(null);
    if (name.trim() === '') {
      setError(t('prof.validation.name'));
      return;
    }
    if (!isValidHexColor(avatar.color) || !isValidHexColor(avatar.eye_color)) {
      setError(t('errors.validation'));
      return;
    }
    const payload = {
      name: name.trim(),
      role,
      avatar,
      language,
      tone,
      allowed_tools: allowedTools,
    };
    setBusy(true);
    try {
      if (profile === null) {
        await profilesApi.create(payload);
        toast.push({ level: 'success', message: t('prof.toast.created') });
      } else {
        await profilesApi.update(profile.id, payload);
        toast.push({ level: 'success', message: t('prof.toast.updated') });
      }
      onSaved();
    } catch (cause) {
      setError(t(errorMessageKey(cause)));
    } finally {
      setBusy(false);
    }
  };

  const removeFace = async (): Promise<void> => {
    if (profile === null) return;
    setFaceBusy(true);
    try {
      await profilesApi.deleteFace(profile.id);
      setFaceEnrolled(false);
      toast.push({ level: 'success', message: t('prof.face.deleted') });
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setFaceBusy(false);
    }
  };

  const toolCount = useMemo(() => tools.length, [tools]);

  return (
    <Modal
      open
      onClose={busy ? () => undefined : onClose}
      locked={busy}
      size="xl"
      title={profile === null ? t('prof.create') : t('prof.edit')}
      description={t('prof.subtitle')}
      footer={
        <>
          <button type="button" className="btn btn-secondary" onClick={onClose} disabled={busy}>
            {t('common.cancel')}
          </button>
          <button type="button" className="btn btn-primary" onClick={() => void submit()} disabled={busy}>
            {busy ? <Spinner size={16} className="text-[#04121a]" /> : <Check size={16} />}
            {busy ? t('common.saving') : t('common.save')}
          </button>
        </>
      }
    >
      <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_260px]">
        <div className="space-y-5">
          {error !== null ? (
            <p role="alert" className="rounded-xl border border-coral-400/25 bg-coral-500/[0.08] px-3.5 py-2.5 text-[13px] text-coral-300">
              {error}
            </p>
          ) : null}

          <div className="grid gap-4 sm:grid-cols-2">
            <Field label={t('prof.name')} htmlFor="profile-name" required>
              <input
                id="profile-name"
                className="input"
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder={t('prof.namePlaceholder')}
              />
            </Field>
            <Field label={t('prof.role')}>
              <Segmented
                value={role}
                onChange={setRole}
                options={(Object.keys(ROLE_KEYS) as ProfileRole[]).map((value) => ({
                  value,
                  label: t(ROLE_KEYS[value]),
                }))}
              />
            </Field>
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <Field label={t('prof.tone')}>
              <Segmented
                value={tone}
                onChange={setTone}
                options={(Object.keys(TONE_KEYS) as Tone[]).map((value) => ({
                  value,
                  label: t(TONE_KEYS[value]),
                }))}
              />
            </Field>
            <Field label={t('prof.language')}>
              <Segmented
                value={language}
                onChange={setLanguage}
                options={[
                  { value: 'es', label: 'Español' },
                  { value: 'en', label: 'English' },
                ]}
              />
            </Field>
          </div>

          <div className="space-y-3 rounded-2xl border border-white/[0.06] bg-white/[0.02] p-4">
            <p className="text-[13px] font-medium text-ink-100">{t('prof.avatar.title')}</p>
            <div>
              <p className="field-label">{t('prof.avatar.shape')}</p>
              <div className="flex flex-wrap gap-2">
                {SHAPES.map((shape) => (
                  <button
                    key={shape}
                    type="button"
                    onClick={() => setAvatar((current) => ({ ...current, shape }))}
                    aria-pressed={avatar.shape === shape}
                    title={t(`avatar.shape.${shape}` as DictKey)}
                    className={cn(
                      'rounded-xl border p-1.5 transition',
                      avatar.shape === shape
                        ? 'border-mint-400/40 bg-mint-400/[0.07]'
                        : 'border-white/[0.07] hover:border-white/[0.16]',
                    )}
                  >
                    <AvatarFigure
                      avatar={{ ...avatar, shape }}
                      state="idle"
                      size={44}
                      decorative
                    />
                  </button>
                ))}
              </div>
            </div>

            <div className="grid gap-4 sm:grid-cols-2">
              <ColorPicker
                label={t('prof.avatar.color')}
                value={avatar.color}
                onChange={(color) => setAvatar((current) => ({ ...current, color }))}
              />
              <ColorPicker
                label={t('prof.avatar.eyeColor')}
                value={avatar.eye_color}
                onChange={(eyeColor) => setAvatar((current) => ({ ...current, eye_color: eyeColor }))}
              />
            </div>

            <Field label={`${t('prof.avatar.tilt')} · ${Math.round(avatar.eye_tilt_deg)}°`} htmlFor="avatar-tilt">
              <input
                id="avatar-tilt"
                type="range"
                min="-45"
                max="45"
                step="1"
                value={avatar.eye_tilt_deg}
                onChange={(event) =>
                  setAvatar((current) => ({ ...current, eye_tilt_deg: Number(event.target.value) }))
                }
                className="h-11 w-full cursor-pointer accent-[#6ee7ff]"
              />
            </Field>

            <div>
              <p className="field-label">{t('prof.avatar.state')}</p>
              <div className="flex flex-wrap gap-1.5">
                {PREVIEW_STATES.map((state) => (
                  <button
                    key={state}
                    type="button"
                    onClick={() => setPreviewState(state)}
                    aria-pressed={previewState === state}
                    className={cn(
                      'min-h-[32px] rounded-full border px-2.5 text-[11px] transition',
                      previewState === state
                        ? 'border-mint-400/35 bg-mint-400/[0.09] text-mint-200'
                        : 'border-white/[0.08] text-ink-400 hover:text-ink-200',
                    )}
                  >
                    {t(`avatar.state.${state}` as DictKey)}
                  </button>
                ))}
              </div>
            </div>
          </div>

          <div className="space-y-3 rounded-2xl border border-white/[0.06] bg-white/[0.02] p-4">
            <div className="flex items-center justify-between gap-3">
              <div>
                <p className="text-[13px] font-medium text-ink-100">{t('prof.permissions')}</p>
                <p className="text-[11px] text-ink-500">{t('prof.permissionsHint')}</p>
              </div>
              <span className="text-[11px] tabular text-ink-500">
                {allowedTools.length}/{toolCount}
              </span>
            </div>
            {tools.length === 0 ? (
              <p className="text-xs text-ink-500">{t('agent.tools.empty')}</p>
            ) : (
              <div className="grid gap-2 sm:grid-cols-2">
                {tools.map((tool) => (
                  <label
                    key={tool.name}
                    className={cn(
                      'flex cursor-pointer gap-2.5 rounded-xl border px-3 py-2.5 transition',
                      allowedTools.includes(tool.name)
                        ? 'border-mint-400/25 bg-mint-400/[0.05]'
                        : 'border-white/[0.06] bg-white/[0.015] hover:border-white/[0.12]',
                    )}
                  >
                    <input
                      type="checkbox"
                      checked={allowedTools.includes(tool.name)}
                      onChange={(event) => toggleTool(tool.name, event.target.checked)}
                      className="mt-0.5 h-4 w-4 shrink-0 accent-[#34d399]"
                    />
                    <span className="min-w-0">
                      <span className="block truncate font-mono text-[11.5px] text-ink-100">{tool.name}</span>
                      <span className="mt-0.5 line-clamp-2 block text-[10.5px] leading-relaxed text-ink-500">
                        {tool.description}
                      </span>
                    </span>
                  </label>
                ))}
              </div>
            )}
          </div>

          {profile !== null ? (
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-white/[0.06] bg-white/[0.02] p-4">
              <div className="flex items-start gap-3">
                <ScanFace size={18} className={faceEnrolled ? 'mt-0.5 text-mint-400' : 'mt-0.5 text-ink-500'} />
                <div className="min-w-0">
                  <p className="text-[13px] font-medium text-ink-100">{t('prof.face.title')}</p>
                  <p className="mt-0.5 max-w-md text-[11px] leading-relaxed text-ink-500">{t('prof.face.note')}</p>
                </div>
              </div>
              <div className="flex items-center gap-2">
                <StatusPill
                  label={faceEnrolled ? t('prof.face.enrolled') : t('prof.face.notEnrolled')}
                  tone={faceEnrolled ? 'success' : 'neutral'}
                />
                {faceEnrolled ? (
                  <button
                    type="button"
                    className="btn btn-danger btn-sm"
                    onClick={() => void removeFace()}
                    disabled={faceBusy}
                  >
                    {faceBusy ? <Spinner size={13} /> : <Trash2 size={13} />}
                    {t('prof.face.delete')}
                  </button>
                ) : null}
              </div>
            </div>
          ) : null}
        </div>

        {/* Preview --------------------------------------------------------- */}
        <aside className="flex flex-col items-center gap-4 rounded-2xl border border-white/[0.06] bg-white/[0.02] p-5">
          <p className="section-label">{t('prof.avatar.preview')}</p>
          <AvatarFigure avatar={avatar} state={previewState} size={190} />
          <div className="w-full space-y-2 text-center">
            <p className="font-display text-[16px] font-medium text-ink-50">{name === '' ? '—' : name}</p>
            <div className="flex flex-wrap justify-center gap-1.5">
              <StatusPill label={t(ROLE_KEYS[role])} tone="accent" />
              <StatusPill label={t(TONE_KEYS[tone])} tone="neutral" />
            </div>
            <p className="text-[11px] text-ink-500">
              {t('prof.toolsCount', { count: allowedTools.length })}
            </p>
          </div>
          {tools.length > 0 ? (
            <Toggle
              checked={allowedTools.length === tools.length}
              onChange={(all) =>
                setAllowedTools(all ? tools.map((tool) => tool.name) : [])
              }
              label={t('common.all')}
              className="w-full"
            />
          ) : null}
        </aside>
      </div>
    </Modal>
  );
}

function ColorPicker({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}): React.JSX.Element {
  return (
    <Field label={label}>
      <div className="flex items-center gap-2">
        <input
          type="color"
          value={isValidHexColor(value) ? value : '#6ee7ff'}
          onChange={(event) => onChange(event.target.value)}
          aria-label={label}
          className="h-11 w-12 cursor-pointer rounded-lg border border-white/[0.09] bg-transparent p-1"
        />
        <input
          className="input input-mono"
          value={value}
          onChange={(event) => onChange(event.target.value)}
          spellCheck={false}
          aria-label={label}
        />
      </div>
      <div className="mt-2 flex flex-wrap gap-1.5">
        {PROFILE_COLORS.map((color) => (
          <button
            key={color}
            type="button"
            onClick={() => onChange(color)}
            aria-label={color}
            aria-pressed={value === color}
            className={cn(
              'h-6 w-6 rounded-full border-2 transition',
              value.toLowerCase() === color.toLowerCase()
                ? 'border-white/80'
                : 'border-transparent hover:border-white/30',
            )}
            style={{ backgroundColor: color }}
          />
        ))}
      </div>
    </Field>
  );
}
